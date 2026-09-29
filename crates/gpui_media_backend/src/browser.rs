use gpui_media_core::*;
use gpui_util::browser::{BrowserResource, BrowserVideoFrame};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::Arc,
    time::Duration,
};
use wasm_bindgen::{JsCast, closure::Closure};
use web_sys::HtmlVideoElement;

#[wasm_bindgen::prelude::wasm_bindgen]
extern "C" {
    #[wasm_bindgen(extends = HtmlVideoElement)]
    type FrameVideo;
    #[wasm_bindgen(method, catch, structural, js_name = requestVideoFrameCallback)]
    fn request_frame(
        this: &FrameVideo,
        callback: &js_sys::Function,
    ) -> Result<u32, wasm_bindgen::JsValue>;
    #[wasm_bindgen(method, structural, js_name = cancelVideoFrameCallback)]
    fn cancel_frame(this: &FrameVideo, id: u32);
}

fn js_error(error: impl std::fmt::Debug) -> MediaError {
    MediaError::backend(format!("browser media operation failed: {error:?}"))
}

pub(super) fn initialize() -> MediaResult<()> {
    web_sys::window()
        .ok_or_else(|| MediaError::unsupported("browser media requires the window thread"))?;
    Ok(())
}

pub(super) fn open_playback(
    request: MediaPlaybackRequest,
    output: MediaOutputSink,
) -> MediaResult<Box<dyn MediaPlaybackSession>> {
    Ok(Box::new(BrowserSession::open(request.source, output)?))
}

pub(super) fn open_frame_extractor(
    _request: FrameExtractorBackendRequest,
) -> MediaResult<Box<dyn FrameExtractionSession>> {
    Err(MediaError::unsupported(
        "browser media does not support independent frame extraction",
    ))
}

struct State {
    element: HtmlVideoElement,
    output: MediaOutputSink,
    handle: FrameHandle,
    sequence: Cell<u64>,
    last_time: Cell<Option<f64>>,
    revision: Cell<u64>,
    alive: Cell<bool>,
    failed: Cell<bool>,
    frame_callback_id: Cell<Option<u32>>,
    frame_callback: RefCell<Option<Closure<dyn FnMut(f64)>>>,
    listeners: RefCell<Vec<(&'static str, Closure<dyn FnMut(web_sys::Event)>)>>,
}

impl State {
    fn timeline(&self) -> PlaybackTimeline {
        let seconds = self.element.duration();
        let duration =
            (seconds.is_finite() && seconds >= 0.).then(|| Duration::from_secs_f64(seconds));
        let time = self.element.current_time();
        PlaybackTimeline::new(
            Duration::from_secs_f64(if time.is_finite() { time.max(0.) } else { 0. }),
            duration,
            self.element.seekable().length() > 0,
        )
    }

    fn capture(&self) -> MediaResult<()> {
        let video = &self.element;
        if video.ready_state() < 2
            || video.seeking()
            || video.video_width() == 0
            || video.video_height() == 0
        {
            return Ok(());
        }
        let time = video.current_time();
        if self.last_time.get() == Some(time) {
            return Ok(());
        }
        let frame = web_sys::VideoFrame::new_with_html_video_element(video).map_err(js_error)?;
        let snapshot = BrowserVideoFrame::new(&frame).map_err(js_error);
        frame.close();
        let snapshot = snapshot?;
        let size = FrameSize::new(snapshot.width() as i32, snapshot.height() as i32);
        let sequence = self.sequence.get().wrapping_add(1);
        let buffer = FrameBuffer::with_backing(
            self.handle,
            sequence,
            size,
            FrameRect {
                origin: Default::default(),
                size,
            },
            size,
            PixelFormat::Rgba8,
            FrameBacking::Browser(snapshot),
            Default::default(),
        )?;
        self.output.publish_video_frame(Arc::new(VideoFrame::new(
            Arc::new(buffer),
            Some(self.timeline().position()),
            None,
        )));
        self.sequence.set(sequence);
        self.last_time.set(Some(time));
        Ok(())
    }

    fn schedule(&self) {
        if self.alive.get() {
            if let Some(callback) = self.frame_callback.borrow().as_ref() {
                match self
                    .element
                    .unchecked_ref::<FrameVideo>()
                    .request_frame(callback.as_ref().unchecked_ref())
                {
                    Ok(id) => self.frame_callback_id.set(Some(id)),
                    Err(error) => {
                        self.output
                            .emit(MediaBackendEvent::Error(Arc::new(js_error(error))));
                    }
                }
            }
        }
    }

