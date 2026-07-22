use crate::{CompositorGpuHint, WgpuAtlas, WgpuContext};
use bytemuck::{Pod, Zeroable};
use gpui::{
    AtlasTextureId, Background, Bounds, ColorRange, DevicePixels, EffectQuad, EffectShader,
    GpuSpecs, MonochromeSprite, Path, Point, PolychromeSprite, PrimitiveBatch, Quad, ScaledPixels,
    Scene, Shadow, Size, SubpixelSprite, SurfaceColorInfo, SurfaceFormat, SurfaceFrame, SurfaceId,
    Underline, WeakSurfaceHandle, YuvMatrix, get_gamma_correction_ratios,
};
#[cfg(target_os = "linux")]
use gpui::{
    DRM_FORMAT_NV12, DmaBufHandle, DmaBufId, DmaBufImage, DmaBufPlane, DrmDevice,
    SurfaceFrameBacking, WeakDmaBufHandle,
};
use log::warn;
#[cfg(not(target_family = "wasm"))]
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::num::NonZeroU64;
#[cfg(target_os = "linux")]
use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd, OwnedFd};
use std::rc::Rc;
use std::sync::{Arc, Mutex};

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GlobalParams {
    viewport_size: [f32; 2],
    premultiplied_alpha: u32,
    pad: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct PodBounds {
    origin: [f32; 2],
    size: [f32; 2],
}

impl From<Bounds<ScaledPixels>> for PodBounds {
    fn from(bounds: Bounds<ScaledPixels>) -> Self {
        Self {
            origin: [bounds.origin.x.0, bounds.origin.y.0],
            size: [bounds.size.width.0, bounds.size.height.0],
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(super) struct SurfaceParams {
    bounds: PodBounds,
    clip_bounds: PodBounds,
    content_mask: PodBounds,
    uv_bounds: PodBounds,
    corner_radii: [f32; 4],
    color_rows: [[f32; 4]; 3],
    opacity: f32,
    _pad: [f32; 3],
}

pub(super) fn yuv_to_rgb_rows(color: SurfaceColorInfo) -> [[f32; 4]; 3] {
    let (y_scale, y_offset, chroma_center, r_cr, g_cb, g_cr, b_cb) =
        match (color.matrix, color.range) {
            (YuvMatrix::Bt601, ColorRange::Limited) => (
                255.0 / 219.0,
                16.0 / 255.0,
                128.0 / 255.0,
                1.596_027,
                -0.391_762,
                -0.812_968,
                2.017_232,
            ),
            (YuvMatrix::Bt709, ColorRange::Limited) => (
                255.0 / 219.0,
                16.0 / 255.0,
                128.0 / 255.0,
                1.792_741,
                -0.213_249,
                -0.532_909,
                2.112_402,
            ),
            (YuvMatrix::Bt601, ColorRange::Full) => (
                1.0,
                0.0,
                128.0 / 255.0,
                1.402,
                -0.344_136,
                -0.714_136,
                1.772,
            ),
            (YuvMatrix::Bt709, ColorRange::Full) => (
                1.0,
                0.0,
                128.0 / 255.0,
                1.5748,
                -0.187_324,
                -0.468_124,
                1.8556,
            ),
        };

    [
        [
            y_scale,
            0.0,
            r_cr,
            -y_scale * y_offset - r_cr * chroma_center,
        ],
        [
            y_scale,
            g_cb,
            g_cr,
            -y_scale * y_offset - (g_cb + g_cr) * chroma_center,
        ],
        [
            y_scale,
            b_cb,
            0.0,
            -y_scale * y_offset - b_cb * chroma_center,
        ],
    ]
}

enum CachedSurfaceTextures {
    Rgba {
        _texture: wgpu::Texture,
        view: wgpu::TextureView,
    },
    Nv12 {
        _y_texture: wgpu::Texture,
        y_view: wgpu::TextureView,
        _uv_texture: wgpu::Texture,
        uv_view: wgpu::TextureView,
    },
}

struct CachedSurface {
    sequence: u64,
    format: SurfaceFormat,
    size: Size<DevicePixels>,
    textures: CachedSurfaceTextures,
    owner: WeakSurfaceHandle,
}

#[cfg(target_os = "linux")]
struct CachedDmaBuf {
    format: SurfaceFormat,
    size: Size<DevicePixels>,
    textures: CachedSurfaceTextures,
    owner: WeakDmaBufHandle,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SurfaceCacheAction {
    Create,
    Recreate,
    Upload,
    Reuse,
}

pub(super) fn surface_cache_action(
    cached: Option<(u64, SurfaceFormat, Size<DevicePixels>)>,
    frame: &SurfaceFrame,
) -> SurfaceCacheAction {
    let Some((sequence, format, size)) = cached else {
        return SurfaceCacheAction::Create;
    };
    if format != frame.format() || size != frame.coded_size() {
        SurfaceCacheAction::Recreate
    } else if sequence != frame.sequence() {
        SurfaceCacheAction::Upload
    } else {
        SurfaceCacheAction::Reuse
    }
}

pub(super) fn surface_uv_bounds(frame: &SurfaceFrame) -> ([f32; 2], [f32; 2]) {
    let coded_size = frame.coded_size();
    let visible_rect = frame.visible_rect();
    (
        [
            visible_rect.origin.x.0 as f32 / coded_size.width.0 as f32,
            visible_rect.origin.y.0 as f32 / coded_size.height.0 as f32,
        ],
        [
            visible_rect.size.width.0 as f32 / coded_size.width.0 as f32,
            visible_rect.size.height.0 as f32 / coded_size.height.0 as f32,
        ],
    )
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GammaParams {
    gamma_ratios: [f32; 4],
    grayscale_enhanced_contrast: f32,
    subpixel_enhanced_contrast: f32,
    is_bgr: u32,
    _pad: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct PodTransformationMatrix {
    rotation_scale: [[f32; 2]; 2],
    translation: [f32; 2],
}

impl From<gpui::TransformationMatrix> for PodTransformationMatrix {
    fn from(value: gpui::TransformationMatrix) -> Self {
        Self {
            rotation_scale: value.rotation_scale,
            translation: value.translation,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(super) struct EffectInstance {
    bounds: PodBounds,
    effect_bounds: PodBounds,
    transformation: PodTransformationMatrix,
    content_mask: PodBounds,
    corner_radii: [f32; 4],
    image_bounds: PodBounds,
    second_image_bounds: PodBounds,
    third_image_bounds: PodBounds,
    fourth_image_bounds: PodBounds,
    opacity: f32,
    time: f32,
    pad: [f32; 2],
    alignment_pad: [f32; 2],
    uniforms: [[f32; 4]; gpui::EFFECT_UNIFORM_SLOTS],
}

impl From<&EffectQuad> for EffectInstance {
    fn from(effect: &EffectQuad) -> Self {
        Self {
            bounds: effect.bounds.into(),
            effect_bounds: effect.effect_bounds.into(),
            transformation: effect.transformation.into(),
            content_mask: effect.content_mask.bounds.into(),
            corner_radii: [
                effect.corner_radii.top_left.0,
                effect.corner_radii.top_right.0,
                effect.corner_radii.bottom_right.0,
                effect.corner_radii.bottom_left.0,
            ],
            image_bounds: effect
                .image_tile
                .map(|tile| tile.bounds.map(|value| ScaledPixels(value.0 as f32)).into())
                .unwrap_or_else(|| Bounds::<ScaledPixels>::default().into()),
            second_image_bounds: effect
                .second_image_tile
                .map(|tile| tile.bounds.map(|value| ScaledPixels(value.0 as f32)).into())
                .unwrap_or_else(|| Bounds::<ScaledPixels>::default().into()),
            third_image_bounds: effect
                .third_image_tile
                .map(|tile| tile.bounds.map(|value| ScaledPixels(value.0 as f32)).into())
                .unwrap_or_else(|| Bounds::<ScaledPixels>::default().into()),
            fourth_image_bounds: effect
                .fourth_image_tile
                .map(|tile| tile.bounds.map(|value| ScaledPixels(value.0 as f32)).into())
                .unwrap_or_else(|| Bounds::<ScaledPixels>::default().into()),
            opacity: effect.opacity,
            time: effect.time,
            pad: [0.0; 2],
            alignment_pad: [0.0; 2],
            uniforms: *effect.uniforms.slots(),
        }
    }
}

#[derive(Clone, Debug)]
#[repr(C)]
struct PathSprite {
    bounds: Bounds<ScaledPixels>,
}

#[derive(Clone, Debug)]
#[repr(C)]
pub(super) struct PathRasterizationVertex {
    xy_position: Point<ScaledPixels>,
    st_position: Point<f32>,
    color: Background,
    bounds: Bounds<ScaledPixels>,
}

pub struct WgpuSurfaceConfig {
    pub size: Size<DevicePixels>,
    pub transparent: bool,
    /// Preferred presentation mode. When `Some`, the renderer will use this
    /// mode if supported by the surface, falling back to `Fifo`.
    /// When `None`, defaults to `Fifo` (VSync).
    ///
    /// Mobile platforms may prefer `Mailbox` (triple-buffering) to avoid
    /// blocking in `get_current_texture()` during lifecycle transitions.
    pub preferred_present_mode: Option<wgpu::PresentMode>,
}

struct WgpuPipelines {
    quads: wgpu::RenderPipeline,
    shadows: wgpu::RenderPipeline,
    path_rasterization: wgpu::RenderPipeline,
    paths: wgpu::RenderPipeline,
    underlines: wgpu::RenderPipeline,
    mono_sprites: wgpu::RenderPipeline,
    subpixel_sprites: Option<wgpu::RenderPipeline>,
    poly_sprites: wgpu::RenderPipeline,
    surfaces_rgba: wgpu::RenderPipeline,
    surfaces_nv12: wgpu::RenderPipeline,
}

struct WgpuBindGroupLayouts {
    globals: wgpu::BindGroupLayout,
    instances: wgpu::BindGroupLayout,
    instances_with_texture: wgpu::BindGroupLayout,
    instances_with_two_textures: wgpu::BindGroupLayout,
    instances_with_four_textures: wgpu::BindGroupLayout,
    surfaces: wgpu::BindGroupLayout,
}

/// Shared GPU context reference, used to coordinate device recovery across multiple windows.
pub type GpuContext = Rc<RefCell<Option<WgpuContext>>>;

/// GPU resources that must be dropped together during device recovery.
struct WgpuResources {
    instance: wgpu::Instance,
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
    surface: wgpu::Surface<'static>,
    pipelines: WgpuPipelines,
    effect_pipelines: HashMap<u64, wgpu::RenderPipeline>,
    failed_effect_pipelines: HashSet<u64>,
    bind_group_layouts: WgpuBindGroupLayouts,
    atlas_sampler: wgpu::Sampler,
    globals_buffer: wgpu::Buffer,
    globals_bind_group: wgpu::BindGroup,
    path_globals_bind_group: wgpu::BindGroup,
    instance_buffer: wgpu::Buffer,
    path_intermediate_texture: Option<wgpu::Texture>,
    path_intermediate_view: Option<wgpu::TextureView>,
    path_msaa_texture: Option<wgpu::Texture>,
    path_msaa_view: Option<wgpu::TextureView>,
    surfaces: HashMap<SurfaceId, CachedSurface>,
    #[cfg(target_os = "linux")]
    dma_bufs: HashMap<DmaBufId, CachedDmaBuf>,
    #[cfg(target_os = "linux")]
    failed_dma_bufs: HashMap<DmaBufId, WeakDmaBufHandle>,
    #[cfg(target_os = "linux")]
    drm_render_device: Option<DrmDevice>,
    #[cfg(target_os = "linux")]
    native_nv12_dma_buf_modifiers: Vec<gpui::DmaBufModifier>,
}

impl WgpuResources {
    fn invalidate_intermediate_textures(&mut self) {
        self.path_intermediate_texture = None;
        self.path_intermediate_view = None;
        self.path_msaa_texture = None;
        self.path_msaa_view = None;
    }
}

pub struct WgpuRenderer {
    /// Shared GPU context for device recovery coordination (unused on WASM).
    #[allow(dead_code)]
    context: Option<GpuContext>,
    /// Compositor GPU hint for adapter selection (unused on WASM).
    #[allow(dead_code)]
    compositor_gpu: Option<CompositorGpuHint>,
    resources: Option<WgpuResources>,
    surface_config: wgpu::SurfaceConfiguration,
    atlas: Arc<WgpuAtlas>,
    path_globals_offset: u64,
    gamma_offset: u64,
    instance_buffer_capacity: u64,
    max_buffer_size: u64,
    storage_buffer_alignment: u64,
    rendering_params: RenderingParameters,
    is_bgr: bool,
    dual_source_blending: bool,
    dma_buf_import: bool,
    adapter_info: wgpu::AdapterInfo,
    transparent_alpha_mode: wgpu::CompositeAlphaMode,
    opaque_alpha_mode: wgpu::CompositeAlphaMode,
    max_texture_size: u32,
    last_error: Arc<Mutex<Option<String>>>,
    failed_frame_count: u32,
    device_lost: std::sync::Arc<std::sync::atomic::AtomicBool>,
    surface_configured: bool,
    needs_redraw: bool,
}

impl WgpuRenderer {
    fn resources(&self) -> &WgpuResources {
        self.resources
            .as_ref()
            .expect("GPU resources not available")
    }

    fn resources_mut(&mut self) -> &mut WgpuResources {
        self.resources
            .as_mut()
            .expect("GPU resources not available")
    }

    /// Creates a new WgpuRenderer from raw window handles.
    ///
    /// The `gpu_context` is a shared reference that coordinates GPU context across
    /// multiple windows. The first window to create a renderer will initialize the
    /// context; subsequent windows will share it.
    ///
    /// # Safety
    /// The caller must ensure that the window handle remains valid for the lifetime
    /// of the returned renderer.
    #[cfg(not(target_family = "wasm"))]
    pub fn new<W>(
        gpu_context: GpuContext,
        window: &W,
        config: WgpuSurfaceConfig,
        compositor_gpu: Option<CompositorGpuHint>,
    ) -> anyhow::Result<Self>
    where
        W: HasWindowHandle + HasDisplayHandle + std::fmt::Debug + Send + Sync + Clone + 'static,
    {
        let window_handle = window
            .window_handle()
            .map_err(|e| anyhow::anyhow!("Failed to get window handle: {e}"))?;

        let target = wgpu::SurfaceTargetUnsafe::RawHandle {
            // Fall back to the display handle already provided via InstanceDescriptor::display.
            raw_display_handle: None,
            raw_window_handle: window_handle.as_raw(),
        };

        // Use the existing context's instance if available, otherwise create a new one.
        // The surface must be created with the same instance that will be used for
        // adapter selection, otherwise wgpu will panic.
        let instance = gpu_context
            .borrow()
            .as_ref()
            .map(|ctx| ctx.instance.clone())
            .unwrap_or_else(|| WgpuContext::instance(Box::new(window.clone())));

        // Safety: The caller guarantees that the window handle is valid for the
        // lifetime of this renderer. In practice, the RawWindow struct is created
        // from the native window handles and the surface is dropped before the window.
        let surface = unsafe {
            instance
                .create_surface_unsafe(target)
                .map_err(|e| anyhow::anyhow!("Failed to create surface: {e}"))?
        };

        let mut ctx_ref = gpu_context.borrow_mut();
        let context = match ctx_ref.as_mut() {
            Some(context) => {
                context.check_compatible_with_surface(&surface)?;
                context
            }
            None => ctx_ref.insert(WgpuContext::new(instance, &surface, compositor_gpu)?),
        };

        let atlas = Arc::new(WgpuAtlas::from_context(context));

        Self::new_internal(
            Some(Rc::clone(&gpu_context)),
            context,
            surface,
            config,
            compositor_gpu,
            atlas,
        )
    }

    #[cfg(target_family = "wasm")]
    pub fn new_from_canvas(
        context: &WgpuContext,
        canvas: &web_sys::HtmlCanvasElement,
        config: WgpuSurfaceConfig,
    ) -> anyhow::Result<Self> {
        let surface = context
            .instance
            .create_surface(wgpu::SurfaceTarget::Canvas(canvas.clone()))
            .map_err(|e| anyhow::anyhow!("Failed to create surface: {e}"))?;

        let atlas = Arc::new(WgpuAtlas::from_context(context));

        Self::new_internal(None, context, surface, config, None, atlas)
    }

    fn new_internal(
        gpu_context: Option<GpuContext>,
        context: &WgpuContext,
        surface: wgpu::Surface<'static>,
        config: WgpuSurfaceConfig,
        compositor_gpu: Option<CompositorGpuHint>,
        atlas: Arc<WgpuAtlas>,
    ) -> anyhow::Result<Self> {
        let surface_caps = surface.get_capabilities(&context.adapter);
        let preferred_formats = [
            wgpu::TextureFormat::Bgra8Unorm,
            wgpu::TextureFormat::Rgba8Unorm,
        ];
        let surface_format = preferred_formats
            .iter()
            .find(|f| surface_caps.formats.contains(f))
            .copied()
            .or_else(|| surface_caps.formats.iter().find(|f| !f.is_srgb()).copied())
            .or_else(|| surface_caps.formats.first().copied())
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Surface reports no supported texture formats for adapter {:?}",
                    context.adapter.get_info().name
                )
            })?;

        let pick_alpha_mode =
            |preferences: &[wgpu::CompositeAlphaMode]| -> anyhow::Result<wgpu::CompositeAlphaMode> {
                preferences
                    .iter()
                    .find(|p| surface_caps.alpha_modes.contains(p))
                    .copied()
                    .or_else(|| surface_caps.alpha_modes.first().copied())
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "Surface reports no supported alpha modes for adapter {:?}",
                            context.adapter.get_info().name
                        )
                    })
            };

        let transparent_alpha_mode = pick_alpha_mode(&[
            wgpu::CompositeAlphaMode::PreMultiplied,
            wgpu::CompositeAlphaMode::Inherit,
        ])?;

        let opaque_alpha_mode = pick_alpha_mode(&[
            wgpu::CompositeAlphaMode::Opaque,
            wgpu::CompositeAlphaMode::Inherit,
        ])?;

        let alpha_mode = if config.transparent {
            transparent_alpha_mode
        } else {
            opaque_alpha_mode
        };

        let device = Arc::clone(&context.device);
        let max_texture_size = device.limits().max_texture_dimension_2d;

        let requested_width = config.size.width.0 as u32;
        let requested_height = config.size.height.0 as u32;
        let clamped_width = requested_width.min(max_texture_size);
        let clamped_height = requested_height.min(max_texture_size);

        if clamped_width != requested_width || clamped_height != requested_height {
            warn!(
                "Requested surface size ({}, {}) exceeds maximum texture dimension {}. \
                 Clamping to ({}, {}). Window content may not fill the entire window.",
                requested_width, requested_height, max_texture_size, clamped_width, clamped_height
            );
        }

        let surface_config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width: clamped_width.max(1),
            height: clamped_height.max(1),
            present_mode: config
                .preferred_present_mode
                .filter(|mode| surface_caps.present_modes.contains(mode))
                .unwrap_or(wgpu::PresentMode::Fifo),
            desired_maximum_frame_latency: 2,
            alpha_mode,
            view_formats: vec![],
            color_space: wgpu::SurfaceColorSpace::Auto,
        };
        // Configure the surface immediately. The adapter selection process already validated
        // that this adapter can successfully configure this surface.
        surface.configure(&context.device, &surface_config);

        let queue = Arc::clone(&context.queue);
        let dual_source_blending = context.supports_dual_source_blending();
        let dma_buf_import = context.supports_dma_buf_import();

        let rendering_params = RenderingParameters::new(&context.adapter, surface_format);
        let bind_group_layouts = Self::create_bind_group_layouts(&device);
        let pipelines = Self::create_pipelines(
            &device,
            &bind_group_layouts,
            surface_format,
            alpha_mode,
            rendering_params.path_sample_count,
            dual_source_blending,
        );

        let atlas_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("atlas_sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let uniform_alignment = device.limits().min_uniform_buffer_offset_alignment as u64;
        let globals_size = std::mem::size_of::<GlobalParams>() as u64;
        let gamma_size = std::mem::size_of::<GammaParams>() as u64;
        let path_globals_offset = globals_size.next_multiple_of(uniform_alignment);
        let gamma_offset = (path_globals_offset + globals_size).next_multiple_of(uniform_alignment);

        let globals_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("globals_buffer"),
            size: gamma_offset + gamma_size,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let max_buffer_size = device.limits().max_buffer_size;
        let storage_buffer_alignment = device.limits().min_storage_buffer_offset_alignment as u64;
        let initial_instance_buffer_capacity = 2 * 1024 * 1024;
        let instance_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("instance_buffer"),
            size: initial_instance_buffer_capacity,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let globals_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("globals_bind_group"),
            layout: &bind_group_layouts.globals,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &globals_buffer,
                        offset: 0,
                        size: Some(NonZeroU64::new(globals_size).unwrap()),
                    }),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &globals_buffer,
                        offset: gamma_offset,
                        size: Some(NonZeroU64::new(gamma_size).unwrap()),
                    }),
                },
            ],
        });

        let path_globals_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("path_globals_bind_group"),
            layout: &bind_group_layouts.globals,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &globals_buffer,
                        offset: path_globals_offset,
                        size: Some(NonZeroU64::new(globals_size).unwrap()),
                    }),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &globals_buffer,
                        offset: gamma_offset,
                        size: Some(NonZeroU64::new(gamma_size).unwrap()),
                    }),
                },
            ],
        });

        let adapter_info = context.adapter.get_info();

        let last_error: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let last_error_clone = Arc::clone(&last_error);
        device.on_uncaptured_error(Arc::new(move |error| {
            let mut guard = last_error_clone.lock().unwrap();
            *guard = Some(error.to_string());
        }));

        let resources = WgpuResources {
            instance: context.instance.clone(),
            device,
            queue,
            surface,
            pipelines,
            effect_pipelines: HashMap::default(),
            failed_effect_pipelines: HashSet::default(),
            bind_group_layouts,
            atlas_sampler,
            globals_buffer,
            globals_bind_group,
            path_globals_bind_group,
            instance_buffer,
            // Defer intermediate texture creation to first draw call via ensure_intermediate_textures().
            // This avoids panics when the device/surface is in an invalid state during initialization.
            path_intermediate_texture: None,
            path_intermediate_view: None,
            path_msaa_texture: None,
            path_msaa_view: None,
            surfaces: HashMap::default(),
            #[cfg(target_os = "linux")]
            dma_bufs: HashMap::default(),
            #[cfg(target_os = "linux")]
            failed_dma_bufs: HashMap::default(),
            #[cfg(target_os = "linux")]
            drm_render_device: context.drm_render_device(),
            #[cfg(target_os = "linux")]
            native_nv12_dma_buf_modifiers: context.native_nv12_dma_buf_modifiers(),
        };

        Ok(Self {
            context: gpu_context,
            compositor_gpu,
            resources: Some(resources),
            surface_config,
            atlas,
            path_globals_offset,
            gamma_offset,
            instance_buffer_capacity: initial_instance_buffer_capacity,
            max_buffer_size,
            storage_buffer_alignment,
            rendering_params,
            is_bgr: false,
            dual_source_blending,
            dma_buf_import,
            adapter_info,
            transparent_alpha_mode,
            opaque_alpha_mode,
            max_texture_size,
            last_error,
            failed_frame_count: 0,
            device_lost: context.device_lost_flag(),
            surface_configured: true,
            needs_redraw: false,
        })
    }

    fn create_bind_group_layouts(device: &wgpu::Device) -> WgpuBindGroupLayouts {
        let globals =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("globals_layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: NonZeroU64::new(
                                std::mem::size_of::<GlobalParams>() as u64
                            ),
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: NonZeroU64::new(
                                std::mem::size_of::<GammaParams>() as u64
                            ),
                        },
                        count: None,
                    },
                ],
            });

        let storage_buffer_entry = |binding: u32| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };

        let instances = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("instances_layout"),
            entries: &[storage_buffer_entry(0)],
        });

        let instances_with_texture =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("instances_with_texture_layout"),
                entries: &[
                    storage_buffer_entry(0),
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            });

        let instances_with_two_textures =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("instances_with_two_textures_layout"),
                entries: &[
                    storage_buffer_entry(0),
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 3,
                        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                ],
            });

        let instances_with_four_textures =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("instances_with_four_textures_layout"),
                entries: &[
                    storage_buffer_entry(0),
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 3,
                        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 4,
                        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 5,
                        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                ],
            });

        let surfaces = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("surfaces_layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: NonZeroU64::new(
                            std::mem::size_of::<SurfaceParams>() as u64
                        ),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        WgpuBindGroupLayouts {
            globals,
            instances,
            instances_with_texture,
            instances_with_two_textures,
            instances_with_four_textures,
            surfaces,
        }
    }

    fn create_effect_pipeline(
        device: &wgpu::Device,
        layouts: &WgpuBindGroupLayouts,
        surface_format: wgpu::TextureFormat,
        alpha_mode: wgpu::CompositeAlphaMode,
        shader: &EffectShader,
    ) -> anyhow::Result<wgpu::RenderPipeline> {
        let source = gpui::compose_effect_shader_wgsl(shader);
        let module = wgpu::naga::front::wgsl::parse_str(&source)
            .map_err(|error| anyhow::anyhow!("WGSL parse error: {error}"))?;
        wgpu::naga::valid::Validator::new(
            wgpu::naga::valid::ValidationFlags::all(),
            wgpu::naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .map_err(|error| anyhow::anyhow!("WGSL validation error: {error}"))?;

        let shader_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("gpui_effect_shader"),
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Owned(source)),
        });
        let effect_instances_layout = match shader.image_count() {
            0 => &layouts.instances,
            1 => &layouts.instances_with_texture,
            2 => &layouts.instances_with_two_textures,
            _ => &layouts.instances_with_four_textures,
        };
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("gpui_effect_pipeline_layout"),
            bind_group_layouts: &[Some(&layouts.globals), Some(effect_instances_layout)],
            immediate_size: 0,
        });
        let blend = match alpha_mode {
            wgpu::CompositeAlphaMode::PreMultiplied => {
                wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING
            }
            _ => wgpu::BlendState::ALPHA_BLENDING,
        };

        Ok(
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("gpui_effect_pipeline"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader_module,
                    entry_point: Some("vs_effect"),
                    buffers: &[],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader_module,
                    entry_point: Some("fs_effect"),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: surface_format,
                        blend: Some(blend),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleStrip,
                    strip_index_format: None,
                    front_face: wgpu::FrontFace::Ccw,
                    cull_mode: None,
                    polygon_mode: wgpu::PolygonMode::Fill,
                    unclipped_depth: false,
                    conservative: false,
                },
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            }),
        )
    }

    fn ensure_effect_pipelines(&mut self, scene: &Scene) {
        let shaders = scene
            .effects
            .iter()
            .map(|effect| effect.shader.clone())
            .collect::<Vec<_>>();

        for shader in shaders {
            let key = shader.id().as_u64();
            if self.resources().effect_pipelines.contains_key(&key)
                || self.resources().failed_effect_pipelines.contains(&key)
            {
                continue;
            }

            let result = Self::create_effect_pipeline(
                &self.resources().device,
                &self.resources().bind_group_layouts,
                self.surface_config.format,
                self.surface_config.alpha_mode,
                &shader,
            );
            match result {
                Ok(pipeline) => {
                    self.resources_mut().effect_pipelines.insert(key, pipeline);
                }
                Err(error) => {
                    log::error!("failed to compile GPUI effect {key:016x}: {error:#}");
                    self.resources_mut().failed_effect_pipelines.insert(key);
                }
            }
        }
    }

    fn create_pipelines(
        device: &wgpu::Device,
        layouts: &WgpuBindGroupLayouts,
        surface_format: wgpu::TextureFormat,
        alpha_mode: wgpu::CompositeAlphaMode,
        path_sample_count: u32,
        dual_source_blending: bool,
    ) -> WgpuPipelines {
        // Diagnostic guard: verify the device actually has
        // DUAL_SOURCE_BLENDING. We have a crash report (ZED-5G1) where a
        // feature mismatch caused a wgpu-hal abort, but we haven't
        // identified the code path that produces the mismatch. This
        // guard prevents the crash and logs more evidence.
        // Remove this check once:
        // a) We find and fix the root cause, or
        // b) There are no reports of this warning appearing for some time.
        let device_has_feature = device
            .features()
            .contains(wgpu::Features::DUAL_SOURCE_BLENDING);
        if dual_source_blending && !device_has_feature {
            log::error!(
                "BUG: dual_source_blending flag is true but device does not \
                 have DUAL_SOURCE_BLENDING enabled (device features: {:?}). \
                 Falling back to mono text rendering. Please report this at \
                 https://github.com/zed-industries/zed/issues",
                device.features(),
            );
        }
        let dual_source_blending = dual_source_blending && device_has_feature;

        let base_shader_source = include_str!("shaders.wgsl");
        let shader_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("gpui_shaders"),
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(base_shader_source)),
        });

        let subpixel_shader_source = include_str!("shaders_subpixel.wgsl");
        let subpixel_shader_module = if dual_source_blending {
            let combined = format!(
                "enable dual_source_blending;\n{base_shader_source}\n{subpixel_shader_source}"
            );
            Some(device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("gpui_subpixel_shaders"),
                source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Owned(combined)),
            }))
        } else {
            None
        };

        let blend_mode = match alpha_mode {
            wgpu::CompositeAlphaMode::PreMultiplied => {
                wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING
            }
            _ => wgpu::BlendState::ALPHA_BLENDING,
        };

        let color_target = wgpu::ColorTargetState {
            format: surface_format,
            blend: Some(blend_mode),
            write_mask: wgpu::ColorWrites::ALL,
        };

        let create_pipeline = |name: &str,
                               vs_entry: &str,
                               fs_entry: &str,
                               globals_layout: &wgpu::BindGroupLayout,
                               data_layout: &wgpu::BindGroupLayout,
                               topology: wgpu::PrimitiveTopology,
                               color_targets: &[Option<wgpu::ColorTargetState>],
                               sample_count: u32,
                               module: &wgpu::ShaderModule| {
            let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(&format!("{name}_layout")),
                bind_group_layouts: &[Some(globals_layout), Some(data_layout)],
                immediate_size: 0,
            });

            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(name),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module,
                    entry_point: Some(vs_entry),
                    buffers: &[],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module,
                    entry_point: Some(fs_entry),
                    targets: color_targets,
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                }),
                primitive: wgpu::PrimitiveState {
                    topology,
                    strip_index_format: None,
                    front_face: wgpu::FrontFace::Ccw,
                    cull_mode: None,
                    polygon_mode: wgpu::PolygonMode::Fill,
                    unclipped_depth: false,
                    conservative: false,
                },
                depth_stencil: None,
                multisample: wgpu::MultisampleState {
                    count: sample_count,
                    mask: !0,
                    alpha_to_coverage_enabled: false,
                },
                multiview_mask: None,
                cache: None,
            })
        };

        let quads = create_pipeline(
            "quads",
            "vs_quad",
            "fs_quad",
            &layouts.globals,
            &layouts.instances,
            wgpu::PrimitiveTopology::TriangleStrip,
            &[Some(color_target.clone())],
            1,
            &shader_module,
        );

        let shadows = create_pipeline(
            "shadows",
            "vs_shadow",
            "fs_shadow",
            &layouts.globals,
            &layouts.instances,
            wgpu::PrimitiveTopology::TriangleStrip,
            &[Some(color_target.clone())],
            1,
            &shader_module,
        );

        let path_rasterization = create_pipeline(
            "path_rasterization",
            "vs_path_rasterization",
            "fs_path_rasterization",
            &layouts.globals,
            &layouts.instances,
            wgpu::PrimitiveTopology::TriangleList,
            &[Some(wgpu::ColorTargetState {
                format: surface_format,
                blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
            path_sample_count,
            &shader_module,
        );

        let paths_blend = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
        };

        let paths = create_pipeline(
            "paths",
            "vs_path",
            "fs_path",
            &layouts.globals,
            &layouts.instances_with_texture,
            wgpu::PrimitiveTopology::TriangleStrip,
            &[Some(wgpu::ColorTargetState {
                format: surface_format,
                blend: Some(paths_blend),
                write_mask: wgpu::ColorWrites::ALL,
            })],
            1,
            &shader_module,
        );

        let underlines = create_pipeline(
            "underlines",
            "vs_underline",
            "fs_underline",
            &layouts.globals,
            &layouts.instances,
            wgpu::PrimitiveTopology::TriangleStrip,
            &[Some(color_target.clone())],
            1,
            &shader_module,
        );

        let mono_sprites = create_pipeline(
            "mono_sprites",
            "vs_mono_sprite",
            "fs_mono_sprite",
            &layouts.globals,
            &layouts.instances_with_texture,
            wgpu::PrimitiveTopology::TriangleStrip,
            &[Some(color_target.clone())],
            1,
            &shader_module,
        );

        let subpixel_sprites = if let Some(subpixel_module) = &subpixel_shader_module {
            let subpixel_blend = wgpu::BlendState {
                color: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::Src1,
                    dst_factor: wgpu::BlendFactor::OneMinusSrc1,
                    operation: wgpu::BlendOperation::Add,
                },
                alpha: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::One,
                    dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                    operation: wgpu::BlendOperation::Add,
                },
            };

            Some(create_pipeline(
                "subpixel_sprites",
                "vs_subpixel_sprite",
                "fs_subpixel_sprite",
                &layouts.globals,
                &layouts.instances_with_texture,
                wgpu::PrimitiveTopology::TriangleStrip,
                &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    blend: Some(subpixel_blend),
                    write_mask: wgpu::ColorWrites::COLOR,
                })],
                1,
                subpixel_module,
            ))
        } else {
            None
        };

        let poly_sprites = create_pipeline(
            "poly_sprites",
            "vs_poly_sprite",
            "fs_poly_sprite",
            &layouts.globals,
            &layouts.instances_with_texture,
            wgpu::PrimitiveTopology::TriangleStrip,
            &[Some(color_target.clone())],
            1,
            &shader_module,
        );

        let surfaces_rgba = create_pipeline(
            "surfaces_rgba",
            "vs_surface",
            "fs_surface_rgba",
            &layouts.globals,
            &layouts.surfaces,
            wgpu::PrimitiveTopology::TriangleStrip,
            &[Some(color_target.clone())],
            1,
            &shader_module,
        );

        let surfaces_nv12 = create_pipeline(
            "surfaces_nv12",
            "vs_surface",
            "fs_surface_nv12",
            &layouts.globals,
            &layouts.surfaces,
            wgpu::PrimitiveTopology::TriangleStrip,
            &[Some(color_target)],
            1,
            &shader_module,
        );

        WgpuPipelines {
            quads,
            shadows,
            path_rasterization,
            paths,
            underlines,
            mono_sprites,
            subpixel_sprites,
            poly_sprites,
            surfaces_rgba,
            surfaces_nv12,
        }
    }

    fn create_path_intermediate(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        width: u32,
        height: u32,
    ) -> (wgpu::Texture, wgpu::TextureView) {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("path_intermediate"),
            size: wgpu::Extent3d {
                width: width.max(1),
                height: height.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        (texture, view)
    }

    fn create_msaa_if_needed(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        width: u32,
        height: u32,
        sample_count: u32,
    ) -> Option<(wgpu::Texture, wgpu::TextureView)> {
        if sample_count <= 1 {
            return None;
        }
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("path_msaa"),
            size: wgpu::Extent3d {
                width: width.max(1),
                height: height.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        Some((texture, view))
    }

    pub fn update_drawable_size(&mut self, size: Size<DevicePixels>) {
        let width = size.width.0 as u32;
        let height = size.height.0 as u32;

        if width != self.surface_config.width || height != self.surface_config.height {
            let clamped_width = width.min(self.max_texture_size);
            let clamped_height = height.min(self.max_texture_size);

            if clamped_width != width || clamped_height != height {
                warn!(
                    "Requested surface size ({}, {}) exceeds maximum texture dimension {}. \
                     Clamping to ({}, {}). Window content may not fill the entire window.",
                    width, height, self.max_texture_size, clamped_width, clamped_height
                );
            }

            self.surface_config.width = clamped_width.max(1);
            self.surface_config.height = clamped_height.max(1);
            let surface_config = self.surface_config.clone();

            let Some(resources) = self.resources.as_mut() else {
                return;
            };

            // Wait for any in-flight GPU work to complete before destroying textures
            if let Err(e) = resources.device.poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: None,
            }) {
                warn!("Failed to poll device during resize: {e:?}");
            }

            // Destroy old textures before allocating new ones to avoid GPU memory spikes
            if let Some(ref texture) = resources.path_intermediate_texture {
                texture.destroy();
            }
            if let Some(ref texture) = resources.path_msaa_texture {
                texture.destroy();
            }

            resources
                .surface
                .configure(&resources.device, &surface_config);

            // Invalidate intermediate textures - they will be lazily recreated
            // in draw() after we confirm the surface is healthy. This avoids
            // panics when the device/surface is in an invalid state during resize.
            resources.invalidate_intermediate_textures();
        }
    }

    fn ensure_intermediate_textures(&mut self) {
        if self.resources().path_intermediate_texture.is_some() {
            return;
        }

        let format = self.surface_config.format;
        let width = self.surface_config.width;
        let height = self.surface_config.height;
        let path_sample_count = self.rendering_params.path_sample_count;
        let resources = self.resources_mut();

        let (t, v) = Self::create_path_intermediate(&resources.device, format, width, height);
        resources.path_intermediate_texture = Some(t);
        resources.path_intermediate_view = Some(v);

        let (path_msaa_texture, path_msaa_view) = Self::create_msaa_if_needed(
            &resources.device,
            format,
            width,
            height,
            path_sample_count,
        )
        .map(|(t, v)| (Some(t), Some(v)))
        .unwrap_or((None, None));
        resources.path_msaa_texture = path_msaa_texture;
        resources.path_msaa_view = path_msaa_view;
    }

    pub fn set_subpixel_layout(&mut self, is_bgr: bool) {
        self.is_bgr = is_bgr;
    }

    pub fn update_transparency(&mut self, transparent: bool) {
        let new_alpha_mode = if transparent {
            self.transparent_alpha_mode
        } else {
            self.opaque_alpha_mode
        };

        if new_alpha_mode != self.surface_config.alpha_mode {
            self.surface_config.alpha_mode = new_alpha_mode;
            let surface_config = self.surface_config.clone();
            let path_sample_count = self.rendering_params.path_sample_count;
            let dual_source_blending = self.dual_source_blending;
            let Some(resources) = self.resources.as_mut() else {
                return;
            };
            resources
                .surface
                .configure(&resources.device, &surface_config);
            resources.pipelines = Self::create_pipelines(
                &resources.device,
                &resources.bind_group_layouts,
                surface_config.format,
                surface_config.alpha_mode,
                path_sample_count,
                dual_source_blending,
            );
            resources.effect_pipelines.clear();
            resources.failed_effect_pipelines.clear();
        }
    }

    #[allow(dead_code)]
    pub fn viewport_size(&self) -> Size<DevicePixels> {
        Size {
            width: DevicePixels(self.surface_config.width as i32),
            height: DevicePixels(self.surface_config.height as i32),
        }
    }

    pub fn sprite_atlas(&self) -> &Arc<WgpuAtlas> {
        &self.atlas
    }

    pub fn supports_dual_source_blending(&self) -> bool {
        self.dual_source_blending
    }

    pub fn gpu_specs(&self) -> GpuSpecs {
        let resources = self.resources();
        GpuSpecs {
            is_software_emulated: self.adapter_info.device_type == wgpu::DeviceType::Cpu,
            device_name: self.adapter_info.name.clone(),
            driver_name: self.adapter_info.driver.clone(),
            driver_info: self.adapter_info.driver_info.clone(),
            supports_dma_buf_import: self.dma_buf_import,
            supports_native_nv12_dma_buf_import: self.dma_buf_import
                && resources
                    .device
                    .features()
                    .contains(wgpu::Features::TEXTURE_FORMAT_NV12),
            #[cfg(target_os = "linux")]
            native_nv12_dma_buf_modifiers: resources.native_nv12_dma_buf_modifiers.clone(),
            #[cfg(target_os = "linux")]
            drm_render_device: resources.drm_render_device,
        }
    }

    pub fn max_texture_size(&self) -> u32 {
        self.max_texture_size
    }

    pub fn draw(&mut self, scene: &Scene) -> bool {
        // Bail out early if the surface has been unconfigured (e.g. during
        // Android background/rotation transitions).  Attempting to acquire
        // a texture from an unconfigured surface can block indefinitely on
        // some drivers (Adreno).
        if !self.surface_configured {
            return false;
        }

        let last_error = self.last_error.lock().unwrap().take();
        if let Some(error) = last_error {
            self.failed_frame_count += 1;
            log::error!(
                "GPU error during frame (failure {} of 10): {error}",
                self.failed_frame_count
            );

            // TBD. Does retrying more actually help?
            if self.failed_frame_count > 10 {
                panic!("Too many consecutive GPU errors. Last error: {error}");
            } else if self.failed_frame_count > 5 {
                if let Some(res) = self.resources.as_mut() {
                    res.invalidate_intermediate_textures();
                }
                self.atlas.clear();
                self.needs_redraw = true;
                self.failed_frame_count = 0;
                return false;
            }
        } else {
            self.failed_frame_count = 0;
        }

        self.atlas.before_frame();
        self.ensure_effect_pipelines(scene);

        let frame = match self.resources().surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame) => frame,
            wgpu::CurrentSurfaceTexture::Suboptimal(frame) => {
                // Textures must be destroyed before the surface can be reconfigured.
                drop(frame);
                let surface_config = self.surface_config.clone();
                let resources = self.resources_mut();
                resources
                    .surface
                    .configure(&resources.device, &surface_config);
                return false;
            }
            wgpu::CurrentSurfaceTexture::Lost | wgpu::CurrentSurfaceTexture::Outdated => {
                let surface_config = self.surface_config.clone();
                let resources = self.resources_mut();
                resources
                    .surface
                    .configure(&resources.device, &surface_config);
                return false;
            }
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                return false;
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                *self.last_error.lock().unwrap() =
                    Some("Surface texture validation error".to_string());
                return false;
            }
        };

        // Now that we know the surface is healthy, ensure intermediate textures exist
        self.ensure_intermediate_textures();
        self.prepare_surfaces(scene);
        #[cfg(target_os = "linux")]
        let dma_buf_leases = {
            let mut ids = HashSet::new();
            scene
                .surfaces
                .iter()
                .filter_map(|surface| surface.source.frame())
                .filter_map(|frame| match frame.backing() {
                    SurfaceFrameBacking::Cpu(_) => None,
                    SurfaceFrameBacking::DmaBuf(dma_buf) => Some(dma_buf),
                })
                .filter(|dma_buf| ids.insert(dma_buf.id()))
                .cloned()
                .collect::<Vec<_>>()
        };

        let frame_view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let gamma_params = GammaParams {
            gamma_ratios: self.rendering_params.gamma_ratios,
            grayscale_enhanced_contrast: self.rendering_params.grayscale_enhanced_contrast,
            subpixel_enhanced_contrast: self.rendering_params.subpixel_enhanced_contrast,
            is_bgr: self.is_bgr as u32,
            _pad: 0,
        };

        let globals = GlobalParams {
            viewport_size: [
                self.surface_config.width as f32,
                self.surface_config.height as f32,
            ],
            premultiplied_alpha: if self.surface_config.alpha_mode
                == wgpu::CompositeAlphaMode::PreMultiplied
            {
                1
            } else {
                0
            },
            pad: 0,
        };

        let path_globals = GlobalParams {
            premultiplied_alpha: 0,
            ..globals
        };

        {
            let resources = self.resources();
            resources.queue.write_buffer(
                &resources.globals_buffer,
                0,
                bytemuck::bytes_of(&globals),
            );
            resources.queue.write_buffer(
                &resources.globals_buffer,
                self.path_globals_offset,
                bytemuck::bytes_of(&path_globals),
            );
            resources.queue.write_buffer(
                &resources.globals_buffer,
                self.gamma_offset,
                bytemuck::bytes_of(&gamma_params),
            );
        }

        loop {
            let mut instance_offset: u64 = 0;
            let mut overflow = false;

            let mut encoder =
                self.resources()
                    .device
                    .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                        label: Some("main_encoder"),
                    });

            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("main_pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &frame_view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                        depth_slice: None,
                    })],
                    depth_stencil_attachment: None,
                    ..Default::default()
                });

                for batch in scene.batches() {
                    let ok = match batch {
                        PrimitiveBatch::Quads(range) => {
                            self.draw_quads(&scene.quads[range], &mut instance_offset, &mut pass)
                        }
                        PrimitiveBatch::Effects(range) => self.draw_effects(
                            &scene.effects[range],
                            &mut instance_offset,
                            &mut pass,
                        ),
                        PrimitiveBatch::Shadows(range) => self.draw_shadows(
                            &scene.shadows[range],
                            &mut instance_offset,
                            &mut pass,
                        ),
                        PrimitiveBatch::Paths(range) => {
                            let paths = &scene.paths[range];
                            if paths.is_empty() {
                                continue;
                            }

                            drop(pass);

                            let did_draw = self.draw_paths_to_intermediate(
                                &mut encoder,
                                paths,
                                &mut instance_offset,
                            );

                            pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                                label: Some("main_pass_continued"),
                                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                                    view: &frame_view,
                                    resolve_target: None,
                                    ops: wgpu::Operations {
                                        load: wgpu::LoadOp::Load,
                                        store: wgpu::StoreOp::Store,
                                    },
                                    depth_slice: None,
                                })],
                                depth_stencil_attachment: None,
                                ..Default::default()
                            });

                            if did_draw {
                                self.draw_paths_from_intermediate(
                                    paths,
                                    &mut instance_offset,
                                    &mut pass,
                                )
                            } else {
                                false
                            }
                        }
                        PrimitiveBatch::Underlines(range) => self.draw_underlines(
                            &scene.underlines[range],
                            &mut instance_offset,
                            &mut pass,
                        ),
                        PrimitiveBatch::MonochromeSprites { texture_id, range } => self
                            .draw_monochrome_sprites(
                                &scene.monochrome_sprites[range],
                                texture_id,
                                &mut instance_offset,
                                &mut pass,
                            ),
                        PrimitiveBatch::SubpixelSprites { texture_id, range } => self
                            .draw_subpixel_sprites(
                                &scene.subpixel_sprites[range],
                                texture_id,
                                &mut instance_offset,
                                &mut pass,
                            ),
                        PrimitiveBatch::PolychromeSprites { texture_id, range } => self
                            .draw_polychrome_sprites(
                                &scene.polychrome_sprites[range],
                                texture_id,
                                &mut instance_offset,
                                &mut pass,
                            ),
                        PrimitiveBatch::Surfaces(range) => self.draw_surfaces(
                            &scene.surfaces[range],
                            &mut instance_offset,
                            &mut pass,
                        ),
                    };
                    if !ok {
                        overflow = true;
                        break;
                    }
                }
            }

            if overflow {
                drop(encoder);
                if self.instance_buffer_capacity >= self.max_buffer_size {
                    log::error!(
                        "instance buffer size grew too large: {}",
                        self.instance_buffer_capacity
                    );
                    self.resources().queue.present(frame);
                    return true;
                }
                self.grow_instance_buffer();
                continue;
            }

            let resources = self.resources();
            resources.queue.submit(std::iter::once(encoder.finish()));
            #[cfg(target_os = "linux")]
            if !dma_buf_leases.is_empty() {
                resources
                    .queue
                    .on_submitted_work_done(move || drop(dma_buf_leases));
            }
            resources.queue.present(frame);
            return true;
        }
    }

    fn prepare_surfaces(&mut self, scene: &Scene) {
        let mut frames = HashMap::<SurfaceId, Arc<SurfaceFrame>>::new();
        for surface in &scene.surfaces {
            let Some(frame) = surface.source.frame() else {
                continue;
            };
            if let Some(previous) = frames.insert(frame.handle().id(), frame.clone())
                && previous.sequence() != frame.sequence()
            {
                log::warn!(
                    "surface {:?} was painted with multiple sequences in one scene; using {}",
                    frame.handle().id(),
                    frame.sequence()
                );
            }
        }

        let max_texture_size = self.max_texture_size;
        let resources = self.resources_mut();
        resources
            .surfaces
            .retain(|_, cached| cached.owner.is_alive());
        #[cfg(target_os = "linux")]
        {
            resources
                .dma_bufs
                .retain(|_, cached| cached.owner.is_alive());
            resources
                .failed_dma_bufs
                .retain(|_, owner| owner.is_alive());
        }

        for (id, frame) in frames {
            let size = frame.coded_size();
            let width = size.width.0.max(0) as u32;
            let height = size.height.0.max(0) as u32;
            if width > max_texture_size || height > max_texture_size {
                log::error!(
                    "surface {:?} size {}x{} exceeds the GPU texture limit {}",
                    id,
                    width,
                    height,
                    max_texture_size
                );
                #[cfg(target_os = "linux")]
                if let SurfaceFrameBacking::DmaBuf(dma_buf) = frame.backing() {
                    dma_buf.report_import_failed(format!(
                        "DMA-BUF size {width}x{height} exceeds the GPU texture limit {max_texture_size}"
                    ));
                }
                resources.surfaces.remove(&id);
                continue;
            }

            #[cfg(target_os = "linux")]
            if let SurfaceFrameBacking::DmaBuf(dma_buf) = frame.backing() {
                let dma_buf_id = dma_buf.id();
                if let Err(error) = frame.wait_for_dma_buf_acquire_fence() {
                    log::error!(
                        "failed to wait for DMA-BUF {:?} acquire fence: {error}",
                        dma_buf_id
                    );
                    dma_buf.report_import_failed(format!(
                        "failed to wait for DMA-BUF acquire fence: {error}"
                    ));
                    resources.dma_bufs.remove(&dma_buf_id);
                    continue;
                }
                if resources.failed_dma_bufs.contains_key(&dma_buf_id) {
                    continue;
                }

                if let Some(cached) = resources.dma_bufs.get(&dma_buf_id) {
                    if cached.format != frame.format() || cached.size != frame.coded_size() {
                        log::error!(
                            "DMA-BUF {:?} was reused with incompatible frame metadata",
                            dma_buf_id
                        );
                        dma_buf.report_import_failed(
                            "DMA-BUF allocation was reused with incompatible frame metadata",
                        );
                        resources
                            .failed_dma_bufs
                            .insert(dma_buf_id, dma_buf.downgrade());
                        resources.dma_bufs.remove(&dma_buf_id);
                    }
                    continue;
                }

                match Self::import_dma_buf(
                    &resources.instance,
                    &resources.device,
                    resources.drm_render_device,
                    &frame,
                    dma_buf,
                ) {
                    Ok(cached) => {
                        dma_buf.report_import_ready();
                        resources.dma_bufs.insert(dma_buf_id, cached);
                    }
                    Err(error) => {
                        log::error!("failed to import DMA-BUF {:?}: {error:#}", dma_buf_id);
                        dma_buf.report_import_failed(format!("{error:#}"));
                        resources
                            .failed_dma_bufs
                            .insert(dma_buf_id, dma_buf.downgrade());
                    }
                }
                continue;
            }

            let action = surface_cache_action(
                resources
                    .surfaces
                    .get(&id)
                    .map(|cached| (cached.sequence, cached.format, cached.size)),
                &frame,
            );
            match action {
                SurfaceCacheAction::Create | SurfaceCacheAction::Recreate => {
                    let textures = Self::create_surface_textures(&resources.device, &frame);
                    let cached = CachedSurface {
                        sequence: frame.sequence(),
                        format: frame.format(),
                        size: frame.coded_size(),
                        textures,
                        owner: frame.handle().downgrade(),
                    };
                    Self::upload_surface(&resources.queue, &cached.textures, &frame);
                    resources.surfaces.insert(id, cached);
                }
                SurfaceCacheAction::Upload => {
                    let cached = resources.surfaces.get_mut(&id).unwrap();
                    Self::upload_surface(&resources.queue, &cached.textures, &frame);
                    cached.sequence = frame.sequence();
                }
                SurfaceCacheAction::Reuse => {}
            }
        }
    }

    #[cfg(target_os = "linux")]
    fn import_dma_buf(
        instance: &wgpu::Instance,
        device: &wgpu::Device,
        render_device: Option<DrmDevice>,
        frame: &SurfaceFrame,
        dma_buf: &DmaBufHandle,
    ) -> anyhow::Result<CachedDmaBuf> {
        if !device
            .features()
            .contains(wgpu::Features::VULKAN_EXTERNAL_MEMORY_DMA_BUF)
        {
            anyhow::bail!("the selected WGPU device does not support Vulkan DMA-BUF import");
        }

        let size = frame.coded_size();
        let width = size.width.0 as u32;
        let height = size.height.0 as u32;
        if let Some(image) = dma_buf.image() {
            if let Some(producer_device) = image.drm_device() {
                let Some(render_device) = render_device else {
                    anyhow::bail!(
                        "DMA-BUF producer device {}:{} is known but the Vulkan adapter has no DRM render device",
                        producer_device.major,
                        producer_device.minor
                    );
                };
                if producer_device != render_device {
                    anyhow::bail!(
                        "DMA-BUF producer device {}:{} does not match Vulkan render device {}:{}",
                        producer_device.major,
                        producer_device.minor,
                        render_device.major,
                        render_device.minor
                    );
                }
            }
            return Self::import_native_dma_buf_image(instance, device, frame, dma_buf, image);
        }
        let textures = match frame.format() {
            SurfaceFormat::Bgra8 | SurfaceFormat::Rgba8 => {
                let format = match frame.format() {
                    SurfaceFormat::Bgra8 => wgpu::TextureFormat::Bgra8Unorm,
                    SurfaceFormat::Rgba8 => wgpu::TextureFormat::Rgba8Unorm,
                    SurfaceFormat::Nv12 => unreachable!(),
                };
                let plane = dma_buf
                    .plane(0)
                    .ok_or_else(|| anyhow::anyhow!("RGB DMA-BUF is missing plane 0"))?;
                let (texture, view) = Self::import_dma_buf_plane(
                    device,
                    plane,
                    "gpui_surface_dma_buf_rgba",
                    format,
                    width,
                    height,
                )?;
                CachedSurfaceTextures::Rgba {
                    _texture: texture,
                    view,
                }
            }
            SurfaceFormat::Nv12 => {
                let y_plane = dma_buf
                    .plane(0)
                    .ok_or_else(|| anyhow::anyhow!("NV12 DMA-BUF is missing Y plane"))?;
                let uv_plane = dma_buf
                    .plane(1)
                    .ok_or_else(|| anyhow::anyhow!("NV12 DMA-BUF is missing UV plane"))?;
                let (y_texture, y_view) = Self::import_dma_buf_plane(
                    device,
                    y_plane,
                    "gpui_surface_dma_buf_nv12_y",
                    wgpu::TextureFormat::R8Unorm,
                    width,
                    height,
                )?;
                let (uv_texture, uv_view) = Self::import_dma_buf_plane(
                    device,
                    uv_plane,
                    "gpui_surface_dma_buf_nv12_uv",
                    wgpu::TextureFormat::Rg8Unorm,
                    width.div_ceil(2),
                    height.div_ceil(2),
                )?;
                CachedSurfaceTextures::Nv12 {
                    _y_texture: y_texture,
                    y_view,
                    _uv_texture: uv_texture,
                    uv_view,
                }
            }
        };

        Ok(CachedDmaBuf {
            format: frame.format(),
            size,
            textures,
            owner: dma_buf.downgrade(),
        })
    }

    #[cfg(target_os = "linux")]
    fn import_native_dma_buf_image(
        instance: &wgpu::Instance,
        device: &wgpu::Device,
        frame: &SurfaceFrame,
        dma_buf: &DmaBufHandle,
        image: &DmaBufImage,
    ) -> anyhow::Result<CachedDmaBuf> {
        if frame.format() != SurfaceFormat::Nv12 || image.drm_fourcc() != DRM_FORMAT_NV12 {
            anyhow::bail!(
                "native DMA-BUF import currently supports only NV12, got fourcc {:#010x}",
                image.drm_fourcc()
            );
        }
        if image.objects().len() != 1 {
            anyhow::bail!(
                "native tiled NV12 import currently requires one DMA-BUF object, got {}",
                image.objects().len()
            );
        }
        if image.planes().len() != 2 {
            anyhow::bail!(
                "native tiled NV12 import requires two plane layouts, got {}",
                image.planes().len()
            );
        }
        if !device
            .features()
            .contains(wgpu::Features::TEXTURE_FORMAT_NV12)
        {
            anyhow::bail!("the selected Vulkan device does not support native NV12 textures");
        }

        let size = frame.coded_size();
        let extent = wgpu::Extent3d {
            width: size.width.0 as u32,
            height: size.height.0 as u32,
            depth_or_array_layers: 1,
        };
        let view_formats = vec![wgpu::TextureFormat::R8Unorm, wgpu::TextureFormat::Rg8Unorm];
        let hal_descriptor = wgpu::hal::TextureDescriptor {
            label: Some("gpui_surface_native_dma_buf_nv12"),
            size: extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::NV12,
            usage: wgpu::wgt::TextureUses::RESOURCE,
            memory_flags: wgpu::hal::MemoryFlags::empty(),
            view_formats: view_formats.clone(),
        };
        let descriptor = wgpu::TextureDescriptor {
            label: Some("gpui_surface_native_dma_buf_nv12"),
            size: extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::NV12,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &view_formats,
        };

        let hal_instance = unsafe { instance.as_hal::<wgpu::hal::vulkan::Api>() }
            .ok_or_else(|| anyhow::anyhow!("the selected WGPU backend is not Vulkan"))?;
        let hal_device = unsafe { device.as_hal::<wgpu::hal::vulkan::Api>() }
            .ok_or_else(|| anyhow::anyhow!("the selected WGPU backend is not Vulkan"))?;
        let hal_texture = unsafe {
            Self::create_native_nv12_dma_buf_texture(
                &hal_instance,
                &hal_device,
                image,
                &hal_descriptor,
            )
        }?;
        drop(hal_device);

        let texture = unsafe {
            device.create_texture_from_hal::<wgpu::hal::vulkan::Api>(
                hal_texture,
                &descriptor,
                wgpu::wgt::TextureUses::RESOURCE,
            )
        };
        let y_view = texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("gpui_surface_native_dma_buf_nv12_y"),
            format: Some(wgpu::TextureFormat::R8Unorm),
            aspect: wgpu::TextureAspect::Plane0,
            ..Default::default()
        });
        let uv_view = texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("gpui_surface_native_dma_buf_nv12_uv"),
            format: Some(wgpu::TextureFormat::Rg8Unorm),
            aspect: wgpu::TextureAspect::Plane1,
            ..Default::default()
        });

        Ok(CachedDmaBuf {
            format: frame.format(),
            size,
            textures: CachedSurfaceTextures::Nv12 {
                _y_texture: texture.clone(),
                y_view,
                _uv_texture: texture,
                uv_view,
            },
            owner: dma_buf.downgrade(),
        })
    }

    #[cfg(target_os = "linux")]
    unsafe fn create_native_nv12_dma_buf_texture(
        instance: &wgpu::hal::vulkan::Instance,
        device: &wgpu::hal::vulkan::Device,
        image: &DmaBufImage,
        descriptor: &wgpu::hal::TextureDescriptor<'_>,
    ) -> anyhow::Result<wgpu::hal::vulkan::Texture> {
        use ash::vk;

        let raw_instance = instance.shared_instance().raw_instance();
        let raw_device = device.raw_device();
        let physical_device = device.raw_physical_device();
        let modifier = image.objects()[0].modifier();
        let format = vk::Format::G8_B8R8_2PLANE_420_UNORM;
        let image_flags =
            vk::ImageCreateFlags::MUTABLE_FORMAT | vk::ImageCreateFlags::EXTENDED_USAGE;
        let usage = vk::ImageUsageFlags::SAMPLED;

        let mut modifier_list = vk::DrmFormatModifierPropertiesListEXT::default();
        let mut modifier_format_properties =
            vk::FormatProperties2::default().push_next(&mut modifier_list);
        unsafe {
            raw_instance.get_physical_device_format_properties2(
                physical_device,
                format,
                &mut modifier_format_properties,
            );
        }
        let mut modifier_properties = vec![
            vk::DrmFormatModifierPropertiesEXT::default();
            modifier_list.drm_format_modifier_count as usize
        ];
        let mut modifier_list = vk::DrmFormatModifierPropertiesListEXT::default()
            .drm_format_modifier_properties(&mut modifier_properties);
        let mut modifier_format_properties =
            vk::FormatProperties2::default().push_next(&mut modifier_list);
        unsafe {
            raw_instance.get_physical_device_format_properties2(
                physical_device,
                format,
                &mut modifier_format_properties,
            );
        }
        let modifier_properties = modifier_properties
            .iter()
            .find(|properties| properties.drm_format_modifier == modifier)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Vulkan does not advertise NV12 modifier {modifier:#018x} on the selected adapter"
                )
            })?;
        if modifier_properties.drm_format_modifier_plane_count as usize != image.planes().len() {
            anyhow::bail!(
                "NV12 modifier {modifier:#018x} requires {} memory-plane layouts, but the descriptor supplies {}",
                modifier_properties.drm_format_modifier_plane_count,
                image.planes().len()
            );
        }
        if !modifier_properties
            .drm_format_modifier_tiling_features
            .contains(vk::FormatFeatureFlags::SAMPLED_IMAGE)
        {
            anyhow::bail!(
                "Vulkan does not support sampled images for NV12 modifier {modifier:#018x}"
            );
        }

        let mut external_query = vk::PhysicalDeviceExternalImageFormatInfo::default()
            .handle_type(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT);
        let mut modifier_query = vk::PhysicalDeviceImageDrmFormatModifierInfoEXT::default()
            .drm_format_modifier(modifier)
            .sharing_mode(vk::SharingMode::EXCLUSIVE);
        let format_query = vk::PhysicalDeviceImageFormatInfo2::default()
            .format(format)
            .ty(vk::ImageType::TYPE_2D)
            .tiling(vk::ImageTiling::DRM_FORMAT_MODIFIER_EXT)
            .usage(usage)
            .flags(image_flags)
            .push_next(&mut external_query)
            .push_next(&mut modifier_query);
        let mut external_properties = vk::ExternalImageFormatProperties::default();
        let mut format_properties =
            vk::ImageFormatProperties2::default().push_next(&mut external_properties);
        unsafe {
            raw_instance.get_physical_device_image_format_properties2(
                physical_device,
                &format_query,
                &mut format_properties,
            )
        }
        .map_err(|error| {
            anyhow::anyhow!(
                "Vulkan does not support NV12 modifier {modifier:#018x} for sampled DMA-BUF import: {error:?}"
            )
        })?;
        let external = external_properties.external_memory_properties;
        if !external
            .external_memory_features
            .contains(vk::ExternalMemoryFeatureFlags::IMPORTABLE)
            || !external
                .compatible_handle_types
                .contains(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT)
        {
            anyhow::bail!(
                "Vulkan reports NV12 modifier {modifier:#018x} as non-importable DMA-BUF memory"
            );
        }

        let plane_layouts = image
            .planes()
            .iter()
            .map(|plane| vk::SubresourceLayout {
                offset: plane.offset(),
                size: 0,
                row_pitch: u64::from(plane.stride()),
                array_pitch: 0,
                depth_pitch: 0,
            })
            .collect::<Vec<_>>();
        let view_formats = [format, vk::Format::R8_UNORM, vk::Format::R8G8_UNORM];
        let mut external_create = vk::ExternalMemoryImageCreateInfo::default()
            .handle_types(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT);
        let mut modifier_create = vk::ImageDrmFormatModifierExplicitCreateInfoEXT::default()
            .drm_format_modifier(modifier)
            .plane_layouts(&plane_layouts);
        let mut format_list = vk::ImageFormatListCreateInfo::default().view_formats(&view_formats);
        let create_info = vk::ImageCreateInfo::default()
            .flags(image_flags)
            .image_type(vk::ImageType::TYPE_2D)
            .format(format)
            .extent(vk::Extent3D {
                width: descriptor.size.width,
                height: descriptor.size.height,
                depth: 1,
            })
            .mip_levels(1)
            .array_layers(1)
            .samples(vk::SampleCountFlags::TYPE_1)
            .tiling(vk::ImageTiling::DRM_FORMAT_MODIFIER_EXT)
            .usage(usage)
            .sharing_mode(vk::SharingMode::EXCLUSIVE)
            .initial_layout(vk::ImageLayout::UNDEFINED)
            .push_next(&mut external_create)
            .push_next(&mut modifier_create)
            .push_next(&mut format_list);
        let raw_image =
            unsafe { raw_device.create_image(&create_info, None) }.map_err(|error| {
                anyhow::anyhow!(
                    "failed to create native NV12 image for modifier {modifier:#018x}: {error:?}"
                )
            })?;

        let requirements = unsafe { raw_device.get_image_memory_requirements(raw_image) };
        let external_memory_fd =
            ash::khr::external_memory_fd::Device::new(raw_instance, raw_device);
        let fd = match image.objects()[0].try_clone_fd() {
            Ok(fd) => fd,
            Err(error) => {
                unsafe { raw_device.destroy_image(raw_image, None) };
                return Err(anyhow::anyhow!("failed to duplicate DMA-BUF fd: {error}"));
            }
        };
        let mut fd_properties = vk::MemoryFdPropertiesKHR::default();
        if let Err(error) = unsafe {
            external_memory_fd.get_memory_fd_properties(
                vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT,
                fd.as_raw_fd(),
                &mut fd_properties,
            )
        } {
            unsafe { raw_device.destroy_image(raw_image, None) };
            return Err(anyhow::anyhow!(
                "failed to query DMA-BUF memory properties: {error:?}"
            ));
        }

        let type_bits = requirements.memory_type_bits & fd_properties.memory_type_bits;
        let memory_properties =
            unsafe { raw_instance.get_physical_device_memory_properties(physical_device) };
        let memory_type_index = memory_properties
            .memory_types_as_slice()
            .iter()
            .enumerate()
            .find(|(index, memory_type)| {
                type_bits & (1 << index) != 0
                    && memory_type
                        .property_flags
                        .contains(vk::MemoryPropertyFlags::DEVICE_LOCAL)
            })
            .or_else(|| {
                memory_properties
                    .memory_types_as_slice()
                    .iter()
                    .enumerate()
                    .find(|(index, _)| type_bits & (1 << index) != 0)
            })
            .map(|(index, _)| index as u32);
        let Some(memory_type_index) = memory_type_index else {
            unsafe { raw_device.destroy_image(raw_image, None) };
            anyhow::bail!("DMA-BUF has no memory type compatible with the Vulkan NV12 image");
        };

        let mut dedicated = vk::MemoryDedicatedAllocateInfo::default().image(raw_image);
        let raw_fd = fd.into_raw_fd();
        let mut import = vk::ImportMemoryFdInfoKHR::default()
            .handle_type(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT)
            .fd(raw_fd);
        let allocation_info = vk::MemoryAllocateInfo::default()
            .allocation_size(requirements.size)
            .memory_type_index(memory_type_index)
            .push_next(&mut import)
            .push_next(&mut dedicated);
        let memory = match unsafe { raw_device.allocate_memory(&allocation_info, None) } {
            Ok(memory) => memory,
            Err(error) => {
                drop(unsafe { OwnedFd::from_raw_fd(raw_fd) });
                unsafe { raw_device.destroy_image(raw_image, None) };
                return Err(anyhow::anyhow!(
                    "failed to import native NV12 DMA-BUF memory: {error:?}"
                ));
            }
        };
        if let Err(error) = unsafe { raw_device.bind_image_memory(raw_image, memory, 0) } {
            unsafe {
                raw_device.free_memory(memory, None);
                raw_device.destroy_image(raw_image, None);
            }
            return Err(anyhow::anyhow!(
                "failed to bind native NV12 DMA-BUF memory: {error:?}"
            ));
        }

        Ok(unsafe {
            device.texture_from_raw(
                raw_image,
                descriptor,
                None,
                wgpu::hal::vulkan::TextureMemory::Dedicated(memory),
            )
        })
    }

    #[cfg(target_os = "linux")]
    fn import_dma_buf_plane(
        device: &wgpu::Device,
        plane: &DmaBufPlane,
        label: &'static str,
        format: wgpu::TextureFormat,
        width: u32,
        height: u32,
    ) -> anyhow::Result<(wgpu::Texture, wgpu::TextureView)> {
        let extent = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };
        let hal_descriptor = wgpu::hal::TextureDescriptor {
            label: Some(label),
            size: extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::wgt::TextureUses::RESOURCE,
            memory_flags: wgpu::hal::MemoryFlags::empty(),
            view_formats: Vec::new(),
        };
        let descriptor = wgpu::TextureDescriptor {
            label: Some(label),
            size: extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        };

        let fd = plane
            .try_clone_fd()
            .map_err(|error| anyhow::anyhow!("failed to duplicate DMA-BUF fd: {error}"))?;
        let hal_device = unsafe { device.as_hal::<wgpu::hal::vulkan::Api>() }
            .ok_or_else(|| anyhow::anyhow!("the selected WGPU backend is not Vulkan"))?;
        let hal_texture = unsafe {
            hal_device.texture_from_dmabuf_fd(
                fd,
                &hal_descriptor,
                plane.drm_modifier(),
                u64::from(plane.stride()),
                plane.offset(),
            )
        }
        .map_err(|error| anyhow::anyhow!("Vulkan rejected the DMA-BUF: {error:?}"))?;
        drop(hal_device);

        // The producer contract guarantees that the imported pixels are fully initialized
        // and ready for sampled reads before the frame is published.
        let texture = unsafe {
            device.create_texture_from_hal::<wgpu::hal::vulkan::Api>(
                hal_texture,
                &descriptor,
                wgpu::wgt::TextureUses::RESOURCE,
            )
        };
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        Ok((texture, view))
    }

    fn create_surface_textures(
        device: &wgpu::Device,
        frame: &SurfaceFrame,
    ) -> CachedSurfaceTextures {
        let size = frame.coded_size();
        let width = size.width.0 as u32;
        let height = size.height.0 as u32;
        let descriptor = |label, format, width, height| wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        };

        match frame.format() {
            SurfaceFormat::Bgra8 | SurfaceFormat::Rgba8 => {
                let format = match frame.format() {
                    SurfaceFormat::Bgra8 => wgpu::TextureFormat::Bgra8Unorm,
                    SurfaceFormat::Rgba8 => wgpu::TextureFormat::Rgba8Unorm,
                    SurfaceFormat::Nv12 => unreachable!(),
                };
                let texture =
                    device.create_texture(&descriptor("gpui_surface_rgba", format, width, height));
                let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
                CachedSurfaceTextures::Rgba {
                    _texture: texture,
                    view,
                }
            }
            SurfaceFormat::Nv12 => {
                let y_texture = device.create_texture(&descriptor(
                    "gpui_surface_y",
                    wgpu::TextureFormat::R8Unorm,
                    width,
                    height,
                ));
                let y_view = y_texture.create_view(&wgpu::TextureViewDescriptor::default());
                let uv_texture = device.create_texture(&descriptor(
                    "gpui_surface_uv",
                    wgpu::TextureFormat::Rg8Unorm,
                    width.div_ceil(2),
                    height.div_ceil(2),
                ));
                let uv_view = uv_texture.create_view(&wgpu::TextureViewDescriptor::default());
                CachedSurfaceTextures::Nv12 {
                    _y_texture: y_texture,
                    y_view,
                    _uv_texture: uv_texture,
                    uv_view,
                }
            }
        }
    }

    fn upload_surface(queue: &wgpu::Queue, textures: &CachedSurfaceTextures, frame: &SurfaceFrame) {
        let size = frame.coded_size();
        let width = size.width.0 as u32;
        let height = size.height.0 as u32;
        let write_plane =
            |texture: &wgpu::Texture, plane: &gpui::SurfacePlane, width: u32, height: u32| {
                queue.write_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture,
                        mip_level: 0,
                        origin: wgpu::Origin3d::ZERO,
                        aspect: wgpu::TextureAspect::All,
                    },
                    plane.bytes(),
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(plane.stride()),
                        rows_per_image: None,
                    },
                    wgpu::Extent3d {
                        width,
                        height,
                        depth_or_array_layers: 1,
                    },
                );
            };

        let planes = frame
            .cpu_planes()
            .expect("CPU surface upload requires CPU planes");
        match textures {
            CachedSurfaceTextures::Rgba { _texture, .. } => {
                write_plane(_texture, &planes[0], width, height);
            }
            CachedSurfaceTextures::Nv12 {
                _y_texture,
                _uv_texture,
                ..
            } => {
                write_plane(_y_texture, &planes[0], width, height);
                write_plane(
                    _uv_texture,
                    &planes[1],
                    width.div_ceil(2),
                    height.div_ceil(2),
                );
            }
        }
    }

    fn draw_surfaces(
        &self,
        surfaces: &[gpui::PaintSurface],
        instance_offset: &mut u64,
        pass: &mut wgpu::RenderPass<'_>,
    ) -> bool {
        for surface in surfaces {
            let Some(frame) = surface.source.frame() else {
                continue;
            };
            let resources = self.resources();
            #[cfg(target_os = "linux")]
            let cached_textures = match frame.backing() {
                SurfaceFrameBacking::Cpu(_) => resources
                    .surfaces
                    .get(&frame.handle().id())
                    .map(|cached| &cached.textures),
                SurfaceFrameBacking::DmaBuf(dma_buf) => resources
                    .dma_bufs
                    .get(&dma_buf.id())
                    .map(|cached| &cached.textures),
            };
            #[cfg(not(target_os = "linux"))]
            let cached_textures = resources
                .surfaces
                .get(&frame.handle().id())
                .map(|cached| &cached.textures);
            let Some(cached_textures) = cached_textures else {
                continue;
            };

            let (uv_origin, uv_size) = surface_uv_bounds(frame);
            let params = SurfaceParams {
                bounds: surface.bounds.into(),
                clip_bounds: surface.clip_bounds.into(),
                content_mask: surface.content_mask.bounds.into(),
                uv_bounds: PodBounds {
                    origin: uv_origin,
                    size: uv_size,
                },
                corner_radii: [
                    surface.corner_radii.top_left.0,
                    surface.corner_radii.top_right.0,
                    surface.corner_radii.bottom_right.0,
                    surface.corner_radii.bottom_left.0,
                ],
                color_rows: yuv_to_rgb_rows(frame.color()),
                opacity: surface.opacity,
                _pad: [0.0; 3],
            };
            let Some((offset, size)) =
                self.write_to_instance_buffer(instance_offset, bytemuck::bytes_of(&params))
            else {
                return false;
            };

            let (first_view, second_view, pipeline) = match cached_textures {
                CachedSurfaceTextures::Rgba { view, .. } => {
                    (view, view, &resources.pipelines.surfaces_rgba)
                }
                CachedSurfaceTextures::Nv12 {
                    y_view, uv_view, ..
                } => (y_view, uv_view, &resources.pipelines.surfaces_nv12),
            };
            let bind_group = resources
                .device
                .create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("gpui_surface_bind_group"),
                    layout: &resources.bind_group_layouts.surfaces,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: self.instance_binding(offset, size),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::TextureView(first_view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: wgpu::BindingResource::TextureView(second_view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 3,
                            resource: wgpu::BindingResource::Sampler(&resources.atlas_sampler),
                        },
                    ],
                });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &resources.globals_bind_group, &[]);
            pass.set_bind_group(1, &bind_group, &[]);
            pass.draw(0..4, 0..1);
        }
        true
    }

    fn draw_quads(
        &self,
        quads: &[Quad],
        instance_offset: &mut u64,
        pass: &mut wgpu::RenderPass<'_>,
    ) -> bool {
        let data = unsafe { Self::instance_bytes(quads) };
        self.draw_instances(
            data,
            quads.len() as u32,
            &self.resources().pipelines.quads,
            instance_offset,
            pass,
        )
    }

    fn draw_effects(
        &self,
        effects: &[EffectQuad],
        instance_offset: &mut u64,
        pass: &mut wgpu::RenderPass<'_>,
    ) -> bool {
        let mut start = 0;
        while start < effects.len() {
            let shader_id = effects[start].shader.id().as_u64();
            let texture_id = effects[start].image_tile.map(|tile| tile.texture_id);
            let second_texture_id = effects[start].second_image_tile.map(|tile| tile.texture_id);
            let third_texture_id = effects[start].third_image_tile.map(|tile| tile.texture_id);
            let fourth_texture_id = effects[start].fourth_image_tile.map(|tile| tile.texture_id);
            let mut end = start + 1;
            while end < effects.len()
                && effects[end].shader.id().as_u64() == shader_id
                && effects[end].image_tile.map(|tile| tile.texture_id) == texture_id
                && effects[end].second_image_tile.map(|tile| tile.texture_id) == second_texture_id
                && effects[end].third_image_tile.map(|tile| tile.texture_id) == third_texture_id
                && effects[end].fourth_image_tile.map(|tile| tile.texture_id) == fourth_texture_id
            {
                end += 1;
            }

            let Some(pipeline) = self.resources().effect_pipelines.get(&shader_id) else {
                start = end;
                continue;
            };
            let instances = effects[start..end]
                .iter()
                .map(EffectInstance::from)
                .collect::<Vec<_>>();
            let drawn = if effects[start].shader.image_count() >= 4 {
                let (
                    Some(texture_id),
                    Some(second_texture_id),
                    Some(third_texture_id),
                    Some(fourth_texture_id),
                ) = (
                    texture_id,
                    second_texture_id,
                    third_texture_id,
                    fourth_texture_id,
                )
                else {
                    start = end;
                    continue;
                };
                let texture = self.atlas.get_texture_info(texture_id);
                let second_texture = self.atlas.get_texture_info(second_texture_id);
                let third_texture = self.atlas.get_texture_info(third_texture_id);
                let fourth_texture = self.atlas.get_texture_info(fourth_texture_id);
                self.draw_instances_with_four_textures(
                    bytemuck::cast_slice(&instances),
                    instances.len() as u32,
                    [
                        &texture.view,
                        &second_texture.view,
                        &third_texture.view,
                        &fourth_texture.view,
                    ],
                    pipeline,
                    instance_offset,
                    pass,
                )
            } else if effects[start].shader.image_count() >= 2 {
                let (Some(texture_id), Some(second_texture_id)) = (texture_id, second_texture_id)
                else {
                    start = end;
                    continue;
                };
                let texture = self.atlas.get_texture_info(texture_id);
                let second_texture = self.atlas.get_texture_info(second_texture_id);
                self.draw_instances_with_two_textures(
                    bytemuck::cast_slice(&instances),
                    instances.len() as u32,
                    &texture.view,
                    &second_texture.view,
                    pipeline,
                    instance_offset,
                    pass,
                )
            } else if effects[start].shader.uses_image() {
                let Some(texture_id) = texture_id else {
                    start = end;
                    continue;
                };
                let texture = self.atlas.get_texture_info(texture_id);
                self.draw_instances_with_texture(
                    bytemuck::cast_slice(&instances),
                    instances.len() as u32,
                    &texture.view,
                    pipeline,
                    instance_offset,
                    pass,
                )
            } else {
                self.draw_instances(
                    bytemuck::cast_slice(&instances),
                    instances.len() as u32,
                    pipeline,
                    instance_offset,
                    pass,
                )
            };
            if !drawn {
                return false;
            }
            start = end;
        }
        true
    }

    fn draw_shadows(
        &self,
        shadows: &[Shadow],
        instance_offset: &mut u64,
        pass: &mut wgpu::RenderPass<'_>,
    ) -> bool {
        let data = unsafe { Self::instance_bytes(shadows) };
        self.draw_instances(
            data,
            shadows.len() as u32,
            &self.resources().pipelines.shadows,
            instance_offset,
            pass,
        )
    }

    fn draw_underlines(
        &self,
        underlines: &[Underline],
        instance_offset: &mut u64,
        pass: &mut wgpu::RenderPass<'_>,
    ) -> bool {
        let data = unsafe { Self::instance_bytes(underlines) };
        self.draw_instances(
            data,
            underlines.len() as u32,
            &self.resources().pipelines.underlines,
            instance_offset,
            pass,
        )
    }

    fn draw_monochrome_sprites(
        &self,
        sprites: &[MonochromeSprite],
        texture_id: AtlasTextureId,
        instance_offset: &mut u64,
        pass: &mut wgpu::RenderPass<'_>,
    ) -> bool {
        let tex_info = self.atlas.get_texture_info(texture_id);
        let data = unsafe { Self::instance_bytes(sprites) };
        self.draw_instances_with_texture(
            data,
            sprites.len() as u32,
            &tex_info.view,
            &self.resources().pipelines.mono_sprites,
            instance_offset,
            pass,
        )
    }

    fn draw_subpixel_sprites(
        &self,
        sprites: &[SubpixelSprite],
        texture_id: AtlasTextureId,
        instance_offset: &mut u64,
        pass: &mut wgpu::RenderPass<'_>,
    ) -> bool {
        let tex_info = self.atlas.get_texture_info(texture_id);
        let data = unsafe { Self::instance_bytes(sprites) };
        let resources = self.resources();
        let pipeline = resources
            .pipelines
            .subpixel_sprites
            .as_ref()
            .unwrap_or(&resources.pipelines.mono_sprites);
        self.draw_instances_with_texture(
            data,
            sprites.len() as u32,
            &tex_info.view,
            pipeline,
            instance_offset,
            pass,
        )
    }

    fn draw_polychrome_sprites(
        &self,
        sprites: &[PolychromeSprite],
        texture_id: AtlasTextureId,
        instance_offset: &mut u64,
        pass: &mut wgpu::RenderPass<'_>,
    ) -> bool {
        let tex_info = self.atlas.get_texture_info(texture_id);
        let data = unsafe { Self::instance_bytes(sprites) };
        self.draw_instances_with_texture(
            data,
            sprites.len() as u32,
            &tex_info.view,
            &self.resources().pipelines.poly_sprites,
            instance_offset,
            pass,
        )
    }

    fn draw_instances(
        &self,
        data: &[u8],
        instance_count: u32,
        pipeline: &wgpu::RenderPipeline,
        instance_offset: &mut u64,
        pass: &mut wgpu::RenderPass<'_>,
    ) -> bool {
        if instance_count == 0 {
            return true;
        }
        let Some((offset, size)) = self.write_to_instance_buffer(instance_offset, data) else {
            return false;
        };
        let resources = self.resources();
        let bind_group = resources
            .device
            .create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &resources.bind_group_layouts.instances,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.instance_binding(offset, size),
                }],
            });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &resources.globals_bind_group, &[]);
        pass.set_bind_group(1, &bind_group, &[]);
        pass.draw(0..4, 0..instance_count);
        true
    }

    fn draw_instances_with_texture(
        &self,
        data: &[u8],
        instance_count: u32,
        texture_view: &wgpu::TextureView,
        pipeline: &wgpu::RenderPipeline,
        instance_offset: &mut u64,
        pass: &mut wgpu::RenderPass<'_>,
    ) -> bool {
        if instance_count == 0 {
            return true;
        }
        let Some((offset, size)) = self.write_to_instance_buffer(instance_offset, data) else {
            return false;
        };
        let resources = self.resources();
        let bind_group = resources
            .device
            .create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &resources.bind_group_layouts.instances_with_texture,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: self.instance_binding(offset, size),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(texture_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Sampler(&resources.atlas_sampler),
                    },
                ],
            });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &resources.globals_bind_group, &[]);
        pass.set_bind_group(1, &bind_group, &[]);
        pass.draw(0..4, 0..instance_count);
        true
    }

    fn draw_instances_with_two_textures(
        &self,
        data: &[u8],
        instance_count: u32,
        texture_view: &wgpu::TextureView,
        second_texture_view: &wgpu::TextureView,
        pipeline: &wgpu::RenderPipeline,
        instance_offset: &mut u64,
        pass: &mut wgpu::RenderPass<'_>,
    ) -> bool {
        if instance_count == 0 {
            return true;
        }
        let Some((offset, size)) = self.write_to_instance_buffer(instance_offset, data) else {
            return false;
        };
        let resources = self.resources();
        let bind_group = resources
            .device
            .create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &resources.bind_group_layouts.instances_with_two_textures,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: self.instance_binding(offset, size),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(texture_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Sampler(&resources.atlas_sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::TextureView(second_texture_view),
                    },
                ],
            });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &resources.globals_bind_group, &[]);
        pass.set_bind_group(1, &bind_group, &[]);
        pass.draw(0..4, 0..instance_count);
        true
    }

    fn draw_instances_with_four_textures(
        &self,
        data: &[u8],
        instance_count: u32,
        texture_views: [&wgpu::TextureView; 4],
        pipeline: &wgpu::RenderPipeline,
        instance_offset: &mut u64,
        pass: &mut wgpu::RenderPass<'_>,
    ) -> bool {
        if instance_count == 0 {
            return true;
        }
        let Some((offset, size)) = self.write_to_instance_buffer(instance_offset, data) else {
            return false;
        };
        let resources = self.resources();
        let bind_group = resources
            .device
            .create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &resources.bind_group_layouts.instances_with_four_textures,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: self.instance_binding(offset, size),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(texture_views[0]),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Sampler(&resources.atlas_sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::TextureView(texture_views[1]),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: wgpu::BindingResource::TextureView(texture_views[2]),
                    },
                    wgpu::BindGroupEntry {
                        binding: 5,
                        resource: wgpu::BindingResource::TextureView(texture_views[3]),
                    },
                ],
            });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &resources.globals_bind_group, &[]);
        pass.set_bind_group(1, &bind_group, &[]);
        pass.draw(0..4, 0..instance_count);
        true
    }

    unsafe fn instance_bytes<T>(instances: &[T]) -> &[u8] {
        unsafe {
            std::slice::from_raw_parts(
                instances.as_ptr() as *const u8,
                std::mem::size_of_val(instances),
            )
        }
    }

    fn draw_paths_from_intermediate(
        &self,
        paths: &[Path<ScaledPixels>],
        instance_offset: &mut u64,
        pass: &mut wgpu::RenderPass<'_>,
    ) -> bool {
        let first_path = &paths[0];
        let sprites: Vec<PathSprite> = if paths.last().map(|p| &p.order) == Some(&first_path.order)
        {
            paths
                .iter()
                .map(|p| PathSprite {
                    bounds: p.clipped_bounds(),
                })
                .collect()
        } else {
            let mut bounds = first_path.clipped_bounds();
            for path in paths.iter().skip(1) {
                bounds = bounds.union(&path.clipped_bounds());
            }
            vec![PathSprite { bounds }]
        };

        let resources = self.resources();
        let Some(path_intermediate_view) = resources.path_intermediate_view.as_ref() else {
            return true;
        };

        let sprite_data = unsafe { Self::instance_bytes(&sprites) };
        self.draw_instances_with_texture(
            sprite_data,
            sprites.len() as u32,
            path_intermediate_view,
            &resources.pipelines.paths,
            instance_offset,
            pass,
        )
    }

    fn draw_paths_to_intermediate(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        paths: &[Path<ScaledPixels>],
        instance_offset: &mut u64,
    ) -> bool {
        let mut vertices = Vec::new();
        for path in paths {
            let bounds = path.clipped_bounds();
            vertices.extend(path.vertices.iter().map(|v| PathRasterizationVertex {
                xy_position: v.xy_position,
                st_position: v.st_position,
                color: path.color,
                bounds,
            }));
        }

        if vertices.is_empty() {
            return true;
        }

        let vertex_data = unsafe { Self::instance_bytes(&vertices) };
        let Some((vertex_offset, vertex_size)) =
            self.write_to_instance_buffer(instance_offset, vertex_data)
        else {
            return false;
        };

        let resources = self.resources();
        let data_bind_group = resources
            .device
            .create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("path_rasterization_bind_group"),
                layout: &resources.bind_group_layouts.instances,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.instance_binding(vertex_offset, vertex_size),
                }],
            });

        let Some(path_intermediate_view) = resources.path_intermediate_view.as_ref() else {
            return true;
        };

        let (target_view, resolve_target) = if let Some(ref msaa_view) = resources.path_msaa_view {
            (msaa_view, Some(path_intermediate_view))
        } else {
            (path_intermediate_view, None)
        };

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("path_rasterization_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target_view,
                    resolve_target,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                ..Default::default()
            });

            pass.set_pipeline(&resources.pipelines.path_rasterization);
            pass.set_bind_group(0, &resources.path_globals_bind_group, &[]);
            pass.set_bind_group(1, &data_bind_group, &[]);
            pass.draw(0..vertices.len() as u32, 0..1);
        }

        true
    }

    fn grow_instance_buffer(&mut self) {
        let new_capacity = (self.instance_buffer_capacity * 2).min(self.max_buffer_size);
        log::info!("increased instance buffer size to {}", new_capacity);
        let resources = self.resources_mut();
        resources.instance_buffer = resources.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("instance_buffer"),
            size: new_capacity,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.instance_buffer_capacity = new_capacity;
    }

    fn write_to_instance_buffer(
        &self,
        instance_offset: &mut u64,
        data: &[u8],
    ) -> Option<(u64, NonZeroU64)> {
        let offset = (*instance_offset).next_multiple_of(self.storage_buffer_alignment);
        let size = (data.len() as u64).max(16);
        if offset + size > self.instance_buffer_capacity {
            return None;
        }
        let resources = self.resources();
        resources
            .queue
            .write_buffer(&resources.instance_buffer, offset, data);
        *instance_offset = offset + size;
        Some((offset, NonZeroU64::new(size).expect("size is at least 16")))
    }

    fn instance_binding(&self, offset: u64, size: NonZeroU64) -> wgpu::BindingResource<'_> {
        wgpu::BindingResource::Buffer(wgpu::BufferBinding {
            buffer: &self.resources().instance_buffer,
            offset,
            size: Some(size),
        })
    }

    /// Mark the surface as unconfigured so rendering is skipped until a new
    /// surface is provided via [`replace_surface`](Self::replace_surface).
    ///
    /// This does **not** drop the renderer — the device, queue, atlas, and
    /// pipelines stay alive.  Use this when the native window is destroyed
    /// (e.g. Android `TerminateWindow`) but you intend to re-create the
    /// surface later without losing cached atlas textures.
    pub fn unconfigure_surface(&mut self) {
        self.surface_configured = false;
        // Drop intermediate textures since they reference the old surface size.
        if let Some(res) = self.resources.as_mut() {
            res.invalidate_intermediate_textures();
        }
    }

    /// Replace the wgpu surface with a new one (e.g. after Android destroys
    /// and recreates the native window).  Keeps the device, queue, atlas, and
    /// all pipelines intact so cached `AtlasTextureId`s remain valid.
    ///
    /// The `instance` **must** be the same [`wgpu::Instance`] that was used to
    /// create the adapter and device (i.e. from the [`WgpuContext`]).  Using a
    /// different instance will cause a "Device does not exist" panic because
    /// the wgpu device is bound to its originating instance.
    #[cfg(not(target_family = "wasm"))]
    pub fn replace_surface<W: HasWindowHandle>(
        &mut self,
        window: &W,
        config: WgpuSurfaceConfig,
        instance: &wgpu::Instance,
    ) -> anyhow::Result<()> {
        let window_handle = window
            .window_handle()
            .map_err(|e| anyhow::anyhow!("Failed to get window handle: {e}"))?;

        let surface = create_surface(instance, window_handle.as_raw())?;

        let width = (config.size.width.0 as u32).max(1);
        let height = (config.size.height.0 as u32).max(1);

        let alpha_mode = if config.transparent {
            self.transparent_alpha_mode
        } else {
            self.opaque_alpha_mode
        };

        self.surface_config.width = width;
        self.surface_config.height = height;
        self.surface_config.alpha_mode = alpha_mode;
        if let Some(mode) = config.preferred_present_mode {
            self.surface_config.present_mode = mode;
        }

        {
            let res = self
                .resources
                .as_mut()
                .expect("GPU resources not available");
            surface.configure(&res.device, &self.surface_config);
            res.surface = surface;

            // Invalidate intermediate textures — they'll be recreated lazily.
            res.invalidate_intermediate_textures();
        }

        self.surface_configured = true;

        Ok(())
    }

    pub fn destroy(&mut self) {
        // Release surface-bound GPU resources eagerly so the underlying native
        // window can be destroyed before the renderer itself is dropped.
        self.resources.take();
    }

    /// Returns true if the GPU device was lost and recovery is needed.
    pub fn device_lost(&self) -> bool {
        self.device_lost.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Returns true if a redraw is needed because GPU state was cleared.
    /// Calling this method clears the flag.
    pub fn needs_redraw(&mut self) -> bool {
        std::mem::take(&mut self.needs_redraw)
    }

    /// Recovers from a lost GPU device by recreating the renderer with a new context.
    ///
    /// Call this after detecting `device_lost()` returns true.
    ///
    /// This method coordinates recovery across multiple windows:
    /// - The first window to call this will recreate the shared context
    /// - Subsequent windows will adopt the already-recovered context
    #[cfg(not(target_family = "wasm"))]
    pub fn recover<W>(&mut self, window: &W) -> anyhow::Result<()>
    where
        W: HasWindowHandle + HasDisplayHandle + std::fmt::Debug + Send + Sync + Clone + 'static,
    {
        let gpu_context = self.context.as_ref().expect("recover requires gpu_context");

        // Check if another window already recovered the context
        let needs_new_context = gpu_context
            .borrow()
            .as_ref()
            .is_none_or(|ctx| ctx.device_lost());

        let window_handle = window
            .window_handle()
            .map_err(|e| anyhow::anyhow!("Failed to get window handle: {e}"))?;

        let surface = if needs_new_context {
            log::warn!("GPU device lost, recreating context...");

            // Drop old resources to release Arc<Device>/Arc<Queue> and GPU resources
            self.resources = None;
            *gpu_context.borrow_mut() = None;

            // Wait briefly for the GPU driver to stabilize, then try to
            // recreate the context without software renderers. If this fails
            // the caller should request another frame and retry — the real GPU
            // may need more time to come back (e.g. after suspend/resume).
            std::thread::sleep(std::time::Duration::from_millis(350));

            let instance = WgpuContext::instance(Box::new(window.clone()));
            let surface = create_surface(&instance, window_handle.as_raw())?;
            let new_context =
                WgpuContext::new_rejecting_software(instance, &surface, self.compositor_gpu)?;
            *gpu_context.borrow_mut() = Some(new_context);
            surface
        } else {
            let ctx_ref = gpu_context.borrow();
            let instance = &ctx_ref.as_ref().unwrap().instance;
            create_surface(instance, window_handle.as_raw())?
        };

        let config = WgpuSurfaceConfig {
            size: gpui::Size {
                width: gpui::DevicePixels(self.surface_config.width as i32),
                height: gpui::DevicePixels(self.surface_config.height as i32),
            },
            transparent: self.surface_config.alpha_mode != wgpu::CompositeAlphaMode::Opaque,
            preferred_present_mode: Some(self.surface_config.present_mode),
        };
        let gpu_context = Rc::clone(gpu_context);
        let ctx_ref = gpu_context.borrow();
        let context = ctx_ref.as_ref().expect("context should exist");

        self.resources = None;
        self.atlas.handle_device_lost(context);

        *self = Self::new_internal(
            Some(gpu_context.clone()),
            context,
            surface,
            config,
            self.compositor_gpu,
            self.atlas.clone(),
        )?;

        log::info!("GPU recovery complete");
        Ok(())
    }
}

