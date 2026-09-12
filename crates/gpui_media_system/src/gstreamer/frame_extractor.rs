//! GStreamer-backed still-frame extraction for `SystemBackend`.

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use gpui_media_core::FrameHandle;
use gst::prelude::*;

use gpui_media_core::{
    FrameExtractionSession, MediaError, MediaErrorKind, MediaRecovery, MediaResult, MediaSource,
    SeekMode, VideoFrame,
};

use super::{
    add_required_allocation_metas, appsink_caps, clock_time, gst_backend_error, gst_decode_error,
    network::configure_playbin_network, sample_to_video_frame, seek_flags,
};

pub(super) struct GstreamerFrameExtractionSession {
    playbin: gst::Element,
    appsink: gst_app::AppSink,
    surface_handle: FrameHandle,
    sequence: u64,
    timeout: Duration,
    initial_frame: Option<Arc<VideoFrame>>,
    end_guard: Duration,
}

impl GstreamerFrameExtractionSession {
    pub(super) fn new(source: &MediaSource, timeout: Duration) -> MediaResult<Self> {
        super::initialize()?;
        let caps = appsink_caps(None)?;
        let appsink = gst_app::AppSink::builder()
            .caps(&caps)
            .max_buffers(1)
            .wait_on_eos(false)
            .sync(false)
            .build();
        #[cfg(feature = "v1_28")]
        appsink.set_leaky_type(gst_app::AppLeakyType::Downstream);
        #[cfg(not(feature = "v1_28"))]
        appsink.set_drop(true);
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
            .map_err(|error| {
                gst_backend_error("GStreamer element 'fakesink' is not installed", error)
            })?;
        let playbin = gst::ElementFactory::make("playbin3")
            .build()
            .map_err(|error| {
                gst_backend_error("GStreamer element 'playbin3' is not installed", error)
            })?;
        configure_playbin_network(&playbin, source.network_options());
        playbin.set_property("uri", source.uri());
        playbin.set_property("video-sink", &appsink);
        playbin.set_property("audio-sink", &audio_sink);

        Ok(Self {
            playbin,
            appsink,
            surface_handle: FrameHandle::new(),
            sequence: 1,
            timeout,
            initial_frame: None,
            end_guard: Duration::from_millis(1),
        })
    }

    fn extract_frame(
        &mut self,
        requested: Duration,
        seek_mode: SeekMode,
    ) -> MediaResult<Arc<VideoFrame>> {
        let deadline = self.request_deadline()?;
        let initial_frame = self.preroll_initial_frame(deadline)?;
        if requested.is_zero() {
            return Ok(initial_frame);
        }

        let duration = self
            .appsink
            .query_duration::<gst::ClockTime>()
            .map(Duration::from);
        let position = clamp_extraction_position(requested, duration, self.end_guard);
        for candidate in extraction_candidates(position) {
            remaining_timeout(deadline)?;
            self.playbin
                .seek_simple(seek_flags(seek_mode), clock_time(candidate)?)
                .map_err(|error| {
                    gst_decode_error(
                        format!("failed to seek frame extractor to {candidate:?}"),
                        error,
                    )
                })?;

            if let Some(sample) = self.appsink.try_pull_preroll(remaining_timeout(deadline)?) {
                let frame = sample_to_video_frame(
                    &sample,
                    self.surface_handle,
                    self.sequence,
                    #[cfg(target_os = "linux")]
                    None,
                )?;
                self.sequence = self.sequence.wrapping_add(1).max(1);
                return Ok(frame);
            }
        }

        remaining_timeout(deadline)?;
        Err(MediaError::new(
            MediaErrorKind::Decode,
            format!("failed to extract a video frame at or before {position:?}"),
            MediaRecovery::None,
        ))
    }

    fn request_deadline(&self) -> MediaResult<Instant> {
        clock_time(self.timeout)?;
        Instant::now().checked_add(self.timeout).ok_or_else(|| {
            MediaError::invalid_input("frame extraction timeout exceeds the clock range")
        })
    }