    fn play(self: &Rc<Self>) -> MediaResult<()> {
        let promise = self.element.play().map_err(js_error)?;
        let weak = Rc::downgrade(self);
        let revision = self.revision.get().wrapping_add(1);
        self.revision.set(revision);
        wasm_bindgen_futures::spawn_local(async move {
            let result = wasm_bindgen_futures::JsFuture::from(promise).await;
            if let Some(state) = weak.upgrade() {
                if state.alive.get() && state.revision.get() == revision {
                    match result {
                        Ok(_) => {
                            state.failed.set(false);
                            if !state.failed.get() {
                                state.output.emit(MediaBackendEvent::Ready);
                            }
                        }
                        Err(error) => {
                            state.failed.set(true);
                            state
                                .output
                                .emit(MediaBackendEvent::Error(Arc::new(js_error(error))));
                        }
                    }
                }
            }
        });
        Ok(())
    }
}

struct Owner(Rc<State>);
impl Drop for Owner {
    fn drop(&mut self) {
        let state = &self.0;
        state.alive.set(false);
        state.revision.set(state.revision.get().wrapping_add(1));
        if let Some(id) = state.frame_callback_id.take() {
            state.element.unchecked_ref::<FrameVideo>().cancel_frame(id);
        }
        for (name, callback) in state.listeners.borrow_mut().drain(..) {
            let _ = state
                .element
                .remove_event_listener_with_callback(name, callback.as_ref().unchecked_ref());
        }
        let _ = state.element.pause();
        let _ = state.element.remove_attribute("src");
        state.element.load();
    }
}

/// Browser-owned playback controlled on the window thread.
///
/// Supports HTTP(S), blob and data URLs. Decoding, audio output and A/V timing
/// belong to the browser. Calls on another thread return an unsupported error.
struct BrowserSession {
    owner: BrowserResource<Owner>,
}

impl BrowserSession {
    fn open(source: MediaSource, output: MediaOutputSink) -> MediaResult<Self> {
        initialize()?;
        if !["http:", "https:", "blob:", "data:"]
            .iter()
            .any(|scheme| source.uri().starts_with(scheme))
        {
            return Err(MediaError::unsupported(
                "browser media requires an HTTP(S), blob, or data URL",
            ));
        }
        if source.network_options() != &NetworkSourceOptions::default() {
            return Err(MediaError::unsupported(
                "browser media requests use browser-managed networking; custom network options are unsupported",
            ));
        }
        let element: HtmlVideoElement = web_sys::window()
            .unwrap()
            .document()
            .ok_or_else(|| MediaError::unsupported("no browser document"))?
            .create_element("video")
            .map_err(js_error)?
            .dyn_into()
            .map_err(js_error)?;
        if !js_sys::Reflect::get(&element, &"requestVideoFrameCallback".into())
            .map_err(js_error)?
            .is_function()
            || !js_sys::Reflect::get(&js_sys::global(), &"VideoFrame".into())
                .map_err(js_error)?
                .is_function()
        {
            return Err(MediaError::unsupported(
                "browser video requires VideoFrame and requestVideoFrameCallback",
            ));
        }
        element.set_attribute("playsinline", "").map_err(js_error)?;
        element.set_cross_origin(Some("anonymous"));
        element.set_preload("auto");
        let state = Rc::new(State {
            element,
            output,
            handle: FrameHandle::new(),
            sequence: Cell::new(0),
            last_time: Cell::new(None),
            revision: Cell::new(0),
            alive: Cell::new(true),
            failed: Cell::new(false),
            frame_callback_id: Cell::new(None),
            frame_callback: RefCell::new(None),
            listeners: RefCell::new(Vec::new()),
        });
        let owner = Owner(state.clone());
        for name in [
            "loadeddata",
            "seeked",
            "ended",
            "error",
            "waiting",
            "canplay",
            "playing",
        ] {
            let weak = Rc::downgrade(&state);
            let callback =
                Closure::wrap(Box::new(move |_: web_sys::Event| {
                    let Some(state) = weak.upgrade() else { return };
                    match name {
                        "loadeddata" | "seeked" => {
                            state.last_time.set(None);
                            if let Err(error) = state.capture() {
                                state.output.emit(MediaBackendEvent::Error(Arc::new(error)));
                                return;
                            }
                            if !state.failed.get() {
                                state.output.emit(MediaBackendEvent::Ready);
                            }
                        }
                        "ended" => {
                            state.output.emit(MediaBackendEvent::Ended);
                        }
                        "waiting" => {
                            state.output.emit(MediaBackendEvent::Buffering(0));
                        }
                        // Readiness must clear buffering even when playback was
                        // paused while waiting for data (including during seek).
                        "canplay" | "playing" => {
                            // Seeking to the end can become ready before the
                            // browser delivers `ended`. Do not resume at EOF.
                            if state.element.ended() {
                                state.output.emit(MediaBackendEvent::Ended);
                            }
                            state.output.emit(MediaBackendEvent::Buffering(100));
                        }
                        "error" => {
                            state.failed.set(true);
                            let detail = state
                                .element
                                .error()
                                .map(|error| format!("code {}: {}", error.code(), error.message()))
                                .unwrap_or_default();
                            state.output.emit(MediaBackendEvent::Error(Arc::new(
                                MediaError::backend(format!("browser media error {detail}")),
                            )));
                        }
                        _ => {}
                    }
                }) as Box<dyn FnMut(web_sys::Event)>);
            state
                .element
                .add_event_listener_with_callback(name, callback.as_ref().unchecked_ref())
                .map_err(js_error)?;
            state.listeners.borrow_mut().push((name, callback));
        }
        {
            let weak = Rc::downgrade(&state);
            *state.frame_callback.borrow_mut() = Some(Closure::wrap(Box::new(move |_: f64| {
                let Some(state) = weak.upgrade() else { return };
                state.frame_callback_id.set(None);
                if let Err(error) = state.capture() {
                    state.output.emit(MediaBackendEvent::Error(Arc::new(error)));
                    return;
                }
                state.schedule();
            })
                as Box<dyn FnMut(f64)>));
            state.schedule();
        }
        state.element.set_src(source.uri());
        Ok(Self {
            owner: BrowserResource::new(owner),
        })
    }