#[cfg(not(target_family = "wasm"))]
fn create_surface(
    instance: &wgpu::Instance,
    raw_window_handle: raw_window_handle::RawWindowHandle,
) -> anyhow::Result<wgpu::Surface<'static>> {
    unsafe {
        instance
            .create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
                // Fall back to the display handle already provided via InstanceDescriptor::display.
                raw_display_handle: None,
                raw_window_handle,
            })
            .map_err(|e| anyhow::anyhow!("{e}"))
    }
}

struct RenderingParameters {
    path_sample_count: u32,
    gamma_ratios: [f32; 4],
    grayscale_enhanced_contrast: f32,
    subpixel_enhanced_contrast: f32,
}

impl RenderingParameters {
    fn new(adapter: &wgpu::Adapter, surface_format: wgpu::TextureFormat) -> Self {
        use std::env;

        let format_features = adapter.get_texture_format_features(surface_format);
        let path_sample_count = [4, 2, 1]
            .into_iter()
            .find(|&n| format_features.flags.sample_count_supported(n))
            .unwrap_or(1);

        let gamma = env::var("ZED_FONTS_GAMMA")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(1.8_f32)
            .clamp(1.0, 2.2);
        let gamma_ratios = get_gamma_correction_ratios(gamma);

        let grayscale_enhanced_contrast = env::var("ZED_FONTS_GRAYSCALE_ENHANCED_CONTRAST")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(1.0_f32)
            .max(0.0);

        let subpixel_enhanced_contrast = env::var("ZED_FONTS_SUBPIXEL_ENHANCED_CONTRAST")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0.5_f32)
            .max(0.0);

        Self {
            path_sample_count,
            gamma_ratios,
            grayscale_enhanced_contrast,
            subpixel_enhanced_contrast,
        }
    }
}
