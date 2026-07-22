use std::{
    fmt,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
    time::Duration,
};

use anyhow::{Context as _, Result, anyhow};
use gpui::SurfaceHandle;
use gst::prelude::*;

use crate::{
    MediaSource, SeekMode, VideoFrame,
    gstreamer_backend::{
        add_required_allocation_metas, appsink_caps, clock_time, sample_to_video_frame, seek_flags,
    },
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

/// Extracts frames without changing a [`crate::VideoPlayer`] playback timeline.
///
/// A single paused GStreamer pipeline is reused by all requests. Clones share
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
    response: async_channel::Sender<Result<Arc<VideoFrame>>>,
}

impl VideoFrameExtractor {
    pub fn new(source: MediaSource) -> Result<Self> {
        Self::with_options(source, VideoFrameExtractorOptions::default())
    }

    pub fn with_options(source: MediaSource, options: VideoFrameExtractorOptions) -> Result<Self> {
        crate::init()?;
        if options.timeout.is_zero() {
            anyhow::bail!("frame extraction timeout must be greater than zero");
        }
        if options.request_queue_capacity == 0 {
            anyhow::bail!("frame extraction request queue capacity must be greater than zero");
        }

        let pipeline = ExtractorPipeline::new(&source, options.timeout)?;
        let (request_tx, request_rx) =
            async_channel::bounded::<WorkerRequest>(options.request_queue_capacity);
        let shutdown = Arc::new(AtomicBool::new(false));
        let shutdown_for_worker = shutdown.clone();
        let latest_request = Arc::new(Mutex::new(None));
        let latest_request_for_worker = latest_request.clone();
        let worker = std::thread::Builder::new()
            .name("gpui-video-frame-extractor".into())
            .spawn(move || {
                let mut pipeline = pipeline;
                while let Ok(request) = request_rx.recv_blocking() {
                    if shutdown_for_worker.load(Ordering::Acquire) {
                        break;
                    }

                    match request {
                        WorkerRequest::Exact(request) => {
                            process_frame_request(&mut pipeline, request);
                        }
                        WorkerRequest::Latest => {
                            if let Some(request) = take_latest_request(&latest_request_for_worker) {
                                process_frame_request(&mut pipeline, request);
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
                        process_frame_request(&mut pipeline, request);
                    }
                }
            })
            .context("failed to start frame extraction worker")?;

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
    pub async fn frame_at(&self, position: Duration) -> Result<Arc<VideoFrame>> {
        self.frame_at_with_mode(position, self.inner.default_seek_mode)
            .await
    }

    /// Extracts a frame asynchronously with an explicit seek mode.
    pub async fn frame_at_with_mode(
        &self,
        position: Duration,
        seek_mode: SeekMode,
    ) -> Result<Arc<VideoFrame>> {
        let (response_tx, response_rx) = async_channel::bounded(1);
        self.inner
            .requests
            .send(WorkerRequest::Exact(FrameRequest {
                position,
                seek_mode,
                response: response_tx,
            }))
            .await
            .map_err(|_| anyhow!("frame extraction worker has stopped"))?;
        response_rx
            .recv()
            .await
            .map_err(|_| anyhow!("frame extraction worker stopped before returning a frame"))?
    }

    /// Extracts the newest requested preview frame and supersedes an older
    /// latest-only request that has not started decoding yet.
    ///
    /// This is intended for interactive scrubbers and hover previews. It does
    /// not cancel the seek currently executing on the worker. Use [`Self::frame_at`]
    /// when every submitted request must complete, such as thumbnail generation.
    pub async fn frame_at_latest(&self, position: Duration) -> Result<Arc<VideoFrame>> {
        self.frame_at_latest_with_mode(position, self.inner.default_seek_mode)
            .await
    }

    /// Latest-only counterpart to [`Self::frame_at_with_mode`].
    pub async fn frame_at_latest_with_mode(
        &self,
        position: Duration,
        seek_mode: SeekMode,
    ) -> Result<Arc<VideoFrame>> {
        let (response_tx, response_rx) = async_channel::bounded(1);
        self.submit_latest(FrameRequest {
            position,
            seek_mode,
            response: response_tx,
        })?;
        response_rx
            .recv()
            .await
            .map_err(|_| anyhow!("frame extraction worker stopped before returning a frame"))?
    }

    /// Blocking counterpart to [`Self::frame_at`].
    pub fn frame_at_blocking(&self, position: Duration) -> Result<Arc<VideoFrame>> {
        self.frame_at_blocking_with_mode(position, self.inner.default_seek_mode)
    }

    /// Blocking counterpart to [`Self::frame_at_with_mode`].
    pub fn frame_at_blocking_with_mode(
        &self,
        position: Duration,
        seek_mode: SeekMode,
    ) -> Result<Arc<VideoFrame>> {
        let (response_tx, response_rx) = async_channel::bounded(1);
        self.inner
            .requests
            .send_blocking(WorkerRequest::Exact(FrameRequest {
                position,
                seek_mode,
                response: response_tx,
            }))
            .map_err(|_| anyhow!("frame extraction worker has stopped"))?;
        response_rx
            .recv_blocking()
            .map_err(|_| anyhow!("frame extraction worker stopped before returning a frame"))?
    }

    fn submit_latest(&self, request: FrameRequest) -> Result<()> {
        let should_notify = replace_latest_request(&self.inner.latest_request, request);
        if !should_notify {
            return Ok(());
        }

        match self.inner.requests.try_send(WorkerRequest::Latest) {
            Ok(()) | Err(async_channel::TrySendError::Full(_)) => Ok(()),
            Err(async_channel::TrySendError::Closed(_)) => {
                let _ = take_latest_request(&self.inner.latest_request);
                Err(anyhow!("frame extraction worker has stopped"))
            }
        }
    }
}

fn process_frame_request(pipeline: &mut ExtractorPipeline, request: FrameRequest) {
    let result = pipeline.frame_at(request.position, request.seek_mode);
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
            .try_send(Err(anyhow!(FrameExtractionSuperseded)));
    }
    should_notify
}

impl Drop for ExtractorInner {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Release);
        self.requests.close();
        if let Some(worker) = self.worker.lock().expect("extractor mutex poisoned").take() {
            // A GStreamer seek may remain blocked until the configured timeout.
            // Never make the thread dropping the extractor wait for that I/O.
            if worker.is_finished() {
                let _ = worker.join();
            }
        }
    }
}

