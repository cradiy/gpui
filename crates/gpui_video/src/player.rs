use std::{sync::Arc, time::Duration};

use anyhow::Result;
use gpui::{
    Context, EventEmitter, GpuSpecs, IntoElement, Render, SharedString, Window, div, prelude::*,
    surface,
};
#[cfg(target_os = "linux")]
use gpui::{DmaBufImportStatus, SurfaceFrameBacking};

use crate::{
    FrameTransport, MediaSource, PlaybackTimeline, SeekMode, VideoFrame, VideoFrameExtractor,
    VideoPlaybackStats,
    gstreamer_backend::{BackendEvent, GstreamerPlayback},
};

/// Initial behavior for a [`VideoPlayer`].
#[derive(Clone, Copy, Debug)]
pub struct VideoPlayerOptions {
    pub autoplay: bool,
    pub volume: f64,
    pub muted: bool,
    pub timeline_update_interval: Duration,
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

/// Current high-level playback state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlaybackState {
    Loading,
    Playing,
    Paused,
    Seeking,
    Ended,
    Error(SharedString),
}

/// Events emitted by [`VideoPlayer`] for host controls and custom player UIs.
#[derive(Clone, Debug)]
pub enum VideoPlayerEvent {
    StateChanged(PlaybackState),
    TimelineChanged(PlaybackTimeline),
    BufferingChanged(u8),
    FrameReady(Arc<VideoFrame>),
    FrameTransportChanged(FrameTransport),
    DmaBufImportFailed(SharedString),
    PlaybackRateChanged(f64),
    VolumeChanged { volume: f64, muted: bool },
}

/// A reusable GPUI video playback component.
///
/// The component owns demuxing, decoding, audio output and clock integration,
/// while exposing control and observation APIs for custom player interfaces.
/// Its [`Render`] implementation is intentionally limited to the current video
/// frame and has no built-in interaction or player chrome.
pub struct VideoPlayer {
    source: MediaSource,
    playback: GstreamerPlayback,
    frame: Option<Arc<VideoFrame>>,
    frame_transport: Option<FrameTransport>,
    state: PlaybackState,
    state_after_seek: Option<PlaybackState>,
    timeline: PlaybackTimeline,
    buffering_percent: Option<u8>,
    play_when_ready: bool,
    playback_rate: f64,
    delivered_frames: u64,
    volume: f64,
    muted: bool,
}

impl VideoPlayer {
    pub fn new(
        source: MediaSource,
        options: VideoPlayerOptions,
        cx: &mut Context<Self>,
    ) -> Result<Self> {
        Self::new_with_gpu_specs(source, options, None, cx)
    }

