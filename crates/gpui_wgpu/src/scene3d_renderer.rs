use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc,
};

use anyhow::{Context as _, Result, ensure};
use gpui::{MeshTexture3d, Scene3dFrame};

use crate::{
    WgpuAtlas, WgpuContext,
    wgpu_renderer::scene3d::{RenderRegion, Scene3dRenderer},
};

mod statistics;
pub use statistics::Scene3dDrawStatistics;
mod capabilities;
pub use capabilities::{Scene3dDeviceCapabilities, Scene3dFormatCapabilities};
mod memory;
pub use memory::Scene3dTargetMemory;
mod readback;
pub use readback::{Scene3dReadbackConfig, Scene3dReadbackMemory};

bitflags::bitflags! {
    /// Independently selectable outputs. Non-color channels use the pixel center.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct Scene3dChannels: u8 {
        const COLOR = 1;
        const OBJECT_ID = 2;
        const LINEAR_DEPTH = 4;
        const WORLD_NORMAL = 8;
        const LINEAR_COLOR = 16;
    }
}
impl Default for Scene3dChannels {
    fn default() -> Self {
        Self::COLOR | Self::OBJECT_ID
    }
}
impl Scene3dChannels {
    fn shaded(self) -> bool {
        self.intersects(Self::COLOR | Self::LINEAR_COLOR)
    }
    fn color(self) -> bool {
        self.contains(Self::COLOR)
    }
    fn ids(self) -> bool {
        self.contains(Self::OBJECT_ID)
    }
}

/// Physical output dimensions and color sampling. Color is transparent unless
/// an environment background is configured.
/// COLOR uses display-encoded RGBA8 after exposure and tone
/// mapping; LINEAR_COLOR uses premultiplied RGBA16Float before display mapping.
#[derive(Clone, Copy, Debug)]
pub struct Scene3dOutputConfig {
    pub size: [u32; 2],
    pub channels: Scene3dChannels,
    /// One or four samples. Non-color channels always use the pixel-center sample.
    pub color_samples: u32,
}
impl Scene3dOutputConfig {
    pub fn new(size: [u32; 2]) -> Self {
        Self {
            size,
            channels: Scene3dChannels::default(),
            color_samples: 4,
        }
    }
}

/// Limits for direct 3D rendering. At most one CPU readback is pending per renderer.
#[derive(Clone, Copy, Debug)]
pub struct Scene3dCapabilities {
    pub max_dimension: u32,
    pub max_pixels: u64,
    pub color_msaa4: bool,
    /// Four-sample linear color resolve support.
    pub linear_color_msaa4: bool,
    /// Maximum size of each staging buffer, including padded rows.
    pub max_readback_buffer_bytes: u64,
    pub geometry_outputs: bool,
}
impl Scene3dCapabilities {
    /// Channels exposed by this renderer, including the joint depth/normal path.
    pub fn channels(self) -> Scene3dChannels {
        let mut channels =
            Scene3dChannels::COLOR | Scene3dChannels::LINEAR_COLOR | Scene3dChannels::OBJECT_ID;
        if self.geometry_outputs {
            channels |= Scene3dChannels::LINEAR_DEPTH | Scene3dChannels::WORLD_NORMAL;
        }
        channels
    }

    /// Color sample counts accepted for a channel selection. Geometry channels
    /// remain pixel-center sampled regardless of the chosen color sample count.
    pub fn color_sample_counts(self, channels: Scene3dChannels) -> &'static [u32] {
        if channels.is_empty() || !self.channels().contains(channels) {
            &[]
        } else if (channels.color() && !self.color_msaa4)
            || (channels.contains(Scene3dChannels::LINEAR_COLOR) && !self.linear_color_msaa4)
        {
            &[1]
        } else {
            &[1, 4]
        }
    }

    pub fn validate(self, config: Scene3dOutputConfig) -> Result<()> {
        ensure!(
            !config.channels.is_empty() && Scene3dChannels::all().contains(config.channels),
            "3D outputs must select known channels"
        );
        ensure!(
            self.channels().contains(config.channels),
            "3D output channels {:?} are unavailable on this device",
            config.channels - self.channels()
        );
        let [width, height] = config.size;
        ensure!(
            width > 0 && height > 0 && width <= self.max_dimension && height <= self.max_dimension,
            "3D output dimensions must be positive and at most {}",
            self.max_dimension
        );
        ensure!(
            u64::from(width) * u64::from(height) <= self.max_pixels,
            "3D output exceeds the pixel budget"
        );
        ensure!(
            config.color_samples == 1 || config.color_samples == 4,
            "3D color sampling must be 1 or 4"
        );
        ensure!(
            !config.channels.color() || config.color_samples == 1 || self.color_msaa4,
            "4x color MSAA is unavailable on this device"
        );
        ensure!(
            !config.channels.contains(Scene3dChannels::LINEAR_COLOR)
                || config.color_samples == 1
                || self.linear_color_msaa4,
            "4x linear color resolve is unavailable on this device"
        );
        let bytes_per_pixel = if config.channels.contains(Scene3dChannels::WORLD_NORMAL) {
            16
        } else if config.channels.contains(Scene3dChannels::LINEAR_COLOR) {
            8
        } else {
            4
        };
        let stride = readback_stride(width, bytes_per_pixel);
        ensure!(
            stride <= u64::from(u32::MAX)
                && stride * u64::from(height) <= self.max_readback_buffer_bytes,
            "3D output exceeds the readback buffer limit"
        );
        Ok(())
    }
}

