#![doc = include_str!("../../docs/topics/headless.md")]

mod coverage;
mod depth;
mod gpu_labels;
mod images;
mod labels;
pub use coverage::{CoverageError, FrameCoverage, ObjectCoverage};
pub use depth::{DepthComparison, DepthQueryError, DepthRelation};
pub use gpu_labels::RenderedLabels;
pub use images::{ImageCacheLimits, ImageCacheUsage};
pub use labels::{FrameLabels, LabelError};

use std::sync::Arc;

use anyhow::Result;
use gpui::{Bounds, point, px, size};
use gpui_wgpu::{Scene3dGpuOutput, Scene3dReadback, WgpuScene3dRenderer};

pub use crate::RenderObject;
use crate::{Camera, CameraError, PreparationCache, Scene};

pub use gpui_wgpu::{
    IdRemapConfig, Scene3dCapabilities, Scene3dChannels, Scene3dDeviceCapabilities,
    Scene3dDrawStatistics, Scene3dFormatCapabilities, Scene3dGeometryMemory, Scene3dOutputConfig,
    Scene3dPixels, Scene3dReadbackConfig, Scene3dReadbackMemory, Scene3dTargetMemory, WgpuContext,
    WgpuIdRemapper,
};

/// Window-free renderer for solid and decoded-image materials. Does not load
/// resources, execute custom image callbacks, or capture UI subtrees.
pub struct HeadlessRenderer {
    renderer: WgpuScene3dRenderer,
    images: images::ImageCache,
    preparation: PreparationCache,
}
impl HeadlessRenderer {
    pub fn new() -> Result<Self> {
        Ok(Self {
            renderer: WgpuScene3dRenderer::new_headless()?,
            images: images::ImageCache::default(),
            preparation: PreparationCache::new(),
        })
    }
    /// Reuses a GPU context instead of creating a device for each renderer.
    pub fn with_context(context: WgpuContext) -> Result<Self> {
        Ok(Self {
            renderer: WgpuScene3dRenderer::new(context)?,
            images: images::ImageCache::default(),
            preparation: PreparationCache::new(),
        })
    }
    pub fn context(&self) -> &WgpuContext {
        self.renderer.context()
    }
    pub fn capabilities(&self) -> Scene3dCapabilities {
        self.renderer.capabilities()
    }
    pub fn device_capabilities(&self) -> &Scene3dDeviceCapabilities {
        self.renderer.device_capabilities()
    }

    /// Per-request output, attachment, and shadow payload limit, or `None` (default).
    pub fn target_byte_limit(&self) -> Option<u64> {
        self.renderer.target_byte_limit()
    }

    /// Changes admission for future requests, without releasing existing resources.
    /// Zero rejects every render; `None` disables this limit. Not a total GPU budget.
    pub fn set_target_byte_limit(&mut self, bytes: Option<u64>) {
        self.renderer.set_target_byte_limit(bytes);
    }

    /// Optional per-request vertex/index payload limit. Defaults to `None`.
    pub fn geometry_byte_limit(&self) -> Option<u64> {
        self.renderer.geometry_byte_limit()
    }

    /// Checks geometry before mesh allocation; decoded images are resolved first.
    /// Existing outputs remain valid. Zero admits only requests with no active geometry.
    pub fn set_geometry_byte_limit(&mut self, bytes: Option<u64>) {
        self.renderer.set_geometry_byte_limit(bytes);
    }

    /// Maximum retained CPU preparations. Defaults to one; zero disables retention.
    pub fn preparation_capacity(&self) -> usize {
        self.preparation.capacity()
    }

    /// Bounds CPU preparation reuse across scenes, cameras, and aspect ratios.
    /// Shrinking evicts least-recently-used entries immediately. Does not change
    /// GPU cache budgets or invalidate returned frames and readbacks.
    pub fn set_preparation_capacity(&mut self, capacity: usize) {
        self.preparation.set_capacity(capacity);
    }

    pub fn image_cache_limits(&self) -> ImageCacheLimits {
        self.images.limits()
    }

    /// Per-preparation decoded-image pixel payload limit. Defaults to `None`.
    pub fn image_byte_limit(&self) -> Option<u64> {
        self.images.byte_limit()
    }

    /// Counts each active image identity once, including resident images. Checked
    /// before allocating the image that would exceed the limit. Earlier new
    /// allocations are released on failure; previous residency remains unchanged.
    /// Zero admits only preparations with no image inputs. Not a GPU memory quota.
    pub fn set_image_byte_limit(&mut self, bytes: Option<u64>) {
        self.images.set_byte_limit(bytes);
    }

