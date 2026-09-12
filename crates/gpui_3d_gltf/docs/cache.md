# Encoded resource cache

`ResourceCache<K>` retains immutable external resource bytes across preparations.
Clones share entries and limits, so caller-owned loaders can use the cache from
different workers. It does not parse assets, decode images, access files, resolve
URIs, or own a task queue.

## Keys and loading

Choose keys that identify the resolved resource and its revision, not just a
relative glTF URI. Suitable inputs include an asset namespace, resolved path and
content revision. The cache does not watch files, normalize aliases, compare
timestamps or expire entries automatically. Reusing a key asserts that its bytes
remain interchangeable.

`load(key, byte_limit, loader)` returns `Arc<[u8]>`. On a hit, it promotes the
entry and does not invoke `loader`. On a miss, it awaits the caller's future
returning `anyhow::Result<Vec<u8>>`. No cache lock is held during that future.
Both hits and new payloads must fit the request's `byte_limit`.

Use `Document::prepare_shared_async` to retain shared payloads without copying:

```rust
use std::future::Future;
use gpui_3d_gltf::{Document, PreparedDocument, ResourceCache, ResourceRequest};

async fn prepare<F, Fut>(
    document: &Document,
    cache: &ResourceCache<(String, String)>,
    asset_revision: &str,
    read: F,
) -> anyhow::Result<PreparedDocument>
where
    F: Fn(ResourceRequest) -> Fut + Clone,
    Fut: Future<Output = anyhow::Result<Vec<u8>>>,
{
    document.prepare_shared_async(|request| {
        let key = (asset_revision.to_owned(), request.uri.clone());
        let read = read.clone();
        cache.load(key, request.byte_limit, move || read(request))
    }).await
}
```

Here `asset_revision` must uniquely identify both the asset's namespace and its
version. The reader still enforces scheme/path policy and bounds allocations by
`request.byte_limit`. Shared cache bytes count toward each document's independent
resource budget. Identical URI references within one preparation are charged once,
as with the ordinary `prepare_async` entry point.

## Capacity and release

`ResourceCacheLimits` defaults to 64 MiB of encoded payloads and 256 entries.
Least-recently-used entries are evicted until both limits admit a new entry.
`get(&key)` and successful cache hits promote recency. Either zero limit disables
retention. A payload larger than the cache capacity, but within its request budget,
is returned without retention and without evicting unrelated entries.

`set_limits()` applies new limits immediately. `invalidate(&key)` removes an
individual entry, and `clear()` releases all entries. Previously returned `Arc`s
and prepared documents keep their data valid. `cached_bytes()` reports payload
lengths referenced by retained entries, not total live allocations; keys, metadata,
in-flight I/O, returned payloads and decoded/GPU data are outside that count.
Separate keys are charged separately even if their contents are equal.

## In-flight work and freshness

Invalidation, clearing, and a limit change prevent all already-started misses from
inserting on completion. This applies even when an invalidated key has no retained
entry yet. Other completed entries remain present after single-key invalidation.

These operations do not cancel a loader or retract its return value. An in-flight
caller may still receive its loaded bytes; use [load slots](load_slots.md) to reject
superseded asset results before publication. Requests started after invalidation
can populate the cache normally.

Concurrent misses are not coalesced: they execute independently, and only the
first completed insertion for a retained key is cached. Each caller receives its
own loader result. Applications may impose their own concurrency or shared-request
policy. Errors and request-budget failures are not cached. Dropping a pending load
drops its loader future and inserts nothing; detached loader work remains the
caller's responsibility. Successfully cached resources remain reusable if a later
stage of that asset's preparation fails or is cancelled.
