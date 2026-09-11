use crate::{CompositorGpuHint, WgpuAtlas, WgpuContext};
use bytemuck::{Pod, Zeroable};
use gpui::{
    AtlasTextureId, BackdropBlur, BackdropShader, Background, Bounds, ColorRange, DevicePixels,
    EffectQuad, EffectShader, GpuSpecs, MonochromeSprite, Path, Point, PolychromeSprite,
    PrimitiveBatch, Quad, ScaledPixels, Scene, Shadow, Size, SubpixelSprite, SurfaceColorInfo,
    SurfaceFormat, SurfaceFrame, SurfaceId, Underline, WeakSurfaceHandle, YuvMatrix,
    get_gamma_correction_ratios,
};
#[cfg(target_os = "linux")]
use gpui::{
    DRM_FORMAT_NV12, DmaBufHandle, DmaBufId, DmaBufImage, DmaBufPlane, DrmDevice,
    SurfaceFrameBacking, WeakDmaBufHandle,
};
use log::warn;
#[cfg(not(target_family = "wasm"))]
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::num::NonZeroU64;
#[cfg(target_os = "linux")]
use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd, OwnedFd};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

mod distance_field;
mod fluid;
mod particle_transition;
mod particles;
pub(crate) mod scene3d;
mod scene_snapshot;
#[cfg(not(target_family = "wasm"))]
mod texture_effect;
mod ui_capture;
#[cfg(not(target_family = "wasm"))]
pub use texture_effect::{TextureEffectConfig, WgpuTextureEffect};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SceneEncoding {
    Complete,
    InstanceCapacity,
    CaptureCapacity,
}

#[derive(Clone, Copy)]
struct FeedbackSnapshot {
    texture: usize,
    generation: u64,
    frame: u64,
    time: Duration,
}

struct FeedbackTextures {
    textures: [wgpu::Texture; 2],
    bounds: Bounds<ScaledPixels>,
    scale_factor: f32,
    viewport: (u32, u32),
    committed: Cell<Option<FeedbackSnapshot>>,
    pending: Cell<Option<FeedbackSnapshot>>,
}

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

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(super) struct BackdropInstance {
    bounds: PodBounds,
    content_mask: PodBounds,
    corner_radii: [f32; 4],
    blur_radius: f32,
    opacity: f32,
    time: f32,
    pointer_active: f32,
    direction: [f32; 2],
    pointer: [f32; 2],
    uniforms: [[f32; 4]; gpui::EFFECT_UNIFORM_SLOTS],
}

impl BackdropInstance {
    fn blur(
        viewport_size: [f32; 2],
        render_size: [f32; 2],
        blur_radius: f32,
        direction: [f32; 2],
    ) -> Self {
        let viewport = PodBounds {
            origin: [0.0, 0.0],
            size: viewport_size,
        };
        Self {
            bounds: viewport,
            content_mask: PodBounds {
                origin: [0.0, 0.0],
                size: render_size,
            },
            corner_radii: [0.0; 4],
            blur_radius,
            opacity: 1.0,
            time: 0.0,
            pointer_active: 0.0,
            direction,
            pointer: [0.5; 2],
            uniforms: [[0.0; 4]; gpui::EFFECT_UNIFORM_SLOTS],
        }
    }
}

