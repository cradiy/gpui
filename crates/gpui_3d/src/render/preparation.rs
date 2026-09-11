use std::{error::Error, fmt, sync::Arc};

use gpui::{ImageSource, MeshTexture3d, Scene3dFrame, UiTexture3d};

use crate::{NodeHandle, ObjectId, Scene, Texture, TextureSlot};

/// An object's identity within a prepared scene, including omitted objects.
/// Numeric IDs are frame-local; retain this mapping with the rendered output.
#[derive(Clone, Debug)]
pub struct RenderObject {
    pub output_id: u32,
    pub object_index: usize,
    pub id: Option<ObjectId>,
    pub node: Option<NodeHandle>,
}

/// Material input requested during scene preparation.
#[derive(Clone, Copy)]
pub enum TextureSource<'a> {
    /// No texture. Resolve to `ResolvedTexture::None`.
    Solid,
    /// First image frame. Resolve to a renderer-local atlas image.
    Image(&'a ImageSource),
    /// The viewport's UI capture. Resolve to `ResolvedTexture::Subtree`.
    Ui,
}

/// One active material input. Object indices and IDs refer to the original scene.
#[derive(Clone, Copy)]
pub struct TextureRequest<'a> {
    pub object_index: usize,
    pub output_id: u32,
    pub object_id: Option<&'a ObjectId>,
    pub node: Option<NodeHandle>,
    pub slot: TextureSlot,
    pub source: TextureSource<'a>,
}

/// Readiness of a material input. Resource failures use the resolver's `Err` result.
#[derive(Clone, Copy, Debug)]
pub enum TextureState {
    /// Renderer-local texture storage is ready and remains resident through submission.
    Ready(MeshTexture3d),
    /// Omit the object until a subsequent preparation resolves every active input.
    Pending,
}

/// An unresolved input, identified without retaining a borrowed image callback.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PendingTexture {
    pub object_index: usize,
    pub output_id: u32,
    pub slot: TextureSlot,
}

/// Scene validation or resource-resolution failure. No prepared frame is returned.
#[derive(Debug)]
pub enum PrepareError {
    /// Invalid camera, light, material, or object parameters.
    InvalidScene(anyhow::Error),
    /// Resolver failure associated with its original object and material slot.
    Resource {
        object_index: usize,
        slot: TextureSlot,
        source: anyhow::Error,
    },
    /// The resolved texture kind does not match the requested input.
    InvalidResolution {
        object_index: usize,
        slot: TextureSlot,
    },
}

impl fmt::Display for PrepareError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidScene(source) => write!(f, "invalid 3D scene: {source}"),
            Self::Resource {
                object_index,
                slot,
                source,
            } => write!(f, "object {object_index} {slot:?} resource: {source}"),
            Self::InvalidResolution { object_index, slot } => write!(
                f,
                "object {object_index} {slot:?}: resolved texture kind does not match the input"
            ),
        }
    }
}

impl Error for PrepareError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::InvalidScene(source) | Self::Resource { source, .. } => Some(source.as_ref()),
            Self::InvalidResolution { .. } => None,
        }
    }
}

/// Owned render inputs, identity mapping, and outstanding texture requests.
/// Atlas allocations belong to the resolver's renderer and are not retained by this value.
#[derive(Clone, Debug)]
pub struct PreparedScene {
    frame: Scene3dFrame,
    objects: Arc<[RenderObject]>,
    pending: Vec<PendingTexture>,
}

pub(super) struct PreparationPlan {
    frame: Scene3dFrame,
    objects: Arc<[RenderObject]>,
}

impl PreparedScene {
    pub fn frame(&self) -> &Scene3dFrame {
        &self.frame
    }
    pub fn into_frame(self) -> Scene3dFrame {
        self.frame
    }
    pub fn objects(&self) -> &[RenderObject] {
        &self.objects
    }
    /// Zero is background; unknown values return `None`.
    pub fn object(&self, output_id: u32) -> Option<&RenderObject> {
        self.objects.get(output_id.checked_sub(1)? as usize)
    }
    pub fn pending_textures(&self) -> &[PendingTexture] {
        &self.pending
    }
    /// All requested inputs are ready. Culled objects do not affect readiness.
    pub fn is_ready(&self) -> bool {
        self.pending.is_empty()
    }
    /// Shared identity mapping for retaining with renderer-owned outputs.
    pub fn identities(&self) -> Arc<[RenderObject]> {
        self.objects.clone()
    }
}

impl Scene {
    /// Validates and prepares visible camera/shadow inputs without a window or GPU.
    /// The resolver owns image loading, upload, cache lifetime, and redraw scheduling.
    /// Pending inputs omit their object but do not prevent requests for its other inputs.
    /// Scene validation completes before the first resource request.
    /// Failures return no frame; resolver side effects are not rolled back.
    pub fn prepare(
        &self,
        aspect: f32,
        ui_texture: Option<UiTexture3d>,
        resolve: impl FnMut(TextureRequest<'_>) -> anyhow::Result<TextureState>,
    ) -> Result<PreparedScene, PrepareError> {
        self.resolve_plan(&self.prepare_plan(aspect, ui_texture)?, resolve)
    }

    pub(super) fn prepare_plan(
        &self,
        aspect: f32,
        ui_texture: Option<UiTexture3d>,
    ) -> Result<PreparationPlan, PrepareError> {
        let frame = self
            .plan_frame(aspect, ui_texture)
            .map_err(PrepareError::InvalidScene)?;
        let objects = self
            .objects
            .iter()
            .enumerate()
            .map(|(index, object)| RenderObject {
                output_id: index as u32 + 1,
                object_index: index,
                id: object.id.clone(),
                node: object.node,
            })
            .collect();
        Ok(PreparationPlan { frame, objects })
    }

    pub(super) fn resolve_plan(
        &self,
        plan: &PreparationPlan,
        mut resolve: impl FnMut(TextureRequest<'_>) -> anyhow::Result<TextureState>,
    ) -> Result<PreparedScene, PrepareError> {
        let mut pending = Vec::new();
        let frame = self
            .bind_frame(&plan.frame, |index, slot, texture| {
                let object = &self.objects[index];
                let source = match texture {
                    Texture::None => TextureSource::Solid,
                    Texture::Image(image) => TextureSource::Image(image),
                    Texture::Ui => TextureSource::Ui,
                };
                let request = TextureRequest {
                    object_index: index,
                    output_id: index as u32 + 1,
                    object_id: object.id.as_ref(),
                    node: object.node,
                    slot,
                    source,
                };
                match resolve(request).map_err(|source| PrepareError::Resource {
                    object_index: index,
                    slot,
                    source,
                })? {
                    TextureState::Pending => {
                        pending.push(PendingTexture {
                            object_index: index,
                            output_id: request.output_id,
                            slot,
                        });
                        Ok(None)
                    }
                    TextureState::Ready(texture) => {
                        if !matches!(
                            (source, texture),
                            (TextureSource::Solid, MeshTexture3d::None)
                                | (TextureSource::Image(_), MeshTexture3d::Image(_))
                                | (TextureSource::Ui, MeshTexture3d::Subtree)
                        ) {
                            return Err(PrepareError::InvalidResolution {
                                object_index: index,
                                slot,
                            }
                            .into());
                        }
                        Ok(Some(texture))
                    }
                }
            })
            .map_err(|error| {
                error
                    .downcast::<PrepareError>()
                    .unwrap_or_else(PrepareError::InvalidScene)
            })?;
        Ok(PreparedScene {
            frame,
            objects: plan.objects.clone(),
            pending,
        })
    }
}
