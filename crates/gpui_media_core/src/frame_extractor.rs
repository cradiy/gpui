use std::{
    fmt,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
    time::Duration,
};

use crate::{
    FrameExtractionSession, FrameExtractorBackendRequest, MediaBackend, MediaError, MediaErrorKind,
    MediaRecovery, MediaResult, MediaSource, SeekMode, VideoFrame,
};

/// Indicates that a pending latest-only preview request was replaced by a
/// newer request before decoding started.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameExtractionSuperseded;

impl fmt::Display for FrameExtractionSuperseded {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("frame extraction request was superseded by a newer preview request")
    }
}

impl std::error::Error for FrameExtractionSuperseded {}

impl From<FrameExtractionSuperseded> for MediaError {
    fn from(error: FrameExtractionSuperseded) -> Self {
        Self::new(
            MediaErrorKind::Superseded,
            error.to_string(),
            MediaRecovery::None,
        )
    }
}

/// Configuration for an independent frame extraction pipeline.
#[derive(Clone, Copy, Debug)]
pub struct VideoFrameExtractorOptions {
    pub timeout: Duration,
    pub seek_mode: SeekMode,
    /// Maximum number of extraction requests waiting behind the active seek.
    ///
    /// A bounded queue applies backpressure when a thumbnail or scrubber client
    /// submits requests faster than the decoder can satisfy them.
    pub request_queue_capacity: usize,
}

impl Default for VideoFrameExtractorOptions {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(10),
            seek_mode: SeekMode::Accurate,
            request_queue_capacity: 2,
        }
    }
}

/// Extracts frames without changing an active playback session's timeline.
///
/// A single backend extraction session is reused by all requests. Clones share
/// the same worker and serialize frame extraction requests.
#[derive(Clone)]
pub struct VideoFrameExtractor {
    inner: Arc<ExtractorInner>,
}

struct ExtractorInner {
    requests: async_channel::Sender<WorkerRequest>,
    latest_request: Arc<Mutex<Option<FrameRequest>>>,
    worker: Mutex<Option<JoinHandle<()>>>,
    shutdown: Arc<AtomicBool>,
    default_seek_mode: SeekMode,
}

enum WorkerRequest {
    Exact(FrameRequest),
    Latest,
}

struct FrameRequest {
    position: Duration,
    seek_mode: SeekMode,
    response: async_channel::Sender<MediaResult<Arc<VideoFrame>>>,
}

impl VideoFrameExtractor {
    pub fn new(source: MediaSource, backend: Arc<dyn MediaBackend>) -> MediaResult<Self> {
        Self::with_options(source, VideoFrameExtractorOptions::default(), backend)
    }

    pub fn with_options(
        source: MediaSource,
        options: VideoFrameExtractorOptions,
        backend: Arc<dyn MediaBackend>,
    ) -> MediaResult<Self> {
        if options.timeout.is_zero() {
            return Err(MediaError::invalid_input(
                "frame extraction timeout must be greater than zero",
            ));
        }
        if options.request_queue_capacity == 0 {
            return Err(MediaError::invalid_input(
                "frame extraction request queue capacity must be greater than zero",
            ));
        }

        let session = backend.open_frame_extractor(FrameExtractorBackendRequest {
            source,
            timeout: options.timeout,
        })?;
        let (request_tx, request_rx) =
            async_channel::bounded::<WorkerRequest>(options.request_queue_capacity);
        let shutdown = Arc::new(AtomicBool::new(false));
        let shutdown_for_worker = shutdown.clone();
        let latest_request = Arc::new(Mutex::new(None));
        let latest_request_for_worker = latest_request.clone();
        let worker = std::thread::Builder::new()
            .name("gpui-video-frame-extractor".into())
            .spawn(move || {
                let mut session = session;
                while let Ok(request) = request_rx.recv_blocking() {
                    if shutdown_for_worker.load(Ordering::Acquire) {
                        break;
                    }

                    match request {
                        WorkerRequest::Exact(request) => {
                            process_frame_request(session.as_mut(), request);
                        }
                        WorkerRequest::Latest => {
                            if let Some(request) = take_latest_request(&latest_request_for_worker) {
                                process_frame_request(session.as_mut(), request);
                            }
                        }
                    }

                    // If a latest-only notification could not enter a full
                    // exact-request queue, service it immediately after the
                    // current request instead.
                    if shutdown_for_worker.load(Ordering::Acquire) {
                        break;
                    }
                    if let Some(request) = take_latest_request(&latest_request_for_worker) {
                        process_frame_request(session.as_mut(), request);
                    }
                }
            })
            .map_err(|error| MediaError::io("failed to start frame extraction worker", error))?;

        Ok(Self {
            inner: Arc::new(ExtractorInner {
                requests: request_tx,
                latest_request,
                worker: Mutex::new(Some(worker)),
                shutdown,
                default_seek_mode: options.seek_mode,
            }),
        })
    }

    /// Extracts a frame asynchronously with the default seek mode.
    pub async fn frame_at(&self, position: Duration) -> MediaResult<Arc<VideoFrame>> {
        self.frame_at_with_mode(position, self.inner.default_seek_mode)
            .await
    }

    /// Returns the first decoded frame without issuing a seek.
    ///
    /// Hosts can call this before opening a window to determine the video's
    /// display size without briefly showing a provisional window. Unlike an
    /// arbitrary timestamp request, this also works for sequential HTTP
    /// responses that do not support byte ranges.
    pub async fn initial_frame(&self) -> MediaResult<Arc<VideoFrame>> {
        self.frame_at(Duration::ZERO).await
    }

