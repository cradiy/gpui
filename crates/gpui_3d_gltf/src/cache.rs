use std::{future::Future, hash::Hash, sync::Arc};

use anyhow::{Result, ensure};
use indexmap::IndexMap;
use parking_lot::Mutex;

/// Retained encoded payload limits, independent of per-document admission.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResourceCacheLimits {
    pub bytes: usize,
    pub entries: usize,
}

impl Default for ResourceCacheLimits {
    fn default() -> Self {
        Self {
            bytes: 64 * 1024 * 1024,
            entries: 256,
        }
    }
}

pub(crate) struct Retention<K, V> {
    entries: IndexMap<K, (V, usize)>,
    bytes: usize,
    limits: ResourceCacheLimits,
    epoch: Arc<()>,
}

/// Caller-keyed LRU cache of immutable external resource bytes. Clones share
/// entries and limits. Keys must identify both the resource location and revision;
/// URI resolution, freshness detection, I/O and scheduling remain caller-owned.
pub struct ResourceCache<K = String> {
    state: Arc<Mutex<Retention<K, Arc<[u8]>>>>,
}

impl<K> Clone for ResourceCache<K> {
    fn clone(&self) -> Self {
        Self {
            state: self.state.clone(),
        }
    }
}

impl<K: Eq + Hash> Default for ResourceCache<K> {
    fn default() -> Self {
        Self::new(ResourceCacheLimits::default())
    }
}

impl<K: Eq + Hash> ResourceCache<K> {
    pub fn new(limits: ResourceCacheLimits) -> Self {
        Self {
            state: Arc::new(Mutex::new(Retention::new(limits))),
        }
    }

    pub fn limits(&self) -> ResourceCacheLimits {
        self.state.lock().limits
    }

    pub fn len(&self) -> usize {
        self.state.lock().entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Sum of payload lengths retained by entries, excluding keys and bookkeeping.
    /// Eviction releases only cache references, not bytes retained by consumers.
    pub fn cached_bytes(&self) -> usize {
        self.state.lock().bytes
    }

    /// Updates both limits and immediately evicts least-recently-used entries.
    /// Either zero limit disables retention. In-flight loads cannot repopulate
    /// the cache across a limit change.
    pub fn set_limits(&self, limits: ResourceCacheLimits) {
        self.state.lock().set_limits(limits);
    }

    /// Promotes and returns a retained payload without loading. The caller must
    /// still enforce its own request budget when using this lookup directly.
    pub fn get(&self, key: &K) -> Option<Arc<[u8]>> {
        self.state.lock().get(key)
    }

    /// Removes one key. Also prevents every currently in-flight cache load from
    /// inserting on completion, including when this key has no retained entry.
    pub fn invalidate(&self, key: &K) -> bool {
        self.state.lock().invalidate(key)
    }

    /// Releases all cache entries and prevents in-flight loads from reinserting.
    /// Returned payloads and prepared documents remain valid.
    pub fn clear(&self) {
        self.state.lock().clear();
    }

    /// Returns a hit or awaits a caller-provided loader without holding a lock.
    /// Both hits and new payloads must fit `byte_limit`. Failed, oversized-request
    /// and dropped loads do not populate the cache. Payloads exceeding cache limits
    /// are returned without retention and without evicting unrelated entries.
    ///
    /// Concurrent misses run independently. Only the first completed insertion is
    /// retained for a key; each caller receives its own loaded result. Invalidation
    /// prevents insertion but does not cancel I/O or retract that caller's result.
    pub async fn load<F, Fut>(&self, key: K, byte_limit: usize, load: F) -> Result<Arc<[u8]>>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<Vec<u8>>>,
    {
        let epoch = {
            let mut state = self.state.lock();
            if let Some(bytes) = state.peek(&key) {
                ensure!(
                    bytes.len() <= byte_limit,
                    "cached resource exceeds request byte limit {byte_limit}"
                );
                return Ok(state.get(&key).unwrap());
            }
            state.epoch.clone()
        };
        let bytes = load().await?;
        ensure!(
            bytes.len() <= byte_limit,
            "resource exceeds request byte limit {byte_limit}"
        );
        let bytes: Arc<[u8]> = bytes.into();
        self.state
            .lock()
            .insert(key, bytes.clone(), bytes.len(), &epoch);
        Ok(bytes)
    }
}

impl<K: Eq + Hash, V: Clone> Retention<K, V> {
    pub(crate) fn new(limits: ResourceCacheLimits) -> Self {
        Self {
            entries: IndexMap::new(),
            bytes: 0,
            limits,
            epoch: Arc::new(()),
        }
    }

    pub(crate) fn limits(&self) -> ResourceCacheLimits {
        self.limits
    }
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }
    pub(crate) fn bytes(&self) -> usize {
        self.bytes
    }
    pub(crate) fn epoch(&self) -> Arc<()> {
        self.epoch.clone()
    }

    pub(crate) fn peek(&self, key: &K) -> Option<&V> {
        self.entries.get(key).map(|(value, _)| value)
    }

    pub(crate) fn set_limits(&mut self, limits: ResourceCacheLimits) {
        if self.limits == limits {
            return;
        }
        self.limits = limits;
        self.epoch = Arc::new(());
        while !self.entries.is_empty()
            && (limits.bytes == 0
                || self.bytes > limits.bytes
                || self.entries.len() > limits.entries)
        {
            self.evict();
        }
    }

    pub(crate) fn invalidate(&mut self, key: &K) -> bool {
        self.epoch = Arc::new(());
        if let Some((_, bytes)) = self.entries.shift_remove(key) {
            self.bytes -= bytes;
            true
        } else {
            false
        }
    }

    pub(crate) fn clear(&mut self) {
        self.entries = IndexMap::new();
        self.bytes = 0;
        self.epoch = Arc::new(());
    }

    pub(crate) fn insert(&mut self, key: K, value: V, bytes: usize, epoch: &Arc<()>) {
        if Arc::ptr_eq(epoch, &self.epoch)
            && !self.entries.contains_key(&key)
            && self.limits.entries > 0
            && self.limits.bytes > 0
            && bytes <= self.limits.bytes
        {
            while self.entries.len() >= self.limits.entries
                || self.bytes > self.limits.bytes - bytes
            {
                self.evict();
            }
            self.bytes += bytes;
            self.entries.insert(key, (value, bytes));
        }
    }

    pub(crate) fn get(&mut self, key: &K) -> Option<V> {
        let index = self.entries.get_index_of(key)?;
        let value = self.entries.get_index(index)?.1.0.clone();
        self.entries.move_index(index, self.entries.len() - 1);
        Some(value)
    }

    fn evict(&mut self) {
        if let Some((_, (_, bytes))) = self.entries.shift_remove_index(0) {
            self.bytes -= bytes;
        }
    }
}
