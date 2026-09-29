pub use super::frame_extractor_options::*;
use crate::{
    FrameExtractionSession, FrameExtractorBackendRequest, MediaBackend, MediaError, MediaResult,
    MediaSource, SeekMode, VideoFrame,
};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

/// Extracts video frames asynchronously without changing a player's timeline.
/// Clones share a bounded queue and an independent backend session.
#[derive(Clone)]
pub struct VideoFrameExtractor {
    inner: Arc<Inner>,
}
struct Inner {
    requests: async_channel::Sender<Request>,
    latest: Arc<Mutex<Option<FrameRequest>>>,
    shutdown: Arc<AtomicBool>,
    mode: SeekMode,
}
enum Request {
    Exact(FrameRequest),
    Latest,
}
struct FrameRequest {
    position: Duration,
    mode: SeekMode,
    response: async_channel::Sender<MediaResult<Arc<VideoFrame>>>,
}
impl Drop for Inner {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Release);
        self.requests.close();
        self.latest.lock().unwrap().take();
    }
}
fn stopped() -> MediaError {
    MediaError::backend("frame extraction worker stopped")
}
fn blocking<T>() -> MediaResult<T> {
    Err(MediaError::unsupported(
        "browser frame extraction cannot block; await initial_frame() or frame_at()",
    ))
}
async fn process(session: &mut dyn FrameExtractionSession, request: FrameRequest) {
    if request.response.is_closed() {
        return;
    }
    let result = session.frame_at_async(request.position, request.mode).await;
    let _ = request.response.try_send(result);
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
        if options.timeout.is_zero() || options.request_queue_capacity == 0 {
            return Err(MediaError::invalid_input(
                "frame extraction timeout and queue capacity must be greater than zero",
            ));
        }
        let mut session = backend.open_frame_extractor(FrameExtractorBackendRequest {
            source,
            timeout: options.timeout,
            video_decoder: options.video_decoder,
        })?;
        let (requests, receiver) = async_channel::bounded(options.request_queue_capacity);
        let latest = Arc::new(Mutex::new(None));
        let shutdown = Arc::new(AtomicBool::new(false));
        let inner = Arc::new(Inner {
            requests,
            latest: latest.clone(),
            shutdown: shutdown.clone(),
            mode: options.seek_mode,
        });
        wasm_bindgen_futures::spawn_local(async move {
            while let Ok(request) = receiver.recv().await {
                if shutdown.load(Ordering::Acquire) {
                    break;
                }
                let request = match request {
                    Request::Exact(request) => Some(request),
                    Request::Latest => latest.lock().unwrap().take(),
                };
                if let Some(request) = request {
                    process(session.as_mut(), request).await;
                }
                if shutdown.load(Ordering::Acquire) {
                    break;
                }
                let request = latest.lock().unwrap().take();
                if let Some(request) = request {
                    process(session.as_mut(), request).await;
                }
            }
        });
        Ok(Self { inner })
    }
    pub async fn frame_at(&self, position: Duration) -> MediaResult<Arc<VideoFrame>> {
        self.frame_at_with_mode(position, self.inner.mode).await
    }
    pub async fn initial_frame(&self) -> MediaResult<Arc<VideoFrame>> {
        self.frame_at(Duration::ZERO).await
    }
    pub async fn frame_at_with_mode(
        &self,
        position: Duration,
        mode: SeekMode,
    ) -> MediaResult<Arc<VideoFrame>> {
        let (response, receiver) = async_channel::bounded(1);
        self.inner
            .requests
            .send(Request::Exact(FrameRequest {
                position,
                mode,
                response,
            }))
            .await
            .map_err(|_| stopped())?;
        receiver.recv().await.map_err(|_| stopped())?
    }
    pub async fn frame_at_latest(&self, position: Duration) -> MediaResult<Arc<VideoFrame>> {
        self.frame_at_latest_with_mode(position, self.inner.mode)
            .await
    }
    pub async fn frame_at_latest_with_mode(
        &self,
        position: Duration,
        mode: SeekMode,
    ) -> MediaResult<Arc<VideoFrame>> {
        let (response, receiver) = async_channel::bounded(1);
        let previous = self.inner.latest.lock().unwrap().replace(FrameRequest {
            position,
            mode,
            response,
        });
        if let Some(previous) = previous {
            let _ = previous
                .response
                .try_send(Err(FrameExtractionSuperseded.into()));
        } else if let Err(async_channel::TrySendError::Closed(_)) =
            self.inner.requests.try_send(Request::Latest)
        {
            self.inner.latest.lock().unwrap().take();
            return Err(stopped());
        }
        receiver.recv().await.map_err(|_| stopped())?
    }
    pub fn frame_at_blocking(&self, _position: Duration) -> MediaResult<Arc<VideoFrame>> {
        blocking()
    }
    pub fn initial_frame_blocking(&self) -> MediaResult<Arc<VideoFrame>> {
        blocking()
    }
    pub fn frame_at_blocking_with_mode(
        &self,
        _position: Duration,
        _mode: SeekMode,
    ) -> MediaResult<Arc<VideoFrame>> {
        blocking()
    }
}
