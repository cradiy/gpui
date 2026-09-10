use std::{future::Future, sync::Arc};

use futures::future::{AbortHandle, AbortRegistration, Abortable};

use crate::SceneAsset;

/// State of the latest scene load, independent of any retained successful asset.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SceneLoadStatus {
    #[default]
    Idle,
    Loading,
    Ready,
    Failed,
    Cancelled,
}

struct Pending {
    identity: Arc<()>,
    abort: AbortHandle,
}

/// One asset destination with latest-request-only publication. This value owns
/// no executor or I/O policy. Failed, cancelled and superseded loads preserve the
/// last successful asset until it is replaced or explicitly cleared.
#[derive(Default)]
pub struct SceneLoadSlot {
    asset: Option<SceneAsset>,
    error: Option<anyhow::Error>,
    status: SceneLoadStatus,
    pending: Option<Pending>,
}

impl SceneLoadSlot {
    pub fn new() -> Self {
        Self::default()
    }

    /// Starts a request and signals cancellation of any previous pending request.
    /// The caller runs the returned request on its chosen executor.
    pub fn begin(&mut self) -> SceneLoadRequest {
        self.abort_pending();
        let identity = Arc::new(());
        let (abort, registration) = AbortHandle::new_pair();
        self.pending = Some(Pending {
            identity: identity.clone(),
            abort: abort.clone(),
        });
        self.error = None;
        self.status = SceneLoadStatus::Loading;
        SceneLoadRequest {
            identity,
            abort,
            registration: Some(registration),
            finished: false,
        }
    }

    pub fn asset(&self) -> Option<&SceneAsset> {
        self.asset.as_ref()
    }

    /// Error from the latest accepted failed load, cleared by a new request.
    pub fn error(&self) -> Option<&anyhow::Error> {
        self.error.as_ref()
    }

    /// Dropping a request, its unfinished future, or an unaccepted completion is
    /// observed as `Cancelled`.
    /// Successful worker completion remains `Loading` until `accept` is called.
    pub fn status(&self) -> SceneLoadStatus {
        if self.pending.as_ref().is_some_and(|p| p.abort.is_aborted()) {
            SceneLoadStatus::Cancelled
        } else {
            self.status
        }
    }

    /// Consumes a completion. Returns false for superseded, cancelled, or foreign
    /// requests without changing the asset, error or current request. Rejected
    /// results are dropped. Accepted failures retain the previous asset.
    pub fn accept(&mut self, completion: SceneLoadCompletion) -> bool {
        self.accept_with(completion, Ok)
    }

    /// Accepts a worker payload and resolves it into an asset on this thread.
    /// Stale or foreign completions never invoke `resolve`. Resolution errors
    /// become the current failure while preserving the previously accepted asset.
    pub fn accept_with<T>(
        &mut self,
        mut completion: SceneLoadCompletion<T>,
        resolve: impl FnOnce(T) -> anyhow::Result<SceneAsset>,
    ) -> bool {
        let Some(pending) = &self.pending else {
            return false;
        };
        if !Arc::ptr_eq(&pending.identity, &completion.identity) || pending.abort.is_aborted() {
            return false;
        }
        let Some(result) = completion.result.take() else {
            return false;
        };
        let result = result.and_then(resolve);
        self.pending = None;
        match result {
            Ok(asset) => {
                self.asset = Some(asset);
                self.error = None;
                self.status = SceneLoadStatus::Ready;
            }
            Err(error) => {
                self.error = Some(error);
                self.status = SceneLoadStatus::Failed;
            }
        }
        true
    }

    /// Invalidates a pending request and signals cancellation, retaining the asset.
    /// Returns whether a request was invalidated; a settled slot is unchanged.
    pub fn cancel(&mut self) -> bool {
        if self.pending.is_none() {
            return false;
        }
        self.abort_pending();
        self.status = SceneLoadStatus::Cancelled;
        true
    }

    /// Cancels pending work and releases this slot's asset and error references.
    pub fn clear(&mut self) {
        self.abort_pending();
        self.asset = None;
        self.error = None;
        self.status = SceneLoadStatus::Idle;
    }

    fn abort_pending(&mut self) {
        if let Some(pending) = self.pending.take() {
            pending.abort.abort();
        }
    }
}

impl Drop for SceneLoadSlot {
    fn drop(&mut self) {
        self.abort_pending();
    }
}

/// Single-use request identity and cancellation registration. Dropping an unrun
/// request or its unfinished `run` future marks the request cancelled.
#[must_use = "run the request on the caller's executor"]
pub struct SceneLoadRequest {
    identity: Arc<()>,
    abort: AbortHandle,
    registration: Option<AbortRegistration>,
    finished: bool,
}

impl SceneLoadRequest {
    /// Wraps a caller-provided loading pipeline. No task is spawned. Cancellation
    /// wakes a pending task; polling or dropping it releases its loading future.
    /// Synchronous work and detached tasks are not preempted. Return the completion
    /// to the originating slot with `accept` or `accept_with`, even when completion
    /// precedes cancellation. Transferable payloads can be resolved on the owner thread.
    pub async fn run<T>(
        mut self,
        load: impl Future<Output = anyhow::Result<T>>,
    ) -> SceneLoadCompletion<T> {
        let result = Abortable::new(load, self.registration.take().unwrap())
            .await
            .ok();
        self.finished = true;
        SceneLoadCompletion {
            identity: self.identity.clone(),
            abort: self.abort.clone(),
            result,
        }
    }
}

impl Drop for SceneLoadRequest {
    fn drop(&mut self) {
        if !self.finished {
            self.abort.abort();
        }
    }
}

/// Worker result bound to one request in one slot. Consuming it through `accept`
/// validates publication ownership, including results completed before cancellation.
/// Dropping an unaccepted completion marks its originating request cancelled.
#[must_use = "return the completion to its scene load slot"]
pub struct SceneLoadCompletion<T = SceneAsset> {
    identity: Arc<()>,
    abort: AbortHandle,
    result: Option<anyhow::Result<T>>,
}

impl<T> Drop for SceneLoadCompletion<T> {
    fn drop(&mut self) {
        self.abort.abort();
    }
}