/// Direct mesh rendering on an owned or shared GPU context, without GPUI layout.
/// Uses the same mesh pass and material shader as GPUI viewports.
pub struct WgpuScene3dRenderer {
    context: WgpuContext,
    atlas: Arc<WgpuAtlas>,
    color: Option<(u32, Scene3dRenderer)>,
    ids: Option<Scene3dRenderer>,
    depth: Option<Scene3dRenderer>,
    normals: Option<Scene3dRenderer>,
    capabilities: Scene3dCapabilities,
    device_capabilities: Scene3dDeviceCapabilities,
    readback_busy: Arc<AtomicBool>,
    target_byte_limit: Option<u64>,
}
impl WgpuScene3dRenderer {
    #[cfg(not(target_family = "wasm"))]
    pub fn new_headless() -> Result<Self> {
        Self::new(WgpuContext::new_headless()?)
    }

    pub fn new(context: WgpuContext) -> Result<Self> {
        let device_capabilities = Scene3dDeviceCapabilities::query(&context);
        let capabilities = device_capabilities.rendering()?;
        Ok(Self {
            atlas: Arc::new(WgpuAtlas::from_context(&context)),
            context,
            color: None,
            ids: None,
            depth: None,
            normals: None,
            capabilities,
            device_capabilities,
            readback_busy: Arc::new(AtomicBool::new(false)),
            target_byte_limit: None,
        })
    }
    pub fn context(&self) -> &WgpuContext {
        &self.context
    }
    pub fn sprite_atlas(&self) -> &Arc<WgpuAtlas> {
        &self.atlas
    }
    pub fn capabilities(&self) -> Scene3dCapabilities {
        self.capabilities
    }

    pub fn device_capabilities(&self) -> &Scene3dDeviceCapabilities {
        &self.device_capabilities
    }

    /// Optional per-request target payload limit; `None` is unlimited (the default).
    pub fn target_byte_limit(&self) -> Option<u64> {
        self.target_byte_limit
    }

    /// Limits output, attachment, and shadow texture payload for future requests.
    /// Zero rejects every render. Does not release existing resources, poll the
    /// device, or bound total GPU residency. Use `clear_caches` to release caches.
    pub fn set_target_byte_limit(&mut self, bytes: Option<u64>) {
        self.target_byte_limit = bytes;
    }

    /// Checks device output limits and this renderer's per-request target budget
    /// before resource uploads. Does not validate scene content or access the GPU.
    pub fn validate_target_memory(
        &self,
        config: Scene3dOutputConfig,
        shadow_resolution: Option<u32>,
    ) -> Result<Scene3dTargetMemory> {
        self.capabilities.validate(config)?;
        ensure!(
            shadow_resolution.is_none_or(|size| size <= self.capabilities.max_dimension),
            "shadow resolution exceeds device limits"
        );
        let memory = config.target_memory(shadow_resolution)?;
        if let Some(limit) = self.target_byte_limit {
            ensure!(
                memory.total_bytes <= limit,
                "3D target request requires {} bytes, exceeding the {} byte limit",
                memory.total_bytes,
                limit
            );
        }
        Ok(memory)
    }

    /// Releases renderer-owned mesh resources, targets, and pipelines.
    /// Subsequent renders rebuild them lazily. Atlas allocations, returned
    /// outputs, and pending readbacks remain valid. Does not wait for the GPU.
    pub fn clear_caches(&mut self) {
        self.color = None;
        self.ids = None;
        self.depth = None;
        self.normals = None;
    }