struct ExtractorPipeline {
    playbin: gst::Element,
    appsink: gst_app::AppSink,
    surface_handle: SurfaceHandle,
    sequence: u64,
    timeout: Duration,
    prerolled: bool,
    end_guard: Duration,
}

impl ExtractorPipeline {
    fn new(source: &MediaSource, timeout: Duration) -> Result<Self> {
        let caps = appsink_caps(None)?;
        let appsink = gst_app::AppSink::builder()
            .caps(&caps)
            .max_buffers(1)
            .drop(true)
            .wait_on_eos(false)
            .sync(false)
            .build();
        appsink.set_callbacks(
            gst_app::AppSinkCallbacks::builder()
                .propose_allocation(|_, query| {
                    add_required_allocation_metas(query);
                    true
                })
                .build(),
        );

        let audio_sink = gst::ElementFactory::make("fakesink")
            .property("sync", false)
            .build()
            .context("GStreamer element 'fakesink' is not installed")?;
        let playbin = gst::ElementFactory::make("playbin3")
            .build()
            .context("GStreamer element 'playbin3' is not installed")?;
        playbin.set_property("uri", source.uri());
        playbin.set_property("video-sink", &appsink);
        playbin.set_property("audio-sink", &audio_sink);

        Ok(Self {
            playbin,
            appsink,
            surface_handle: SurfaceHandle::new(),
            sequence: 1,
            timeout,
            prerolled: false,
            end_guard: Duration::from_millis(1),
        })
    }

