# Scene load queue

`SceneLoadQueue` bounds concurrent loading pipelines and waiting work across scene
destinations. Clones share the same queue. Execution, I/O, retries and publication
remain caller-owned; the queue does not create threads or spawn tasks.

## Admission

Construct a queue with a nonzero concurrency limit and a separate waiting capacity:

```rust
use std::num::NonZeroUsize;
use gpui_3d_gltf::SceneLoadQueue;

let queue = SceneLoadQueue::new(NonZeroUsize::new(4).unwrap(), 32);
```

`run(|| pipeline)` takes a factory returning `Future<Output = anyhow::Result<T>>`.
Admission begins on first poll, not when `run` is called. The factory is invoked
only after a permit is available, on the task's polling thread. Waiters receive
permits in FIFO admission order; their executor determines when they resume.
No queue lock is held while invoking or polling a pipeline, or waking another task.

When all permits and waiting entries are occupied, `run` returns an error that
can be identified with `error.is::<SceneLoadQueueFull>()`. The factory is not
invoked. A waiting capacity of zero permits only immediate admission. Limits are
fixed for a queue and available through `concurrency()` and `capacity()`.

The concurrency limit covers each entire pipeline, including time awaiting I/O.
It does not control fan-out or detached work created inside that pipeline. Do not
await nested work on the same saturated queue while holding a permit. Capacity
limits counts, not bytes: closures may retain inputs while waiting. Decode and
resource budgets remain independent.

## Cancellation and publication

Wrap queue admission inside a [scene load request](load_slots.md) so cancellation
covers both waiting and active work:

```rust
use gpui_3d_gltf::{
    DecodedScene, ImageCache, ImageDecodeLimits, SceneDefinition, SceneLoadCompletion,
    SceneLoadQueue, SceneLoadRequest,
};

async fn load(
    queue: SceneLoadQueue,
    request: SceneLoadRequest,
    definition: SceneDefinition,
    images: ImageCache,
) -> SceneLoadCompletion<DecodedScene> {
    request.run(queue.run(move || async move {
        definition.decode_resources_cached(&images, ImageDecodeLimits::default())
    })).await
}
```

Run CPU-heavy conversion or decoding on an appropriate worker. Return the completion
to the owner thread and use `slot.accept_with(completion, |decoded| decoded.resolve())`.
Queue saturation is a loading failure; accepting it preserves the slot's previous
successful asset. Retry policy belongs to the caller.

Dropping a waiting future removes its entry without invoking the factory. Dropping
an active future, returning an error or success, or unwinding releases its permit
and wakes the next waiter. A permit handed to a task that is dropped before its
next poll is also released. Request cancellation releases queued or active work
when the request's future is polled or dropped; synchronous operations and detached
tasks are not preempted.

`stats()` returns one snapshot of `active` permits and `waiting` entries. Active
includes permits granted to tasks that have not resumed yet; it is not a count of
threads currently executing. Unpolled futures occupy neither count. The queue emits
no notifications and retains no completed results.
