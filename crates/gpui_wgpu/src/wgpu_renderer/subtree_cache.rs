use super::scene_snapshot::{OutputValidity, SceneSnapshot};
use super::{GammaParams, WgpuRenderer};
use gpui::{Scene, SubtreeLayer};

#[derive(Default)]
struct Entry {
    snapshot: Option<SceneSnapshot>,
    validity: OutputValidity,
    layer: Option<usize>,
    reuse: bool,
}

/// Retains metadata for existing depth-indexed textures, without allocating targets.
#[derive(Default)]
pub(super) struct SubtreeCaptureCache {
    entries: Vec<Entry>,
    parameters: Option<GammaParams>,
    // Caller-owned encoders may be submitted after a later frame. Until the
    // textures are replaced, their eventual contents cannot be trusted.
    external_writes: bool,
    #[cfg(test)]
    pub(super) hits: std::cell::Cell<usize>,
}

impl SubtreeCaptureCache {
    pub(super) fn reuse(&self, depth: usize, layer: &SubtreeLayer) -> bool {
        let reuse = self
            .entries
            .get(depth)
            .is_some_and(|entry| entry.reuse && entry.layer == Some(layer as *const _ as usize));
        #[cfg(test)]
        if reuse {
            self.hits.set(self.hits.get() + 1);
        }
        reuse
    }

    pub(super) fn encoded(&self, depth: usize, layer: &SubtreeLayer) {
        if let Some(entry) = self.entries.get(depth)
            && entry.layer == Some(layer as *const _ as usize)
        {
            entry.validity.encoded();
        }
    }

    pub(super) fn commit(&self, submitted: bool) {
        for entry in &self.entries {
            entry.validity.commit(submitted);
        }
    }
}

impl WgpuRenderer {
    pub(super) fn prepare_subtree_cache(
        &mut self,
        scene: &Scene,
        retain: bool,
        parameters: GammaParams,
    ) {
        // Siblings share a texture at their depth. Only a sole writer can leave
        // reusable pixels there. Multipass and two-input layers also reuse other
        // depths as scratch targets; conservatively bypass such frames entirely.
        fn collect<'a>(
            scene: &'a Scene,
            depth: usize,
            levels: &mut Vec<Vec<&'a SubtreeLayer>>,
        ) -> bool {
            for layer in &scene.subtree_layers {
                if layer.scene3d.is_some()
                    || layer.second_scene.is_some()
                    || !layer.intermediate_effects.is_empty()
                {
                    return false;
                }
                if levels.len() <= depth {
                    levels.resize_with(depth + 1, Vec::new);
                }
                levels[depth].push(layer);
                if !collect(&layer.scene, depth + 1, levels) {
                    return false;
                }
            }
            true
        }
        let mut levels = Vec::new();
        let supported = retain
            && !self.resources().subtree_cache.external_writes
            && collect(scene, 0, &mut levels)
            && levels.iter().flatten().all(|layer| {
                self.resources()
                    .subtree_effect_pipelines
                    .get(&(
                        layer.composite.shader.id().as_u64(),
                        self.surface_config.format,
                    ))
                    .is_some_and(Option::is_some)
            });
        let snapshots = supported.then(|| {
            levels
                .iter()
                .map(|layers| {
                    if layers.len() != 1 {
                        return None;
                    }
                    let layer = layers[0];
                    SceneSnapshot::new(&layer.scene, |tile| self.atlas.tile_generation(tile))
                        .map(|snapshot| (layer as *const _ as usize, snapshot))
                })
                .collect::<Vec<_>>()
        });
        let cache = &mut self.resources_mut().subtree_cache;
        cache.external_writes |= !retain;
        #[cfg(test)]
        cache.hits.set(0);
        if cache.parameters.as_ref() != Some(&parameters) {
            cache.entries.clear();
            cache.parameters = Some(parameters);
        }
        let Some(snapshots) = snapshots else {
            cache.entries.clear();
            return;
        };
        cache.entries.resize_with(snapshots.len(), Entry::default);
        for (entry, snapshot) in cache.entries.iter_mut().zip(snapshots) {
            let Some((layer, snapshot)) = snapshot else {
                *entry = Entry::default();
                continue;
            };
            let reusable = entry.validity.reusable()
                && entry
                    .snapshot
                    .as_ref()
                    .is_some_and(|old| snapshot.matches(old));
            if !reusable {
                entry.validity = OutputValidity::default();
            }
            entry.snapshot = Some(snapshot);
            entry.layer = Some(layer);
            entry.reuse = reusable;
        }
    }
}

#[cfg(all(test, not(target_family = "wasm")))]
mod tests;
