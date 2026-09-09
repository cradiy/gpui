use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc,
};

use anyhow::{Context as _, Result, ensure};
use gpui::{MeshTexture3d, Scene3dFrame};

use crate::{WgpuAtlas, WgpuContext, wgpu_renderer::scene3d::Scene3dRenderer};

/// Requested 3D outputs. ID pixels are single-sampled and never color-resolved.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Scene3dChannels {
    Color,
    ObjectId,
    #[default]
    ColorAndObjectId,
}
impl Scene3dChannels {
    fn color(self) -> bool {
        self != Self::ObjectId
    }
    fn ids(self) -> bool {
        self != Self::Color
    }
}

/// Physical output dimensions and color sampling. The background is transparent
/// black for color and zero for IDs. Color uses display-encoded RGBA8 after
/// linear HDR shading, exposure, and tone mapping.
#[derive(Clone, Copy, Debug)]
pub struct Scene3dOutputConfig {
    pub size: [u32; 2],
    pub channels: Scene3dChannels,
    /// One or four samples. Object IDs always use the pixel-center sample.
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
}
impl Scene3dCapabilities {
    pub fn validate(self, config: Scene3dOutputConfig) -> Result<()> {
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
    capabilities: Scene3dCapabilities,
    readback_busy: Arc<AtomicBool>,
}
impl WgpuScene3dRenderer {
    #[cfg(not(target_family = "wasm"))]
    pub fn new_headless() -> Result<Self> {
        Self::new(WgpuContext::new_headless()?)
    }

    pub fn new(context: WgpuContext) -> Result<Self> {
        let usages = wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC;
        for format in [
            wgpu::TextureFormat::Rgba8Unorm,
            wgpu::TextureFormat::Rgba16Float,
            wgpu::TextureFormat::R32Uint,
        ] {
            ensure!(
                context
                    .adapter
                    .get_texture_format_features(format)
                    .allowed_usages
                    .contains(usages),
                "3D output format {format:?} is unavailable"
            );
        }
        let supports_msaa = |format| {
            context
                .adapter
                .get_texture_format_features(format)
                .flags
                .contains(wgpu::TextureFormatFeatureFlags::MULTISAMPLE_X4)
        };
        let capabilities = Scene3dCapabilities {
            max_dimension: context.device.limits().max_texture_dimension_2d,
            max_pixels: 16_777_216,
            color_msaa4: supports_msaa(wgpu::TextureFormat::Rgba16Float)
                && supports_msaa(wgpu::TextureFormat::Depth32Float),
        };
        Ok(Self {
            atlas: Arc::new(WgpuAtlas::from_context(&context)),
            context,
            color: None,
            ids: None,
            capabilities,
            readback_busy: Arc::new(AtomicBool::new(false)),
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

    /// Submits a frame and returns owned GPU outputs. Image atlas tiles must come
    /// from this renderer. UI subtree textures are not supported by this entry point.
    pub fn render(
        &mut self,
        frame: &Scene3dFrame,
        config: Scene3dOutputConfig,
    ) -> Result<Scene3dGpuOutput> {
        self.capabilities.validate(config)?;
        ensure!(!self.context.device_lost(), "3D rendering device is lost");
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
            ensure!(
                object.sort_depth.is_finite(),
                "3D object {} has invalid sort depth",
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
                    || object.mesh.tangents().is_some(),
                "3D object {}: normal maps require mesh tangents",
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
        self.atlas.before_frame();
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("scene3d.direct"),
        });
        let rect = [0., 0., width as f32, height as f32];
        let color = if config.channels.color() {
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
            renderer.prepare_frames(device, [frame], width, height);
            let texture = output_texture(device, config.size, wgpu::TextureFormat::Rgba8Unorm);
            renderer.encode_frame(
                device,
                queue,
                &self.atlas,
                frame,
                rect,
                0,
                None,
                &texture.create_view(&Default::default()),
                &mut encoder,
            );
            Some(texture)
        } else {
            self.color = None;
            None
        };
        let ids = if config.channels.ids() {
            let renderer = self.ids.get_or_insert_with(|| {
                Scene3dRenderer::new(device, queue, wgpu::TextureFormat::R32Uint, 1)
            });
            if let Some((_, color)) = &self.color {
                renderer.reuse_geometry_from(color);
            }
            renderer.prepare_frames(device, [frame], width, height);
            let texture = output_texture(device, config.size, wgpu::TextureFormat::R32Uint);
            renderer.encode_frame(
                device,
                queue,
                &self.atlas,
                frame,
                rect,
                0,
                None,
                &texture.create_view(&Default::default()),
                &mut encoder,
            );
            Some(texture)
        } else {
            self.ids = None;
            None
        };
        self.context.queue.submit([encoder.finish()]);
        Ok(Scene3dGpuOutput {
            context: self.context.clone(),
            config,
            color,
            ids,
            readback_busy: self.readback_busy.clone(),
        })
    }
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
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    })
}

/// An owned submitted frame. Textures remain valid across subsequent renders
/// and resizes. GPU consumers must use the same device and queue ordering.
pub struct Scene3dGpuOutput {
    context: WgpuContext,
    config: Scene3dOutputConfig,
    color: Option<wgpu::Texture>,
    ids: Option<wgpu::Texture>,
    readback_busy: Arc<AtomicBool>,
}
impl Scene3dGpuOutput {
    pub fn config(&self) -> Scene3dOutputConfig {
        self.config
    }
    /// Premultiplied display-encoded RGBA8 after the frame's exposure and tone mapping.
    pub fn color(&self) -> Option<&wgpu::Texture> {
        self.color.as_ref()
    }
    /// R32Uint with zero background and exact, unfiltered object IDs.
    pub fn object_ids(&self) -> Option<&wgpu::Texture> {
        self.ids.as_ref()
    }

