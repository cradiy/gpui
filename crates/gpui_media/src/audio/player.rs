use std::{sync::Arc, time::Duration};

use gpui::{Context, EventEmitter, SharedString};

use super::{
    source::AudioSource,
    worker::{AudioSession, AudioWorkerEvent},
};
use crate::{MediaError, MediaResult, PlaybackState, PlaybackTimeline, SeekMode};

/// Properties of the active decoded audio stream.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AudioInfo {
    pub(crate) sample_rate: u32,
    pub(crate) channels: u16,
    pub(crate) duration: Option<Duration>,
    pub(crate) seekable: bool,
    pub(crate) codec: SharedString,
}

impl AudioInfo {
    /// Sample rate, or zero when the browser does not expose it.
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Channel count, or zero when the browser does not expose it.
    pub fn channels(&self) -> u16 {
        self.channels
    }

    pub fn duration(&self) -> Option<Duration> {
        self.duration
    }

    pub fn is_seekable(&self) -> bool {
        self.seekable
    }

    pub fn codec(&self) -> &str {
        &self.codec
    }
}

/// Initial behavior for an [`AudioPlayer`].
#[derive(Clone, Copy, Debug)]
pub struct AudioPlayerOptions {
    pub autoplay: bool,
    pub volume: f64,
    pub muted: bool,
    pub timeline_update_interval: Duration,
}

impl Default for AudioPlayerOptions {
    fn default() -> Self {
        Self {
            autoplay: true,
            volume: 1.0,
            muted: false,
            timeline_update_interval: Duration::from_millis(100),
        }
    }
}

/// Configures an [`AudioPlayer`] before its decoder worker is opened.
pub struct AudioPlayerBuilder {
    source: AudioSource,
    options: AudioPlayerOptions,
}

impl AudioPlayerBuilder {
    pub fn options(mut self, options: AudioPlayerOptions) -> Self {
        self.options = options;
        self
    }

    pub fn build(self, cx: &mut Context<AudioPlayer>) -> MediaResult<AudioPlayer> {
        AudioPlayer::new(self.source, self.options, cx)
    }
}

/// Events emitted by [`AudioPlayer`] for application-provided controls.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum AudioPlayerEvent {
    StateChanged(PlaybackState),
    TimelineChanged(PlaybackTimeline),
    InfoChanged(Arc<AudioInfo>),
    VolumeChanged { volume: f64, muted: bool },
}

/// A cross-platform, presentation-free audio player.
///
/// Symphonia owns demuxing and decoding on a worker thread. CPAL owns device
/// output on desktop; Web uses browser audio playback directly. The component
/// renders no UI; applications compose controls from the
/// public state, methods and events.
pub struct AudioPlayer {
    source: AudioSource,
    session: AudioSession,
    state: PlaybackState,
    timeline: PlaybackTimeline,
    info: Option<Arc<AudioInfo>>,
    play_when_ready: bool,
    volume: f64,
    muted: bool,
}

impl AudioPlayer {
    pub fn builder(source: impl Into<AudioSource>) -> AudioPlayerBuilder {
        AudioPlayerBuilder {
            source: source.into(),
            options: AudioPlayerOptions::default(),
        }
    }