    /// Maximum instances in one draw batch for this device's buffer limits.
    pub fn max_instances_per_batch(&self) -> usize {
        crate::wgpu_renderer::scene3d::instance_limit(&self.context.device)
    }

    /// Submits a frame and returns owned GPU outputs. Image atlas tiles must come
    /// from this renderer. UI subtree textures are not supported by this entry point.
    pub fn render(
        &mut self,
        frame: &Scene3dFrame,
        config: Scene3dOutputConfig,
    ) -> Result<Scene3dGpuOutput> {
        let target_memory = self.validate_target_memory(
            config,
            frame.directional_shadow.map(|shadow| shadow.resolution),
        )?;
        ensure!(
            frame.shadow_is_valid(),
            "invalid directional shadow parameters or source"
        );
        if let Some(lights) = &frame.lights {
            ensure!(
                lights.len() <= gpui::MAX_PUNCTUAL_LIGHTS_3D,
                "too many direct lights"
            );
            for (index, light) in lights.iter().enumerate() {
                ensure!(
                    light.is_valid(),
                    "direct light {index} has invalid parameters"
                );
            }
        }
        ensure!(
            frame
                .diffuse_environment
                .is_none_or(|environment| environment.is_valid()),
            "invalid diffuse environment parameters"
        );
        ensure!(!self.context.device_lost(), "3D rendering device is lost");
        if config.channels.shaded()
            && let Some(environment) = &frame.specular_environment
        {
            ensure!(
                environment.is_valid(),
                "invalid specular environment parameters"
            );
            ensure!(
                environment.map.size() <= self.capabilities.max_dimension,
                "specular environment exceeds device texture dimensions"
            );
        }
        if config.channels.shaded()
            && let Some(background) = &frame.background
        {
            ensure!(
                background.is_valid(),
                "invalid environment background parameters"
            );
            ensure!(
                background
                    .map
                    .size()
                    .iter()
                    .all(|v| *v <= self.capabilities.max_dimension),
                "environment map exceeds device texture dimensions"
            );
        }
        ensure!(
            frame.color_output.is_valid(),
            "3D exposure must be finite and between -16 and 16 stops"
        );
        ensure!(
            frame
                .objects
                .iter()
                .all(|object| !matches!(object.texture, MeshTexture3d::Subtree)),
            "direct 3D rendering does not capture UI textures"
        );
        ensure!(
            frame
                .view_projection
                .iter()
                .flatten()
                .chain(frame.world_to_view.iter().flatten())
                .chain(&frame.camera_position)
                .chain(frame.orthographic_view_direction.iter().flatten())
                .chain(&frame.light_direction)
                .chain(&frame.light)
                .chain([&frame.ambient])
                .all(|value| value.is_finite()),
            "3D frame contains non-finite camera or light parameters"
        );
        ensure!(
            frame
                .orthographic_view_direction
                .is_none_or(|direction| direction.iter().any(|v| *v != 0.)),
            "3D orthographic view direction must be nonzero"
        );
        for object in frame.objects.iter() {
            for set in object.texture_uv_sets() {
                ensure!(
                    object.mesh.uv_at(set, 0).is_some(),
                    "3D object {}: missing UV set {set}",
                    object.output_id
                );
            }
            ensure!(
                !matches!(object.texture, gpui::MeshTexture3d::Image(_))
                    || object.sampling.is_valid(),
                "3D object {} has invalid image sampling",
                object.output_id
            );
            for (map, active) in [
                (
                    object.metallic_roughness_texture,
                    object.pbr.is_some() && !object.unlit,
                ),
                (
                    object.emissive_texture,
                    object.pbr.is_some() && !object.unlit,
                ),
                (
                    object.normal_texture,
                    object.pbr.is_some() && !object.unlit && object.normal_scale > 0.,
                ),
                (
                    object.occlusion_texture,
                    !object.unlit && object.occlusion_strength > 0.,
                ),
            ] {
                ensure!(
                    !active || map.is_none_or(|map| map.sampling.is_valid()),
                    "3D object {} has invalid material-map sampling",
                    object.output_id
                );
            }
            ensure!(
                object.sort_depth.is_finite(),
                "3D object {} has invalid sort depth",
                object.output_id
            );
            ensure!(
                object.occlusion_strength.is_finite()
                    && (0. ..=1.).contains(&object.occlusion_strength),
                "object {} has invalid occlusion strength",
                object.output_id
            );
            ensure!(
                object.normal_scale.is_finite() && object.normal_scale >= 0.,
                "3D object {} has invalid normal scale",
                object.output_id
            );
            ensure!(
                object.normal_texture.is_none()
                    || object.pbr.is_none()
                    || object.unlit
                    || object.normal_scale == 0.
                    || object.mesh.tangent_uv_set() == object.normal_texture.map(|map| map.uv_set),
                "3D object {}: normal maps require mesh tangents for the selected UV set",
                object.output_id
            );
            ensure!(
                object.pbr.is_none_or(|pbr| pbr.is_valid()),
                "3D object {} has invalid PBR parameters",
                object.output_id
            );
            ensure!(
                object
                    .model
                    .iter()
                    .flatten()
                    .chain(object.normal.iter().flatten())
                    .chain([
                        &object.color.r,
                        &object.color.g,
                        &object.color.b,
                        &object.color.a,
                        &object.alpha_cutoff
                    ])
                    .all(|value| value.is_finite()),
                "3D object {} contains non-finite parameters",
                object.output_id
            );
            ensure!(
                object.alpha_cutoff >= 0.,
                "3D object {} has a negative alpha cutoff",
                object.output_id
            );
            ensure!(
                object.output_id != 0 || !config.channels.ids(),
                "zero is reserved for the ID background"
            );
        }
        let [width, height] = config.size;
        let stride = (u64::from(width) * 4).div_ceil(256) * 256;
        ensure!(
            stride * u64::from(height) <= self.context.device.limits().max_buffer_size,
            "3D output exceeds the device readback buffer limit"
        );
        let device = &self.context.device;
        let queue = &self.context.queue;
        let mut primary = true;
        let mut plan_source = None;
        for (enabled, renderer) in [
            (
                config.channels.shaded(),
                self.color.as_mut().map(|(_, renderer)| renderer),
            ),
            (config.channels.ids(), self.ids.as_mut()),
            (
                config.channels.contains(Scene3dChannels::LINEAR_DEPTH),
                self.depth.as_mut(),
            ),
            (
                config.channels.contains(Scene3dChannels::WORLD_NORMAL),
                self.normals.as_mut(),
            ),
        ] {
            let retain_geometry = !enabled || !primary;
            if enabled {
                primary = false;
            }
            if let Some(renderer) = renderer {
                if enabled && let Some(source) = plan_source {
                    renderer.reuse_plans_from(source);
                }
                renderer.prepare_frame_retention(enabled.then_some(frame), retain_geometry);
                if enabled {
                    plan_source = Some(renderer);
                }
            }
        }
        self.atlas.before_frame();
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("scene3d.direct"),
        });
        let region = RenderRegion::full(config.size);
        let mut draw_statistics = Scene3dDrawStatistics::default();
        let mut linear_color = None;
        let color = if config.channels.shaded() {
            if self
                .color
                .as_ref()
                .is_none_or(|(samples, _)| *samples != config.color_samples)
            {
                self.color = Some((
                    config.color_samples,
                    Scene3dRenderer::new(
                        device,
                        queue,
                        wgpu::TextureFormat::Rgba8Unorm,
                        config.color_samples,
                    ),
                ));
            }
            let renderer = &mut self.color.as_mut().unwrap().1;
            renderer.prepare_frames(device, queue, [frame], [config.size]);
            draw_statistics += renderer.draw_statistics(frame);
            let texture = config
                .channels
                .color()
                .then(|| output_texture(device, config.size, wgpu::TextureFormat::Rgba8Unorm));
            let view = texture
                .as_ref()
                .map(|texture| texture.create_view(&Default::default()));
            renderer.encode_frame(
                device,
                queue,
                &self.atlas,
                frame,
                region,
                0,
                None,
                view.as_ref(),
                &mut encoder,
            );
            if config.channels.contains(Scene3dChannels::LINEAR_COLOR) {
                let output = output_texture(device, config.size, wgpu::TextureFormat::Rgba16Float);
                renderer.copy_linear_color(&output, &mut encoder);
                linear_color = Some(output);
            }
            texture
        } else {
            self.color = None;
            None
        };
        let (mut ids, mut depth, mut normals) = (None, None, None);
        let mut resource_source = self.color.as_ref().map(|(_, renderer)| renderer);
        for (kind, cache, output) in [
            (OutputKind::ObjectId, &mut self.ids, &mut ids),
            (OutputKind::LinearDepth, &mut self.depth, &mut depth),
            (OutputKind::WorldNormal, &mut self.normals, &mut normals),
        ] {
            if !config.channels.contains(kind.channel()) {
                *cache = None;
                continue;
            }
            let renderer =
                cache.get_or_insert_with(|| Scene3dRenderer::new(device, queue, kind.format(), 1));
            if let Some(source) = resource_source {
                renderer.reuse_resources_from(source);
            }
            renderer.prepare_frames(device, queue, [frame], [config.size]);
            draw_statistics += renderer.draw_statistics(frame);
            let texture = output_texture(device, config.size, kind.format());
            renderer.encode_frame(
                device,
                queue,
                &self.atlas,
                frame,
                region,
                0,
                None,
                Some(&texture.create_view(&Default::default())),
                &mut encoder,
            );
            *output = Some(texture);
            resource_source = Some(renderer);
        }
        self.context.queue.submit([encoder.finish()]);
        Ok(Scene3dGpuOutput {
            depth_background: frame.depth_background,
            context: self.context.clone(),
            draw_statistics,
            config,
            color,
            ids,
            depth,
            normals,
            linear_color,
            readback_busy: self.readback_busy.clone(),
            target_memory,
        })
    }
}

