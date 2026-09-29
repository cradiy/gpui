use super::{player::AudioInfo, source::AudioSource};
use crate::{MediaError, MediaResult, NetworkSourceOptions, PlaybackTimeline, SeekMode};
use std::{cell::Cell, rc::Rc, sync::Arc, time::Duration};
use wasm_bindgen::{JsCast, closure::Closure};

pub(super) enum AudioWorkerEvent {
    Ready(AudioInfo),
    Paused,
    Ended,
    Error(Arc<MediaError>),
}

struct State {
    audio: web_sys::HtmlAudioElement,
    events: async_channel::Sender<AudioWorkerEvent>,
    revision: Cell<u64>,
    failed: Cell<bool>,
}
impl State {
    fn timeline(&self) -> PlaybackTimeline {
        let duration = self.audio.duration();
        let position = self.audio.current_time();
        PlaybackTimeline::new(
            Duration::from_secs_f64(if position.is_finite() {
                position.max(0.)
            } else {
                0.
            }),
            (duration.is_finite() && duration >= 0.).then(|| Duration::from_secs_f64(duration)),
            self.audio.seekable().length() > 0,
        )
    }
    fn ready(&self) {
        if self.failed.get() {
            return;
        }
        let timeline = self.timeline();
        let _ = self.events.try_send(AudioWorkerEvent::Ready(AudioInfo {
            sample_rate: 0,
            channels: 0,
            codec: "browser".into(),
            duration: timeline.duration(),
            seekable: timeline.is_seekable(),
        }));
    }
    fn error(&self, error: impl std::fmt::Debug) {
        self.failed.set(true);
        let _ = self
            .events
            .try_send(AudioWorkerEvent::Error(Arc::new(MediaError::backend(
                format!("browser audio operation failed: {error:?}"),
            ))));
    }
}

pub(super) struct AudioSession {
    state: Rc<State>,
    listeners: Vec<(&'static str, Closure<dyn FnMut(web_sys::Event)>)>,
}
impl AudioSession {
    pub(super) fn open(
        source: AudioSource,
        autoplay: bool,
        volume: f64,
        muted: bool,
        events: async_channel::Sender<AudioWorkerEvent>,
    ) -> MediaResult<Self> {
        let source = source.browser_source()?;
        if !["http:", "https:", "blob:", "data:"]
            .iter()
            .any(|scheme| source.uri().starts_with(scheme))
        {
            return Err(MediaError::unsupported(
                "browser audio requires an HTTP(S), blob, or data URL",
            ));
        }
        if source.network_options() != &NetworkSourceOptions::default() {
            return Err(MediaError::unsupported(
                "browser audio uses browser-managed networking; custom network options are unsupported",
            ));
        }
        let audio = web_sys::HtmlAudioElement::new().map_err(|error| {
            MediaError::backend(format!("cannot create browser audio: {error:?}"))
        })?;
        audio.set_preload("auto");
        audio.set_volume(volume);
        audio.set_muted(muted);
        let state = Rc::new(State {
            audio,
            events,
            revision: Cell::new(0),
            failed: Cell::new(false),
        });
        let mut session = Self {
            state,
            listeners: Vec::new(),
        };
        for name in ["loadeddata", "seeked", "ended", "error", "pause"] {
            let weak = Rc::downgrade(&session.state);
            let callback = Closure::wrap(Box::new(move |_: web_sys::Event| {
                let Some(state) = weak.upgrade() else { return };
                match name {
                    "loadeddata" | "seeked" => state.ready(),
                    "pause" if state.audio.paused() && !state.audio.ended() => {
                        let _ = state.events.try_send(AudioWorkerEvent::Paused);
                    }
                    "ended" => {
                        let _ = state.events.try_send(AudioWorkerEvent::Ended);
                    }
                    "error" => state.error(
                        state
                            .audio
                            .error()
                            .map(|error| (error.code(), error.message())),
                    ),
                    _ => {}
                }
            }) as Box<dyn FnMut(web_sys::Event)>);
            session
                .state
                .audio
                .add_event_listener_with_callback(name, callback.as_ref().unchecked_ref())
                .map_err(|error| {
                    MediaError::backend(format!("cannot listen to browser audio: {error:?}"))
                })?;
            session.listeners.push((name, callback));
        }
        session.state.audio.set_src(source.uri());
        if autoplay {
            session.play();
        }
        Ok(session)
    }
    pub(super) fn play(&self) {
        let revision = self.state.revision.get().wrapping_add(1);
        self.state.revision.set(revision);
        match self.state.audio.play() {
            Ok(promise) => {
                let weak = Rc::downgrade(&self.state);
                wasm_bindgen_futures::spawn_local(async move {
                    let result = wasm_bindgen_futures::JsFuture::from(promise).await;
                    let Some(state) = weak.upgrade() else { return };
                    if state.revision.get() != revision {
                        return;
                    }
                    match result {
                        Ok(_) => {
                            state.failed.set(false);
                            state.ready();
                        }
                        Err(error) => state.error(error),
                    }
                });
            }
            Err(error) => self.state.error(error),
        }
    }
    pub(super) fn pause(&self) {
        self.state
            .revision
            .set(self.state.revision.get().wrapping_add(1));
        if let Err(error) = self.state.audio.pause() {
            self.state.error(error);
        }
    }
    pub(super) fn set_volume(&self, volume: f64) {
        self.state.audio.set_volume(volume);
    }
    pub(super) fn set_muted(&self, muted: bool) {
        self.state.audio.set_muted(muted);
    }
    pub(super) fn seek_to(&self, position: Duration, _mode: SeekMode) -> MediaResult<()> {
        let timeline = self.timeline();
        if !timeline.is_seekable() {
            return Err(MediaError::unsupported("browser audio is not seekable"));
        }
        let seconds = timeline
            .duration()
            .map_or(position, |duration| position.min(duration))
            .as_secs_f64();
        js_sys::Reflect::set(&self.state.audio, &"currentTime".into(), &seconds.into()).map_err(
            |error| MediaError::backend(format!("browser audio seek failed: {error:?}")),
        )?;
        Ok(())
    }
    pub(super) fn timeline(&self) -> PlaybackTimeline {
        self.state.timeline()
    }
    pub(super) fn is_seekable(&self) -> bool {
        self.timeline().is_seekable()
    }
}
impl Drop for AudioSession {
    fn drop(&mut self) {
        self.state
            .revision
            .set(self.state.revision.get().wrapping_add(1));
        for (name, callback) in self.listeners.drain(..) {
            let _ = self
                .state
                .audio
                .remove_event_listener_with_callback(name, callback.as_ref().unchecked_ref());
        }
        let _ = self.state.audio.pause();
        let _ = self.state.audio.remove_attribute("src");
        self.state.audio.load();
    }
}