    pub fn image_cache_usage(&self) -> ImageCacheUsage {
        self.images.usage()
    }

    /// Changes idle atlas retention, evicting least-recently-used images immediately
    /// when either limit is exceeded. Active images and returned outputs remain valid.
    /// Images used in the same preparation have equal recency.
    pub fn set_image_cache_limits(&mut self, limits: ImageCacheLimits) {
        let atlas = self.renderer.sprite_atlas();
        self.images.set_limits(limits, |image_id| {
            images::remove_image(atlas.as_ref(), image_id)
        });
    }

    /// Releases retained CPU preparation and cached GPU resources, including the image atlas.
    /// Subsequent renders rebuild resources from the supplied scene. Returned
    /// frames and pending readbacks remain valid. Does not wait for the GPU.
    pub fn clear_caches(&mut self) {
        self.renderer.clear_caches();
        self.renderer.sprite_atlas().clear();
        self.images.clear();
        self.preparation.clear();
    }

    /// Renders the supplied scene without a native window or UI layout. Geometry,
    /// projection, lighting, and alpha modes share the viewport implementation.
    pub fn render(&mut self, scene: &Scene, config: Scene3dOutputConfig) -> Result<RenderedFrame> {
        self.renderer.validate_target_memory(
            config,
            scene.directional_shadow.map(|shadow| shadow.resolution),
        )?;
        let max_dimension = self.capabilities().max_dimension;
        let atlas = self.renderer.sprite_atlas();
        let prepared = self.images.prepare(
            &mut self.preparation,
            scene,
            config.size[0] as f32 / config.size[1] as f32,
            max_dimension,
            atlas.as_ref(),
        )?;
        let output = self.renderer.render(prepared.frame(), config)?;
        let objects = prepared.identities();
        Ok(RenderedFrame {
            output,
            objects,
            camera: scene.camera,
        })
    }
}

/// GPU outputs and their immutable identity mapping. Older frames survive scene
/// edits, subsequent renders, renderer destruction, and output size changes.
pub struct RenderedFrame {
    output: Scene3dGpuOutput,
    objects: Arc<[RenderObject]>,
    camera: Camera,
}
impl RenderedFrame {
    /// Camera used for this output, independent of subsequent scene changes.
    pub fn camera(&self) -> Camera {
        self.camera
    }

    pub fn gpu(&self) -> &Scene3dGpuOutput {
        &self.output
    }
    pub fn objects(&self) -> &[RenderObject] {
        &self.objects
    }
    /// Zero is background. Unknown values return None.
    pub fn object(&self, output_id: u32) -> Option<&RenderObject> {
        lookup(&self.objects, output_id)
    }
    pub fn readback(&self) -> Result<FrameReadback> {
        self.readback_with(Scene3dReadbackConfig::new(self.output.config().channels))
    }
    /// Starts a bounded readback of selected available channels while retaining
    /// this frame's complete object identity mapping and camera.
    pub fn readback_with(&self, config: Scene3dReadbackConfig) -> Result<FrameReadback> {
        Ok(FrameReadback {
            pending: self.output.readback_with(config)?,
            objects: self.objects.clone(),
            camera: self.camera,
        })
    }
}

/// Nonblocking GPU readback with the same frame-local object mapping.
pub struct FrameReadback {
    pending: Scene3dReadback,
    objects: Arc<[RenderObject]>,
    camera: Camera,
}
impl FrameReadback {
    pub fn memory(&self) -> Scene3dReadbackMemory {
        self.pending.memory()
    }
    pub fn try_read(&mut self) -> Result<Option<ReadFrame>> {
        Ok(self.pending.try_read()?.map(|pixels| ReadFrame {
            pixels,
            objects: self.objects.clone(),
            camera: self.camera,
        }))
    }
}

/// Tightly packed, top-left-origin pixels and the identities that produced them.
pub struct ReadFrame {
    pub pixels: Scene3dPixels,
    objects: Arc<[RenderObject]>,
    camera: Camera,
}
impl ReadFrame {
    /// Camera used to produce these pixels.
    pub fn camera(&self) -> Camera {
        self.camera
    }