#[derive(Clone, Copy)]
enum OutputKind {
    Color,
    LinearColor,
    ObjectId,
    LinearDepth,
    WorldNormal,
}
impl OutputKind {
    fn channel(self) -> Scene3dChannels {
        match self {
            Self::Color => Scene3dChannels::COLOR,
            Self::LinearColor => Scene3dChannels::LINEAR_COLOR,
            Self::ObjectId => Scene3dChannels::OBJECT_ID,
            Self::LinearDepth => Scene3dChannels::LINEAR_DEPTH,
            Self::WorldNormal => Scene3dChannels::WORLD_NORMAL,
        }
    }
    fn format(self) -> wgpu::TextureFormat {
        match self {
            Self::Color => wgpu::TextureFormat::Rgba8Unorm,
            Self::LinearColor => wgpu::TextureFormat::Rgba16Float,
            Self::ObjectId => wgpu::TextureFormat::R32Uint,
            Self::LinearDepth => wgpu::TextureFormat::R32Float,
            Self::WorldNormal => wgpu::TextureFormat::Rgba32Float,
        }
    }
    fn bytes_per_pixel(self) -> u32 {
        if matches!(self, Self::WorldNormal) {
            16
        } else if matches!(self, Self::LinearColor) {
            8
        } else {
            4
        }
    }
}

