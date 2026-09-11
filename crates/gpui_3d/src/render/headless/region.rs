use super::*;
use anyhow::ensure;

/// Raster provenance of a full or regional readback. Coordinates use physical output pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FrameReadbackLayout {
    /// Complete source texture dimensions, independent of the readback region.
    pub output_size: [u32; 2],
    /// Sample rectangle in the source texture. Returned pixel arrays start at its origin.
    pub region: Scene3dReadbackRegion,
    /// Full camera rectangle [x, y, width, height], possibly extending beyond a clipped texture.
    pub projection_rect: [f32; 4],
}

impl FrameReadbackLayout {
    pub(super) fn full(size: [u32; 2]) -> Self {
        Self {
            output_size: size,
            region: Scene3dReadbackRegion {
                origin: [0; 2],
                size,
            },
            projection_rect: [0., 0., size[0] as f32, size[1] as f32],
        }
    }
    pub(super) fn valid(&self, size: [u32; 2]) -> bool {
        self.region.size == size
            && self.region.validate(self.output_size).is_ok()
            && self.projection_rect.iter().all(|v| v.is_finite())
            && self.projection_rect[2] > 0.
            && self.projection_rect[3] > 0.
    }

    pub(super) fn viewport(&self) -> Bounds<gpui::Pixels> {
        let [x, y, width, height] = self.projection_rect;
        Bounds::new(point(px(x), px(y)), size(px(width), px(height)))
    }
}

impl RenderedFrame {
    /// Reads a physical rectangle while retaining the source camera, object map and identity.
    /// Budgets apply to the region. Empty or out-of-bounds rectangles are rejected, not clipped.
    pub fn readback_region(
        &self,
        region: Scene3dReadbackRegion,
        config: Scene3dReadbackConfig,
    ) -> Result<FrameReadback> {
        FrameReadback::new(
            &self.output,
            self.objects.clone(),
            self.camera,
            region,
            config,
            None,
        )
    }
}

impl FrameReadback {
    pub(in crate::render) fn new(
        output: &Scene3dGpuOutput,
        objects: Arc<[RenderObject]>,
        camera: Camera,
        region: Scene3dReadbackRegion,
        config: Scene3dReadbackConfig,
        projection_rect: Option<[f32; 4]>,
    ) -> Result<Self> {
        let mut layout = FrameReadbackLayout::full(output.config().size);
        layout.region = region;
        if let Some(rect) = projection_rect {
            layout.projection_rect = rect;
        }
        ensure!(layout.valid(region.size), "invalid readback layout");
        Ok(Self {
            pending: output.readback_region(region, config)?,
            objects,
            camera,
            layout,
        })
    }

    pub fn layout(&self) -> FrameReadbackLayout {
        self.layout
    }
}

impl ReadFrame {
    pub fn layout(&self) -> FrameReadbackLayout {
        self.layout
    }
}

#[cfg(test)]
mod tests;