    /// Reconstructs the nearest surface at a physical pixel center using this
    /// frame's camera and linear depth. Background, out-of-bounds coordinates,
    /// or missing depth samples return `None`. Invalid non-background depth values and
    /// unrepresentable world coordinates return an error.
    pub fn world_position_at(&self, x: u32, y: u32) -> Result<Option<[f32; 3]>, CameraError> {
        let [width, height] = self.pixels.size;
        if x >= width || y >= height {
            return Ok(None);
        }
        let index = (y as usize)
            .checked_mul(width as usize)
            .and_then(|row| row.checked_add(x as usize));
        let depth = index.and_then(|index| self.pixels.linear_depth.as_ref()?.get(index));
        let Some(&depth) = depth else {
            return Ok(None);
        };
        if self.pixels.depth_background.is_background(depth) {
            return Ok(None);
        }
        self.camera
            .screen_to_world(
                Bounds::new(
                    point(px(0.), px(0.)),
                    size(px(width as f32), px(height as f32)),
                ),
                point(px(x as f32 + 0.5), px(y as f32 + 0.5)),
                depth,
            )
            .map(Some)
    }

    pub fn objects(&self) -> &[RenderObject] {
        &self.objects
    }
    pub fn object(&self, output_id: u32) -> Option<&RenderObject> {
        lookup(&self.objects, output_id)
    }
    pub fn object_at(&self, x: u32, y: u32) -> Option<&RenderObject> {
        let [width, height] = self.pixels.size;
        if x >= width || y >= height {
            return None;
        }
        self.object(
            *self
                .pixels
                .object_ids
                .as_ref()?
                .get(y as usize * width as usize + x as usize)?,
        )
    }
}
fn lookup(objects: &[RenderObject], output_id: u32) -> Option<&RenderObject> {
    objects.get(output_id.checked_sub(1)? as usize)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Projection;

    fn depth_frame(projection: Projection) -> ReadFrame {
        ReadFrame {
            pixels: Scene3dPixels {
                depth_background: Default::default(),
                size: [3, 2],
                rgba: None,
                linear_rgba: None,
                object_ids: None,
                linear_depth: Some(vec![0., 2., 4., 6., 0., 8.]),
                world_normals: None,
            },
            objects: Arc::from([]),
            camera: Camera {
                eye: [2., 3., 5.],
                target: [2., 3., 4.],
                projection,
                lens_shift: [0.25, -0.5],
                ..Default::default()
            },
        }
    }

    #[test]
    fn depth_pixels_reconstruct_world_surfaces_without_an_id_channel() {
        for (projection, upper_right, lower_left) in [
            (
                Projection::Perspective {
                    vertical_fov: std::f32::consts::FRAC_PI_2,
                },
                [7.5, 3., 1.],
                [-1.75, -3., -1.],
            ),
            (
                Projection::Orthographic { vertical_size: 4. },
                [4.75, 3., 1.],
                [0.75, 1., -1.],
            ),
        ] {
            let frame = depth_frame(projection);
            for (pixel, expected) in [([2, 0], upper_right), ([0, 1], lower_left)] {
                let world = frame
                    .world_position_at(pixel[0], pixel[1])
                    .unwrap()
                    .unwrap();
                for (actual, expected) in world.into_iter().zip(expected) {
                    assert!((actual - expected).abs() < 1e-5, "{actual} != {expected}");
                }
            }
            assert_eq!(frame.world_position_at(0, 0).unwrap(), None);
            assert_eq!(frame.world_position_at(1, 1).unwrap(), None);
            assert_eq!(frame.world_position_at(3, 0).unwrap(), None);
            assert_eq!(frame.world_position_at(0, 2).unwrap(), None);
            assert_eq!(frame.world_position_at(u32::MAX, u32::MAX).unwrap(), None);
        }
    }

    #[test]
    fn depth_pixel_queries_handle_missing_samples_and_report_invalid_values() {
        let mut frame = depth_frame(Projection::default());
        for depth in [-1., f32::INFINITY, f32::NAN] {
            frame.pixels.linear_depth.as_mut().unwrap()[2] = depth;
            assert_eq!(
                frame.world_position_at(2, 0),
                Err(CameraError::InvalidDepth)
            );
        }
        frame.pixels.linear_depth = Some(vec![0.]);
        assert_eq!(frame.world_position_at(2, 0).unwrap(), None);
        frame.pixels.linear_depth = None;
        assert_eq!(frame.world_position_at(2, 0).unwrap(), None);
        frame.pixels.size = [0, 0];
        assert_eq!(frame.world_position_at(0, 0).unwrap(), None);
    }
}