fn readback_stride(width: u32, bytes_per_pixel: u32) -> u64 {
    let alignment = u64::from(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
    (u64::from(width) * u64::from(bytes_per_pixel)).div_ceil(alignment) * alignment
}

fn output_texture(
    device: &wgpu::Device,
    [width, height]: [u32; 2],
    format: wgpu::TextureFormat,
) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("scene3d.output"),
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
            | if format == wgpu::TextureFormat::Rgba16Float {
                wgpu::TextureUsages::COPY_DST
            } else {
                wgpu::TextureUsages::empty()
            },
        view_formats: &[],
    })
}

/// An owned submitted frame. Textures remain valid across subsequent renders
/// and resizes. GPU consumers must use the same device and queue ordering.
pub struct Scene3dGpuOutput {
    depth_background: gpui::DepthBackground3d,
    context: WgpuContext,
    draw_statistics: Scene3dDrawStatistics,
    config: Scene3dOutputConfig,
    color: Option<wgpu::Texture>,
    linear_color: Option<wgpu::Texture>,
    ids: Option<wgpu::Texture>,
    depth: Option<wgpu::Texture>,
    normals: Option<wgpu::Texture>,
    readback_busy: Arc<AtomicBool>,
    target_memory: Scene3dTargetMemory,
}
impl Scene3dGpuOutput {
    /// Background sentinel used by this frame's linear-depth texture.
    pub fn depth_background(&self) -> gpui::DepthBackground3d {
        self.depth_background
    }
    /// Target payload of this submission's configuration, independent of cache reuse.
    pub fn target_memory(&self) -> Scene3dTargetMemory {
        self.target_memory
    }
    /// Counts from this submission's prepared mesh plans, without GPU readback.
    pub fn draw_statistics(&self) -> Scene3dDrawStatistics {
        self.draw_statistics
    }

