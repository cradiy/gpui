# Scene load slots

`SceneLoadSlot` manages one destination for an asynchronously loaded `SceneAsset`.
It retains the last successful asset while a replacement loads. The slot owns
request identity, cancellation and publication state; callers provide I/O,
execution, retries and UI notification.

Use a shared [scene load queue](load_queue.md) to bound concurrent pipelines and
waiting work across slots.

## Request lifecycle

1. `slot.begin()` creates a `SceneLoadRequest`, clears the previous error and
   signals cancellation of any older pending request.
2. `request.run(pipeline)` wraps a future returning `anyhow::Result<T>`.
   Run that future on the caller's chosen executor. `T` can be a transferable
   `DecodedScene` or an owner-thread `SceneAsset`.
3. Return its `SceneLoadCompletion<T>` to `slot.accept_with(completion, resolve)`,
   or use `slot.accept(completion)` for an existing `SceneAsset`. Only the current
   request from that slot can publish. Acceptance returns `true` for both success
   and failure; rejected results are consumed and dropped.

The request and completion carry their identity internally. A completion from
another slot, an older request, a cancelled request or a cleared slot cannot
replace the asset or current error. This also applies when the old work completed
successfully before cancellation but its result was delivered afterward.

A loading pipeline can combine asynchronous resource preparation with scene
conversion and image decoding:

```rust
use std::future::Future;
use gpui_3d_gltf::{
    DecodedScene, Document, ImageDecodeLimits, ResourceRequest, SceneOptions,
};

async fn load_scene<F, Fut>(document: Document, load_uri: F) -> anyhow::Result<DecodedScene>
where
    F: FnMut(ResourceRequest) -> Fut,
    Fut: Future<Output = anyhow::Result<Vec<u8>>>,
{
    let prepared = document.prepare_async(load_uri).await?;
    let definition = prepared.scene(None, SceneOptions::default())?;
    definition.decode_resources(ImageDecodeLimits::default())
}
```

Create the request before handing the work to an executor:

```rust
let request = slot.begin();
let work = request.run(load_scene(document, load_uri));
```

The future owns the request, not a borrow of the slot. The slot can start another
request or cancel while work is pending. Return the completion through the
application's event/update path:

```rust
let replacement = if slot.accept_with(completion, |decoded| decoded.resolve())
    && slot.status() == gpui_3d_gltf::SceneLoadStatus::Ready
{
    Some(slot.asset().unwrap().instantiate(&mut graph, None)?)
} else {
    None
};
```

Instantiation and removal of an earlier graph instance remain explicit. Accepting
an asset does not mutate a scene graph or upload GPU resources. Run CPU-heavy
conversion and decoding on an appropriate worker; the wrapper does not move work
off the polling thread or impose `Send`/`'static` bounds.

`DecodedScene` is `Send + Sync`; it holds the converted definition and shared
decoded pixels. `SceneAsset` contains GPUI material values and is not transferable
between threads. `accept_with` checks request identity before invoking its resolver
on the calling thread. Stale payloads are dropped without constructing core
materials or subtrees. Resolver failure sets `Failed` and retains the previous
asset. Final resolution includes authored initial deformation and does not decode
images again. See [image decoding](images.md) for budgets and resource ownership.

## State and retention

| Status | Meaning | Retained asset |
| --- | --- | --- |
| `Idle` | No request has started, or the slot was cleared. | None. |
| `Loading` | Latest request is running or its completion awaits acceptance. | Previous success, if any. |
| `Ready` | Latest success was accepted. | Accepted asset. |
| `Failed` | Latest error was accepted; available through `error()`. | Previous success, if any. |
| `Cancelled` | Latest request was invalidated or abandoned. | Previous success, if any. |

`cancel()` invalidates a pending request and signals cancellation. It returns
whether a pending request existed; settled slots remain unchanged.
`clear()` cancels pending work, releases the slot's asset and error references,
and returns to `Idle`. Other asset clones, graph instances and retained evaluated
scenes are unaffected.

Dropping a request before running it, dropping its unfinished future, or discarding
an unaccepted completion marks the originating request cancelled. `status()`
observes this state without a separate completion event. Dropping a slot signals
cancellation of pending work. No UI notifications are emitted by these operations.

Cancellation wakes a waiting task. Its pipeline future is released when the task
is polled or dropped; synchronous work and detached tasks are not preempted.
The publication check remains authoritative even if that work finishes despite
cancellation. See [asynchronous resource loading](loading.md) for resolver
allocation, cancellation and retry contracts.