    /// Extracts a frame asynchronously with an explicit seek mode.
    pub async fn frame_at_with_mode(
        &self,
        position: Duration,
        seek_mode: SeekMode,
    ) -> MediaResult<Arc<VideoFrame>> {
        let (response_tx, response_rx) = async_channel::bounded(1);
        self.inner
            .requests
            .send(WorkerRequest::Exact(FrameRequest {
                position,
                seek_mode,
                response: response_tx,
            }))
            .await
            .map_err(|_| frame_extraction_worker_stopped())?;
        response_rx
            .recv()
            .await
            .map_err(|_| frame_extraction_worker_stopped())?
    }

    /// Extracts the newest requested preview frame and supersedes an older
    /// latest-only request that has not started decoding yet.
    ///
    /// This is intended for interactive scrubbers and hover previews. It does
    /// not cancel the seek currently executing on the worker. Use [`Self::frame_at`]
    /// when every submitted request must complete, such as thumbnail generation.
    pub async fn frame_at_latest(&self, position: Duration) -> MediaResult<Arc<VideoFrame>> {
        self.frame_at_latest_with_mode(position, self.inner.default_seek_mode)
            .await
    }

    /// Latest-only counterpart to [`Self::frame_at_with_mode`].
    pub async fn frame_at_latest_with_mode(
        &self,
        position: Duration,
        seek_mode: SeekMode,
    ) -> MediaResult<Arc<VideoFrame>> {
        let (response_tx, response_rx) = async_channel::bounded(1);
        self.submit_latest(FrameRequest {
            position,
            seek_mode,
            response: response_tx,
        })?;
        response_rx
            .recv()
            .await
            .map_err(|_| frame_extraction_worker_stopped())?
    }

    /// Blocking counterpart to [`Self::frame_at`].
    pub fn frame_at_blocking(&self, position: Duration) -> MediaResult<Arc<VideoFrame>> {
        self.frame_at_blocking_with_mode(position, self.inner.default_seek_mode)
    }

    /// Blocking counterpart to [`Self::initial_frame`].
    pub fn initial_frame_blocking(&self) -> MediaResult<Arc<VideoFrame>> {
        self.frame_at_blocking(Duration::ZERO)
    }

    /// Blocking counterpart to [`Self::frame_at_with_mode`].
    pub fn frame_at_blocking_with_mode(
        &self,
        position: Duration,
        seek_mode: SeekMode,
    ) -> MediaResult<Arc<VideoFrame>> {
        let (response_tx, response_rx) = async_channel::bounded(1);
        self.inner
            .requests
            .send_blocking(WorkerRequest::Exact(FrameRequest {
                position,
                seek_mode,
                response: response_tx,
            }))
            .map_err(|_| frame_extraction_worker_stopped())?;
        response_rx
            .recv_blocking()
            .map_err(|_| frame_extraction_worker_stopped())?
    }

    fn submit_latest(&self, request: FrameRequest) -> MediaResult<()> {
        let should_notify = replace_latest_request(&self.inner.latest_request, request);
        if !should_notify {
            return Ok(());
        }

        match self.inner.requests.try_send(WorkerRequest::Latest) {
            Ok(()) | Err(async_channel::TrySendError::Full(_)) => Ok(()),
            Err(async_channel::TrySendError::Closed(_)) => {
                let _ = take_latest_request(&self.inner.latest_request);
                Err(frame_extraction_worker_stopped())
            }
        }
    }
}

fn process_frame_request(session: &mut dyn FrameExtractionSession, request: FrameRequest) {
    let result = if request.position.is_zero() {
        session.initial_frame()
    } else {
        session.frame_at(request.position, request.seek_mode)
    };
    let _ = request.response.send_blocking(result);
}

fn take_latest_request(latest_request: &Mutex<Option<FrameRequest>>) -> Option<FrameRequest> {
    latest_request
        .lock()
        .expect("latest frame request mutex poisoned")
        .take()
}

fn replace_latest_request(
    latest_request: &Mutex<Option<FrameRequest>>,
    request: FrameRequest,
) -> bool {
    let previous = latest_request
        .lock()
        .expect("latest frame request mutex poisoned")
        .replace(request);
    let should_notify = previous.is_none();
    if let Some(previous) = previous {
        let _ = previous
            .response
            .try_send(Err(FrameExtractionSuperseded.into()));
    }
    should_notify
}

fn frame_extraction_worker_stopped() -> MediaError {
    MediaError::backend_retryable("frame extraction worker has stopped")
}

impl Drop for ExtractorInner {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Release);
        self.requests.close();
        if let Some(worker) = self.worker.lock().expect("extractor mutex poisoned").take() {
            // A backend seek may remain blocked until its configured timeout.
            // Never make the thread dropping the extractor wait for that I/O.
            if worker.is_finished() {
                let _ = worker.join();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{FrameRequest, VideoFrameExtractorOptions, replace_latest_request};
    use crate::{MediaErrorKind, SeekMode};

    #[test]
    fn extraction_requests_are_bounded_by_default() {
        assert_eq!(
            VideoFrameExtractorOptions::default().request_queue_capacity,
            2
        );
    }

    #[test]
    fn latest_extraction_request_supersedes_pending_preview() {
        let latest = std::sync::Mutex::new(None);
        let (first_response, first_result) = async_channel::bounded(1);
        let (second_response, _) = async_channel::bounded(1);

        assert!(replace_latest_request(
            &latest,
            FrameRequest {
                position: Duration::from_secs(1),
                seek_mode: SeekMode::KeyFrame,
                response: first_response,
            },
        ));
        assert!(!replace_latest_request(
            &latest,
            FrameRequest {
                position: Duration::from_secs(2),
                seek_mode: SeekMode::KeyFrame,
                response: second_response,
            },
        ));

        let error = first_result.try_recv().unwrap().unwrap_err();
        assert_eq!(error.kind, MediaErrorKind::Superseded);
    }
}
