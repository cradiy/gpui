use std::{collections::VecDeque, sync::Arc};

use gpui::{MeshTexture3d, UiTexture3d};

use super::preparation::PreparationPlan;

use crate::{
    Camera, PrepareError, PreparedScene, Scene, Texture, TextureRequest, TextureSlot,
    TextureSource, TextureState,
};

/// Retains bounded CPU preparations, shared across unchanged scene clones.
/// Every call refreshes active texture requests through the supplied resolver.
/// This cache neither owns atlas allocations nor caches rendered pixels.
pub struct PreparationCache {
    entries: VecDeque<Entry>,
    capacity: usize,
}

impl Default for PreparationCache {
    fn default() -> Self {
        Self::with_capacity(1)
    }
}

struct Entry {
    revision: Arc<()>,
    camera: Camera,
    aspect: f32,
    ui: Option<[f32; 3]>,
    resources: Vec<Resource>,
    plan: PreparationPlan,
    prepared: Arc<PreparedScene>,
}

struct Resource {
    object_index: usize,
    slot: TextureSlot,
    state: TextureState,
}

impl PreparationCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Retains at most `capacity` preparations, evicting the least recently used.
    /// Zero disables retention. This is an entry limit, not a byte budget.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            entries: VecDeque::new(),
            capacity,
        }
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Applies the entry limit immediately, preserving the most recently used
    /// preparations. Previously returned preparations remain valid.
    pub fn set_capacity(&mut self, capacity: usize) {
        self.capacity = capacity;
        self.entries.truncate(capacity);
    }

    /// Releases retained CPU inputs. Previously returned preparations remain valid.
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Reuses validation, culling, matrices, and identity mapping when inputs match.
    /// Scene content, camera, aspect, or UI dimensions/density changes select or
    /// rebuild a plan. Texture changes only rebind its resources and update
    /// readiness. Each active input is resolved once per call, including cache
    /// hits. Failures discard only the matching entry
    /// and return no frame; other entries remain available.
    /// The caller owns resource lifetime, completion notifications, and redraws.
    pub fn prepare(
        &mut self,
        scene: &Scene,
        aspect: f32,
        ui_texture: Option<UiTexture3d>,
        mut resolve: impl FnMut(TextureRequest<'_>) -> anyhow::Result<TextureState>,
    ) -> Result<Arc<PreparedScene>, PrepareError> {
        let ui = ui_texture.map(|ui| {
            [
                f32::from(ui.logical_size().width),
                f32::from(ui.logical_size().height),
                ui.scale_factor(),
            ]
        });
        let previous = self
            .entries
            .iter()
            .position(|entry| {
                Arc::ptr_eq(&entry.revision, &scene.preparation_revision)
                    && entry.camera == scene.camera
                    && entry.aspect == aspect
                    && entry.ui == ui
            })
            .and_then(|index| self.entries.remove(index));
        let entry = if let Some(mut entry) = previous {
            let mut changed = false;
            for resource in &mut entry.resources {
                let object = &scene.objects[resource.object_index];
                let source = if resource.slot == TextureSlot::BaseColor {
                    match &object.material.texture {
                        Texture::None => TextureSource::Solid,
                        Texture::Image(image) => TextureSource::Image(image),
                        Texture::Ui => TextureSource::Ui,
                    }
                } else {
                    let (_, texture) = object
                        .material
                        .lighting_textures()
                        .find(|(slot, _)| *slot == resource.slot)
                        .expect("retained material input");
                    TextureSource::Image(&texture.image)
                };
                let state = resolve(TextureRequest {
                    object_index: resource.object_index,
                    output_id: resource.object_index as u32 + 1,
                    object_id: object.id.as_ref(),
                    node: object.node,
                    slot: resource.slot,
                    source,
                })
                .map_err(|source| PrepareError::Resource {
                    object_index: resource.object_index,
                    slot: resource.slot,
                    source,
                })?;
                changed |= !same_state(resource.state, state);
                resource.state = state;
            }
            if changed {
                let mut resources = entry.resources.iter();
                entry.prepared = Arc::new(scene.resolve_plan(&entry.plan, |request| {
                    let resource = resources.next().expect("retained texture request");
                    debug_assert_eq!(
                        (resource.object_index, resource.slot),
                        (request.object_index, request.slot)
                    );
                    Ok(resource.state)
                })?);
            }
            entry
        } else {
            let mut resources = Vec::new();
            let plan = scene.prepare_plan(aspect, ui_texture)?;
            let prepared = scene.resolve_plan(&plan, |request| {
                let state = resolve(request)?;
                resources.push(Resource {
                    object_index: request.object_index,
                    slot: request.slot,
                    state,
                });
                Ok(state)
            })?;
            Entry {
                revision: scene.preparation_revision.clone(),
                camera: scene.camera,
                aspect,
                ui,
                resources,
                plan,
                prepared: Arc::new(prepared),
            }
        };
        let prepared = entry.prepared.clone();
        if self.capacity > 0 {
            self.entries.truncate(self.capacity - 1);
            self.entries.push_front(entry);
        }
        Ok(prepared)
    }
}

#[cfg(test)]
mod tests;

fn same_state(left: TextureState, right: TextureState) -> bool {
    match (left, right) {
        (TextureState::Pending, TextureState::Pending)
        | (TextureState::Ready(MeshTexture3d::None), TextureState::Ready(MeshTexture3d::None))
        | (
            TextureState::Ready(MeshTexture3d::Subtree),
            TextureState::Ready(MeshTexture3d::Subtree),
        ) => true,
        (
            TextureState::Ready(MeshTexture3d::Image(left)),
            TextureState::Ready(MeshTexture3d::Image(right)),
        ) => left == right,
        _ => false,
    }
}