    /// Creates a player configured for the renderer backing `window`.
    ///
    /// Use this constructor to enable capability-gated native NV12 DMA-BUF
    /// negotiation. [`Self::new`] retains the portable CPU and linear DMA-BUF
    /// paths when no window is available during construction.
    pub fn new_in_window(
        source: MediaSource,
        options: VideoPlayerOptions,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Result<Self> {
        Self::new_with_gpu_specs(source, options, window.gpu_specs(), cx)
    }

    fn new_with_gpu_specs(
        source: MediaSource,
        options: VideoPlayerOptions,
        gpu_specs: Option<GpuSpecs>,
        cx: &mut Context<Self>,
    ) -> Result<Self> {
        let (playback, output) = GstreamerPlayback::new(&source, gpu_specs.as_ref())?;
        let initial_volume = normalize_volume(options.volume);
        playback.set_volume(initial_volume);
        playback.set_muted(options.muted);

        cx.spawn(async move |this, cx| {
            while let Ok(frame) = output.frames.recv().await {
                let Some(this) = this.upgrade() else {
                    break;
                };
                this.update(cx, |player, cx| {
                    player.delivered_frames = player.delivered_frames.saturating_add(1);
                    if let Err(error) = player.check_frame_import(cx) {
                        player.set_state(PlaybackState::Error(error.to_string().into()), cx);
                    }
                    if let Some(timestamp) = frame.timestamp() {
                        player.timeline = PlaybackTimeline::new(
                            timestamp,
                            player.timeline.duration(),
                            player.timeline.is_seekable(),
                        );
                    }
                    let transport = frame.transport();
                    if player.frame_transport != Some(transport) {
                        player.frame_transport = Some(transport);
                        cx.emit(VideoPlayerEvent::FrameTransportChanged(transport));
                    }
                    player.frame = Some(frame.clone());
                    let next_state = player.state_after_seek.take().or_else(|| {
                        (player.state == PlaybackState::Loading).then_some(PlaybackState::Playing)
                    });
                    if let Some(next_state) = next_state {
                        player.set_state(
                            if next_state == PlaybackState::Playing && player.is_buffering() {
                                PlaybackState::Loading
                            } else {
                                next_state
                            },
                            cx,
                        );
                    }
                    cx.emit(VideoPlayerEvent::FrameReady(frame));
                    cx.notify();
                });
            }
        })
        .detach();

        cx.spawn(async move |this, cx| {
            while let Ok(event) = output.events.recv().await {
                let Some(this) = this.upgrade() else {
                    break;
                };
                this.update(cx, |player, cx| match event {
                    BackendEvent::Buffering(percent) => {
                        let was_buffering = player.is_buffering();
                        if player.buffering_percent != Some(percent) {
                            player.buffering_percent = Some(percent);
                            cx.emit(VideoPlayerEvent::BufferingChanged(percent));
                            cx.notify();
                        }
                        let is_buffering = player.is_buffering();
                        if is_buffering && !was_buffering && player.play_when_ready {
                            if let Err(error) = player.playback.pause() {
                                player
                                    .set_state(PlaybackState::Error(error.to_string().into()), cx);
                            } else if player.state != PlaybackState::Seeking {
                                player.set_state(PlaybackState::Loading, cx);
                            }
                        } else if !is_buffering && was_buffering && player.play_when_ready {
                            if let Err(error) = player.playback.play() {
                                player
                                    .set_state(PlaybackState::Error(error.to_string().into()), cx);
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
                    BackendEvent::Ended => {
                        player.play_when_ready = false;
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
                    BackendEvent::Error(error) => {
                        player.set_state(PlaybackState::Error(error.into()), cx);
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
                        player.set_state(PlaybackState::Error(error.to_string().into()), cx);
                    }
                    player.refresh_timeline(cx);
                });
            }
        })
        .detach();

        let player = Self {
            source,
            playback,
            frame: None,
            frame_transport: None,
            state: if options.autoplay {
                PlaybackState::Loading
            } else {
                PlaybackState::Paused
            },
            state_after_seek: None,
            timeline: PlaybackTimeline::default(),
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

    pub fn state(&self) -> &PlaybackState {
        &self.state
    }

    pub fn timeline(&self) -> PlaybackTimeline {
        self.timeline
    }

    pub fn duration(&self) -> Option<Duration> {
        self.timeline.duration()
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
    pub fn frame_extractor(&self) -> Result<VideoFrameExtractor> {
        VideoFrameExtractor::new(self.source.clone())
    }

    pub fn frame_transport(&self) -> Option<FrameTransport> {
        self.frame.as_deref().map(VideoFrame::transport)
    }

    pub fn playback_rate(&self) -> f64 {
        self.playback_rate
    }

    /// Returns cumulative frame-delivery statistics for this player.
    pub fn stats(&self) -> VideoPlaybackStats {
        self.playback.stats(self.delivered_frames)
    }

    pub fn volume(&self) -> f64 {
        self.volume
    }

    pub fn is_muted(&self) -> bool {
        self.muted
    }

    pub fn play(&mut self, cx: &mut Context<Self>) -> Result<()> {
        self.play_when_ready = true;
        if self.state == PlaybackState::Ended {
            self.playback.restart()?;
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

    pub fn pause(&mut self, cx: &mut Context<Self>) -> Result<()> {
        self.play_when_ready = false;
        self.playback.pause()?;
        self.state_after_seek = None;
        self.set_state(PlaybackState::Paused, cx);
        Ok(())
    }

    pub fn stop(&mut self, cx: &mut Context<Self>) -> Result<()> {
        self.play_when_ready = false;
        self.playback.pause()?;
        self.playback.seek_to(Duration::ZERO, SeekMode::KeyFrame)?;
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

    pub fn toggle_playback(&mut self, cx: &mut Context<Self>) -> Result<()> {
        match self.state {
            PlaybackState::Playing | PlaybackState::Loading => self.pause(cx),
            PlaybackState::Paused
            | PlaybackState::Seeking
            | PlaybackState::Ended
            | PlaybackState::Error(_) => self.play(cx),
        }
    }

    /// Recreates the active GStreamer pipeline for the same media source.
    ///
    /// This is useful after a network or decoder error. The host decides when
    /// and how often to retry; the player only performs one explicit reload.
    pub fn reload(&mut self, autoplay: bool, cx: &mut Context<Self>) -> Result<()> {
        self.playback.reload(autoplay)?;
        self.frame = None;
        self.frame_transport = None;
        self.state_after_seek = None;
        self.timeline = PlaybackTimeline::default();
        self.buffering_percent = None;
        self.play_when_ready = autoplay;
        self.delivered_frames = 0;
        self.playback_rate = 1.0;
        cx.emit(VideoPlayerEvent::TimelineChanged(self.timeline));
        cx.emit(VideoPlayerEvent::PlaybackRateChanged(self.playback_rate));
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
    ) -> Result<()> {
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
    ) -> Result<()> {
        self.seek_to(self.timeline.target_after(amount), mode, cx)
    }

    pub fn skip_backward(
        &mut self,
        amount: Duration,
        mode: SeekMode,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        self.seek_to(self.timeline.target_before(amount), mode, cx)
    }

    /// Advances a paused pipeline by a number of decoded video frames.
    pub fn step_forward(&mut self, frames: u64, cx: &mut Context<Self>) -> Result<()> {
        self.playback.pause()?;
        self.set_state(PlaybackState::Paused, cx);
        self.playback.step_forward(frames)
    }

    /// Seeks backward by the current frame duration, or 1/30 second when the
    /// stream does not expose frame duration metadata.
    pub fn step_backward(&mut self, frames: u64, cx: &mut Context<Self>) -> Result<()> {
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

    pub fn set_playback_rate(&mut self, rate: f64, cx: &mut Context<Self>) -> Result<()> {
        self.playback.set_playback_rate(rate)?;
        self.playback_rate = rate;
        cx.emit(VideoPlayerEvent::PlaybackRateChanged(rate));
        cx.notify();
        Ok(())
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
        let timeline = self.playback.timeline();
        if timeline != self.timeline {
            self.timeline = timeline;
            cx.emit(VideoPlayerEvent::TimelineChanged(timeline));
            cx.notify();
        }
    }

    #[cfg(target_os = "linux")]
    fn check_frame_import(&mut self, cx: &mut Context<Self>) -> Result<()> {
        let Some(frame) = self.frame.as_deref() else {
            return Ok(());
        };
        let SurfaceFrameBacking::DmaBuf(dma_buf) = frame.surface().backing() else {
            return Ok(());
        };
        let DmaBufImportStatus::Failed(reason) = dma_buf.import_status() else {
            return Ok(());
        };

        if self.playback.switch_to_cpu_fallback()? {
            cx.emit(VideoPlayerEvent::DmaBufImportFailed(
                reason.to_string().into(),
            ));
        }
        Ok(())
    }

    #[cfg(not(target_os = "linux"))]
    fn check_frame_import(&mut self, _: &mut Context<Self>) -> Result<()> {
        Ok(())
    }

    fn emit_volume(&self, cx: &mut Context<Self>) {
        cx.emit(VideoPlayerEvent::VolumeChanged {
            volume: self.volume,
            muted: self.muted,
        });
        cx.notify();
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

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{multiply_duration, normalize_volume};

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
