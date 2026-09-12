use std::{sync::Arc, time::Duration};

use gpui::{
    Context, EventEmitter, GpuSpecs, IntoElement, Render, SharedString, Window, div, prelude::*,
    surface,
};
#[cfg(target_os = "linux")]
use gpui::{DmaBufImportStatus, SurfaceFrameBacking};

use crate::{
    FrameTransport, FrameTransportPreference, MediaBackend, MediaBackendEvent, MediaCapabilities,
    MediaError, MediaInfo, MediaOutputSink, MediaPlaybackRequest, MediaPlaybackSession,
    MediaResult, MediaSource, MediaStreamId, PlaybackState, PlaybackTimeline, SeekMode,
    SubtitleEvent, TransportChange, VideoFrame, VideoFrameExtractor, VideoPlaybackStats,
};

use super::stats::PlaybackCounters;

/// Initial behavior for a [`VideoPlayer`].
#[derive(Clone, Copy, Debug)]
pub struct VideoPlayerOptions {
    pub autoplay: bool,
    pub volume: f64,
    pub muted: bool,
    pub timeline_update_interval: Duration,
}

/// Configures a [`VideoPlayer`] before opening its backend session.
pub struct VideoPlayerBuilder {
    source: MediaSource,
    options: VideoPlayerOptions,
    backend: Arc<dyn MediaBackend>,
}

impl VideoPlayerBuilder {
    pub fn options(mut self, options: VideoPlayerOptions) -> Self {
        self.options = options;
        self
    }

    pub fn backend(mut self, backend: impl MediaBackend) -> Self {
        self.backend = Arc::new(backend);
        self
    }

    pub fn shared_backend(mut self, backend: Arc<dyn MediaBackend>) -> Self {
        self.backend = backend;
        self
    }

    pub fn build(self, cx: &mut Context<VideoPlayer>) -> MediaResult<VideoPlayer> {
        VideoPlayer::new(self.source, self.options, self.backend, cx)
    }

    pub fn build_in_window(
        self,
        window: &Window,
        cx: &mut Context<VideoPlayer>,
    ) -> MediaResult<VideoPlayer> {
        VideoPlayer::new_in_window(self.source, self.options, self.backend, window, cx)
    }
}

impl Default for VideoPlayerOptions {
    fn default() -> Self {
        Self {
            autoplay: true,
            volume: 1.0,
            muted: false,
            timeline_update_interval: Duration::from_millis(100),
        }
    }
}

/// Events emitted by [`VideoPlayer`] for host controls and custom player UIs.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum VideoPlayerEvent {
    StateChanged(PlaybackState),
    TimelineChanged(PlaybackTimeline),
    BufferingChanged(u8),
    MediaInfoChanged(Arc<MediaInfo>),
    Subtitle(SubtitleEvent),
    FrameReady(Arc<VideoFrame>),
    FrameTransportChanged(FrameTransport),
    DmaBufImportFailed(SharedString),
    PlaybackRateChanged(f64),
    VolumeChanged { volume: f64, muted: bool },
}

/// A reusable GPUI video playback component.
///
/// The selected backend owns demuxing, decoding, audio output and clock
/// integration. The component exposes backend-independent control and
/// observation APIs for custom player interfaces. Its [`Render`]
/// implementation is intentionally limited to the current video frame and has
/// no built-in interaction or player chrome.
pub struct VideoPlayer {
    source: MediaSource,
    backend: Arc<dyn MediaBackend>,
    playback: Box<dyn MediaPlaybackSession>,
    counters: Arc<PlaybackCounters>,
    frame: Option<Arc<VideoFrame>>,
    frame_transport: Option<FrameTransport>,
    state: PlaybackState,
    state_after_seek: Option<PlaybackState>,
    timeline: PlaybackTimeline,
    media_info: Option<Arc<MediaInfo>>,
    buffering_percent: Option<u8>,
    play_when_ready: bool,
    playback_rate: f64,
    delivered_frames: u64,
    volume: f64,
    muted: bool,
}

impl VideoPlayer {
    pub fn builder(source: MediaSource, backend: impl MediaBackend) -> VideoPlayerBuilder {
        VideoPlayerBuilder {
            source,
            options: VideoPlayerOptions::default(),
            backend: Arc::new(backend),
        }
    }

    pub fn new(
        source: MediaSource,
        options: VideoPlayerOptions,
        backend: Arc<dyn MediaBackend>,
        cx: &mut Context<Self>,
    ) -> MediaResult<Self> {
        Self::new_with_backend_and_gpu_specs(source, options, backend, None, cx)
    }