    pub fn config(&self) -> Scene3dOutputConfig {
        self.config
    }
    /// Premultiplied display-encoded RGBA8 after the frame's exposure and tone mapping.
    pub fn color(&self) -> Option<&wgpu::Texture> {
        self.color.as_ref()
    }
    /// Rgba16Float: premultiplied linear HDR before exposure, tone mapping or sRGB encoding.
    /// Four-sample color averages linear samples; the texture itself is single-sampled.
    pub fn linear_color(&self) -> Option<&wgpu::Texture> {
        self.linear_color.as_ref()
    }
    /// R32Uint with zero background and exact, unfiltered object IDs.
    pub fn object_ids(&self) -> Option<&wgpu::Texture> {
        self.ids.as_ref()
    }

    /// R32Float: nonnegative camera-forward depth in scene units. Consult
    /// `depth_background()` for this frame's no-surface sentinel.
    pub fn linear_depth(&self) -> Option<&wgpu::Texture> {
        self.depth.as_ref()
    }

    /// Rgba32Float: interpolated world-space vertex normal, normalized and oriented
    /// toward the visible side. W is one for a surface and zero for background.
    /// Normal maps are not applied. Zero-length input normals remain zero XYZ.
    pub fn world_normals(&self) -> Option<&wgpu::Texture> {
        self.normals.as_ref()
    }

    /// Starts a bounded, nonblocking readback. Poll its result or drop to cancel.
    /// A second pending readback from this renderer returns an error.
    pub fn readback(&self) -> Result<Scene3dReadback> {
        self.readback_with(Scene3dReadbackConfig::new(self.config.channels))
    }

    /// Reads a nonempty subset of this frame's channels. Validates availability,
    /// payload budgets and device buffer limits before acquiring the queue permit
    /// or allocating staging buffers. Does not modify or release source textures.
    pub fn readback_with(&self, config: Scene3dReadbackConfig) -> Result<Scene3dReadback> {
        let memory = config.validate(
            self.config.size,
            self.config.channels,
            self.context.device.limits().max_buffer_size,
        )?;
        ensure!(!self.context.device_lost(), "3D rendering device is lost");
        self.context.device.poll(wgpu::PollType::Poll)?;
        ensure!(
            self.readback_busy
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_ok(),
            "a 3D readback is already pending"
        );
        let mut pending = Scene3dReadback {
            depth_background: self.depth_background,
            context: self.context.clone(),
            size: self.config.size,
            slots: Vec::new(),
            permit: Some(Arc::new(ReadbackPermit(self.readback_busy.clone()))),
            finished: false,
            memory,
        };
        let [width, height] = self.config.size;
        let mut encoder =
            self.context
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("scene3d.readback"),
                });
        for (kind, texture) in [
            (OutputKind::Color, self.color.as_ref()),
            (OutputKind::LinearColor, self.linear_color.as_ref()),
            (OutputKind::ObjectId, self.ids.as_ref()),
            (OutputKind::LinearDepth, self.depth.as_ref()),
            (OutputKind::WorldNormal, self.normals.as_ref()),
        ] {
            let Some(texture) = texture else {
                continue;
            };
            if !config.channels.contains(kind.channel()) {
                continue;
            }
            let stride = readback_stride(width, kind.bytes_per_pixel()) as u32;
            let buffer = self.context.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("scene3d.readback"),
                size: u64::from(stride) * u64::from(height),
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            encoder.copy_texture_to_buffer(
                texture.as_image_copy(),
                wgpu::TexelCopyBufferInfo {
                    buffer: &buffer,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(stride),
                        rows_per_image: Some(height),
                    },
                },
                texture.size(),
            );
            pending.slots.push(ReadbackSlot {
                buffer,
                receiver: None,
                stride,
                kind,
                ready: false,
            });
        }
        self.context.queue.submit([encoder.finish()]);
        for slot in &mut pending.slots {
            let (sender, receiver) = mpsc::sync_channel(1);
            slot.receiver = Some(receiver);
            let permit = pending.permit.as_ref().unwrap().clone();
            slot.buffer
                .map_async(wgpu::MapMode::Read, .., move |result| {
                    let _permit = permit;
                    let _ = sender.send(result);
                });
        }
        Ok(pending)
    }
}

struct ReadbackSlot {
    buffer: wgpu::Buffer,
    receiver: Option<mpsc::Receiver<Result<(), wgpu::BufferAsyncError>>>,
    stride: u32,
    kind: OutputKind,
    ready: bool,
}

