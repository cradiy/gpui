use std::{collections::VecDeque, future::Future, num::NonZeroUsize, sync::Arc};

use futures::channel::oneshot;
use parking_lot::Mutex;

/// A load could not start and the waiting capacity was exhausted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SceneLoadQueueFull;

impl std::fmt::Display for SceneLoadQueueFull {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("scene load queue is full")
    }
}

impl std::error::Error for SceneLoadQueueFull {}

/// One snapshot of occupied permits and queued loads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SceneLoadQueueStats {
    /// Includes permits granted to tasks that have not been polled again yet.
    pub active: usize,
    pub waiting: usize,
}

struct Waiter {
    identity: Arc<()>,
    ready: oneshot::Sender<Permit>,
}

#[derive(Default)]
struct State {
    active: usize,
    waiting: VecDeque<Waiter>,
}

/// Shared, bounded FIFO admission for caller-polled loading pipelines. The queue
/// owns no executor or threads. Clones share capacity, ordering and active permits.
#[derive(Clone)]
pub struct SceneLoadQueue {
    concurrency: NonZeroUsize,
    capacity: usize,
    state: Arc<Mutex<State>>,
}

impl SceneLoadQueue {
    /// `capacity` limits waiting loads, independently of active concurrency.
    /// Zero capacity allows immediate admission only.
    pub fn new(concurrency: NonZeroUsize, capacity: usize) -> Self {
        Self {
            concurrency,
            capacity,
            state: Arc::new(Mutex::new(State::default())),
        }
    }

    pub fn concurrency(&self) -> NonZeroUsize {
        self.concurrency
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    pub fn stats(&self) -> SceneLoadQueueStats {
        let state = self.state.lock();
        SceneLoadQueueStats {
            active: state.active,
            waiting: state.waiting.len(),
        }
    }

    /// Acquires a permit before invoking `load`. Admission starts on first poll;
    /// a full waiting queue returns `SceneLoadQueueFull` without calling `load`.
    /// Completion, failure, unwinding or dropping the future releases its permit.
    /// Dropping a waiting future removes it without starting the pipeline.
    pub async fn run<T, F, Fut>(&self, load: F) -> anyhow::Result<T>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = anyhow::Result<T>>,
    {
        let _permit = self.acquire().await?;
        load().await
    }

    async fn acquire(&self) -> Result<Permit, SceneLoadQueueFull> {
        let (receiver, waiting) = {
            let mut state = self.state.lock();
            if state.active < self.concurrency.get() {
                state.active += 1;
                return Ok(Permit(Some(self.state.clone())));
            }
            if state.waiting.len() >= self.capacity {
                return Err(SceneLoadQueueFull);
            }
            let (ready, receiver) = oneshot::channel();
            let identity = Arc::new(());
            state.waiting.push_back(Waiter {
                identity: identity.clone(),
                ready,
            });
            (
                receiver,
                Waiting {
                    state: self.state.clone(),
                    identity,
                },
            )
        };
        let permit = receiver.await.expect("queued permit sender remains owned");
        drop(waiting);
        Ok(permit)
    }
}

struct Waiting {
    state: Arc<Mutex<State>>,
    identity: Arc<()>,
}

impl Drop for Waiting {
    fn drop(&mut self) {
        let removed = {
            let mut state = self.state.lock();
            state
                .waiting
                .iter()
                .position(|waiter| Arc::ptr_eq(&waiter.identity, &self.identity))
                .and_then(|index| state.waiting.remove(index))
        };
        drop(removed);
    }
}

struct Permit(Option<Arc<Mutex<State>>>);

impl Drop for Permit {
    fn drop(&mut self) {
        let Some(shared) = self.0.take() else {
            return;
        };
        loop {
            let next = {
                let mut state = shared.lock();
                match state.waiting.pop_front() {
                    Some(waiter) => waiter,
                    None => {
                        state.active -= 1;
                        return;
                    }
                }
            };
            match next.ready.send(Permit(Some(shared.clone()))) {
                Ok(()) => return,
                Err(mut permit) => {
                    // Transfer the same permit past receivers cancelled during handoff.
                    permit.0.take();
                }
            }
        }
    }
}
