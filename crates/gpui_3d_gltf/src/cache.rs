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

struct State<K> {
    entries: IndexMap<K, Arc<[u8]>>,
    bytes: usize,
    limits: ResourceCacheLimits,
    epoch: Arc<()>,
}

/// Caller-keyed LRU cache of immutable external resource bytes. Clones share
/// entries and limits. Keys must identify both the resource location and revision;
/// URI resolution, freshness detection, I/O and scheduling remain caller-owned.
pub struct ResourceCache<K = String> {
    state: Arc<Mutex<State<K>>>,
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
            state: Arc::new(Mutex::new(State {
                entries: IndexMap::new(),
                bytes: 0,
                limits,
                epoch: Arc::new(()),
            })),
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
        let mut state = self.state.lock();
        if state.limits == limits {
            return;
        }
        state.limits = limits;
        state.epoch = Arc::new(());
        while !state.entries.is_empty()
            && (limits.bytes == 0
                || state.bytes > limits.bytes
                || state.entries.len() > limits.entries)
        {
            state.evict();
        }
    }

    /// Promotes and returns a retained payload without loading. The caller must
    /// still enforce its own request budget when using this lookup directly.
    pub fn get(&self, key: &K) -> Option<Arc<[u8]>> {
        self.state.lock().get(key)
    }

    /// Removes one key. Also prevents every currently in-flight cache load from
    /// inserting on completion, including when this key has no retained entry.
    pub fn invalidate(&self, key: &K) -> bool {
        let mut state = self.state.lock();
        state.epoch = Arc::new(());
        if let Some(bytes) = state.entries.shift_remove(key) {
            state.bytes -= bytes.len();
            true
        } else {
            false
        }
    }

    /// Releases all cache entries and prevents in-flight loads from reinserting.
    /// Returned payloads and prepared documents remain valid.
    pub fn clear(&self) {
        let mut state = self.state.lock();
        state.entries = IndexMap::new();
        state.bytes = 0;
        state.epoch = Arc::new(());
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
            if let Some(bytes) = state.entries.get(&key) {
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
        let mut state = self.state.lock();
        if Arc::ptr_eq(&epoch, &state.epoch)
            && !state.entries.contains_key(&key)
            && state.limits.entries > 0
            && state.limits.bytes > 0
            && bytes.len() <= state.limits.bytes
        {
            while state.entries.len() >= state.limits.entries
                || state.bytes > state.limits.bytes - bytes.len()
            {
                state.evict();
            }
            state.bytes += bytes.len();
            state.entries.insert(key, bytes.clone());
        }
        Ok(bytes)
    }
}

impl<K: Eq + Hash> State<K> {
    fn get(&mut self, key: &K) -> Option<Arc<[u8]>> {
        let index = self.entries.get_index_of(key)?;
        let bytes = self.entries.get_index(index)?.1.clone();
        self.entries.move_index(index, self.entries.len() - 1);
        Some(bytes)
    }

    fn evict(&mut self) {
        if let Some((_, bytes)) = self.entries.shift_remove_index(0) {
            self.bytes -= bytes.len();
        }
    }
}
