use std::sync::Arc;

use anyhow::{Result, ensure};
use gpui::RenderImage;
use parking_lot::Mutex;
use sha2::{Digest, Sha256};

use crate::{
    EncodedImage, ImageDecodeLimits, ResourceCacheLimits,
    cache::Retention,
    image::{CachedImage, decode_image},
};

/// Maximum BGRA payload bytes and entries retained by a shared image cache.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImageCacheLimits {
    pub bytes: usize,
    pub entries: usize,
}

impl Default for ImageCacheLimits {
    fn default() -> Self {
        Self {
            bytes: 256 * 1024 * 1024,
            entries: 256,
        }
    }
}

impl ImageCacheLimits {
    fn retention(self) -> ResourceCacheLimits {
        ResourceCacheLimits {
            bytes: self.bytes,
            entries: self.entries,
        }
    }
}

#[derive(PartialEq, Eq, Hash)]
struct Key {
    digest: [u8; 32],
    mime: Option<String>,
}

impl Key {
    fn new(encoded: &EncodedImage) -> Self {
        Self {
            digest: Sha256::digest(encoded.bytes()).into(),
            mime: encoded.mime_type().map(str::to_owned),
        }
    }
}

/// Shared LRU cache of PNG/JPEG pixels keyed by encoded SHA-256 and declared MIME.
/// Cache entries retain pixels and admission metadata, not encoded source buffers.
/// Decoding is synchronous on the calling thread and occurs outside the cache lock.
#[derive(Clone)]
pub struct ImageCache {
    state: Arc<Mutex<Retention<Key, CachedImage>>>,
}

impl Default for ImageCache {
    fn default() -> Self {
        Self::new(ImageCacheLimits::default())
    }
}

impl ImageCache {
    pub fn new(limits: ImageCacheLimits) -> Self {
        Self {
            state: Arc::new(Mutex::new(Retention::new(limits.retention()))),
        }
    }

    pub fn limits(&self) -> ImageCacheLimits {
        let limits = self.state.lock().limits();
        ImageCacheLimits {
            bytes: limits.bytes,
            entries: limits.entries,
        }
    }

    pub fn len(&self) -> usize {
        self.state.lock().len()
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Retained BGRA bytes, excluding metadata, consumers and transient codec work.
    pub fn cached_bytes(&self) -> usize {
        self.state.lock().bytes()
    }

    /// Evicts to both limits immediately. Either zero limit disables retention.
    /// Decodes already in progress cannot insert across a limit change.
    pub fn set_limits(&self, limits: ImageCacheLimits) {
        self.state.lock().set_limits(limits.retention());
    }

    /// Removes matching pixels and prevents all in-progress decodes from inserting.
    /// Previously returned images remain valid. In-progress callers keep their result.
    pub fn invalidate(&self, encoded: &EncodedImage) -> bool {
        let key = Key::new(encoded);
        self.state.lock().invalidate(&key)
    }

    /// Releases retained pixels and prevents in-progress decodes from inserting.
    /// Images already returned to callers remain valid.
    pub fn clear(&self) {
        self.state.lock().clear();
    }

    /// Decodes or reuses one image. Hits still enforce current dimensions, pixels,
    /// output bytes and conservative working admission. Errors are not retained.
    pub fn decode(
        &self,
        encoded: &EncodedImage,
        limits: ImageDecodeLimits,
    ) -> Result<Arc<RenderImage>> {
        self.decode_counted(encoded, limits, &mut 0)
    }

    pub(crate) fn decode_counted(
        &self,
        encoded: &EncodedImage,
        limits: ImageDecodeLimits,
        used: &mut u64,
    ) -> Result<Arc<RenderImage>> {
        ensure!(
            u64::try_from(encoded.bytes().len())? <= limits.working_bytes,
            "encoded image exceeds working byte limit"
        );
        let key = Key::new(encoded);
        let epoch = {
            let mut state = self.state.lock();
            if let Some(cached) = state.peek(&key) {
                *used = cached.admit(limits, *used)?;
                return Ok(state.get(&key).unwrap().image);
            }
            state.epoch()
        };
        let cached = decode_image(encoded, limits, used)?;
        let image = cached.image.clone();
        let bytes = cached.bytes();
        self.state.lock().insert(key, cached, bytes, &epoch);
        Ok(image)
    }
}