    /// Starts a bounded, nonblocking readback. Poll its result or drop to cancel.
    /// A second pending readback from this renderer returns an error.
    pub fn readback(&self) -> Result<Scene3dReadback> {
        ensure!(!self.context.device_lost(), "3D rendering device is lost");
        self.context.device.poll(wgpu::PollType::Poll)?;
        ensure!(
            self.readback_busy
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_ok(),
            "a 3D readback is already pending"
        );
        let mut pending = Scene3dReadback {
            context: self.context.clone(),
            size: self.config.size,
            slots: Vec::new(),
            permit: Some(Arc::new(ReadbackPermit(self.readback_busy.clone()))),
            finished: false,
        };
        let [width, height] = self.config.size;
        let stride = (width * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
            * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let mut encoder =
            self.context
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("scene3d.readback"),
                });
        for (is_id, texture) in [(false, self.color.as_ref()), (true, self.ids.as_ref())] {
            let Some(texture) = texture else {
                continue;
            };
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
                is_id,
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
    is_id: bool,
    ready: bool,
}

struct ReadbackPermit(Arc<AtomicBool>);
impl Drop for ReadbackPermit {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

/// Top-left-origin, tightly packed output. Color rows are width * 4 bytes;
/// ID rows are width u32 values. Channels not requested are None.
pub struct Scene3dPixels {
    pub size: [u32; 2],
    pub rgba: Option<Vec<u8>>,
    pub object_ids: Option<Vec<u32>>,
}

/// Pending readback that owns its staging buffers and renderer queue permit.
pub struct Scene3dReadback {
    context: WgpuContext,
    size: [u32; 2],
    slots: Vec<ReadbackSlot>,
    permit: Option<Arc<ReadbackPermit>>,
    finished: bool,
}
impl Scene3dReadback {
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
            size: self.size,
            rgba: None,
            object_ids: None,
        };
        let width_bytes = self.size[0] as usize * 4;
        for slot in &self.slots {
            let mapped = slot.buffer.get_mapped_range(..)?;
            let packed = mapped
                .chunks_exact(slot.stride as usize)
                .take(self.size[1] as usize)
                .flat_map(|row| row[..width_bytes].iter().copied())
                .collect::<Vec<_>>();
            if slot.is_id {
                pixels.object_ids = Some(
                    packed
                        .chunks_exact(4)
                        .map(|bytes| u32::from_le_bytes(bytes.try_into().unwrap()))
                        .collect(),
                );
            } else {
                pixels.rgba = Some(packed);
            }
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