    fn preroll_initial_frame(&mut self, deadline: Instant) -> MediaResult<Arc<VideoFrame>> {
        if let Some(frame) = &self.initial_frame {
            return Ok(frame.clone());
        }

        remaining_timeout(deadline)?;
        self.playbin
            .set_state(gst::State::Paused)
            .map_err(|error| gst_backend_error("failed to prepare frame extraction", error))?;
        self.playbin
            .state(remaining_timeout(deadline)?)
            .0
            .map_err(|error| gst_decode_error("frame extraction preroll failed", error))?;
        let sample = self
            .appsink
            .try_pull_preroll(remaining_timeout(deadline)?)
            .ok_or_else(|| MediaError::timeout("timed out waiting for the initial video frame"))?;
        self.end_guard = estimated_frame_duration(&sample)
            .unwrap_or(self.end_guard)
            .max(Duration::from_millis(1));
        let frame = sample_to_video_frame(
            &sample,
            self.surface_handle,
            self.sequence,
            #[cfg(target_os = "linux")]
            None,
        )?;
        self.sequence = self.sequence.wrapping_add(1).max(1);
        self.initial_frame = Some(frame.clone());
        Ok(frame)
    }
}

impl FrameExtractionSession for GstreamerFrameExtractionSession {
    fn initial_frame(&mut self) -> MediaResult<Arc<VideoFrame>> {
        self.preroll_initial_frame(self.request_deadline()?)
    }

    fn frame_at(
        &mut self,
        position: Duration,
        seek_mode: SeekMode,
    ) -> MediaResult<Arc<VideoFrame>> {
        self.extract_frame(position, seek_mode)
    }
}

impl Drop for GstreamerFrameExtractionSession {
    fn drop(&mut self) {
        let _ = self.playbin.set_state(gst::State::Null);
    }
}

fn remaining_timeout(deadline: Instant) -> MediaResult<gst::ClockTime> {
    let remaining = deadline
        .checked_duration_since(Instant::now())
        .filter(|remaining| !remaining.is_zero())
        .ok_or_else(|| MediaError::timeout("frame extraction deadline exceeded"))?;
    clock_time(remaining)
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
    use std::{
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
        time::Duration,
    };

    use gpui_media_core::{FrameBuffer, FrameHandle, FrameSize};
    use gpui_media_core::{MediaErrorKind, SeekMode, VideoFrame};
    use gst::prelude::*;

    use super::{
        GstreamerFrameExtractionSession, clamp_extraction_position, extraction_candidates,
    };

    #[test]
    fn stalled_seek_exhausts_request_budget_without_retrying() {
        gst::init().unwrap();
        let pipeline = gst::Pipeline::new();
        let caps = gst::Caps::builder("video/x-raw")
            .field("format", "RGBA")
            .field("width", 1_i32)
            .field("height", 1_i32)
            .field("framerate", gst::Fraction::new(1, 1))
            .build();
        let source = gst_app::AppSrc::builder()
            .caps(&caps)
            .format(gst::Format::Time)
            .stream_type(gst_app::AppStreamType::Seekable)
            .build();
        source.set_callbacks(
            gst_app::AppSrcCallbacks::builder()
                .seek_data(|_, _| true)
                .build(),
        );
        let appsink = gst_app::AppSink::builder().sync(false).build();
        pipeline
            .add_many([source.upcast_ref::<gst::Element>(), appsink.upcast_ref()])
            .unwrap();
        source.link(&appsink).unwrap();

        let seeks = Arc::new(AtomicUsize::new(0));
        let seeks_for_probe = seeks.clone();
        appsink.static_pad("sink").unwrap().add_probe(
            gst::PadProbeType::EVENT_UPSTREAM,
            move |_, info| {
                if info
                    .event()
                    .is_some_and(|event| event.type_() == gst::EventType::Seek)
                {
                    seeks_for_probe.fetch_add(1, Ordering::Relaxed);
                }
                gst::PadProbeReturn::Ok
            },
        );
        pipeline.set_state(gst::State::Paused).unwrap();
        let surface_handle = FrameHandle::new();
        let surface = FrameBuffer::rgba(
            surface_handle,
            1,
            FrameSize::new(1, 1),
            vec![0, 0, 0, 255],
            4,
        )
        .unwrap();
        let mut session = GstreamerFrameExtractionSession {
            playbin: pipeline.upcast(),
            appsink,
            surface_handle,
            sequence: 2,
            timeout: Duration::from_millis(50),
            initial_frame: Some(Arc::new(VideoFrame::new(
                Arc::new(surface),
                Some(Duration::ZERO),
                None,
            ))),
            end_guard: Duration::from_millis(1),
        };

        let error = session
            .extract_frame(Duration::from_secs(3), SeekMode::Accurate)
            .unwrap_err();
        assert_eq!(error.kind, MediaErrorKind::Timeout);
        assert_eq!(seeks.load(Ordering::Relaxed), 1);
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