    /// Creates a player configured for the renderer backing `window`.
    ///
    /// Use this constructor to enable capability-gated native NV12 DMA-BUF
    /// negotiation. [`Self::new`] retains the portable CPU and linear DMA-BUF
    /// paths when no window is available during construction.
    pub fn new_in_window(
        source: MediaSource,
        options: VideoPlayerOptions,
        backend: Arc<dyn MediaBackend>,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> MediaResult<Self> {
        Self::new_with_backend_and_gpu_specs(source, options, backend, window.gpu_specs(), cx)
    }

    fn new_with_backend_and_gpu_specs(
        source: MediaSource,
        options: VideoPlayerOptions,
        backend: Arc<dyn MediaBackend>,
        gpu_specs: Option<GpuSpecs>,
        cx: &mut Context<Self>,
    ) -> MediaResult<Self> {
        let (output_sink, output) = MediaOutputSink::channel();
        let mut playback = backend.open_playback(
            MediaPlaybackRequest {
                source: source.clone(),
                gpu_specs,
            },
            output_sink,
        )?;
        let initial_volume = normalize_volume(options.volume);
        playback.set_volume(initial_volume);
        playback.set_muted(options.muted);
        let media_info = playback.media_info();

        let frames = output.video_frames;
        cx.spawn(async move |this, cx| {
            while let Ok(frame) = frames.recv().await {
                let Some(this) = this.upgrade() else {
                    break;
                };
                this.update(cx, |player, cx| {
                    player.delivered_frames = player.delivered_frames.saturating_add(1);
                    if let Err(error) = player.check_frame_import(cx) {
                        player.set_state(PlaybackState::Error(Arc::new(error)), cx);
                    }
                    if player.state_after_seek.is_none()
                        && let Some(timestamp) = frame.timestamp()
                    {
                        let frame_timeline = PlaybackTimeline::new(
                            timestamp,
                            player.timeline.duration(),
                            player.timeline.is_seekable(),
                        );
                        player.timeline =
                            timeline_without_regression(player.timeline, frame_timeline);
                    }
                    let transport = frame.transport();
                    if player.frame_transport != Some(transport) {
                        player.frame_transport = Some(transport);
                        cx.emit(VideoPlayerEvent::FrameTransportChanged(transport));
                    }
                    player.frame = Some(frame.clone());
                    // A frame may already be queued when a new seek begins.
                    // Such a stale frame can finish initial loading, but only
                    // the backend's causal `Ready` event may finish a seek.
                    if player.state_after_seek.is_none() {
                        player.finish_pending_transition(cx);
                    }
                    cx.emit(VideoPlayerEvent::FrameReady(frame));
                    cx.notify();
                });
            }
        })
        .detach();

        let events = output.events;
        cx.spawn(async move |this, cx| {
            while let Ok(event) = events.recv().await {
                let Some(this) = this.upgrade() else {
                    break;
                };
                this.update(cx, |player, cx| match event {
                    MediaBackendEvent::Ready => {
                        player.finish_pending_transition(cx);
                        player.refresh_timeline(cx);
                    }
                    MediaBackendEvent::Buffering(percent) => {
                        let was_buffering = player.is_buffering();
                        if player.buffering_percent != Some(percent) {
                            player.buffering_percent = Some(percent);
                            cx.emit(VideoPlayerEvent::BufferingChanged(percent));
                            cx.notify();
                        }
                        let is_buffering = player.is_buffering();
                        if is_buffering && !was_buffering && player.play_when_ready {
                            if let Err(error) = player.playback.pause() {
                                player.set_state(PlaybackState::Error(Arc::new(error)), cx);
                            } else if player.state != PlaybackState::Seeking {
                                player.set_state(PlaybackState::Loading, cx);
                            }
                        } else if !is_buffering && was_buffering && player.play_when_ready {
                            if let Err(error) = player.playback.play() {
                                player.set_state(PlaybackState::Error(Arc::new(error)), cx);
                            } else if player.state != PlaybackState::Seeking {
                                let state = if player.frame.is_some() {
                                    PlaybackState::Playing
                                } else {
                                    PlaybackState::Loading
                                };
                                player.set_state(state, cx);
                            }
                        }
                    }
                    MediaBackendEvent::MediaInfoChanged(info) => {
                        player.media_info = Some(info.clone());
                        cx.emit(VideoPlayerEvent::MediaInfoChanged(info));
                        cx.notify();
                    }
                    MediaBackendEvent::Subtitle(event) => {
                        cx.emit(VideoPlayerEvent::Subtitle(event));
                        cx.notify();
                    }
                    MediaBackendEvent::Ended => {
                        player.play_when_ready = false;
                        cx.emit(VideoPlayerEvent::Subtitle(SubtitleEvent::Reset));
                        if let Some(duration) = player.timeline.duration() {
                            player.timeline = PlaybackTimeline::new(
                                duration,
                                Some(duration),
                                player.timeline.is_seekable(),
                            );
                            cx.emit(VideoPlayerEvent::TimelineChanged(player.timeline));
                        }
                        player.set_state(PlaybackState::Ended, cx);
                    }
                    MediaBackendEvent::Error(error) => {
                        player.set_state(PlaybackState::Error(error), cx);
                    }
                });
            }
        })
        .detach();

        let timeline_update_interval = options
            .timeline_update_interval
            .max(Duration::from_millis(16));
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(timeline_update_interval)
                    .await;
                let Some(this) = this.upgrade() else {
                    break;
                };
                this.update(cx, |player, cx| {
                    if let Err(error) = player.check_frame_import(cx) {
                        player.set_state(PlaybackState::Error(Arc::new(error)), cx);
                    }
                    player.refresh_timeline(cx);
                });
            }
        })
        .detach();

        let mut player = Self {
            source,
            backend,
            playback,
            counters: output.counters,
            frame: None,
            frame_transport: None,
            state: if options.autoplay {
                PlaybackState::Loading
            } else {
                PlaybackState::Paused
            },
            state_after_seek: None,
            timeline: PlaybackTimeline::default(),
            media_info,
            buffering_percent: None,
            play_when_ready: options.autoplay,
            playback_rate: 1.0,
            delivered_frames: 0,
            volume: initial_volume,
            muted: options.muted,
        };
        if options.autoplay {
            player.playback.play()?;
        } else {
            player.playback.pause()?;
        }
        Ok(player)
    }

    pub fn source(&self) -> &MediaSource {
        &self.source
    }

    pub fn backend(&self) -> &Arc<dyn MediaBackend> {
        &self.backend
    }

    pub fn backend_name(&self) -> &'static str {
        self.backend.name()
    }

    pub fn backend_capabilities(&self) -> MediaCapabilities {
        self.playback.capabilities()
    }

    pub fn state(&self) -> &PlaybackState {
        &self.state
    }

    pub fn timeline(&self) -> PlaybackTimeline {
        self.timeline
    }

    pub fn duration(&self) -> Option<Duration> {
        self.timeline.duration()
    }

    pub fn media_info(&self) -> Option<&MediaInfo> {
        self.media_info.as_deref()
    }

    pub fn position(&self) -> Duration {
        self.timeline.position()
    }

    pub fn is_seekable(&self) -> bool {
        self.timeline.is_seekable()
    }

    /// Returns the most recently reported buffering percentage.
    ///
    /// `None` means the backend has not emitted a buffering message. The
    /// presentation of loading or buffering state is intentionally left to
    /// the host application.
    pub fn buffering_percent(&self) -> Option<u8> {
        self.buffering_percent
    }

    pub fn is_buffering(&self) -> bool {
        self.buffering_percent.is_some_and(|percent| percent < 100)
    }

    pub fn current_frame(&self) -> Option<&Arc<VideoFrame>> {
        self.frame.as_ref()
    }

    /// Creates an independent extractor for thumbnails, previews and scrubbing.
    /// Reuse the returned extractor for multiple frame requests.
    pub fn frame_extractor(&self) -> MediaResult<VideoFrameExtractor> {
        VideoFrameExtractor::new(self.source.clone(), self.backend.clone())
    }

    pub fn frame_transport(&self) -> Option<FrameTransport> {
        self.frame.as_deref().map(VideoFrame::transport)
    }

    pub fn playback_rate(&self) -> f64 {
        self.playback_rate
    }

    /// Returns cumulative frame-delivery statistics for this player.
    pub fn stats(&self) -> VideoPlaybackStats {
        self.counters.snapshot(self.delivered_frames)
    }

    pub fn volume(&self) -> f64 {
        self.volume
    }

    pub fn is_muted(&self) -> bool {
        self.muted
    }

    pub fn play(&mut self, cx: &mut Context<Self>) -> MediaResult<()> {
        self.play_when_ready = true;
        if self.state == PlaybackState::Ended {
            self.playback.restart()?;
            cx.emit(VideoPlayerEvent::Subtitle(SubtitleEvent::Reset));
            self.state_after_seek = Some(PlaybackState::Playing);
            self.timeline = PlaybackTimeline::new(
                Duration::ZERO,
                self.timeline.duration(),
                self.timeline.is_seekable(),
            );
            self.set_state(PlaybackState::Seeking, cx);
            cx.emit(VideoPlayerEvent::TimelineChanged(self.timeline));
            return Ok(());
        } else if self.is_buffering() {
            self.playback.pause()?;
        } else {
            self.playback.play()?;
        }
        self.state_after_seek = None;
        let state = if self.is_buffering() || self.frame.is_none() {
            PlaybackState::Loading
        } else {
            PlaybackState::Playing
        };
        self.set_state(state, cx);
        Ok(())
    }

    pub fn pause(&mut self, cx: &mut Context<Self>) -> MediaResult<()> {
        self.play_when_ready = false;
        self.playback.pause()?;
        self.state_after_seek = None;
        self.set_state(PlaybackState::Paused, cx);
        Ok(())
    }

    pub fn stop(&mut self, cx: &mut Context<Self>) -> MediaResult<()> {
        self.play_when_ready = false;
        self.playback.pause()?;
        self.playback.seek_to(Duration::ZERO, SeekMode::KeyFrame)?;
        cx.emit(VideoPlayerEvent::Subtitle(SubtitleEvent::Reset));
        self.state_after_seek = Some(PlaybackState::Paused);
        self.timeline = PlaybackTimeline::new(
            Duration::ZERO,
            self.timeline.duration(),
            self.timeline.is_seekable(),
        );
        self.set_state(PlaybackState::Seeking, cx);
        cx.emit(VideoPlayerEvent::TimelineChanged(self.timeline));
        Ok(())
    }

    pub fn toggle_playback(&mut self, cx: &mut Context<Self>) -> MediaResult<()> {
        match self.state {
            PlaybackState::Playing | PlaybackState::Loading => self.pause(cx),
            PlaybackState::Paused
            | PlaybackState::Seeking
            | PlaybackState::Ended
            | PlaybackState::Error(_) => self.play(cx),
        }
    }

    /// Recreates the active backend pipeline for the same media source.
    ///
    /// This is useful after a network or decoder error. The host decides when
    /// and how often to retry; the player only performs one explicit reload.
    pub fn reload(&mut self, autoplay: bool, cx: &mut Context<Self>) -> MediaResult<()> {
        self.playback.reload(autoplay)?;
        self.frame = None;
        self.frame_transport = None;
        self.state_after_seek = None;
        self.timeline = PlaybackTimeline::default();
        self.media_info = self.playback.media_info();
        self.buffering_percent = None;
        self.play_when_ready = autoplay;
        self.delivered_frames = 0;
        self.playback_rate = 1.0;
        cx.emit(VideoPlayerEvent::TimelineChanged(self.timeline));
        cx.emit(VideoPlayerEvent::PlaybackRateChanged(self.playback_rate));
        cx.emit(VideoPlayerEvent::Subtitle(SubtitleEvent::Reset));
        self.set_state(
            if autoplay {
                PlaybackState::Loading
            } else {
                PlaybackState::Paused
            },
            cx,
        );
        Ok(())
    }

    pub fn seek_to(
        &mut self,
        position: Duration,
        mode: SeekMode,
        cx: &mut Context<Self>,
    ) -> MediaResult<()> {
        let target = self
            .timeline
            .duration()
            .map_or(position, |duration| position.min(duration));
        let resume_state = if self.play_when_ready {
            PlaybackState::Playing
        } else {
            PlaybackState::Paused
        };
        self.playback.seek_to(target, mode)?;
        cx.emit(VideoPlayerEvent::Subtitle(SubtitleEvent::Reset));
        self.state_after_seek = Some(resume_state);
        self.timeline = PlaybackTimeline::new(
            target,
            self.timeline.duration(),
            self.timeline.is_seekable(),
        );
        self.set_state(PlaybackState::Seeking, cx);
        cx.emit(VideoPlayerEvent::TimelineChanged(self.timeline));
        Ok(())
    }

    pub fn skip_forward(
        &mut self,
        amount: Duration,
        mode: SeekMode,
        cx: &mut Context<Self>,
    ) -> MediaResult<()> {
        self.seek_to(self.timeline.target_after(amount), mode, cx)
    }

    pub fn skip_backward(
        &mut self,
        amount: Duration,
        mode: SeekMode,
        cx: &mut Context<Self>,
    ) -> MediaResult<()> {
        self.seek_to(self.timeline.target_before(amount), mode, cx)
    }

    /// Advances a paused pipeline by a number of decoded video frames.
    pub fn step_forward(&mut self, frames: u64, cx: &mut Context<Self>) -> MediaResult<()> {
        self.playback.pause()?;
        self.set_state(PlaybackState::Paused, cx);
        self.playback.step_forward(frames)
    }

    /// Seeks backward by the current frame duration, or 1/30 second when the
    /// stream does not expose frame duration metadata.
    pub fn step_backward(&mut self, frames: u64, cx: &mut Context<Self>) -> MediaResult<()> {
        if frames == 0 {
            return Ok(());
        }
        let frame_duration = self
            .frame
            .as_deref()
            .and_then(VideoFrame::duration)
            .unwrap_or(Duration::from_nanos(1_000_000_000 / 30));
        let amount = multiply_duration(frame_duration, frames);
        self.playback.pause()?;
        self.set_state(PlaybackState::Paused, cx);
        self.skip_backward(amount, SeekMode::Accurate, cx)
    }

    pub fn select_audio_stream(
        &mut self,
        id: &MediaStreamId,
        cx: &mut Context<Self>,
    ) -> MediaResult<()> {
        self.playback.select_audio_stream(id)?;
        cx.notify();
        Ok(())
    }

    pub fn select_subtitle_stream(
        &mut self,
        id: Option<&MediaStreamId>,
        cx: &mut Context<Self>,
    ) -> MediaResult<()> {
        self.playback.select_subtitle_stream(id)?;
        cx.emit(VideoPlayerEvent::Subtitle(SubtitleEvent::Reset));
        cx.notify();
        Ok(())
    }

    pub fn set_playback_rate(&mut self, rate: f64, cx: &mut Context<Self>) -> MediaResult<()> {
        self.playback.set_playback_rate(rate)?;
        self.playback_rate = rate;
        cx.emit(VideoPlayerEvent::PlaybackRateChanged(rate));
        cx.notify();
        Ok(())
    }

    pub fn set_frame_transport_preference(
        &mut self,
        preference: FrameTransportPreference,
    ) -> MediaResult<TransportChange> {
        self.playback.set_frame_transport_preference(preference)
    }

    pub fn set_volume(&mut self, volume: f64, cx: &mut Context<Self>) {
        self.volume = normalize_volume(volume);
        self.playback.set_volume(self.volume);
        self.emit_volume(cx);
    }

    pub fn set_muted(&mut self, muted: bool, cx: &mut Context<Self>) {
        self.muted = muted;
        self.playback.set_muted(muted);
        self.emit_volume(cx);
    }

    pub fn toggle_muted(&mut self, cx: &mut Context<Self>) {
        self.set_muted(!self.muted, cx);
    }

    /// Refreshes duration, position and seekability immediately.
    pub fn refresh_timeline(&mut self, cx: &mut Context<Self>) {
        // A backend may continue reporting its running clock while an
        // asynchronous seek is still decoding toward the requested position.
        // Keep the public timeline pinned to the seek target until the backend
        // confirms completion with `MediaBackendEvent::Ready`.
        if !accept_backend_timeline(&self.state) {
            return;
        }

        let timeline = timeline_without_regression(self.timeline, self.playback.timeline());
        if timeline != self.timeline {
            self.timeline = timeline;
            cx.emit(VideoPlayerEvent::TimelineChanged(timeline));
            cx.notify();
        }
    }

    #[cfg(target_os = "linux")]
    fn check_frame_import(&mut self, cx: &mut Context<Self>) -> MediaResult<()> {
        let Some(frame) = self.frame.as_deref() else {
            return Ok(());
        };
        let SurfaceFrameBacking::DmaBuf(dma_buf) = frame.surface().backing() else {
            return Ok(());
        };
        let DmaBufImportStatus::Failed(reason) = dma_buf.import_status() else {
            return Ok(());
        };

        let transport_change = self
            .set_frame_transport_preference(FrameTransportPreference::CpuOnly)
            .map_err(|error| {
                MediaError::new(
                    crate::MediaErrorKind::VideoOutput,
                    error.message,
                    crate::MediaRecovery::Retry,
                )
            })?;
        if transport_change == TransportChange::Reconfigured {
            cx.emit(VideoPlayerEvent::DmaBufImportFailed(
                reason.to_string().into(),
            ));
        }
        Ok(())
    }

    #[cfg(not(target_os = "linux"))]
    fn check_frame_import(&mut self, _: &mut Context<Self>) -> MediaResult<()> {
        Ok(())
    }

    fn emit_volume(&self, cx: &mut Context<Self>) {
        cx.emit(VideoPlayerEvent::VolumeChanged {
            volume: self.volume,
            muted: self.muted,
        });
        cx.notify();
    }

    fn finish_pending_transition(&mut self, cx: &mut Context<Self>) {
        let next_state = self.state_after_seek.take().or_else(|| {
            (self.state == PlaybackState::Loading).then_some(if self.play_when_ready {
                PlaybackState::Playing
            } else {
                PlaybackState::Paused
            })
        });
        if let Some(next_state) = next_state {
            self.set_state(
                if next_state == PlaybackState::Playing && self.is_buffering() {
                    PlaybackState::Loading
                } else {
                    next_state
                },
                cx,
            );
        }
    }

    fn set_state(&mut self, state: PlaybackState, cx: &mut Context<Self>) {
        if self.state != state {
            self.state = state.clone();
            cx.emit(VideoPlayerEvent::StateChanged(state));
            cx.notify();
        }
    }
}

