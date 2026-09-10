#![doc = include_str!("../../docs/topics/headless.md")]

mod coverage;
mod depth;
mod gpu_labels;
mod labels;
pub use coverage::{CoverageError, FrameCoverage, ObjectCoverage};
pub use depth::{DepthComparison, DepthQueryError, DepthRelation};
pub use gpu_labels::RenderedLabels;
pub use labels::{FrameLabels, LabelError};

use std::{borrow::Cow, collections::HashSet, sync::Arc};

use anyhow::{Context as _, Result, bail, ensure};
use gpui::{
    Bounds, ImageId, ImageSource, MeshTexture3d, PlatformAtlas, RenderImageParams, point, px, size,
};
use gpui_wgpu::{Scene3dGpuOutput, Scene3dReadback, WgpuScene3dRenderer};

pub use crate::RenderObject;
use crate::{Camera, CameraError, PreparationCache, Scene, TextureSource, TextureState};

pub use gpui_wgpu::{
    IdRemapConfig, Scene3dCapabilities, Scene3dChannels, Scene3dDeviceCapabilities,
    Scene3dDrawStatistics, Scene3dFormatCapabilities, Scene3dOutputConfig, Scene3dPixels,
    Scene3dTargetMemory, WgpuContext, WgpuIdRemapper,
};

/// Window-free renderer for solid and decoded-image materials. Does not load
/// resources, execute custom image callbacks, or capture UI subtrees.
pub struct HeadlessRenderer {
    renderer: WgpuScene3dRenderer,
    images: HashSet<ImageId>,
    preparation: PreparationCache,
}
impl HeadlessRenderer {
    pub fn new() -> Result<Self> {
        Ok(Self {
            renderer: WgpuScene3dRenderer::new_headless()?,
            images: HashSet::new(),
            preparation: PreparationCache::new(),
        })
    }
    /// Reuses a GPU context instead of creating a device for each renderer.
    pub fn with_context(context: WgpuContext) -> Result<Self> {
        Ok(Self {
            renderer: WgpuScene3dRenderer::new(context)?,
            images: HashSet::new(),
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
        let mut used = HashSet::new();
        let prepared = self.preparation.prepare(
            scene,
            config.size[0] as f32 / config.size[1] as f32,
            None,
            |request| {
                let texture = match request.source {
                    TextureSource::Solid => MeshTexture3d::None,
                    TextureSource::Ui => bail!("UI textures require a viewport capture"),
                    TextureSource::Image(ImageSource::Render(image)) => {
                        let bytes = image.as_bytes(0).context("decoded image has no frame")?;
                        let size = image.size(0);
                        ensure!(
                            size.width.0 > 0
                                && size.height.0 > 0
                                && size.width.0 as u32 <= max_dimension
                                && size.height.0 as u32 <= max_dimension,
                            "decoded image has invalid or unsupported dimensions"
                        );
                        let key = RenderImageParams {
                            image_id: image.id,
                            frame_index: 0,
                        }
                        .into();
                        let tile = atlas
                            .get_or_insert_with(&key, &mut || {
                                Ok(Some((size, Cow::Borrowed(bytes))))
                            })?
                            .context("image allocation failed")?;
                        used.insert(image.id);
                        MeshTexture3d::Image(tile)
                    }
                    TextureSource::Image(_) => bail!(
                        "direct rendering requires an ImageSource::Render with decoded pixels"
                    ),
                };
                Ok(TextureState::Ready(texture))
            },
        );
        for image_id in self.images.difference(&used) {
            atlas.remove(
                &RenderImageParams {
                    image_id: *image_id,
                    frame_index: 0,
                }
                .into(),
            );
        }
        self.images = used;
        let prepared = prepared?;
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
        Ok(FrameReadback {
            pending: self.output.readback()?,
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
    /// or missing depth samples return `None`. Invalid nonzero depth values and
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
        if depth == 0. {
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
