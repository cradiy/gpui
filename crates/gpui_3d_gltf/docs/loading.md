# Asynchronous resource loading

`Document::prepare_async(load_uri)` prepares encoded resources using asynchronous
I/O supplied by the caller. It returns the same `PreparedDocument` as `prepare`,
with identical URI deduplication, buffer/image ownership, sparse validation and
aggregate admission rules. It does not decode images or instantiate scenes.

The loader accepts an owned `ResourceRequest` and returns a future yielding
`anyhow::Result<Vec<u8>>`:

```rust
use std::future::Future;
use gpui_3d_gltf::{Document, PreparedDocument, ResourceRequest};

async fn prepare<F, Fut>(document: &Document, load: F) -> anyhow::Result<PreparedDocument>
where
    F: FnMut(ResourceRequest) -> Fut,
    Fut: Future<Output = anyhow::Result<Vec<u8>>>,
{
    document.prepare_async(load).await
}
```

`request.uri` preserves the original spelling, including percent encoding and
schemes. Resolve it relative to the asset's location under the application's URI
policy. The crate performs no filesystem or network access and does not start an
executor or worker thread. Base64 data URIs and GLB binary chunks are handled
internally without invoking the loader.

## Scheduling and admission

External requests are sequential: buffers in document order, followed by URI
images. Identical URI strings are loaded once across both groups. The next
request starts only after the current payload has completed and passed admission.

`request.byte_limit` is the remaining aggregate encoded-byte allowance after
previous unique payloads and referenced GLB bytes. Bound streaming reads and
allocations by this limit. It may be zero; only an empty returned payload then
fits. Returned payload sizes are checked even if the loader ignores the limit.
Buffer length and sparse-data validation still apply. No partial
`PreparedDocument` is returned on failure.

The future is lazy. Callbacks and validation execute on the thread polling it;
base64 decoding and validation are synchronous within a poll. Use caller-owned
background execution when this work must not occupy the UI thread. The future
can be sent between threads when its loader and loader future are `Send`.
Local-only loaders are also supported; no `Send` or `'static` bound is imposed.

## Cancellation and retry

Dropping a pending preparation future drops its pending loader future and releases
the preparation's partial inputs. It does not modify the `Document`, publish an
asset, or start later URI requests. A new call retries with a fresh per-call URI
cache and budget. Already completed I/O and callback side effects are not undone.
Detached tasks, blocking reads or external requests that outlive their own future
must be cancelled by the loader's implementation.

A caller can use `futures::future::Abortable` to cancel a pending load from another
task without fixing the library to an executor:

```rust
use futures::future::{AbortHandle, Abortable};
# async fn example(document: &gpui_3d_gltf::Document) -> anyhow::Result<()> {
# let load_uri = |_request: gpui_3d_gltf::ResourceRequest| async { Ok(Vec::new()) };
let (cancel, registration) = AbortHandle::new_pair();
let loading = Abortable::new(document.prepare_async(load_uri), registration);
// Retain `cancel` with the caller's request state; cancellation calls `cancel.abort()`.
match loading.await {
    Ok(result) => {
        let prepared = result?;
        // Convert the selected scene and decode its images on the chosen worker.
        let _ = prepared;
    }
    Err(_) => {}
}
# let _ = cancel;
# Ok(())
# }
```

Cancellation is not preemption of synchronous work and cannot retract a result
that has already completed. Before publishing into a view or asset slot, compare
the request's identity with the caller's current request. Discard superseded
results even if they completed successfully. Loading queues, cross-request caches,
retry timing, decoded-image budgets and GPU uploads remain caller-owned policies.
