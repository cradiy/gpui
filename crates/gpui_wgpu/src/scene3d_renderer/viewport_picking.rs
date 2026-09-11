use std::sync::{Arc, Weak, atomic::AtomicBool};

use anyhow::{Result, ensure};
use gpui::Scene3dFrame;

use super::{
    Scene3dCapabilities, Scene3dChannels, Scene3dDrawStatistics, Scene3dGpuOutput,
    Scene3dOutputConfig, gpu_draws, output_texture,
};
use crate::WgpuContext;

/// ID/depth textures submitted with a window viewport, before outer compositing effects.
/// Texture coordinates include clipping and the viewport's actual raster density.
pub struct WgpuScene3dPickFrame {
    output: Scene3dGpuOutput,
    source: Weak<Scene3dFrame>,
    rect: [f32; 4],
    source_rect: [f32; 4],
}

impl WgpuScene3dPickFrame {
    pub fn gpu(&self) -> &Scene3dGpuOutput {
        &self.output
    }

    /// Checks the originating immutable GPUI frame allocation, not just its mesh contents.
    pub fn matches_frame(&self, frame: &Arc<Scene3dFrame>) -> bool {
        self.source.ptr_eq(&Arc::downgrade(frame))
    }

    /// Full projected viewport rectangle in local raster pixels: x, y, width, height.
    /// It can extend outside the textures when the viewport is clipped by the surface.
    pub fn projection_rect(&self) -> [f32; 4] {
        self.rect
    }

    /// Maps normalized viewport coordinates to an unfiltered raster pixel.
    /// Out-of-range, nonfinite, and surface-clipped positions return `None`.
    pub fn pixel_at(&self, uv: [f32; 2]) -> Option<[u32; 2]> {
        pixel_at(self.rect, self.output.config().size, uv)
    }

    /// Maps physical coordinates on the source render surface, using its snapped
    /// viewport bounds. Nested UI captures use their own render-surface coordinates.
    pub fn pixel_at_surface(&self, position: [f32; 2]) -> Option<[u32; 2]> {
        surface_pixel(
            self.source_rect,
            self.rect,
            self.output.config().size,
            position,
        )
    }

    pub(crate) fn allocate(
        context: WgpuContext,
        capabilities: Scene3dCapabilities,
        frame: &Arc<Scene3dFrame>,
        size: [u32; 2],
        rect: [f32; 4],
        source_rect: [f32; 4],
        busy: Arc<AtomicBool>,
    ) -> Result<Self> {
        let config = Scene3dOutputConfig {
            size,
            channels: Scene3dChannels::OBJECT_ID | Scene3dChannels::LINEAR_DEPTH,
            color_samples: 1,
        };
        capabilities.validate(config)?;
        let target_memory = config.target_memory(None)?;
        ensure!(
            target_memory.total_bytes <= frame.pick_capture.as_ref().unwrap().max_bytes(),
            "3D viewport picking exceeds target payload budget"
        );
        let mut ids = std::collections::HashSet::new();
        ensure!(
            frame
                .objects
                .iter()
                .all(|object| object.output_id != 0 && ids.insert(object.output_id)),
            "3D viewport picking requires unique nonzero object IDs"
        );
        let geometry = gpu_draws::validate_frame(&context.device, frame)?;
        let geometry_memory = gpu_draws::memory(frame, config.channels, &geometry)?;
        geometry_memory.validate(context.device.limits().max_buffer_size, None)?;
        let device = &context.device;
        Ok(Self {
            output: Scene3dGpuOutput {
                frame_id: Default::default(),
                depth_background: frame.depth_background,
                draw_statistics: Scene3dDrawStatistics::default(),
                config,
                color: None,
                linear_color: None,
                ids: Some(output_texture(device, size, wgpu::TextureFormat::R32Uint)),
                depth: Some(output_texture(device, size, wgpu::TextureFormat::R32Float)),
                normals: None,
                readback_busy: busy,
                target_memory,
                geometry_memory,
                context,
            },
            source: Arc::downgrade(frame),
            rect,
            source_rect,
        })
    }

    pub(crate) fn set_statistics(&mut self, statistics: Scene3dDrawStatistics) {
        self.output.draw_statistics = statistics;
    }
}

fn pixel_at(rect: [f32; 4], size: [u32; 2], uv: [f32; 2]) -> Option<[u32; 2]> {
    if uv.iter().any(|v| !v.is_finite() || !(0. ..1.).contains(v)) {
        return None;
    }
    let mut pixel = [0; 2];
    for axis in 0..2 {
        let position = f64::from(rect[axis]) + f64::from(uv[axis]) * f64::from(rect[axis + 2]);
        if !position.is_finite() || position < 0. || position >= f64::from(size[axis]) {
            return None;
        }
        pixel[axis] = position.floor() as u32;
    }
    Some(pixel)
}

fn surface_pixel(
    source: [f32; 4],
    rect: [f32; 4],
    size: [u32; 2],
    position: [f32; 2],
) -> Option<[u32; 2]> {
    let mut pixel = [0; 2];
    for axis in 0..2 {
        let offset = f64::from(position[axis]) - f64::from(source[axis]);
        if !offset.is_finite() || offset < 0. || offset >= f64::from(source[axis + 2]) {
            return None;
        }
        let raster = f64::from(rect[axis])
            + offset * f64::from(rect[axis + 2]) / f64::from(source[axis + 2]);
        if !raster.is_finite() || raster < 0. || raster >= f64::from(size[axis]) {
            return None;
        }
        pixel[axis] = raster.floor() as u32;
    }
    Some(pixel)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn physical_pointer_mapping_uses_snapped_bounds_and_raster_density() {
        let source = [-10., 21., 101., 60.];
        let rect = [-20., 0., 202., 120.];
        assert_eq!(
            surface_pixel(source, rect, [182, 120], [40., 51.]),
            Some([80, 60])
        );
        assert_eq!(surface_pixel(source, rect, [182, 120], [-1., 51.]), None);
        assert_eq!(surface_pixel(source, rect, [182, 120], [91., 51.]), None);
        assert_eq!(surface_pixel(source, rect, [182, 120], [40., 81.]), None);
        assert_eq!(
            surface_pixel(source, rect, [182, 120], [f32::NAN, 30.]),
            None
        );
    }

    #[test]
    fn pointer_pixels_follow_projection_rect_and_surface_clipping() {
        let rect = [-20., 0.5, 100., 60.];
        let size = [80, 50];
        assert_eq!(pixel_at(rect, size, [0.5, 0.5]), Some([30, 30]));
        assert_eq!(pixel_at(rect, size, [0.1, 0.5]), None);
        assert_eq!(pixel_at(rect, size, [0.5, 0.9]), None);
        assert_eq!(pixel_at(rect, size, [0.5, 0.]), Some([30, 0]));
        assert_eq!(
            pixel_at([-40., 1., 200., 120.], [160, 100], [0.5, 0.5]),
            Some([60, 61])
        );
        for uv in [
            [1., 0.5],
            [0.5, 1.],
            [-0.1, 0.5],
            [f32::NAN, 0.5],
            [0.5, f32::INFINITY],
        ] {
            assert_eq!(pixel_at(rect, size, uv), None);
        }
    }
}