    fn frame_at(&mut self, requested: Duration, seek_mode: SeekMode) -> Result<Arc<VideoFrame>> {
        if !self.prerolled {
            self.playbin
                .set_state(gst::State::Paused)
                .map_err(|error| anyhow!("failed to prepare frame extraction: {error:?}"))?;
            let timeout = clock_time(self.timeout)?;
            self.playbin
                .state(timeout)
                .0
                .map_err(|error| anyhow!("frame extraction preroll failed: {error:?}"))?;
            let initial_sample = self
                .appsink
                .try_pull_preroll(timeout)
                .context("timed out waiting for the initial video frame")?;
            self.end_guard = estimated_frame_duration(&initial_sample)
                .unwrap_or(self.end_guard)
                .max(Duration::from_millis(1));
            self.prerolled = true;
        }

        let duration = self
            .appsink
            .query_duration::<gst::ClockTime>()
            .map(Duration::from);
        let position = clamp_extraction_position(requested, duration, self.end_guard);
        for candidate in extraction_candidates(position) {
            self.playbin
                .seek_simple(seek_flags(seek_mode), clock_time(candidate)?)
                .with_context(|| format!("failed to seek frame extractor to {candidate:?}"))?;

            if let Some(sample) = self.appsink.try_pull_preroll(clock_time(self.timeout)?) {
                let frame = sample_to_video_frame(
                    &sample,
                    self.surface_handle.clone(),
                    self.sequence,
                    #[cfg(target_os = "linux")]
                    None,
                )?;
                self.sequence = self.sequence.wrapping_add(1).max(1);
                return Ok(frame);
            }
        }

        Err(anyhow!(
            "failed to extract a video frame at or before {position:?}"
        ))
    }
}

impl Drop for ExtractorPipeline {
    fn drop(&mut self) {
        let _ = self.playbin.set_state(gst::State::Null);
    }
}

fn estimated_frame_duration(sample: &gst::Sample) -> Option<Duration> {
    let buffer_duration = sample
        .buffer()
        .and_then(gst::BufferRef::duration)
        .map(Duration::from);
    let nominal_duration = sample
        .caps()
        .and_then(|caps| gst_video::VideoInfo::from_caps(caps).ok())
        .and_then(|info| {
            let rate = info.fps();
            (rate.numer() > 0 && rate.denom() > 0)
                .then(|| Duration::from_secs_f64(f64::from(rate.denom()) / f64::from(rate.numer())))
        });

    match (buffer_duration, nominal_duration) {
        (Some(buffer), Some(nominal)) => Some(buffer.max(nominal)),
        (Some(duration), None) | (None, Some(duration)) => Some(duration),
        (None, None) => None,
    }
}

fn clamp_extraction_position(
    requested: Duration,
    duration: Option<Duration>,
    end_guard: Duration,
) -> Duration {
    let Some(duration) = duration else {
        return requested;
    };
    if duration.is_zero() {
        return Duration::ZERO;
    }
    requested.min(duration.saturating_sub(end_guard))
}

fn extraction_candidates(position: Duration) -> impl Iterator<Item = Duration> {
    const BACKOFFS: [Duration; 9] = [
        Duration::ZERO,
        Duration::from_millis(25),
        Duration::from_millis(50),
        Duration::from_millis(75),
        Duration::from_millis(100),
        Duration::from_millis(250),
        Duration::from_millis(500),
        Duration::from_secs(1),
        Duration::from_secs(2),
    ];

    BACKOFFS
        .into_iter()
        .map(move |backoff| position.saturating_sub(backoff))
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{
        FrameExtractionSuperseded, FrameRequest, VideoFrameExtractorOptions,
        clamp_extraction_position, extraction_candidates, replace_latest_request,
    };
    use crate::SeekMode;

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
        assert!(error.is::<FrameExtractionSuperseded>());
    }

    #[test]
    fn extraction_position_stays_before_eos() {
        assert_eq!(
            clamp_extraction_position(
                Duration::from_secs(20),
                Some(Duration::from_secs(10)),
                Duration::from_millis(40),
            ),
            Duration::from_secs(10) - Duration::from_millis(40)
        );
    }

    #[test]
    fn extraction_candidates_back_off_from_stream_tail() {
        let candidates = extraction_candidates(Duration::from_millis(60)).collect::<Vec<_>>();

        assert_eq!(candidates[0], Duration::from_millis(60));
        assert_eq!(candidates[1], Duration::from_millis(35));
        assert_eq!(candidates[3], Duration::ZERO);
        assert_eq!(candidates.last(), Some(&Duration::ZERO));
    }
}