    pub fn new(
        source: impl Into<AudioSource>,
        options: AudioPlayerOptions,
        cx: &mut Context<Self>,
    ) -> MediaResult<Self> {
        let source = source.into();
        let volume = normalize_volume(options.volume);
        let (event_sender, events) = async_channel::unbounded();
        let session = AudioSession::open(
            source.clone(),
            options.autoplay,
            volume,
            options.muted,
            event_sender,
        )?;

        cx.spawn(async move |this, cx| {
            while let Ok(event) = events.recv().await {
                let Some(this) = this.upgrade() else {
                    break;
                };
                this.update(cx, |player, cx| match event {
                    AudioWorkerEvent::Ready(info) => {
                        let info = Arc::new(info);
                        player.info = Some(info.clone());
                        player.refresh_timeline(cx);
                        cx.emit(AudioPlayerEvent::InfoChanged(info));
                        player.set_state(
                            if player.play_when_ready {
                                PlaybackState::Playing
                            } else {
                                PlaybackState::Paused
                            },
                            cx,
                        );
                    }
                    AudioWorkerEvent::Paused => {
                        player.play_when_ready = false;
                        player.refresh_timeline(cx);
                        player.set_state(PlaybackState::Paused, cx);
                    }
                    AudioWorkerEvent::Ended => {
                        player.play_when_ready = false;
                        player.refresh_timeline(cx);
                        player.set_state(PlaybackState::Ended, cx);
                    }
                    AudioWorkerEvent::Error(error) => {
                        player.play_when_ready = false;
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
                this.update(cx, |player, cx| player.refresh_timeline(cx));
            }
        })
        .detach();

        Ok(Self {
            source,
            session,
            state: if options.autoplay {
                PlaybackState::Loading
            } else {
                PlaybackState::Paused
            },
            timeline: PlaybackTimeline::default(),
            info: None,
            play_when_ready: options.autoplay,
            volume,
            muted: options.muted,
        })
    }

    pub fn source(&self) -> &AudioSource {
        &self.source
    }

    pub fn state(&self) -> &PlaybackState {
        &self.state
    }

    pub fn timeline(&self) -> PlaybackTimeline {
        self.timeline
    }

    pub fn position(&self) -> Duration {
        self.timeline.position()
    }

    pub fn duration(&self) -> Option<Duration> {
        self.timeline.duration()
    }

    pub fn is_seekable(&self) -> bool {
        self.session.is_seekable()
    }

    pub fn info(&self) -> Option<&AudioInfo> {
        self.info.as_deref()
    }

    pub fn volume(&self) -> f64 {
        self.volume
    }

    pub fn is_muted(&self) -> bool {
        self.muted
    }

    pub fn play(&mut self, cx: &mut Context<Self>) -> MediaResult<()> {
        if self.state == PlaybackState::Ended {
            if !self.is_seekable() {
                return Err(MediaError::unsupported(
                    "an ended live audio stream cannot be restarted",
                ));
            }
            self.session.seek_to(Duration::ZERO, SeekMode::Accurate)?;
            self.timeline = PlaybackTimeline::new(
                Duration::ZERO,
                self.timeline.duration(),
                self.timeline.is_seekable(),
            );
            cx.emit(AudioPlayerEvent::TimelineChanged(self.timeline));
            self.set_state(PlaybackState::Seeking, cx);
        } else if self.info.is_none() {
            self.set_state(PlaybackState::Loading, cx);
        } else {
            self.set_state(PlaybackState::Playing, cx);
        }
        self.play_when_ready = true;
        self.session.play();
        Ok(())
    }

    pub fn pause(&mut self, cx: &mut Context<Self>) {
        self.play_when_ready = false;
        self.session.pause();
        self.set_state(PlaybackState::Paused, cx);
    }

    pub fn toggle_playback(&mut self, cx: &mut Context<Self>) -> MediaResult<()> {
        match self.state {
            PlaybackState::Playing | PlaybackState::Loading => {
                self.pause(cx);
                Ok(())
            }
            PlaybackState::Paused
            | PlaybackState::Seeking
            | PlaybackState::Ended
            | PlaybackState::Error(_) => self.play(cx),
        }
    }

    pub fn stop(&mut self, cx: &mut Context<Self>) -> MediaResult<()> {
        self.play_when_ready = false;
        self.session.pause();
        self.seek_to(Duration::ZERO, SeekMode::Accurate, cx)?;
        Ok(())
    }

    pub fn seek_to(
        &mut self,
        position: Duration,
        mode: SeekMode,
        cx: &mut Context<Self>,
    ) -> MediaResult<()> {
        let position = self
            .timeline
            .duration()
            .map_or(position, |duration| position.min(duration));
        self.session.seek_to(position, mode)?;
        self.timeline = PlaybackTimeline::new(
            position,
            self.timeline.duration(),
            self.timeline.is_seekable(),
        );
        self.set_state(PlaybackState::Seeking, cx);
        cx.emit(AudioPlayerEvent::TimelineChanged(self.timeline));
        Ok(())
    }

    pub fn set_volume(&mut self, volume: f64, cx: &mut Context<Self>) {
        self.volume = normalize_volume(volume);
        self.session.set_volume(self.volume);
        self.emit_volume(cx);
    }

    pub fn set_muted(&mut self, muted: bool, cx: &mut Context<Self>) {
        self.muted = muted;
        self.session.set_muted(muted);
        self.emit_volume(cx);
    }

    pub fn toggle_muted(&mut self, cx: &mut Context<Self>) {
        self.set_muted(!self.muted, cx);
    }

    pub fn refresh_timeline(&mut self, cx: &mut Context<Self>) {
        let timeline = self.session.timeline();
        if timeline != self.timeline {
            self.timeline = timeline;
            cx.emit(AudioPlayerEvent::TimelineChanged(timeline));
            cx.notify();
        }
    }

    fn emit_volume(&self, cx: &mut Context<Self>) {
        cx.emit(AudioPlayerEvent::VolumeChanged {
            volume: self.volume,
            muted: self.muted,
        });
        cx.notify();
    }

    fn set_state(&mut self, state: PlaybackState, cx: &mut Context<Self>) {
        if self.state != state {
            self.state = state.clone();
            cx.emit(AudioPlayerEvent::StateChanged(state));
            cx.notify();
        }
    }
}

impl EventEmitter<AudioPlayerEvent> for AudioPlayer {}

fn normalize_volume(volume: f64) -> f64 {
    if volume.is_finite() {
        volume.clamp(0.0, 1.0)
    } else {
        1.0
    }
}