impl EventEmitter<VideoPlayerEvent> for VideoPlayer {}

impl Render for VideoPlayer {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let frame = self.frame.clone();

        div()
            .relative()
            .size_full()
            .overflow_hidden()
            .when_some(frame, |this, frame| {
                this.child(surface(frame.surface().clone()).absolute().size_full())
            })
    }
}

fn multiply_duration(duration: Duration, multiplier: u64) -> Duration {
    let nanos = duration.as_nanos().saturating_mul(u128::from(multiplier));
    Duration::from_nanos(nanos.min(u128::from(u64::MAX)) as u64)
}

fn normalize_volume(volume: f64) -> f64 {
    if volume.is_finite() {
        volume.clamp(0.0, 1.0)
    } else {
        1.0
    }
}

fn accept_backend_timeline(state: &PlaybackState) -> bool {
    state != &PlaybackState::Seeking
}

fn timeline_without_regression(
    current: PlaybackTimeline,
    candidate: PlaybackTimeline,
) -> PlaybackTimeline {
    if candidate.position() >= current.position() {
        return candidate;
    }

    PlaybackTimeline::new(
        current.position(),
        candidate.duration(),
        candidate.is_seekable(),
    )
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{
        PlaybackState, PlaybackTimeline, accept_backend_timeline, multiply_duration,
        normalize_volume, timeline_without_regression,
    };

    #[test]
    fn backend_timeline_is_suspended_while_seeking() {
        assert!(!accept_backend_timeline(&PlaybackState::Seeking));
        assert!(accept_backend_timeline(&PlaybackState::Playing));
        assert!(accept_backend_timeline(&PlaybackState::Paused));
    }

    #[test]
    fn playback_timeline_does_not_move_backward_between_discontinuities() {
        let current = PlaybackTimeline::new(
            Duration::from_millis(5_200),
            Some(Duration::from_secs(10)),
            true,
        );
        let stale_frame = PlaybackTimeline::new(
            Duration::from_millis(5_150),
            Some(Duration::from_secs(10)),
            true,
        );
        let next_clock = PlaybackTimeline::new(
            Duration::from_millis(5_250),
            Some(Duration::from_secs(10)),
            true,
        );

        assert_eq!(
            timeline_without_regression(current, stale_frame).position(),
            current.position()
        );
        assert_eq!(timeline_without_regression(current, next_clock), next_clock);
    }

    #[test]
    fn frame_step_duration_saturates() {
        assert_eq!(
            multiply_duration(Duration::from_millis(40), 3),
            Duration::from_millis(120)
        );
        assert_eq!(
            multiply_duration(Duration::from_secs(u64::MAX), 2),
            Duration::from_nanos(u64::MAX)
        );
    }

    #[test]
    fn volume_is_finite_and_clamped() {
        assert_eq!(normalize_volume(-1.0), 0.0);
        assert_eq!(normalize_volume(2.0), 1.0);
        assert_eq!(normalize_volume(f64::NAN), 1.0);
    }
}
