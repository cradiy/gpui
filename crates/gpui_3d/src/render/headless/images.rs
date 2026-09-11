use std::{
    borrow::Cow,
    collections::{BTreeMap, VecDeque},
    sync::Arc,
};

use crate::{PreparationCache, PrepareError, PreparedScene, Scene, TextureSource, TextureState};
use anyhow::{Context as _, bail, ensure};
use gpui::{ImageId, ImageSource, MeshTexture3d, PlatformAtlas, RenderImageParams};

/// Limits for headless atlas images unused by the latest prepared scene.
/// Defaults to no idle retention. Active images are not subject to these limits.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ImageCacheLimits {
    pub max_idle_images: usize,
    /// Sum of first-frame BGRA pixel payloads, excluding atlas overhead and mipmaps.
    pub max_idle_bytes: u64,
}

/// Active and idle atlas image payloads, excluding caller-owned decoded pixels.
/// Pixel payload is not a measurement of allocated GPU memory.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ImageCacheUsage {
    pub active_images: usize,
    pub active_bytes: u64,
    pub idle_images: usize,
    pub idle_bytes: u64,
}

#[derive(Default)]
pub(super) struct ImageCache {
    limits: ImageCacheLimits,
    byte_limit: Option<u64>,
    active: BTreeMap<ImageId, u64>,
    idle: VecDeque<(ImageId, u64)>,
}

impl ImageCache {
    pub(super) fn prepare(
        &mut self,
        preparation: &mut PreparationCache,
        scene: &Scene,
        aspect: f32,
        max_dimension: u32,
        atlas: &impl PlatformAtlas,
    ) -> Result<Arc<PreparedScene>, PrepareError> {
        let mut used = BTreeMap::new();
        let mut used_bytes = 0u64;
        let prepared = preparation.prepare(scene, aspect, None, |request| {
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
                    if !used.contains_key(&image.id) {
                        used_bytes = used_bytes
                            .checked_add(bytes.len() as u64)
                            .context("decoded image payload exceeds u64")?;
                        if let Some(limit) = self.byte_limit {
                            ensure!(
                                used_bytes <= limit,
                                "decoded images need {used_bytes} bytes, request limit is {limit}"
                            );
                        }
                    }
                    let key = RenderImageParams {
                        image_id: image.id,
                        frame_index: 0,
                    }
                    .into();
                    let tile = atlas
                        .get_or_insert_with(&key, &mut || Ok(Some((size, Cow::Borrowed(bytes)))))?
                        .context("image allocation failed")?;
                    used.insert(image.id, bytes.len() as u64);
                    MeshTexture3d::Image(tile)
                }
                TextureSource::Image(_) => {
                    bail!("direct rendering requires an ImageSource::Render with decoded pixels")
                }
            };
            Ok(TextureState::Ready(texture))
        });
        let prepared = match prepared {
            Ok(prepared) => {
                self.finish(used, |image_id| remove_image(atlas, image_id));
                prepared
            }
            Err(error) => {
                self.abort(used, |image_id| remove_image(atlas, image_id));
                return Err(error);
            }
        };
        Ok(prepared)
    }

    pub(super) fn limits(&self) -> ImageCacheLimits {
        self.limits
    }

    pub(super) fn byte_limit(&self) -> Option<u64> {
        self.byte_limit
    }

    pub(super) fn set_byte_limit(&mut self, bytes: Option<u64>) {
        self.byte_limit = bytes;
    }

    pub(super) fn usage(&self) -> ImageCacheUsage {
        ImageCacheUsage {
            active_images: self.active.len(),
            active_bytes: self.active.values().sum(),
            idle_images: self.idle.len(),
            idle_bytes: self.idle.iter().map(|(_, bytes)| bytes).sum(),
        }
    }

    pub(super) fn set_limits(&mut self, limits: ImageCacheLimits, remove: impl FnMut(ImageId)) {
        self.limits = limits;
        self.trim(remove);
    }

    fn finish(&mut self, used: BTreeMap<ImageId, u64>, remove: impl FnMut(ImageId)) {
        self.idle.retain(|(id, _)| !used.contains_key(id));
        for (id, bytes) in std::mem::replace(&mut self.active, used) {
            if !self.active.contains_key(&id) {
                self.idle.push_front((id, bytes));
            }
        }
        self.trim(remove);
    }

    fn abort(&self, used: BTreeMap<ImageId, u64>, mut remove: impl FnMut(ImageId)) {
        for id in used.keys() {
            if !self.active.contains_key(id) && !self.idle.iter().any(|(idle, _)| idle == id) {
                remove(*id);
            }
        }
    }

    /// Called after the private atlas has been cleared.
    pub(super) fn clear(&mut self) {
        self.active.clear();
        self.idle.clear();
    }

    fn trim(&mut self, mut remove: impl FnMut(ImageId)) {
        let mut bytes: u128 = self.idle.iter().map(|(_, bytes)| u128::from(*bytes)).sum();
        while self.idle.len() > self.limits.max_idle_images
            || bytes > u128::from(self.limits.max_idle_bytes)
        {
            let (id, removed_bytes) = self.idle.pop_back().expect("idle payload exceeds limit");
            bytes -= u128::from(removed_bytes);
            remove(id);
        }
    }
}

#[cfg(test)]
mod tests;

pub(super) fn remove_image(atlas: &impl PlatformAtlas, image_id: ImageId) {
    atlas.remove(
        &RenderImageParams {
            image_id,
            frame_index: 0,
        }
        .into(),
    );
}
