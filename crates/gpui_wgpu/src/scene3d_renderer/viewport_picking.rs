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

    /// Maps a source-surface rectangle [x, y, width, height] to capture pixels.
    /// Intersects the viewport and output extent, rounding outward to include
    /// partially intersected pixels. Empty, nonfinite and fully clipped regions
    /// return None. This starts no GPU work and does not apply outer compositing.
    pub fn region_at_surface(&self, bounds: [f32; 4]) -> Option<super::Scene3dReadbackRegion> {
        surface_region(
            self.source_rect,
            self.rect,
            self.output.config().size,
            bounds,
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
            frame
                .pick_capture
                .as_ref()
                .is_none_or(|capture| target_memory.total_bytes <= capture.max_bytes()),
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
        Ok(Self {
            output: Scene3dGpuOutput {
                occlusion: Vec::new(),
                frame_id: Default::default(),
                depth_background: frame.depth_background,
                draw_statistics: Scene3dDrawStatistics::default(),
                config,
                color: None,
                linear_color: None,
                ids: Some(output_texture(&context, size, wgpu::TextureFormat::R32Uint)),
                depth: Some(output_texture(
                    &context,
                    size,
                    wgpu::TextureFormat::R32Float,
                )),
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
    pub(crate) fn set_edit_payload(&mut self, bytes: u64, instances: u64) {
        self.output.target_memory.attachment_bytes = 0;
        self.output.target_memory.total_bytes = self.output.target_memory.output_bytes;
        self.output.geometry_memory = super::Scene3dGeometryMemory {
            vertex_bytes: bytes,
            total_bytes: bytes,
            max_buffer_bytes: bytes,
            ..Default::default()
        };
        self.output.draw_statistics = Scene3dDrawStatistics {
            camera_draws: u64::from(instances > 0),
            camera_instances: instances,
            camera_triangles: instances * 2,
            instance_upload_bytes: bytes,
            uniform_upload_bytes: 16,
            ..Default::default()
        };
    }
    pub(crate) fn set_occlusion(
        &mut self,
        outputs: Vec<super::Scene3dOcclusionOutput>,
        memory: super::Scene3dTargetMemory,
    ) {
        for output in &outputs {
            self.output.draw_statistics += output.gpu().draw_statistics();
            if let Some(elements) = output.elements() {
                self.output.draw_statistics += elements.draw_statistics();
            }
        }
        self.output.occlusion = outputs;
        self.output.target_memory = memory;
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

fn surface_region(
    source: [f32; 4],
    rect: [f32; 4],
    size: [u32; 2],
    bounds: [f32; 4],
) -> Option<super::Scene3dReadbackRegion> {
    if source
        .iter()
        .chain(&rect)
        .chain(&bounds)
        .any(|v| !v.is_finite())
    {
        return None;
    }
    let mut region = super::Scene3dReadbackRegion {
        origin: [0; 2],
        size: [0; 2],
    };
    for axis in 0..2 {
        if source[axis + 2] <= 0. || rect[axis + 2] <= 0. || bounds[axis + 2] <= 0. {
            return None;
        }
        let source_start = f64::from(source[axis]);
        let source_length = f64::from(source[axis + 2]);
        let start = f64::from(bounds[axis]).max(source_start);
        let end = (f64::from(bounds[axis]) + f64::from(bounds[axis + 2]))
            .min(source_start + source_length);
        if end <= start {
            return None;
        }
        let raster = |value: f64| {
            (f64::from(rect[axis])
                + (value - source_start) * f64::from(rect[axis + 2]) / source_length)
                .clamp(0., f64::from(size[axis]))
        };
        let start = raster(start);
        let end = raster(end);
        if end <= start {
            return None;
        }
        region.origin[axis] = start.floor() as u32;
        region.size[axis] = end.ceil() as u32 - region.origin[axis];
    }
    Some(region)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rectangle_mapping_clips_and_encloses_partial_raster_pixels() {
        let source = [-10., 20., 100., 80.];
        let rect = [-5., 0., 50., 40.];
        let size = [45, 30];
        let mapped = surface_region(source, rect, size, [0., 30., 21., 21.]).unwrap();
        assert_eq!(mapped.origin, [0, 5]);
        assert_eq!(mapped.size, [11, 11]);
        mapped.validate(size).unwrap();
        for point in [[0., 30.], [20.99, 30.], [0., 50.99], [20.99, 50.99]] {
            let pixel = surface_pixel(source, rect, size, point).unwrap();
            for axis in 0..2 {
                assert!(pixel[axis] >= mapped.origin[axis]);
                assert!(pixel[axis] < mapped.origin[axis] + mapped.size[axis]);
            }
        }
        let full = surface_region(source, rect, size, [-100., -100., 500., 500.]).unwrap();
        assert_eq!(full.origin, [0; 2]);
        assert_eq!(full.size, size);
        for bounds in [
            [90., 30., 10., 10.],
            [0., 80., 20., 20.],
            [-10., 30., 9., 10.],
            [0., 30., 0., 10.],
            [0., 30., 10., -1.],
            [f32::NAN, 30., 10., 10.],
            [0., 30., f32::INFINITY, 10.],
        ] {
            assert_eq!(surface_region(source, rect, size, bounds), None);
        }
    }

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
