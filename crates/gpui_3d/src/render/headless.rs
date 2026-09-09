#![doc = include_str!("../../docs/headless.md")]

use std::{borrow::Cow, collections::HashSet, sync::Arc};

use anyhow::{Context as _, Result, bail, ensure};
use gpui::{ImageId, ImageSource, MeshTexture3d, PlatformAtlas, RenderImageParams};
use gpui_wgpu::{Scene3dGpuOutput, Scene3dReadback, WgpuScene3dRenderer};

pub use crate::RenderObject;
use crate::{Scene, TextureSource, TextureState};

pub use gpui_wgpu::{
    Scene3dCapabilities, Scene3dChannels, Scene3dOutputConfig, Scene3dPixels, WgpuContext,
};

/// Window-free renderer for solid and decoded-image materials. Does not load
/// resources, execute custom image callbacks, or capture UI subtrees.
pub struct HeadlessRenderer {
    renderer: WgpuScene3dRenderer,
    images: HashSet<ImageId>,
}
impl HeadlessRenderer {
    pub fn new() -> Result<Self> {
        Ok(Self {
            renderer: WgpuScene3dRenderer::new_headless()?,
            images: HashSet::new(),
        })
    }
    /// Reuses a GPU context instead of creating a device for each renderer.
    pub fn with_context(context: WgpuContext) -> Result<Self> {
        Ok(Self {
            renderer: WgpuScene3dRenderer::new(context)?,
            images: HashSet::new(),
        })
    }
    pub fn context(&self) -> &WgpuContext {
        self.renderer.context()
    }
    pub fn capabilities(&self) -> Scene3dCapabilities {
        self.renderer.capabilities()
    }

    /// Renders the supplied scene without a native window or UI layout. Geometry,
    /// projection, lighting, and alpha modes share the viewport implementation.
    pub fn render(&mut self, scene: &Scene, config: Scene3dOutputConfig) -> Result<RenderedFrame> {
        self.capabilities().validate(config)?;
        let max_dimension = self.capabilities().max_dimension;
        let atlas = self.renderer.sprite_atlas();
        let mut used = HashSet::new();
        let prepared = scene.prepare(
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
        Ok(RenderedFrame { output, objects })
    }
}

/// GPU outputs and their immutable identity mapping. Older frames survive scene
/// edits, subsequent renders, renderer destruction, and output size changes.
pub struct RenderedFrame {
    output: Scene3dGpuOutput,
    objects: Arc<[RenderObject]>,
}
impl RenderedFrame {
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
        })
    }
}

/// Nonblocking GPU readback with the same frame-local object mapping.
pub struct FrameReadback {
    pending: Scene3dReadback,
    objects: Arc<[RenderObject]>,
}
impl FrameReadback {
    pub fn try_read(&mut self) -> Result<Option<ReadFrame>> {
        Ok(self.pending.try_read()?.map(|pixels| ReadFrame {
            pixels,
            objects: self.objects.clone(),
        }))
    }
}

/// Tightly packed, top-left-origin pixels and the identities that produced them.
pub struct ReadFrame {
    pub pixels: Scene3dPixels,
    objects: Arc<[RenderObject]>,
}
impl ReadFrame {
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