struct ReadbackPermit(Arc<AtomicBool>);
impl Drop for ReadbackPermit {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

/// Top-left-origin, tightly packed output. Every channel contains width * height
/// pixels, without GPU row padding. Channels not requested are None.
pub struct Scene3dPixels {
    /// Background sentinel for the linear-depth channel, if requested.
    pub depth_background: gpui::DepthBackground3d,
    pub size: [u32; 2],
    pub rgba: Option<Vec<u8>>,
    /// Premultiplied linear HDR RGBA, widened from binary16 without display conversion.
    pub linear_rgba: Option<Vec<[f32; 4]>>,
    pub object_ids: Option<Vec<u32>>,
    /// Nonnegative camera-forward distance in scene units;
    /// `depth_background` identifies samples without a surface.
    pub linear_depth: Option<Vec<f32>>,
    /// World XYZ normal and surface-validity W. Normal maps are not applied.
    pub world_normals: Option<Vec<[f32; 4]>>,
}

/// Pending readback that owns its staging buffers and renderer queue permit.
pub struct Scene3dReadback {
    depth_background: gpui::DepthBackground3d,
    context: WgpuContext,
    size: [u32; 2],
    slots: Vec<ReadbackSlot>,
    permit: Option<Arc<ReadbackPermit>>,
    finished: bool,
    memory: Scene3dReadbackMemory,
}
impl Scene3dReadback {
    /// Payload admitted for this request, independent of later renderer changes.
    pub fn memory(&self) -> Scene3dReadbackMemory {
        self.memory
    }
    /// Pumps GPU callbacks without waiting. Returns None until all requested
    /// channels are ready. A completed or failed readback cannot be polled again.
    pub fn try_read(&mut self) -> Result<Option<Scene3dPixels>> {
        ensure!(!self.finished, "3D readback is already finished");
        let result = self.read_ready();
        if !matches!(&result, Ok(None)) {
            self.finished = true;
            self.release();
        }
        result
    }
    fn read_ready(&mut self) -> Result<Option<Scene3dPixels>> {
        ensure!(!self.context.device_lost(), "3D rendering device is lost");
        self.context
            .device
            .poll(wgpu::PollType::Poll)
            .context("failed to poll 3D readback")?;
        for slot in &mut self.slots {
            if !slot.ready {
                match slot.receiver.as_ref().unwrap().try_recv() {
                    Ok(result) => {
                        result.context("failed to map 3D output")?;
                        slot.ready = true;
                    }
                    Err(mpsc::TryRecvError::Empty) => return Ok(None),
                    Err(mpsc::TryRecvError::Disconnected) => {
                        anyhow::bail!("3D readback callback was dropped")
                    }
                }
            }
        }
        let mut pixels = Scene3dPixels {
            depth_background: self.depth_background,
            size: self.size,
            rgba: None,
            linear_rgba: None,
            object_ids: None,
            linear_depth: None,
            world_normals: None,
        };
        for slot in &self.slots {
            let mapped = slot.buffer.get_mapped_range(..)?;
            pixels.read_channel(slot.kind, slot.stride, &mapped)?;
        }
        Ok(Some(pixels))
    }
    fn release(&mut self) {
        for slot in self.slots.drain(..) {
            slot.buffer.unmap();
        }
        self.permit.take();
    }
}
impl Drop for Scene3dReadback {
    fn drop(&mut self) {
        self.release();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scene3d_readback_decodes_float_channels_without_row_padding() {
        let mut pixels = Scene3dPixels {
            depth_background: Default::default(),
            size: [3, 2],
            rgba: None,
            linear_rgba: None,
            object_ids: None,
            linear_depth: None,
            world_normals: None,
        };
        let depth: [f32; 6] = [0., 0.125, 70000., 3.25, 1.5, 100.];
        let normals = [
            [0., 0., 0., 0.],
            [-1., 0., 0., 1.],
            [0., -1., 0., 1.],
            [0., 0., -1., 1.],
            [0.6, 0.8, 0., 1.],
            [0.; 4],
        ];
        for (kind, values) in [
            (OutputKind::LinearDepth, depth.to_vec()),
            (
                OutputKind::WorldNormal,
                normals.into_iter().flatten().collect(),
            ),
        ] {
            let stride = readback_stride(3, kind.bytes_per_pixel()) as usize;
            let row_len = 3 * kind.bytes_per_pixel() as usize;
            let packed: Vec<_> = values.into_iter().flat_map(f32::to_le_bytes).collect();
            let mut padded = vec![0xff; stride * 2];
            for (row, bytes) in packed.chunks_exact(row_len).enumerate() {
                padded[row * stride..row * stride + row_len].copy_from_slice(bytes);
            }
            pixels.read_channel(kind, stride as u32, &padded).unwrap();
        }
        assert_eq!(pixels.linear_depth.as_deref(), Some(depth.as_slice()));
        assert_eq!(pixels.world_normals.as_deref(), Some(normals.as_slice()));
        assert!(pixels.rgba.is_none() && pixels.object_ids.is_none());
    }

    #[test]
    fn scene3d_output_limits_account_for_selected_channels_and_padded_rows() {
        let caps = Scene3dCapabilities {
            max_dimension: 4096,
            max_pixels: 1_000_000,
            color_msaa4: false,
            linear_color_msaa4: false,
            max_readback_buffer_bytes: 1024,
            geometry_outputs: true,
        };
        let config = Scene3dOutputConfig {
            size: [17, 3],
            channels: Scene3dChannels::LINEAR_DEPTH,
            color_samples: 4,
        };
        assert!(caps.validate(config).is_ok());
        assert!(
            caps.validate(Scene3dOutputConfig {
                channels: Scene3dChannels::WORLD_NORMAL,
                ..config
            })
            .is_err()
        );
        assert!(
            caps.validate(Scene3dOutputConfig {
                channels: Scene3dChannels::COLOR,
                ..config
            })
            .is_err()
        );
        assert!(
            caps.validate(Scene3dOutputConfig {
                channels: Scene3dChannels::empty(),
                ..config
            })
            .is_err()
        );
        assert!(
            Scene3dCapabilities {
                geometry_outputs: false,
                ..caps
            }
            .validate(config)
            .is_err()
        );
    }

    #[test]
    fn scene3d_linear_color_readback_preserves_hdr_alpha_and_row_order() {
        let mut pixels = Scene3dPixels {
            depth_background: Default::default(),
            size: [2, 2],
            rgba: None,
            linear_rgba: None,
            object_ids: None,
            linear_depth: None,
            world_normals: None,
        };
        let encoded: [[u16; 4]; 4] = [
            [0x4000, 0x3800, 0x3000, 0x3400],
            [0x7bff, 0x0001, 0x3c00, 0x3c00],
            [0, 0, 0, 0],
            [0x4400, 0x4200, 0x4000, 0x3800],
        ];
        let expected = [
            [2., 0.5, 0.125, 0.25],
            [65504., 2_f32.powi(-24), 1., 1.],
            [0.; 4],
            [4., 3., 2., 0.5],
        ];
        let stride = readback_stride(2, 8) as usize;
        let mut padded = vec![0xff; stride * 2];
        for (index, pixel) in encoded.into_iter().enumerate() {
            let offset = index / 2 * stride + index % 2 * 8;
            for (channel, bits) in pixel.into_iter().enumerate() {
                padded[offset + channel * 2..offset + channel * 2 + 2]
                    .copy_from_slice(&bits.to_le_bytes());
            }
        }
        pixels
            .read_channel(OutputKind::LinearColor, stride as u32, &padded)
            .unwrap();
        assert_eq!(pixels.linear_rgba.as_deref(), Some(expected.as_slice()));
        assert!(pixels.rgba.is_none());
    }

    #[test]
    fn scene3d_linear_color_limits_use_half_float_stride_and_resolve_support() {
        let mut caps = Scene3dCapabilities {
            max_dimension: 4096,
            max_pixels: 1_000_000,
            color_msaa4: false,
            linear_color_msaa4: true,
            max_readback_buffer_bytes: 1024,
            geometry_outputs: false,
        };
        let mut config = Scene3dOutputConfig {
            size: [32, 3],
            channels: Scene3dChannels::LINEAR_COLOR,
            color_samples: 4,
        };
        assert!(caps.validate(config).is_ok());
        config.size[0] = 33;
        assert!(caps.validate(config).is_err());
        config.channels = Scene3dChannels::COLOR;
        config.color_samples = 1;
        assert!(caps.validate(config).is_ok());
        config.size[0] = 32;
        config.channels = Scene3dChannels::LINEAR_COLOR;
        config.color_samples = 4;
        caps.linear_color_msaa4 = false;
        caps.color_msaa4 = true;
        assert!(caps.validate(config).is_err());
        config.color_samples = 1;
        assert!(caps.validate(config).is_ok());
    }
}
