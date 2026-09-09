use std::sync::mpsc;

use anyhow::Context as _;
use gpui::{DevicePixels, Scene, Size};

use crate::{WgpuContext, WgpuExternalRendererConfig, WgpuRenderer, wgpu};

/// Renders GPUI scenes into CPU-readable RGBA pixels without a native window.
pub struct WgpuOffscreenRenderer {
    context: WgpuContext,
    renderer: WgpuRenderer,
    target: wgpu::Texture,
    target_view: wgpu::TextureView,
    readback: wgpu::Buffer,
    size: Size<DevicePixels>,
    padded_bytes_per_row: u32,
}

impl WgpuOffscreenRenderer {
    /// Creates an offscreen renderer with the requested device-pixel size.
    pub fn new(size: Size<DevicePixels>) -> anyhow::Result<Self> {
        let context = WgpuContext::new_headless()?;
        let size = clamped_size(size);
        let renderer = WgpuRenderer::new_external(
            &context,
            WgpuExternalRendererConfig {
                size,
                format: wgpu::TextureFormat::Rgba8UnormSrgb,
                alpha_mode: wgpu::CompositeAlphaMode::Opaque,
                target_usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::COPY_SRC,
            },
        )?;
        let (target, target_view, readback, padded_bytes_per_row) = create_targets(&context, size);
        Ok(Self {
            context,
            renderer,
            target,
            target_view,
            readback,
            size,
            padded_bytes_per_row,
        })
    }

    /// Returns the atlas used by GPUI when building scenes for this renderer.
    pub fn sprite_atlas(&self) -> std::sync::Arc<dyn gpui::PlatformAtlas> {
        self.renderer.sprite_atlas().clone()
    }

    /// Mesh output-cache allocations across this renderer and nested UI captures.
    pub fn scene3d_output_cache_stats(&self) -> gpui::Scene3dOutputCacheStats {
        self.renderer.scene3d_output_cache_stats()
    }

    /// Sets the shared mesh output-cache budget. Zero disables mesh pixel reuse;
    /// changing the budget releases current output-cache entries without waiting.
    pub fn set_scene3d_output_cache_budget(&mut self, bytes: u64) {
        self.renderer.set_scene3d_output_cache_budget(bytes);
    }

    /// Changes the target size, recreating its backing texture when necessary.
    pub fn resize(&mut self, size: Size<DevicePixels>) {
        let size = clamped_size(size);
        if size == self.size {
            return;
        }
        self.renderer.update_drawable_size(size);
        let (target, target_view, readback, padded_bytes_per_row) =
            create_targets(&self.context, size);
        self.target = target;
        self.target_view = target_view;
        self.readback = readback;
        self.size = size;
        self.padded_bytes_per_row = padded_bytes_per_row;
    }

    /// Renders a scene and returns tightly packed RGBA8 pixels.
    pub fn render_rgba(&mut self, scene: &Scene) -> anyhow::Result<Vec<u8>> {
        let mut encoder =
            self.context
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("gpui.offscreen.encoder"),
                });
        anyhow::ensure!(
            self.renderer
                .draw_external(scene, &self.target, &self.target_view, wgpu::Color::BLACK),
            "GPUI renderer requested a larger instance buffer; retry the frame"
        );
        encoder.copy_texture_to_buffer(
            self.target.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &self.readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(self.padded_bytes_per_row),
                    rows_per_image: Some(self.size.height.0 as u32),
                },
            },
            wgpu::Extent3d {
                width: self.size.width.0 as u32,
                height: self.size.height.0 as u32,
                depth_or_array_layers: 1,
            },
        );
        self.context.queue.submit([encoder.finish()]);

        let (tx, rx) = mpsc::sync_channel(1);
        self.readback
            .map_async(wgpu::MapMode::Read, .., move |result| {
                let _ = tx.send(result);
            });
        self.context
            .device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: None,
            })
            .context("failed to wait for offscreen GPUI rendering")?;
        rx.recv()
            .context("offscreen mapping callback was dropped")?
            .context("failed to map offscreen GPUI pixels")?;

        let mapped = self.readback.get_mapped_range(..)?;
        let width_bytes = self.size.width.0 as usize * 4;
        let mut rgba = Vec::with_capacity(width_bytes * self.size.height.0 as usize);
        for row in mapped
            .chunks_exact(self.padded_bytes_per_row as usize)
            .take(self.size.height.0 as usize)
        {
            rgba.extend_from_slice(&row[..width_bytes]);
        }
        drop(mapped);
        self.readback.unmap();
        Ok(rgba)
    }
}

fn clamped_size(size: Size<DevicePixels>) -> Size<DevicePixels> {
    Size {
        width: DevicePixels(size.width.0.max(1)),
        height: DevicePixels(size.height.0.max(1)),
    }
}

fn create_targets(
    context: &WgpuContext,
    size: Size<DevicePixels>,
) -> (wgpu::Texture, wgpu::TextureView, wgpu::Buffer, u32) {
    let width = size.width.0 as u32;
    let height = size.height.0 as u32;
    let target = context.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("gpui.offscreen.target"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let target_view = target.create_view(&wgpu::TextureViewDescriptor::default());
    let unpadded_bytes_per_row = width * 4;
    let alignment = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let padded_bytes_per_row = unpadded_bytes_per_row.div_ceil(alignment) * alignment;
    let readback = context.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("gpui.offscreen.readback"),
        size: u64::from(padded_bytes_per_row) * u64::from(height),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    (target, target_view, readback, padded_bytes_per_row)
}