    fn with<T>(&self, f: impl FnOnce(&Rc<State>) -> MediaResult<T>) -> MediaResult<T> {
        self.owner.with(|owner| f(&owner.0)).unwrap_or_else(|| {
            Err(MediaError::unsupported(
                "browser playback must be controlled on its owner thread",
            ))
        })
    }
}

impl MediaPlaybackSession for BrowserSession {
    fn capabilities(&self) -> MediaCapabilities {
        self.with(|state| {
            Ok(MediaCapabilities {
                video: true,
                audio: true,
                seeking: state.timeline().is_seekable(),
                playback_rate: true,
                ..Default::default()
            })
        })
        .unwrap_or_default()
    }
    fn play(&mut self) -> MediaResult<()> {
        self.with(|state| state.play())
    }
    fn pause(&mut self) -> MediaResult<()> {
        self.with(|state| {
            state.revision.set(state.revision.get().wrapping_add(1));
            state.element.pause().map_err(js_error)
        })
    }
    fn timeline(&self) -> PlaybackTimeline {
        self.with(|state| Ok(state.timeline())).unwrap_or_default()
    }
    fn reload(&mut self, autoplay: bool) -> MediaResult<()> {
        self.with(|state| {
            state.revision.set(state.revision.get().wrapping_add(1));
            state.last_time.set(None);
            state.failed.set(false);
            state.element.load();
            if state.frame_callback_id.get().is_none() {
                state.schedule();
            }
            if autoplay {
                state.play()?;
            }
            Ok(())
        })
    }
    fn seek_to(&mut self, position: Duration, _mode: SeekMode) -> MediaResult<()> {
        self.with(|state| {
            if !state.timeline().is_seekable() {
                return Err(MediaError::unsupported("browser source is not seekable"));
            }
            state.last_time.set(None);
            let timeline = state.timeline();
            let seconds = timeline
                .duration()
                .map_or(position, |duration| position.min(duration))
                .as_secs_f64();
            js_sys::Reflect::set(&state.element, &"currentTime".into(), &seconds.into())
                .map_err(js_error)?;
            Ok(())
        })
    }
    fn set_playback_rate(&mut self, rate: f64) -> MediaResult<()> {
        self.with(|state| {
            if !rate.is_finite() || rate <= 0. {
                return Err(MediaError::invalid_input(
                    "playback rate must be finite and positive",
                ));
            }
            js_sys::Reflect::set(&state.element, &"playbackRate".into(), &rate.into())
                .map_err(js_error)
                .map(|_| ())
        })
    }
    fn set_volume(&mut self, volume: f64) {
        let _ = self.with(|state| {
            state.element.set_volume(if volume.is_finite() {
                volume.clamp(0., 1.)
            } else {
                1.
            });
            Ok(())
        });
    }
    fn set_muted(&mut self, muted: bool) {
        let _ = self.with(|state| {
            state.element.set_muted(muted);
            Ok(())
        });
    }
}