impl From<&BackdropBlur> for BackdropInstance {
    fn from(backdrop: &BackdropBlur) -> Self {
        Self {
            bounds: backdrop.bounds.into(),
            content_mask: backdrop.content_mask.bounds.into(),
            corner_radii: [
                backdrop.corner_radii.top_left.0,
                backdrop.corner_radii.top_right.0,
                backdrop.corner_radii.bottom_right.0,
                backdrop.corner_radii.bottom_left.0,
            ],
            blur_radius: backdrop.blur_radius.0,
            opacity: backdrop.opacity,
            time: backdrop.time,
            pointer_active: u32::from(backdrop.pointer_active) as f32,
            direction: [0.0; 2],
            pointer: [backdrop.pointer.x, backdrop.pointer.y],
            uniforms: *backdrop.uniforms.slots(),
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
    backdrop_blur: wgpu::RenderPipeline,
    backdrop_composite: wgpu::RenderPipeline,
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
    capture_context: WgpuContext,
    instance: wgpu::Instance,
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
    surface: Option<wgpu::Surface<'static>>,
    pipelines: WgpuPipelines,
    effect_pipelines: HashMap<u64, wgpu::RenderPipeline>,
    subtree_effect_pipelines: HashMap<(u64, wgpu::TextureFormat), Option<wgpu::RenderPipeline>>,
    subtree_image_effect_pipelines:
        HashMap<(u64, wgpu::TextureFormat), Option<wgpu::RenderPipeline>>,
    subtree_textures: Vec<wgpu::Texture>,
    bloom_textures: HashMap<u32, [wgpu::Texture; 2]>,
    distance_field: Option<distance_field::DistanceFieldRenderer>,
    feedback_textures: HashMap<gpui::EffectHistoryId, FeedbackTextures>,
    particles: Option<particles::ParticleRenderer>,
    particle_transition: Option<particle_transition::ParticleTransitionRenderer>,
    fluid: Option<fluid::FluidRenderer>,
    scene3d: Option<scene3d::ViewportRenderer>,
    ui_captures: Vec<ui_capture::UiCapture>,
    ui_capture_indices: HashMap<usize, usize>,
    failed_effect_pipelines: HashSet<u64>,
    backdrop_effect_pipelines: HashMap<u64, wgpu::RenderPipeline>,
    failed_backdrop_effect_pipelines: HashSet<u64>,
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
    backdrop_source_texture: Option<wgpu::Texture>,
    backdrop_source_view: Option<wgpu::TextureView>,
    backdrop_horizontal_texture: Option<wgpu::Texture>,
    backdrop_horizontal_view: Option<wgpu::TextureView>,
    backdrop_result_texture: Option<wgpu::Texture>,
    backdrop_result_view: Option<wgpu::TextureView>,
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
        self.subtree_textures.clear();
        self.bloom_textures.clear();
        self.distance_field = None;
        self.feedback_textures.clear();
        self.particles = None;
        self.particle_transition = None;
        self.fluid = None;
        self.scene3d = None;
        self.ui_capture_indices.clear();
        self.path_intermediate_texture = None;
        self.path_intermediate_view = None;
        self.path_msaa_texture = None;
        self.path_msaa_view = None;
        self.backdrop_source_texture = None;
        self.backdrop_source_view = None;
        self.backdrop_horizontal_texture = None;
        self.backdrop_horizontal_view = None;
        self.backdrop_result_texture = None;
        self.backdrop_result_view = None;
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
    backdrop_blur_supported: bool,
    scene3d_support: gpui::Scene3dSupport,
    scene3d_output_budget: scene3d::OutputBudget,
    last_error: Arc<Mutex<Option<String>>>,
    failed_frame_count: u32,
    device_lost: std::sync::Arc<std::sync::atomic::AtomicBool>,
    surface_configured: bool,
    needs_redraw: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct WgpuExternalRendererConfig {
    pub size: Size<DevicePixels>,
    pub format: wgpu::TextureFormat,
    pub alpha_mode: wgpu::CompositeAlphaMode,
    pub target_usage: wgpu::TextureUsages,
}

pub struct WgpuExternalRenderTarget<'a> {
    pub texture: &'a wgpu::Texture,
    pub view: &'a wgpu::TextureView,
    pub command_encoder: &'a mut wgpu::CommandEncoder,
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

    pub fn new_external(
        context: &WgpuContext,
        config: WgpuExternalRendererConfig,
    ) -> anyhow::Result<Self> {
        anyhow::ensure!(
            config
                .target_usage
                .contains(wgpu::TextureUsages::RENDER_ATTACHMENT),
            "external GPUI target must support RENDER_ATTACHMENT"
        );
        let max_texture_size = context.device.limits().max_texture_dimension_2d;
        let width = (config.size.width.0.max(1) as u32).min(max_texture_size);
        let height = (config.size.height.0.max(1) as u32).min(max_texture_size);
        // Backdrop copies currently assume a surface-local texture origin. Keep them
        // disabled for embedded viewports until copy origins are carried through the
        // backdrop pipeline as well.
        let backdrop_blur_supported = false;
        let surface_config = wgpu::SurfaceConfiguration {
            usage: config.target_usage,
            format: config.format,
            width,
            height,
            present_mode: wgpu::PresentMode::Fifo,
            desired_maximum_frame_latency: 2,
            alpha_mode: config.alpha_mode,
            view_formats: Vec::new(),
            color_space: wgpu::SurfaceColorSpace::Auto,
        };
        let atlas = Arc::new(WgpuAtlas::from_context(context));
        Self::new_for_target(
            None,
            context,
            None,
            surface_config,
            None,
            atlas,
            config.alpha_mode,
            config.alpha_mode,
            backdrop_blur_supported,
            None,
        )
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

    /// Creates a renderer for another presentation surface while sharing this renderer's atlas.
    ///
    /// This is intended for short-lived auxiliary surfaces, such as Wayland drag icons, whose
    /// scenes reference sprites allocated by the source window.
    #[cfg(not(target_family = "wasm"))]
    pub fn new_with_shared_atlas<W>(
        &self,
        window: &W,
        config: WgpuSurfaceConfig,
    ) -> anyhow::Result<Self>
    where
        W: HasWindowHandle + HasDisplayHandle + std::fmt::Debug + Send + Sync + Clone + 'static,
    {
        let window_handle = window
            .window_handle()
            .map_err(|e| anyhow::anyhow!("Failed to get window handle: {e}"))?;
        let gpu_context = self
            .context
            .as_ref()
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("shared-atlas renderer requires a GPU context"))?;

        let instance = gpu_context
            .borrow()
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("GPU context is unavailable"))?
            .instance
            .clone();
        let surface = create_surface(&instance, window_handle.as_raw())?;

        let context_ref = gpu_context.borrow();
        let context = context_ref
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("GPU context is unavailable"))?;
        context.check_compatible_with_surface(&surface)?;

        Self::new_internal(
            Some(gpu_context.clone()),
            context,
            surface,
            config,
            self.compositor_gpu,
            self.atlas.clone(),
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

        let max_texture_size = context.device.limits().max_texture_dimension_2d;

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

        let backdrop_blur_supported = surface_caps.usages.contains(wgpu::TextureUsages::COPY_SRC);
        let surface_config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | if backdrop_blur_supported {
                    wgpu::TextureUsages::COPY_SRC
                } else {
                    wgpu::TextureUsages::empty()
                },
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

        Self::new_for_target(
            gpu_context,
            context,
            Some(surface),
            surface_config,
            compositor_gpu,
            atlas,
            transparent_alpha_mode,
            opaque_alpha_mode,
            backdrop_blur_supported,
            None,
        )
    }

    fn new_for_target(
        gpu_context: Option<GpuContext>,
        context: &WgpuContext,
        surface: Option<wgpu::Surface<'static>>,
        surface_config: wgpu::SurfaceConfiguration,
        compositor_gpu: Option<CompositorGpuHint>,
        atlas: Arc<WgpuAtlas>,
        transparent_alpha_mode: wgpu::CompositeAlphaMode,
        opaque_alpha_mode: wgpu::CompositeAlphaMode,
        backdrop_blur_supported: bool,
        shared_error: Option<Arc<Mutex<Option<String>>>>,
    ) -> anyhow::Result<Self> {
        let surface_format = surface_config.format;
        let scene3d_support =
            match crate::Scene3dDeviceCapabilities::query_with_formats(context, [surface_format])
                .viewport(surface_format)
            {
                Ok(capabilities) => gpui::Scene3dSupport::Supported(capabilities),
                Err(error) => gpui::Scene3dSupport::Unsupported(
                    gpui::Scene3dUnsupportedReason::MissingCapabilities(error.to_string().into()),
                ),
            };
        let alpha_mode = surface_config.alpha_mode;
        let device = Arc::clone(&context.device);
        let max_texture_size = device.limits().max_texture_dimension_2d;
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

        let last_error = shared_error.unwrap_or_else(|| {
            let error: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
            let sink = error.clone();
            device.on_uncaptured_error(Arc::new(move |error| {
                *sink.lock().unwrap() = Some(error.to_string());
            }));
            error
        });

        let resources = WgpuResources {
            capture_context: context.clone(),
            instance: context.instance.clone(),
            device,
            queue,
            surface,
            pipelines,
            effect_pipelines: HashMap::default(),
            subtree_effect_pipelines: HashMap::default(),
            subtree_image_effect_pipelines: HashMap::default(),
            subtree_textures: Vec::new(),
            bloom_textures: HashMap::new(),
            distance_field: None,
            feedback_textures: HashMap::new(),
            particles: None,
            particle_transition: None,
            fluid: None,
            scene3d: None,
            ui_captures: Vec::new(),
            ui_capture_indices: HashMap::new(),
            failed_effect_pipelines: HashSet::default(),
            backdrop_effect_pipelines: HashMap::default(),
            failed_backdrop_effect_pipelines: HashSet::default(),
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
            backdrop_source_texture: None,
            backdrop_source_view: None,
            backdrop_horizontal_texture: None,
            backdrop_horizontal_view: None,
            backdrop_result_texture: None,
            backdrop_result_view: None,
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
            backdrop_blur_supported,
            scene3d_support,
            scene3d_output_budget: scene3d::OutputBudget::default(),
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
        subtree: bool,
        external_images: bool,
    ) -> anyhow::Result<wgpu::RenderPipeline> {
        let source = if external_images {
            gpui::compose_subtree_image_effect_wgsl(shader)
        } else if subtree {
            gpui::compose_subtree_effect_wgsl(shader)
        } else {
            gpui::compose_effect_shader_wgsl(shader)
        };
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
        let surface_format = self.surface_config.format;
        for layer in &scene.subtree_layers {
            for input in layer.inputs() {
                self.ensure_effect_pipelines(input);
            }
            for effect in layer
                .intermediate_effects
                .iter()
                .filter(|pass| !pass.images.is_empty())
            {
                let key = (effect.shader.id().as_u64(), surface_format);
                if !self
                    .resources()
                    .subtree_image_effect_pipelines
                    .contains_key(&key)
                {
                    let result = Self::create_effect_pipeline(
                        &self.resources().device,
                        &self.resources().bind_group_layouts,
                        surface_format,
                        self.surface_config.alpha_mode,
                        &effect.shader,
                        true,
                        true,
                    );
                    self.resources_mut()
                        .subtree_image_effect_pipelines
                        .insert(key, result.ok());
                }
            }
            for (shader, format) in
                layer
                    .intermediate_effects
                    .iter()
                    .flat_map(|pass| {
                        std::iter::once((&pass.shader, surface_format))
                            .filter(|_| pass.images.is_empty())
                            .chain(pass.bloom.iter().flat_map(|bloom| {
                                [
                                    (&bloom.extract, wgpu::TextureFormat::Rgba16Float),
                                    (&bloom.blur, wgpu::TextureFormat::Rgba16Float),
                                    (&bloom.composite, surface_format),
                                ]
                            }))
                            .chain(pass.feedback.iter().map(|feedback| {
                                (&feedback.shader, wgpu::TextureFormat::Rgba16Float)
                            }))
                            .chain(
                                pass.distance_field
                                    .iter()
                                    .map(|field| (&field.composite, surface_format)),
                            )
                    })
                    .chain(std::iter::once((&layer.composite.shader, surface_format)))
            {
                let key = (shader.id().as_u64(), format);
                if !self.resources().subtree_effect_pipelines.contains_key(&key) {
                    let result = Self::create_effect_pipeline(
                        &self.resources().device,
                        &self.resources().bind_group_layouts,
                        format,
                        self.surface_config.alpha_mode,
                        shader,
                        true,
                        false,
                    );
                    self.resources_mut()
                        .subtree_effect_pipelines
                        .insert(key, result.ok());
                }
            }
        }
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
                false,
                false,
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

    fn create_backdrop_effect_pipeline(
        device: &wgpu::Device,
        layouts: &WgpuBindGroupLayouts,
        surface_format: wgpu::TextureFormat,
        alpha_mode: wgpu::CompositeAlphaMode,
        shader: &BackdropShader,
    ) -> anyhow::Result<wgpu::RenderPipeline> {
        let source = gpui::compose_backdrop_shader_wgsl(shader);
        let module = wgpu::naga::front::wgsl::parse_str(&source)
            .map_err(|error| anyhow::anyhow!("WGSL parse error: {error}"))?;
        wgpu::naga::valid::Validator::new(
            wgpu::naga::valid::ValidationFlags::all(),
            wgpu::naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .map_err(|error| anyhow::anyhow!("WGSL validation error: {error}"))?;

        let shader_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("gpui_backdrop_effect_shader"),
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Owned(source)),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("gpui_backdrop_effect_pipeline_layout"),
            bind_group_layouts: &[
                Some(&layouts.globals),
                Some(&layouts.instances_with_two_textures),
            ],
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
                label: Some("gpui_backdrop_effect_pipeline"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader_module,
                    entry_point: Some("vs_backdrop"),
                    buffers: &[],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader_module,
                    entry_point: Some("fs_backdrop"),
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

    fn ensure_backdrop_effect_pipelines(&mut self, scene: &Scene) {
        for layer in &scene.subtree_layers {
            for input in layer.inputs() {
                self.ensure_backdrop_effect_pipelines(input);
            }
        }
        let shaders = scene
            .backdrop_blurs
            .iter()
            .filter_map(|backdrop| backdrop.shader.clone())
            .collect::<Vec<_>>();

        for shader in shaders {
            let key = shader.id().as_u64();
            if self
                .resources()
                .backdrop_effect_pipelines
                .contains_key(&key)
                || self
                    .resources()
                    .failed_backdrop_effect_pipelines
                    .contains(&key)
            {
                continue;
            }

            let result = Self::create_backdrop_effect_pipeline(
                &self.resources().device,
                &self.resources().bind_group_layouts,
                self.surface_config.format,
                self.surface_config.alpha_mode,
                &shader,
            );
            match result {
                Ok(pipeline) => {
                    self.resources_mut()
                        .backdrop_effect_pipelines
                        .insert(key, pipeline);
                }
                Err(error) => {
                    log::error!("failed to compile GPUI backdrop effect {key:016x}: {error:#}");
                    self.resources_mut()
                        .failed_backdrop_effect_pipelines
                        .insert(key);
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
        let backdrop_shader_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("gpui_backdrop_blur_shader"),
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(include_str!(
                "backdrop_blur.wgsl"
            ))),
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

        let backdrop_blur = create_pipeline(
            "backdrop_blur",
            "vs_backdrop",
            "fs_blur",
            &layouts.globals,
            &layouts.instances_with_texture,
            wgpu::PrimitiveTopology::TriangleStrip,
            &[Some(wgpu::ColorTargetState {
                format: surface_format,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
            1,
            &backdrop_shader_module,
        );

        let backdrop_composite = create_pipeline(
            "backdrop_composite",
            "vs_backdrop",
            "fs_backdrop",
            &layouts.globals,
            &layouts.instances_with_texture,
            wgpu::PrimitiveTopology::TriangleStrip,
            &[Some(color_target.clone())],
            1,
            &backdrop_shader_module,
        );

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
            backdrop_blur,
            backdrop_composite,
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

    fn create_backdrop_intermediate(
        device: &wgpu::Device,
        label: &'static str,
        format: wgpu::TextureFormat,
        width: u32,
        height: u32,
        usage: wgpu::TextureUsages,
    ) -> (wgpu::Texture, wgpu::TextureView) {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: width.max(1),
                height: height.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage,
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
            if let Some(ref texture) = resources.backdrop_source_texture {
                texture.destroy();
            }
            if let Some(ref texture) = resources.backdrop_horizontal_texture {
                texture.destroy();
            }
            if let Some(ref texture) = resources.backdrop_result_texture {
                texture.destroy();
            }

            if let Some(surface) = &resources.surface {
                surface.configure(&resources.device, &surface_config);
            }

            // Invalidate intermediate textures - they will be lazily recreated
            // in draw() after we confirm the surface is healthy. This avoids
            // panics when the device/surface is in an invalid state during resize.
            resources.invalidate_intermediate_textures();
        }
    }

    fn ensure_intermediate_textures(&mut self, needs_backdrop: bool) {
        let needs_path_texture = self.resources().path_intermediate_texture.is_none();
        let needs_backdrop_textures = needs_backdrop
            && self.backdrop_blur_supported
            && self.resources().backdrop_source_texture.is_none();
        if !needs_path_texture && !needs_backdrop_textures {
            return;
        }

        let format = self.surface_config.format;
        let width = self.surface_config.width;
        let height = self.surface_config.height;
        let path_sample_count = self.rendering_params.path_sample_count;
        let backdrop_blur_supported = self.backdrop_blur_supported;
        let resources = self.resources_mut();

        if needs_path_texture {
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

        if needs_backdrop_textures && backdrop_blur_supported {
            let sampled = wgpu::TextureUsages::TEXTURE_BINDING;
            let rendered = sampled | wgpu::TextureUsages::RENDER_ATTACHMENT;
            let (source, source_view) = Self::create_backdrop_intermediate(
                &resources.device,
                "backdrop_source",
                format,
                width,
                height,
                sampled | wgpu::TextureUsages::COPY_DST,
            );
            let (horizontal, horizontal_view) = Self::create_backdrop_intermediate(
                &resources.device,
                "backdrop_horizontal",
                format,
                width.div_ceil(2),
                height.div_ceil(2),
                rendered,
            );
            let (result, result_view) = Self::create_backdrop_intermediate(
                &resources.device,
                "backdrop_result",
                format,
                width.div_ceil(2),
                height.div_ceil(2),
                rendered,
            );
            resources.backdrop_source_texture = Some(source);
            resources.backdrop_source_view = Some(source_view);
            resources.backdrop_horizontal_texture = Some(horizontal);
            resources.backdrop_horizontal_view = Some(horizontal_view);
            resources.backdrop_result_texture = Some(result);
            resources.backdrop_result_view = Some(result_view);
        }
    }

    pub fn set_subpixel_layout(&mut self, is_bgr: bool) {
        if self.is_bgr != is_bgr
            && let Some(renderer) = self
                .resources
                .as_mut()
                .and_then(|resources| resources.scene3d.as_mut())
        {
            renderer.invalidate_outputs();
        }
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
            if let Some(surface) = &resources.surface {
                surface.configure(&resources.device, &surface_config);
            }
            resources.pipelines = Self::create_pipelines(
                &resources.device,
                &resources.bind_group_layouts,
                surface_config.format,
                surface_config.alpha_mode,
                path_sample_count,
                dual_source_blending,
            );
            resources.effect_pipelines.clear();
            resources.subtree_effect_pipelines.clear();
            resources.subtree_image_effect_pipelines.clear();
            resources.failed_effect_pipelines.clear();
            resources.backdrop_effect_pipelines.clear();
            resources.failed_backdrop_effect_pipelines.clear();
            if let Some(renderer) = &mut resources.scene3d {
                renderer.invalidate_outputs();
            }
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

    /// Returns whether the current surface can be captured for backdrop filters.
    pub fn supports_backdrop_blur(&self) -> bool {
        self.backdrop_blur_supported
    }

    pub fn scene3d_support(&self) -> gpui::Scene3dSupport {
        if self.device_lost() {
            gpui::Scene3dSupport::Unsupported(gpui::Scene3dUnsupportedReason::DeviceLost)
        } else if self.resources.is_none() {
            gpui::Scene3dSupport::Unsupported(gpui::Scene3dUnsupportedReason::RendererUnavailable)
        } else {
            self.scene3d_support.clone()
        }
    }

    /// Releases mesh-rendering caches without clearing shared 2D atlas or UI
    /// capture resources. Capture contents are invalidated; the next mesh draw
    /// rebuilds its caches lazily.
    pub fn clear_scene3d_caches(&mut self) {
        if let Some(resources) = self.resources.as_mut() {
            resources.scene3d = None;
            for capture in &mut resources.ui_captures {
                capture.clear_scene3d_caches();
            }
        }
    }

    /// Mesh output-cache allocations across this renderer and its UI captures.
    /// Counts retained allocations, not total device memory or in-flight commands.
    pub fn scene3d_output_cache_stats(&self) -> gpui::Scene3dOutputCacheStats {
        self.scene3d_output_budget.stats()
    }

    /// Sets the shared mesh output-cache budget in bytes; zero disables mesh pixel reuse.
    /// A changed budget releases existing output-cache entries without clearing mesh,
    /// atlas, or UI textures. Does not request a frame or wait for the GPU.
    pub fn set_scene3d_output_cache_budget(&mut self, bytes: u64) {
        if self.scene3d_output_budget.set_limit(bytes) {
            self.invalidate_scene3d_outputs();
        }
    }

    fn invalidate_scene3d_outputs(&mut self) {
        if let Some(resources) = self.resources.as_mut() {
            if let Some(renderer) = &mut resources.scene3d {
                renderer.invalidate_outputs();
            }
            for capture in &mut resources.ui_captures {
                capture.invalidate_scene3d_outputs();
            }
        }
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
        self.ensure_backdrop_effect_pipelines(scene);

        let frame = match self
            .resources()
            .surface
            .as_ref()
            .expect("draw() requires a surface-backed renderer; use encode_external()")
            .get_current_texture()
        {
            wgpu::CurrentSurfaceTexture::Success(frame) => frame,
            wgpu::CurrentSurfaceTexture::Suboptimal(frame) => {
                // Textures must be destroyed before the surface can be reconfigured.
                drop(frame);
                let surface_config = self.surface_config.clone();
                let resources = self.resources_mut();
                resources
                    .surface
                    .as_ref()
                    .expect("surface-backed renderer")
                    .configure(&resources.device, &surface_config);
                return false;
            }
            wgpu::CurrentSurfaceTexture::Lost | wgpu::CurrentSurfaceTexture::Outdated => {
                let surface_config = self.surface_config.clone();
                let resources = self.resources_mut();
                resources
                    .surface
                    .as_ref()
                    .expect("surface-backed renderer")
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
        let mut needs_backdrop = false;
        scene.visit(&mut |scene| needs_backdrop |= !scene.backdrop_blurs.is_empty());
        self.ensure_intermediate_textures(needs_backdrop);
        self.ensure_subtree_textures(scene.subtree_target_count());
        self.ensure_bloom_textures(scene);
        self.ensure_distance_field(scene);
        self.ensure_feedback_textures(scene);
        self.prepare_surfaces(scene);
        #[cfg(target_os = "linux")]
        let dma_buf_leases = {
            let mut ids = HashSet::new();
            let mut leases = Vec::new();
            scene.visit(&mut |scene| {
                for surface in &scene.surfaces {
                    if let Some(frame) = surface.source.frame()
                        && let SurfaceFrameBacking::DmaBuf(dma_buf) = frame.backing()
                        && ids.insert(dma_buf.id())
                    {
                        leases.push(dma_buf.clone());
                    }
                }
            });
            leases
        };

        let frame_view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        loop {
            let mut encoder =
                self.resources()
                    .device
                    .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                        label: Some("main_encoder"),
                    });
            let encoded = self.encode_scene(
                scene,
                &frame.texture,
                &frame_view,
                &mut encoder,
                wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                true,
            );
            if !matches!(encoded, Ok(SceneEncoding::Complete)) {
                self.commit_encoded_scene(false);
                self.commit_scene3d_outputs(false);
                drop(encoder);
                match encoded {
                    Err(_) => return false,
                    Ok(SceneEncoding::InstanceCapacity) => self.grow_instance_buffer(),
                    _ => {}
                }
                continue;
            }

            let resources = self.resources();
            resources.queue.submit(std::iter::once(encoder.finish()));
            self.commit_encoded_scene(true);
            self.commit_scene3d_outputs(true);
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

    /// Encodes into caller-owned commands without reusing mesh or UI capture pixels.
    /// Exposed vertex buffers are not recycled for other mesh snapshots; retained
    /// replacement uploads remain replayable on later draws of the same snapshot.
    /// Use `draw_external` for renderer-owned submission and output reuse.
    pub fn encode_external(&mut self, scene: &Scene, target: WgpuExternalRenderTarget<'_>) -> bool {
        let encoded = matches!(
            self.encode_external_scene(scene, target, false),
            Ok(SceneEncoding::Complete)
        );
        self.commit_encoded_scene(encoded);
        self.retain_external_scene3d_uploads();
        encoded
    }

    /// Clears, encodes, and submits an external target on this renderer's queue.
    /// Enables reuse of submitted 3D viewport and UI capture pixels. A false result submits no
    /// frame commands. Instance capacity growth may require a retry; scene preparation
    /// errors do not grow instance storage. The target must match this
    /// renderer's configured size/format and support render attachments.
    pub fn draw_external(
        &mut self,
        scene: &Scene,
        texture: &wgpu::Texture,
        view: &wgpu::TextureView,
        clear: wgpu::Color,
    ) -> bool {
        let mut encoder =
            self.resources()
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("gpui_external_encoder"),
                });
        drop(encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("gpui_external_clear"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(clear),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        }));
        let encoded = matches!(
            self.encode_external_scene(
                scene,
                WgpuExternalRenderTarget {
                    texture,
                    view,
                    command_encoder: &mut encoder,
                },
                true,
            ),
            Ok(SceneEncoding::Complete)
        );
        if encoded {
            self.resources().queue.submit([encoder.finish()]);
        }
        self.commit_encoded_scene(encoded);
        self.commit_scene3d_outputs(encoded);
        encoded
    }

    fn encode_external_scene(
        &mut self,
        scene: &Scene,
        target: WgpuExternalRenderTarget<'_>,
        retain_outputs: bool,
    ) -> anyhow::Result<SceneEncoding> {
        assert!(
            self.resources().surface.is_none(),
            "encode_external() requires an external renderer"
        );

        self.atlas.before_frame();
        self.ensure_effect_pipelines(scene);
        self.ensure_backdrop_effect_pipelines(scene);
        let mut needs_backdrop = false;
        scene.visit(&mut |scene| needs_backdrop |= !scene.backdrop_blurs.is_empty());
        self.ensure_intermediate_textures(needs_backdrop);
        self.ensure_subtree_textures(scene.subtree_target_count());
        self.ensure_bloom_textures(scene);
        self.ensure_distance_field(scene);
        self.ensure_feedback_textures(scene);
        self.prepare_surfaces(scene);

        let encoded = self.encode_scene(
            scene,
            target.texture,
            target.view,
            target.command_encoder,
            wgpu::LoadOp::Load,
            retain_outputs,
        );
        match &encoded {
            Ok(SceneEncoding::InstanceCapacity) => {
                self.grow_instance_buffer();
                self.needs_redraw = true;
            }
            Ok(SceneEncoding::CaptureCapacity) => self.needs_redraw = true,
            _ => {}
        }
        encoded
    }

    fn ensure_subtree_textures(&mut self, depth: usize) {
        let width = self.surface_config.width;
        let height = self.surface_config.height;
        let format = self.surface_config.format;
        let resources = self.resources_mut();
        if resources.subtree_textures.first().is_some_and(|texture| {
            texture.width() != width || texture.height() != height || texture.format() != format
        }) {
            resources.subtree_textures.clear();
        }
        resources.subtree_textures.truncate(depth);
        while resources.subtree_textures.len() < depth {
            resources
                .subtree_textures
                .push(resources.device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("subtree_target"),
                    size: wgpu::Extent3d {
                        width,
                        height,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::TEXTURE_BINDING
                        | wgpu::TextureUsages::COPY_SRC
                        | wgpu::TextureUsages::COPY_DST,
                    view_formats: &[],
                }));
        }
    }

    fn ensure_bloom_textures(&mut self, scene: &Scene) {
        fn collect(scene: &Scene, divisors: &mut HashSet<u32>) {
            for layer in &scene.subtree_layers {
                for effect in layer.intermediate_effects.iter() {
                    if let Some(bloom) = &effect.bloom {
                        divisors.insert(bloom.downsample.clamp(1, 8));
                    }
                }
                for input in layer.inputs() {
                    collect(input, divisors);
                }
            }
        }
        let mut divisors = HashSet::new();
        collect(scene, &mut divisors);
        let width = self.surface_config.width;
        let height = self.surface_config.height;
        let format = wgpu::TextureFormat::Rgba16Float;
        let resources = self.resources_mut();
        resources.bloom_textures.retain(|divisor, textures| {
            divisors.contains(divisor)
                && textures[0].width() == width.div_ceil(*divisor)
                && textures[0].height() == height.div_ceil(*divisor)
                && textures[0].format() == format
        });
        for divisor in divisors {
            resources.bloom_textures.entry(divisor).or_insert_with(|| {
                std::array::from_fn(|_| {
                    resources.device.create_texture(&wgpu::TextureDescriptor {
                        label: Some("subtree_bloom_target"),
                        size: wgpu::Extent3d {
                            width: width.div_ceil(divisor),
                            height: height.div_ceil(divisor),
                            depth_or_array_layers: 1,
                        },
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format,
                        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                            | wgpu::TextureUsages::TEXTURE_BINDING,
                        view_formats: &[],
                    })
                })
            });
        }
    }

    fn ensure_distance_field(&mut self, scene: &Scene) {
        let viewport = [self.surface_config.width, self.surface_config.height];
        let mut required = [0u32; 2];
        scene.visit(&mut |scene| {
            for layer in &scene.subtree_layers {
                if layer
                    .intermediate_effects
                    .iter()
                    .any(|pass| pass.distance_field.is_some())
                {
                    let region = distance_field::region(layer.composite.bounds, viewport);
                    if region[2] > 0 && region[3] > 0 {
                        required[0] = required[0].max(region[2]);
                        required[1] = required[1].max(region[3]);
                    }
                }
            }
        });
        let resources = self.resources_mut();
        if required.contains(&0) {
            resources.distance_field = None;
        } else if let Some(field) = resources.distance_field.as_mut() {
            field.reserve(&resources.device, required);
        } else {
            resources.distance_field = Some(distance_field::DistanceFieldRenderer::new(
                &resources.device,
                required,
            ));
        }
    }

    fn ensure_feedback_textures(&mut self, scene: &Scene) {
        let mut required = HashMap::new();
        scene.visit(&mut |scene| {
            for layer in &scene.subtree_layers {
                for pass in layer.intermediate_effects.iter() {
                    if let Some(feedback) = &pass.feedback {
                        assert!(
                            required
                                .insert(
                                    feedback.id,
                                    (
                                        layer.composite.bounds,
                                        feedback.scale_factor,
                                        feedback.downsample.clamp(1, 8)
                                    )
                                )
                                .is_none(),
                            "a feedback identity may only occur once in a scene"
                        );
                    }
                }
            }
        });
        let viewport = (self.surface_config.width, self.surface_config.height);
        let resources = self.resources_mut();
        resources.feedback_textures.retain(|id, textures| {
            required
                .get(id)
                .is_some_and(|(bounds, scale_factor, divisor)| {
                    textures.viewport == viewport
                        && textures.bounds == *bounds
                        && textures.scale_factor == *scale_factor
                        && textures.textures[0].width() == viewport.0.div_ceil(*divisor)
                        && textures.textures[0].height() == viewport.1.div_ceil(*divisor)
                })
        });
        for (id, (bounds, scale_factor, divisor)) in required {
            resources
                .feedback_textures
                .entry(id)
                .or_insert_with(|| FeedbackTextures {
                    textures: std::array::from_fn(|_| {
                        resources.device.create_texture(&wgpu::TextureDescriptor {
                            label: Some("subtree_feedback_history"),
                            size: wgpu::Extent3d {
                                width: viewport.0.div_ceil(divisor),
                                height: viewport.1.div_ceil(divisor),
                                depth_or_array_layers: 1,
                            },
                            mip_level_count: 1,
                            sample_count: 1,
                            dimension: wgpu::TextureDimension::D2,
                            format: wgpu::TextureFormat::Rgba16Float,
                            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                                | wgpu::TextureUsages::TEXTURE_BINDING,
                            view_formats: &[],
                        })
                    }),
                    bounds,
                    scale_factor,
                    viewport,
                    committed: Cell::new(None),
                    pending: Cell::new(None),
                });
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn encode_subtree_feedback(
        &self,
        quad: &EffectQuad,
        effect: &gpui::SubtreeEffectPass,
        feedback: &gpui::SubtreeFeedbackPass,
        source: &wgpu::TextureView,
        destination: &wgpu::TextureView,
        resolve_pipeline: &wgpu::RenderPipeline,
        feedback_pipeline: &wgpu::RenderPipeline,
        encoder: &mut wgpu::CommandEncoder,
        instance_offset: &mut u64,
    ) -> bool {
        let textures = &self.resources().feedback_textures[&feedback.id];
        let previous = textures.committed.get().filter(|snapshot| {
            snapshot.generation == feedback.generation
                && snapshot.time <= feedback.time
                && snapshot.frame <= feedback.frame
        });
        let update = previous.is_none_or(|snapshot| snapshot.frame != feedback.frame);
        let read_index = previous.map_or(0, |snapshot| snapshot.texture);
        let output_index = if update { 1 - read_index } else { read_index };
        let views = textures
            .textures
            .each_ref()
            .map(|texture| texture.create_view(&Default::default()));
        let full_bounds: PodBounds = quad.bounds.into();
        let scale = [
            textures.textures[0].width() as f32 / self.surface_config.width as f32,
            textures.textures[0].height() as f32 / self.surface_config.height as f32,
        ];
        let history_bounds = PodBounds {
            origin: std::array::from_fn(|i| full_bounds.origin[i] * scale[i]),
            size: std::array::from_fn(|i| full_bounds.size[i] * scale[i]),
        };
        let mut instance = EffectInstance::from(quad);
        instance.opacity = 1.;
        instance.time = effect.time;
        instance.content_mask = full_bounds;
        instance.uniforms = *effect.uniforms.slots();
        if previous.is_none() {
            let _clear = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("subtree_feedback_clear"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &views[read_index],
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
        }
        if update {
            let retention = previous.map_or(0., |snapshot| {
                let elapsed = feedback.time.saturating_sub(snapshot.time).as_secs_f64();
                let duration = feedback
                    .fade_duration
                    .max(Duration::from_millis(1))
                    .as_secs_f64();
                (-10. * elapsed / duration).exp2() as f32
            });
            instance.uniforms[7] = [
                retention,
                if feedback.capture { 1. } else { 0. },
                1. / 1024.,
                0.,
            ];
            instance.image_bounds = full_bounds;
            instance.second_image_bounds = history_bounds;
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("subtree_feedback_update"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &views[output_index],
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            if !self.draw_instances_with_two_textures(
                bytemuck::bytes_of(&instance),
                1,
                source,
                &views[read_index],
                feedback_pipeline,
                instance_offset,
                &mut pass,
            ) {
                return false;
            }
        }
        instance.image_bounds = history_bounds;
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("subtree_feedback_resolve"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: destination,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        if !self.draw_instances_with_texture(
            bytemuck::bytes_of(&instance),
            1,
            &views[output_index],
            resolve_pipeline,
            instance_offset,
            &mut pass,
        ) {
            return false;
        }
        if update {
            textures.pending.set(Some(FeedbackSnapshot {
                texture: output_index,
                generation: feedback.generation,
                frame: feedback.frame,
                time: feedback.time,
            }));
        }
        true
    }

    #[allow(clippy::too_many_arguments)]
    fn encode_subtree_distance_field(
        &self,
        quad: &EffectQuad,
        effect: &gpui::SubtreeEffectPass,
        field: &gpui::SubtreeDistanceFieldPass,
        source: &wgpu::TextureView,
        destination: &wgpu::TextureView,
        pipeline: &wgpu::RenderPipeline,
        encoder: &mut wgpu::CommandEncoder,
        instance_offset: &mut u64,
    ) -> bool {
        let resources = self.resources();
        let region = distance_field::region(
            quad.bounds,
            [self.surface_config.width, self.surface_config.height],
        );
        let renderer = resources.distance_field.as_ref().unwrap();
        let distance = renderer.encode(&resources.device, source, region, field.threshold, encoder);
        let mut instance = EffectInstance::from(quad);
        instance.opacity = 1.;
        instance.uniforms = *effect.uniforms.slots();
        instance.time = effect.time;
        instance.content_mask = quad.bounds.into();
        instance.image_bounds = quad.bounds.into();
        instance.second_image_bounds = PodBounds {
            origin: [
                quad.bounds.origin.x.0 - region[0] as f32,
                quad.bounds.origin.y.0 - region[1] as f32,
            ],
            size: [quad.bounds.size.width.0, quad.bounds.size.height.0],
        };
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("distance_field_composite"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: destination,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        self.draw_instances_with_two_textures(
            bytemuck::bytes_of(&instance),
            1,
            source,
            &distance,
            pipeline,
            instance_offset,
            &mut pass,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn encode_subtree_bloom(
        &self,
        quad: &EffectQuad,
        effect: &gpui::SubtreeEffectPass,
        bloom: &gpui::SubtreeBloomPass,
        source: &wgpu::TextureView,
        destination: &wgpu::TextureView,
        encoder: &mut wgpu::CommandEncoder,
        instance_offset: &mut u64,
    ) -> bool {
        let resources = self.resources();
        let textures = &resources.bloom_textures[&bloom.downsample.clamp(1, 8)];
        let views = textures
            .each_ref()
            .map(|texture| texture.create_view(&Default::default()));
        let scale = [
            textures[0].width() as f32 / self.surface_config.width as f32,
            textures[0].height() as f32 / self.surface_config.height as f32,
        ];
        let full_bounds: PodBounds = quad.bounds.into();
        let reduced_bounds = PodBounds {
            origin: std::array::from_fn(|i| full_bounds.origin[i] * scale[i]),
            size: std::array::from_fn(|i| full_bounds.size[i] * scale[i]),
        };
        for (index, shader) in [&bloom.extract, &bloom.blur, &bloom.blur, &bloom.composite]
            .into_iter()
            .enumerate()
        {
            let format = if index == 3 {
                self.surface_config.format
            } else {
                wgpu::TextureFormat::Rgba16Float
            };
            let pipeline = resources.subtree_effect_pipelines[&(shader.id().as_u64(), format)]
                .as_ref()
                .unwrap();
            let output = match index {
                0 | 2 => &views[0],
                1 => &views[1],
                _ => destination,
            };
            let input = match index {
                1 => &views[0],
                2 => &views[1],
                _ => source,
            };
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("subtree_bloom_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: output,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                ..Default::default()
            });
            let mut instance = EffectInstance::from(quad);
            instance.opacity = 1.;
            instance.time = effect.time;
            instance.uniforms = *effect.uniforms.slots();
            instance.uniforms[2] = match index {
                0 => [0., 0., scale[0].recip(), scale[1].recip()],
                1 => [1., 0., 0., 0.],
                _ => [0., 1., 0., 0.],
            };
            instance.content_mask = full_bounds;
            instance.image_bounds = if index == 1 || index == 2 {
                reduced_bounds
            } else {
                full_bounds
            };
            instance.second_image_bounds = reduced_bounds;
            let data = bytemuck::bytes_of(&instance);
            let ok = if index == 3 {
                self.draw_instances_with_two_textures(
                    data,
                    1,
                    input,
                    &views[0],
                    pipeline,
                    instance_offset,
                    &mut pass,
                )
            } else {
                self.draw_instances_with_texture(
                    data,
                    1,
                    input,
                    pipeline,
                    instance_offset,
                    &mut pass,
                )
            };
            if !ok {
                return false;
            }
        }
        true
    }

    fn encode_scene(
        &mut self,
        scene: &Scene,
        target_texture: &wgpu::Texture,
        target_view: &wgpu::TextureView,
        encoder: &mut wgpu::CommandEncoder,
        load: wgpu::LoadOp<wgpu::Color>,
        retain_outputs: bool,
    ) -> anyhow::Result<SceneEncoding> {
        let mut encoded = self.encode_scene_inner(
            scene,
            target_texture,
            target_view,
            encoder,
            load,
            retain_outputs,
        );
        if matches!(encoded, Ok(SceneEncoding::InstanceCapacity))
            && self.instance_buffer_capacity >= self.max_buffer_size
        {
            encoded = Err(anyhow::anyhow!(
                "scene instance storage exceeds the device buffer limit of {} bytes",
                self.max_buffer_size
            ));
        }
        if !matches!(encoded, Ok(SceneEncoding::Complete)) {
            let error = match &encoded {
                Err(error) => {
                    let message = format!("{error:#}");
                    *self.last_error.lock().unwrap() = Some(message.clone());
                    message.into()
                }
                _ => "3D picking frame was not submitted".into(),
            };
            scene3d::fail_pick_captures(scene, error);
            for capture in &mut self.resources_mut().ui_captures {
                capture.invalidate_encoding();
            }
        }
        encoded
    }

    fn encode_scene_inner(
        &mut self,
        scene: &Scene,
        target_texture: &wgpu::Texture,
        target_view: &wgpu::TextureView,
        encoder: &mut wgpu::CommandEncoder,
        load: wgpu::LoadOp<wgpu::Color>,
        retain_outputs: bool,
    ) -> anyhow::Result<SceneEncoding> {
        let mut has_scene3d = false;
        scene.visit(&mut |scene| {
            has_scene3d |= scene
                .subtree_layers
                .iter()
                .any(|layer| layer.scene3d.is_some());
        });
        let scene3d_capabilities = if has_scene3d {
            match self.scene3d_support() {
                gpui::Scene3dSupport::Supported(capabilities) => Some(capabilities),
                gpui::Scene3dSupport::Unsupported(reason) => {
                    anyhow::bail!("3D viewport unavailable: {reason}");
                }
            }
        } else {
            None
        };
        if has_scene3d {
            let mut failure = None;
            scene.visit(&mut |scene| {
                for layer in &scene.subtree_layers {
                    if let Some(frame) = &layer.scene3d
                        && let Err(error) = crate::scene3d_renderer::validate_frame_settings(
                            frame,
                            self.resources().device.limits().max_texture_dimension_2d,
                            true,
                        )
                        .and_then(|()| {
                            #[cfg(not(target_family = "wasm"))]
                            scene3d::validate_material_devices(frame, &self.resources().device)?;
                            crate::scene3d_renderer::gpu_draws::validate_frame(
                                &self.resources().device,
                                frame,
                            )
                        })
                    {
                        failure = Some(error);
                    }
                }
            });
            if let Some(error) = failure {
                return Err(error);
            }
        }
        if !self.encode_ui_captures(scene, encoder, retain_outputs)? {
            return Ok(SceneEncoding::CaptureCapacity);
        }
        let format = self.surface_config.format;
        let viewport = [
            self.surface_config.width as f32,
            self.surface_config.height as f32,
        ];
        let mut has_particles = false;
        scene.visit(&mut |scene| {
            has_particles |= !scene.particles.is_empty()
                || scene.subtree_layers.iter().any(|layer| {
                    layer
                        .intermediate_effects
                        .iter()
                        .any(|effect| effect.particles.is_some())
                });
        });
        let mut has_fluid = false;
        let mut has_particle_transition = false;
        scene.visit(&mut |scene| {
            has_particle_transition |= scene.subtree_layers.iter().any(|layer| {
                layer
                    .intermediate_effects
                    .iter()
                    .any(|effect| effect.particle_transition.is_some())
            });
        });
        scene.visit(&mut |scene| has_fluid |= !scene.fluids.is_empty());
        {
            let atlas = self.atlas.clone();
            let output_budget = self.scene3d_output_budget.clone();
            let resources = self.resources_mut();
            if has_scene3d && resources.scene3d.is_none() {
                resources.scene3d = Some(scene3d::ViewportRenderer::new(
                    resources.capture_context.clone(),
                    format,
                    scene3d_capabilities.unwrap(),
                    output_budget,
                ));
            }
            if let Some(renderer) = &mut resources.scene3d {
                renderer.prepare(
                    &resources.device,
                    &resources.queue,
                    scene,
                    viewport[0] as u32,
                    viewport[1] as u32,
                    &atlas,
                    retain_outputs,
                )?;
            }
            if has_particle_transition && resources.particle_transition.is_none() {
                resources.particle_transition = Some(
                    particle_transition::ParticleTransitionRenderer::new(&resources.device, format),
                );
            }
            if has_fluid && resources.fluid.is_none() {
                resources.fluid = Some(fluid::FluidRenderer::new(&resources.device, format));
            }
            if let Some(fluid) = &mut resources.fluid {
                fluid.ensure(&resources.device, scene);
                fluid.encode(&resources.queue, scene, viewport, encoder);
            }
            if has_particles && resources.particles.is_none() {
                resources.particles =
                    Some(particles::ParticleRenderer::new(&resources.device, format));
            }
            if let Some(particles) = &mut resources.particles {
                particles.ensure(&resources.device, scene);
                particles.encode(&resources.queue, scene, viewport, encoder);
            }
        }
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

        for textures in self.resources().feedback_textures.values() {
            textures.pending.set(None);
        }
        Ok(
            if self.encode_scene_batches(
                scene,
                target_texture,
                target_view,
                encoder,
                load,
                &mut 0,
                0,
            ) {
                SceneEncoding::Complete
            } else {
                SceneEncoding::InstanceCapacity
            },
        )
    }

    fn commit_encoded_scene(&self, encoded: bool) {
        self.commit_ui_captures(encoded);
        if let Some(particles) = &self.resources().particles {
            particles.commit(encoded);
        }
        if let Some(fluid) = &self.resources().fluid {
            fluid.commit(encoded);
        }
        for textures in self.resources().feedback_textures.values() {
            if let Some(snapshot) = textures.pending.take()
                && encoded
            {
                textures.committed.set(Some(snapshot));
            }
        }
    }

    fn commit_scene3d_outputs(&self, submitted: bool) {
        if let Some(renderer) = &self.resources().scene3d {
            renderer.commit_outputs(submitted);
        }
        for capture in &self.resources().ui_captures {
            capture.commit_scene3d_outputs(submitted);
        }
    }

    fn retain_external_scene3d_uploads(&self) {
        if let Some(renderer) = &self.resources().scene3d {
            renderer.retain_external_uploads();
        }
        for capture in &self.resources().ui_captures {
            capture.retain_external_scene3d_uploads();
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn encode_scene_batches(
        &self,
        scene: &Scene,
        target_texture: &wgpu::Texture,
        target_view: &wgpu::TextureView,
        encoder: &mut wgpu::CommandEncoder,
        load: wgpu::LoadOp<wgpu::Color>,
        instance_offset: &mut u64,
        depth: usize,
    ) -> bool {
        let mut overflow = false;

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("main_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load,
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                ..Default::default()
            });

            for batch in scene.batches() {
                let ok = match batch {
                    PrimitiveBatch::SubtreeLayers(range) => {
                        drop(pass);
                        let texture = &self.resources().subtree_textures[depth];
                        let capture_view =
                            texture.create_view(&wgpu::TextureViewDescriptor::default());
                        let mut did_draw = true;
                        for layer in &scene.subtree_layers[range] {
                            let pipeline = self
                                .resources()
                                .subtree_effect_pipelines
                                .get(&(
                                    layer.composite.shader.id().as_u64(),
                                    self.surface_config.format,
                                ))
                                .and_then(Option::as_ref);
                            let Some(pipeline) = pipeline else {
                                did_draw &= self.encode_scene_batches(
                                    &layer.scene,
                                    target_texture,
                                    target_view,
                                    encoder,
                                    wgpu::LoadOp::Load,
                                    instance_offset,
                                    depth,
                                );
                                continue;
                            };
                            let captured = self
                                .resources()
                                .ui_capture_indices
                                .get(&(layer as *const _ as usize))
                                .map(|index| &self.resources().ui_captures[*index].texture);
                            if captured.is_none()
                                && !self.encode_scene_batches(
                                    &layer.scene,
                                    texture,
                                    &capture_view,
                                    encoder,
                                    wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                                    instance_offset,
                                    depth + 1,
                                )
                            {
                                did_draw = false;
                                break;
                            }
                            let mut source_view = captured
                                .unwrap_or(texture)
                                .create_view(&wgpu::TextureViewDescriptor::default());
                            let second_view = if let Some(second) = &layer.second_scene {
                                assert!(
                                    layer.intermediate_effects.is_empty(),
                                    "two-input captures cannot have intermediate passes"
                                );
                                let second_texture = &self.resources().subtree_textures[depth + 1];
                                let view = second_texture
                                    .create_view(&wgpu::TextureViewDescriptor::default());
                                if !self.encode_scene_batches(
                                    second,
                                    second_texture,
                                    &view,
                                    encoder,
                                    wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                                    instance_offset,
                                    depth + 2,
                                ) {
                                    did_draw = false;
                                    break;
                                }
                                Some(view)
                            } else {
                                None
                            };
                            let mut source_index = depth;
                            for effect in layer.intermediate_effects.iter() {
                                let pipelines = if effect.images.is_empty() {
                                    &self.resources().subtree_effect_pipelines
                                } else {
                                    &self.resources().subtree_image_effect_pipelines
                                };
                                let Some(effect_pipeline) = pipelines
                                    .get(&(effect.shader.id().as_u64(), self.surface_config.format))
                                    .and_then(Option::as_ref)
                                else {
                                    continue;
                                };
                                let destination_index = if source_index == depth {
                                    depth + 1
                                } else {
                                    depth
                                };
                                let destination_view = self.resources().subtree_textures
                                    [destination_index]
                                    .create_view(&wgpu::TextureViewDescriptor::default());
                                if let Some(feedback) = &effect.feedback {
                                    let Some(pipeline) = self
                                        .resources()
                                        .subtree_effect_pipelines
                                        .get(&(
                                            feedback.shader.id().as_u64(),
                                            wgpu::TextureFormat::Rgba16Float,
                                        ))
                                        .and_then(Option::as_ref)
                                    else {
                                        continue;
                                    };
                                    if !self.encode_subtree_feedback(
                                        &layer.composite,
                                        effect,
                                        feedback,
                                        &source_view,
                                        &destination_view,
                                        effect_pipeline,
                                        pipeline,
                                        encoder,
                                        instance_offset,
                                    ) {
                                        did_draw = false;
                                        break;
                                    }
                                    source_view = destination_view;
                                    source_index = destination_index;
                                    continue;
                                }
                                if let Some(field) = &effect.distance_field {
                                    let region = distance_field::region(
                                        layer.composite.bounds,
                                        [self.surface_config.width, self.surface_config.height],
                                    );
                                    if region[2] == 0 || region[3] == 0 {
                                        continue;
                                    }
                                    let Some(pipeline) = self
                                        .resources()
                                        .subtree_effect_pipelines
                                        .get(&(
                                            field.composite.id().as_u64(),
                                            self.surface_config.format,
                                        ))
                                        .and_then(Option::as_ref)
                                    else {
                                        continue;
                                    };
                                    if !self.encode_subtree_distance_field(
                                        &layer.composite,
                                        effect,
                                        field,
                                        &source_view,
                                        &destination_view,
                                        pipeline,
                                        encoder,
                                        instance_offset,
                                    ) {
                                        did_draw = false;
                                        break;
                                    }
                                    source_view = destination_view;
                                    source_index = destination_index;
                                    continue;
                                }
                                if let Some(bloom) = &effect.bloom {
                                    if [
                                        (&bloom.extract, wgpu::TextureFormat::Rgba16Float),
                                        (&bloom.blur, wgpu::TextureFormat::Rgba16Float),
                                        (&bloom.composite, self.surface_config.format),
                                    ]
                                    .iter()
                                    .any(|(shader, format)| {
                                        self.resources()
                                            .subtree_effect_pipelines
                                            .get(&(shader.id().as_u64(), *format))
                                            .and_then(Option::as_ref)
                                            .is_none()
                                    }) {
                                        continue;
                                    }
                                    if !self.encode_subtree_bloom(
                                        &layer.composite,
                                        effect,
                                        bloom,
                                        &source_view,
                                        &destination_view,
                                        encoder,
                                        instance_offset,
                                    ) {
                                        did_draw = false;
                                        break;
                                    }
                                    source_view = destination_view;
                                    source_index = destination_index;
                                    continue;
                                }
                                if let Some(transition) = &effect.particle_transition
                                    && transition.progress > 0.
                                {
                                    let resources = self.resources();
                                    resources.particle_transition.as_ref().unwrap().encode(
                                        &resources.device,
                                        &layer.composite,
                                        transition,
                                        &source_view,
                                        &destination_view,
                                        [
                                            self.surface_config.width as f32,
                                            self.surface_config.height as f32,
                                        ],
                                        encoder,
                                    );
                                    source_view = destination_view;
                                    source_index = destination_index;
                                    continue;
                                }
                                let particle_draw = effect.particles.as_ref().map(|particles| {
                                    let draw = particles::ParticleRenderer::masked_draw(
                                        &layer.composite,
                                        particles,
                                    );
                                    let resources = self.resources();
                                    resources.particles.as_ref().unwrap().encode_masked(
                                        &resources.device,
                                        &resources.queue,
                                        &draw,
                                        particles.mask,
                                        &source_view,
                                        [
                                            self.surface_config.width as f32,
                                            self.surface_config.height as f32,
                                        ],
                                        encoder,
                                    );
                                    draw
                                });
                                {
                                    let mut effect_pass =
                                        encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                                            label: Some("subtree_effect_pass"),
                                            color_attachments: &[Some(
                                                wgpu::RenderPassColorAttachment {
                                                    view: &destination_view,
                                                    resolve_target: None,
                                                    ops: wgpu::Operations {
                                                        load: wgpu::LoadOp::Clear(
                                                            wgpu::Color::TRANSPARENT,
                                                        ),
                                                        store: wgpu::StoreOp::Store,
                                                    },
                                                    depth_slice: None,
                                                },
                                            )],
                                            ..Default::default()
                                        });
                                    let mut quad = layer.composite.clone();
                                    quad.shader = effect.shader.clone();
                                    quad.uniforms = effect.uniforms;
                                    quad.time = effect.time;
                                    quad.opacity = 1.;
                                    quad.content_mask.bounds = quad.bounds;
                                    quad.second_image_tile = effect.images.first().copied();
                                    quad.third_image_tile = effect.images.get(1).copied();
                                    quad.fourth_image_tile = effect.images.get(2).copied();
                                    let mut instance = EffectInstance::from(&quad);
                                    instance.image_bounds = quad.bounds.into();
                                    let drawn = if effect.images.len() == 3 {
                                        let textures = [0, 1, 2].map(|index| {
                                            self.atlas
                                                .get_texture_info(effect.images[index].texture_id)
                                        });
                                        self.draw_instances_with_four_textures(
                                            bytemuck::bytes_of(&instance),
                                            1,
                                            [
                                                &source_view,
                                                &textures[0].view,
                                                &textures[1].view,
                                                &textures[2].view,
                                            ],
                                            effect_pipeline,
                                            instance_offset,
                                            &mut effect_pass,
                                        )
                                    } else if let Some(tile) = effect.images.first() {
                                        let texture = self.atlas.get_texture_info(tile.texture_id);
                                        self.draw_instances_with_two_textures(
                                            bytemuck::bytes_of(&instance),
                                            1,
                                            &source_view,
                                            &texture.view,
                                            effect_pipeline,
                                            instance_offset,
                                            &mut effect_pass,
                                        )
                                    } else {
                                        self.draw_instances_with_texture(
                                            bytemuck::bytes_of(&instance),
                                            1,
                                            &source_view,
                                            effect_pipeline,
                                            instance_offset,
                                            &mut effect_pass,
                                        )
                                    };
                                    if !drawn {
                                        did_draw = false;
                                        break;
                                    }
                                    if let Some(draw) = &particle_draw {
                                        self.resources()
                                            .particles
                                            .as_ref()
                                            .unwrap()
                                            .draw(draw, &mut effect_pass);
                                    }
                                }
                                source_view = destination_view;
                                source_index = destination_index;
                            }
                            if !did_draw {
                                break;
                            }
                            if layer.scene3d.is_some() {
                                let destination = self.resources().subtree_textures[depth + 1]
                                    .create_view(&Default::default());
                                let resources = self.resources();
                                resources.scene3d.as_ref().unwrap().encode(
                                    &resources.device,
                                    &resources.queue,
                                    &self.atlas,
                                    layer,
                                    &source_view,
                                    &resources.subtree_textures[depth + 1],
                                    encoder,
                                );
                                source_view = destination;
                            }
                            let mut composite_pass =
                                encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                                    label: Some("subtree_composite"),
                                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                                        view: target_view,
                                        resolve_target: None,
                                        ops: wgpu::Operations {
                                            load: wgpu::LoadOp::Load,
                                            store: wgpu::StoreOp::Store,
                                        },
                                        depth_slice: None,
                                    })],
                                    ..Default::default()
                                });
                            let mut instance = EffectInstance::from(&layer.composite);
                            instance.image_bounds = layer.composite.bounds.into();
                            did_draw &= if let Some(second_view) = &second_view {
                                instance.second_image_bounds = layer.composite.bounds.into();
                                self.draw_instances_with_two_textures(
                                    bytemuck::bytes_of(&instance),
                                    1,
                                    &source_view,
                                    second_view,
                                    pipeline,
                                    instance_offset,
                                    &mut composite_pass,
                                )
                            } else {
                                self.draw_instances_with_texture(
                                    bytemuck::bytes_of(&instance),
                                    1,
                                    &source_view,
                                    pipeline,
                                    instance_offset,
                                    &mut composite_pass,
                                )
                            };
                        }
                        pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                            label: Some("main_pass_after_subtree"),
                            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                                view: target_view,
                                resolve_target: None,
                                ops: wgpu::Operations {
                                    load: wgpu::LoadOp::Load,
                                    store: wgpu::StoreOp::Store,
                                },
                                depth_slice: None,
                            })],
                            ..Default::default()
                        });
                        did_draw
                    }
                    PrimitiveBatch::BackdropBlurs(range) => {
                        if !self.backdrop_blur_supported {
                            true
                        } else {
                            drop(pass);
                            let did_draw = self.draw_backdrop_blurs(
                                &scene.backdrop_blurs[range],
                                target_texture,
                                target_view,
                                encoder,
                                instance_offset,
                            );
                            pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                                label: Some("main_pass_after_backdrop"),
                                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                                    view: target_view,
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
                            did_draw
                        }
                    }
                    PrimitiveBatch::Quads(range) => {
                        self.draw_quads(&scene.quads[range], instance_offset, &mut pass)
                    }
                    PrimitiveBatch::Effects(range) => {
                        self.draw_effects(&scene.effects[range], instance_offset, &mut pass)
                    }
                    PrimitiveBatch::Particles(range) => {
                        if let Some(particles) = &self.resources().particles {
                            for draw in &scene.particles[range] {
                                particles.draw(draw, &mut pass);
                            }
                        }
                        true
                    }
                    PrimitiveBatch::Fluids(range) => {
                        if let Some(fluid) = &self.resources().fluid {
                            for draw in &scene.fluids[range] {
                                fluid.draw(draw, &mut pass);
                            }
                        }
                        true
                    }
                    PrimitiveBatch::Shadows(range) => {
                        self.draw_shadows(&scene.shadows[range], instance_offset, &mut pass)
                    }
                    PrimitiveBatch::Paths(range) => {
                        let paths = &scene.paths[range];
                        if paths.is_empty() {
                            continue;
                        }

                        drop(pass);

                        let did_draw =
                            self.draw_paths_to_intermediate(encoder, paths, instance_offset);

                        pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                            label: Some("main_pass_continued"),
                            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                                view: target_view,
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
                            self.draw_paths_from_intermediate(paths, instance_offset, &mut pass)
                        } else {
                            false
                        }
                    }
                    PrimitiveBatch::Underlines(range) => {
                        self.draw_underlines(&scene.underlines[range], instance_offset, &mut pass)
                    }
                    PrimitiveBatch::MonochromeSprites { texture_id, range } => self
                        .draw_monochrome_sprites(
                            &scene.monochrome_sprites[range],
                            texture_id,
                            instance_offset,
                            &mut pass,
                        ),
                    PrimitiveBatch::SubpixelSprites { texture_id, range } => self
                        .draw_subpixel_sprites(
                            &scene.subpixel_sprites[range],
                            texture_id,
                            instance_offset,
                            &mut pass,
                        ),
                    PrimitiveBatch::PolychromeSprites { texture_id, range } => self
                        .draw_polychrome_sprites(
                            &scene.polychrome_sprites[range],
                            texture_id,
                            instance_offset,
                            &mut pass,
                        ),
                    PrimitiveBatch::Surfaces(range) => {
                        self.draw_surfaces(&scene.surfaces[range], instance_offset, &mut pass)
                    }
                };
                if !ok {
                    overflow = true;
                    break;
                }
            }
        }

        !overflow
    }

    fn prepare_surfaces(&mut self, scene: &Scene) {
        let mut frames = HashMap::<SurfaceId, Arc<SurfaceFrame>>::new();
        scene.visit(&mut |scene| {
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
        });

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
                        offset: plane.offset() as u64,
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

    fn draw_backdrop_blurs(
        &self,
        backdrops: &[BackdropBlur],
        frame_texture: &wgpu::Texture,
        frame_view: &wgpu::TextureView,
        encoder: &mut wgpu::CommandEncoder,
        instance_offset: &mut u64,
    ) -> bool {
        let resources = self.resources();
        let (Some(source_texture), Some(source_view), Some(horizontal_view), Some(result_view)) = (
            resources.backdrop_source_texture.as_ref(),
            resources.backdrop_source_view.as_ref(),
            resources.backdrop_horizontal_view.as_ref(),
            resources.backdrop_result_view.as_ref(),
        ) else {
            return true;
        };
        let viewport_size = [
            self.surface_config.width as f32,
            self.surface_config.height as f32,
        ];
        let blur_size = [
            self.surface_config.width.div_ceil(2) as f32,
            self.surface_config.height.div_ceil(2) as f32,
        ];
        let extent = wgpu::Extent3d {
            width: self.surface_config.width,
            height: self.surface_config.height,
            depth_or_array_layers: 1,
        };

        for backdrop in backdrops {
            encoder.copy_texture_to_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: frame_texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyTextureInfo {
                    texture: source_texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                extent,
            );

            let horizontal = BackdropInstance::blur(
                viewport_size,
                blur_size,
                backdrop.blur_radius.0.min(64.0),
                [1.0, 0.0],
            );
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("backdrop_horizontal_pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: horizontal_view,
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
                if !self.draw_instances_with_texture(
                    bytemuck::bytes_of(&horizontal),
                    1,
                    source_view,
                    &resources.pipelines.backdrop_blur,
                    instance_offset,
                    &mut pass,
                ) {
                    return false;
                }
            }

            let vertical = BackdropInstance::blur(
                viewport_size,
                blur_size,
                backdrop.blur_radius.0.min(64.0),
                [0.0, 1.0],
            );
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("backdrop_vertical_pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: result_view,
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
                if !self.draw_instances_with_texture(
                    bytemuck::bytes_of(&vertical),
                    1,
                    horizontal_view,
                    &resources.pipelines.backdrop_blur,
                    instance_offset,
                    &mut pass,
                ) {
                    return false;
                }
            }

            let composite = BackdropInstance::from(backdrop);
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("backdrop_composite_pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: frame_view,
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
                let did_draw = if let Some(shader) = backdrop.shader.as_ref() {
                    if let Some(pipeline) = resources
                        .backdrop_effect_pipelines
                        .get(&shader.id().as_u64())
                    {
                        self.draw_instances_with_two_textures(
                            bytemuck::bytes_of(&composite),
                            1,
                            source_view,
                            result_view,
                            pipeline,
                            instance_offset,
                            &mut pass,
                        )
                    } else {
                        self.draw_instances_with_texture(
                            bytemuck::bytes_of(&composite),
                            1,
                            result_view,
                            &resources.pipelines.backdrop_composite,
                            instance_offset,
                            &mut pass,
                        )
                    }
                } else {
                    self.draw_instances_with_texture(
                        bytemuck::bytes_of(&composite),
                        1,
                        result_view,
                        &resources.pipelines.backdrop_composite,
                        instance_offset,
                        &mut pass,
                    )
                };
                if !did_draw {
                    return false;
                }
            }
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
            res.surface = Some(surface);

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

        let output_budget = self.scene3d_output_budget.clone();
        *self = Self::new_internal(
            Some(gpu_context.clone()),
            context,
            surface,
            config,
            self.compositor_gpu,
            self.atlas.clone(),
        )?;
        self.scene3d_output_budget = output_budget;

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
