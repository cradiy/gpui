use crate::{AtlasKey, PlatformAtlas, RenderImageParams};
use collections::FxHashMap;
use parking_lot::Mutex;
use std::sync::Weak;

/// Backend-side ownership tracking for transient image tiles.
#[doc(hidden)]
#[derive(Default)]
pub struct AtlasImageLifetimes(Mutex<FxHashMap<RenderImageParams, Vec<Weak<()>>>>);

impl AtlasImageLifetimes {
    pub fn retain(&self, key: &RenderImageParams, lifetime: Weak<()>) {
        let mut entries = self.0.lock();
        let owners = entries.entry(key.clone()).or_default();
        owners.retain(|owner| owner.strong_count() > 0);
        if !owners.iter().any(|owner| owner.ptr_eq(&lifetime)) {
            owners.push(lifetime);
        }
    }

    pub fn collect(&self, atlas: &dyn PlatformAtlas) {
        self.0.lock().retain(|key, owners| {
            owners.retain(|owner| owner.strong_count() > 0);
            if owners.is_empty() {
                atlas.remove(&AtlasKey::TransientImage(key.clone()));
                false
            } else {
                true
            }
        });
    }
}
