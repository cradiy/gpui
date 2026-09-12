use std::{
    sync::{
        Arc, RwLock,
        mpsc::{self, Receiver, Sender},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

use gpui::{DevicePixels, SurfaceFrame, SurfaceHandle, size};
use windows::{
    Win32::{
        Foundation::{RECT, S_FALSE},
        Graphics::{
            Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM,
            Imaging::{
                CLSID_WICImagingFactory, GUID_WICPixelFormat32bppBGRA, IWICBitmap,
                IWICImagingFactory, WICBitmapCacheOnLoad, WICBitmapLockRead, WICRect,
            },
        },
        Media::MediaFoundation::{
            CLSID_MFMediaEngineClassFactory, IMFMediaEngine, IMFMediaEngineClassFactory,
            IMFMediaEngineEx, IMFMediaEngineNotify, IMFMediaEngineNotify_Impl, IMFTimedText,
            IMFTimedTextCue, IMFTimedTextNotify, IMFTimedTextNotify_Impl, IMFTimedTextTrackList,
            MF_MEDIA_ENGINE_CALLBACK, MF_MEDIA_ENGINE_EVENT_BUFFERINGENDED,
            MF_MEDIA_ENGINE_EVENT_CANPLAY, MF_MEDIA_ENGINE_EVENT_CANPLAYTHROUGH,
            MF_MEDIA_ENGINE_EVENT_ENDED, MF_MEDIA_ENGINE_EVENT_ERROR,
            MF_MEDIA_ENGINE_EVENT_FIRSTFRAMEREADY, MF_MEDIA_ENGINE_EVENT_LOADEDDATA,
            MF_MEDIA_ENGINE_EVENT_PLAYING, MF_MEDIA_ENGINE_EVENT_SEEKED,
            MF_MEDIA_ENGINE_EVENT_TRACKSCHANGE, MF_MEDIA_ENGINE_VIDEO_OUTPUT_FORMAT,
            MF_MT_AUDIO_NUM_CHANNELS, MF_MT_AUDIO_SAMPLES_PER_SECOND, MF_MT_AVG_BITRATE,
            MF_MT_MAJOR_TYPE, MF_MT_SUBTYPE, MF_SD_LANGUAGE, MF_SD_STREAM_NAME,
            MF_TIMED_TEXT_CUE_EVENT, MF_TIMED_TEXT_CUE_EVENT_ACTIVE, MF_VERSION, MFARGB,
            MFCreateAttributes, MFMediaType_Audio, MFMediaType_Subtitle, MFMediaType_Video,
            MFSTARTUP_FULL, MFShutdown, MFStartup, MFVideoNormalizedRect,
        },
        System::Com::{
            CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx,
            CoTaskMemFree, CoUninitialize,
        },
        System::Variant::VT_CLSID,
    },
    core::{BSTR, GUID, Interface, PWSTR, implement},
};

use gpui_media::{
    AudioStreamInfo, FrameExtractionSession, FrameExtractorBackendRequest, MediaBackendEvent,
    MediaCapabilities, MediaError, MediaErrorKind, MediaInfo, MediaOutputSink,
    MediaPlaybackRequest, MediaPlaybackSession, MediaRecovery, MediaResult, MediaStreamId,
    PlaybackTimeline, SeekMode, SubtitleCue, SubtitleEvent, SubtitleFormat, SubtitleStreamInfo,
    VideoFrame, VideoStreamInfo,
};

const FRAME_POLL_INTERVAL: Duration = Duration::from_millis(5);

pub(super) fn initialize() -> MediaResult<()> {
    Ok(())
}

pub(super) fn open_playback(
    request: MediaPlaybackRequest,
    output: MediaOutputSink,
) -> MediaResult<Box<dyn MediaPlaybackSession>> {
    WindowsPlayback::new(request, output)
        .map(|playback| Box::new(playback) as Box<dyn MediaPlaybackSession>)
}

pub(super) fn open_frame_extractor(
    request: FrameExtractorBackendRequest,
) -> MediaResult<Box<dyn FrameExtractionSession>> {
    WindowsFrameExtractor::new(request)
        .map(|extractor| Box::new(extractor) as Box<dyn FrameExtractionSession>)
}

enum Command {
    Play(Sender<MediaResult<()>>),
    Pause(Sender<MediaResult<()>>),
    Reload {
        autoplay: bool,
        response: Sender<MediaResult<()>>,
    },
    Timeline(Sender<PlaybackTimeline>),
    Seek {
        position: Duration,
        response: Sender<MediaResult<()>>,
    },
    Step {
        frames: u64,
        response: Sender<MediaResult<()>>,
    },
    Rate {
        rate: f64,
        response: Sender<MediaResult<()>>,
    },
    Volume(f64),
    Muted(bool),
    SelectAudio {
        id: MediaStreamId,
        response: Sender<MediaResult<()>>,
    },
    SelectSubtitle {
        id: Option<MediaStreamId>,
        response: Sender<MediaResult<()>>,
    },
    Shutdown,
}

struct WindowsPlayback {
    commands: Sender<Command>,
    worker: Option<JoinHandle<()>>,
    media_info: Arc<RwLock<Option<Arc<MediaInfo>>>>,
    subtitles: bool,
}

impl WindowsPlayback {
    fn new(request: MediaPlaybackRequest, output: MediaOutputSink) -> MediaResult<Self> {
        reject_unsupported_network_options(&request)?;
        let (commands, command_rx) = mpsc::channel();
        let (initialized_tx, initialized_rx) = mpsc::sync_channel(1);
        let source_uri = request.source.uri().to_owned();
        let media_info = Arc::new(RwLock::new(None));
        let media_info_for_worker = media_info.clone();
        let worker = std::thread::Builder::new()
            .name("gpui-video-media-foundation".into())
            .spawn(move || {
                let result = MediaFoundationWorker::new(&source_uri, output, media_info_for_worker);
                match result {
                    Ok(mut worker) => {
                        let subtitles = worker.timed_text.is_some();
                        let _ = initialized_tx.send(Ok(subtitles));
                        worker.run(command_rx);
                    }
                    Err(error) => {
                        let _ = initialized_tx.send(Err(error));
                    }
                }
            })
            .map_err(|error| MediaError::io("failed to start Media Foundation worker", error))?;

        let subtitles = initialized_rx.recv().map_err(|error| {
            worker_channel_error(
                "Media Foundation worker stopped during initialization",
                error,
            )
        })??;

        Ok(Self {
            commands,
            worker: Some(worker),
            media_info,
            subtitles,
        })
    }

    fn request(&self, command: impl FnOnce(Sender<MediaResult<()>>) -> Command) -> MediaResult<()> {
        let (response_tx, response_rx) = mpsc::channel();
        self.commands
            .send(command(response_tx))
            .map_err(|error| worker_channel_error("Media Foundation worker stopped", error))?;
        response_rx
            .recv()
            .map_err(|error| worker_channel_error("Media Foundation worker stopped", error))?
    }
}

impl Drop for WindowsPlayback {
    fn drop(&mut self) {
        let _ = self.commands.send(Command::Shutdown);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl MediaPlaybackSession for WindowsPlayback {
    fn capabilities(&self) -> MediaCapabilities {
        MediaCapabilities {
            video: true,
            audio: true,
            subtitles: self.subtitles,
            seeking: true,
            accurate_seeking: true,
            playback_rate: true,
            frame_stepping: true,
            frame_extraction: true,
            transport_switching: false,
        }
    }

    fn play(&mut self) -> MediaResult<()> {
        self.request(Command::Play)
    }

    fn pause(&mut self) -> MediaResult<()> {
        self.request(Command::Pause)
    }

    fn reload(&mut self, autoplay: bool) -> MediaResult<()> {
        self.request(|response| Command::Reload { autoplay, response })
    }

    fn timeline(&self) -> PlaybackTimeline {
        let (response_tx, response_rx) = mpsc::channel();
        if self.commands.send(Command::Timeline(response_tx)).is_err() {
            return PlaybackTimeline::default();
        }
        response_rx.recv().unwrap_or_default()
    }

    fn seek_to(&mut self, position: Duration, _mode: SeekMode) -> MediaResult<()> {
        self.request(|response| Command::Seek { position, response })
    }

    fn step_forward(&mut self, frames: u64) -> MediaResult<()> {
        self.request(|response| Command::Step { frames, response })
    }

    fn set_playback_rate(&mut self, rate: f64) -> MediaResult<()> {
        if !rate.is_finite() || rate <= 0.0 {
            return Err(MediaError::invalid_input(
                "playback rate must be finite and greater than zero",
            ));
        }
        self.request(|response| Command::Rate { rate, response })
    }

    fn set_volume(&mut self, volume: f64) {
        let volume = if volume.is_finite() {
            volume.clamp(0.0, 1.0)
        } else {
            1.0
        };
        let _ = self.commands.send(Command::Volume(volume));
    }

    fn set_muted(&mut self, muted: bool) {
        let _ = self.commands.send(Command::Muted(muted));
    }

    fn media_info(&self) -> Option<Arc<MediaInfo>> {
        self.media_info.read().ok().and_then(|info| info.clone())
    }

    fn select_audio_stream(&mut self, id: &MediaStreamId) -> MediaResult<()> {
        self.request(|response| Command::SelectAudio {
            id: id.clone(),
            response,
        })
    }

    fn select_subtitle_stream(&mut self, id: Option<&MediaStreamId>) -> MediaResult<()> {
        self.request(|response| Command::SelectSubtitle {
            id: id.cloned(),
            response,
        })
    }
}

#[derive(Clone, Copy)]
struct NativeEvent {
    event: u32,
    param1: usize,
    _param2: u32,
}

#[implement(IMFMediaEngineNotify)]
struct MediaEngineNotify {
    events: Sender<NativeEvent>,
}

#[allow(non_snake_case)]
impl IMFMediaEngineNotify_Impl for MediaEngineNotify_Impl {
    fn EventNotify(&self, event: u32, param1: usize, param2: u32) -> windows::core::Result<()> {
        let _ = self.events.send(NativeEvent {
            event,
            param1,
            _param2: param2,
        });
        Ok(())
    }
}

#[implement(IMFTimedTextNotify)]
struct TimedTextNotify {
    output: MediaOutputSink,
    tracks_changed: Sender<()>,
}

#[allow(non_snake_case)]
impl IMFTimedTextNotify_Impl for TimedTextNotify_Impl {
    fn TrackAdded(&self, _trackid: u32) {
        let _ = self.tracks_changed.send(());
    }

    fn TrackRemoved(&self, _trackid: u32) {
        let _ = self.tracks_changed.send(());
    }

    fn TrackSelected(&self, _trackid: u32, _selected: windows::core::BOOL) {
        let _ = self.tracks_changed.send(());
    }

    fn TrackReadyStateChanged(&self, _trackid: u32) {
        let _ = self.tracks_changed.send(());
    }

    fn Error(
        &self,
        errorcode: windows::Win32::Media::MediaFoundation::MF_TIMED_TEXT_ERROR_CODE,
        extendederrorcode: windows::core::HRESULT,
        sourcetrackid: u32,
    ) {
        log::warn!(
            "Media Foundation timed-text error: code={errorcode:?} extended={extendederrorcode:?} track={sourcetrackid}"
        );
    }

    fn Cue(
        &self,
        cueevent: MF_TIMED_TEXT_CUE_EVENT,
        _currenttime: f64,
        cue: windows::core::Ref<'_, IMFTimedTextCue>,
    ) {
        if cueevent != MF_TIMED_TEXT_CUE_EVENT_ACTIVE {
            return;
        }
        let Some(cue) = cue.as_ref() else {
            return;
        };
        match subtitle_cue_from_media_foundation(cue) {
            Ok(Some((stream_id, cue))) => {
                self.output
                    .emit(MediaBackendEvent::Subtitle(SubtitleEvent::Cue {
                        stream_id,
                        cue: Arc::new(cue),
                    }));
            }
            Ok(None) => {}
            Err(error) => log::warn!("failed to read Media Foundation subtitle cue: {error}"),
        }
    }

    fn Reset(&self) {
        self.output
            .emit(MediaBackendEvent::Subtitle(SubtitleEvent::Reset));
    }
}

struct MediaFoundationWorker {
    engine: IMFMediaEngine,
    imaging_factory: IWICImagingFactory,
    native_events: Receiver<NativeEvent>,
    output: MediaOutputSink,
    media_info: Arc<RwLock<Option<Arc<MediaInfo>>>>,
    timed_text: Option<IMFTimedText>,
    _timed_text_notify: Option<IMFTimedTextNotify>,
    timed_text_changes: Receiver<()>,
    bitmap: Option<(u32, u32, IWICBitmap)>,
    surface_handle: SurfaceHandle,
    sequence: u64,
    com_initialized: bool,
    mf_initialized: bool,
}

impl MediaFoundationWorker {
    fn new(
        source_uri: &str,
        output: MediaOutputSink,
        media_info: Arc<RwLock<Option<Arc<MediaInfo>>>>,
    ) -> MediaResult<Self> {
        unsafe {
            CoInitializeEx(None, COINIT_MULTITHREADED)
                .ok()
                .map_err(|error| windows_backend_error("failed to initialize COM", error))?;
            if let Err(error) = MFStartup(MF_VERSION, MFSTARTUP_FULL) {
                CoUninitialize();
                return Err(windows_backend_error(
                    "failed to initialize Media Foundation",
                    error,
                ));
            }
        }

        let result = (|| unsafe {
            let factory: IMFMediaEngineClassFactory =
                CoCreateInstance(&CLSID_MFMediaEngineClassFactory, None, CLSCTX_INPROC_SERVER)
                    .map_err(|error| {
                        windows_backend_error(
                            "failed to create Media Foundation engine factory",
                            error,
                        )
                    })?;
            let imaging_factory: IWICImagingFactory =
                CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).map_err(
                    |error| {
                        windows_backend_error("failed to create Windows Imaging factory", error)
                    },
                )?;

            let (event_tx, native_events) = mpsc::channel();
            let (timed_text_change_tx, timed_text_changes) = mpsc::channel();
            let notify: IMFMediaEngineNotify = MediaEngineNotify { events: event_tx }.into();
            let mut attributes = None;
            MFCreateAttributes(&mut attributes, 2).map_err(|error| {
                windows_backend_error("failed to create Media Foundation attributes", error)
            })?;
            let attributes = attributes
                .ok_or_else(|| MediaError::backend("Media Foundation returned no attributes"))?;
            attributes
                .SetUnknown(&MF_MEDIA_ENGINE_CALLBACK, &notify)
                .map_err(|error| {
                    windows_backend_error("failed to set Media Foundation callback", error)
                })?;
            attributes
                .SetUINT32(
                    &MF_MEDIA_ENGINE_VIDEO_OUTPUT_FORMAT,
                    DXGI_FORMAT_B8G8R8A8_UNORM.0 as u32,
                )
                .map_err(|error| {
                    windows_backend_error("failed to select Media Foundation video output", error)
                })?;

            let engine = factory.CreateInstance(0, &attributes).map_err(|error| {
                windows_backend_error("failed to create Media Foundation media engine", error)
            })?;
            let timed_text = engine.cast::<IMFTimedText>().ok();
            let timed_text_notify = if let Some(timed_text) = &timed_text {
                timed_text.SetInBandEnabled(true).map_err(|error| {
                    windows_backend_error(
                        "failed to enable Media Foundation embedded subtitles",
                        error,
                    )
                })?;
                let notify: IMFTimedTextNotify = TimedTextNotify {
                    output: output.clone(),
                    tracks_changed: timed_text_change_tx,
                }
                .into();
                timed_text.RegisterNotifications(&notify).map_err(|error| {
                    windows_backend_error(
                        "failed to register Media Foundation subtitle notifications",
                        error,
                    )
                })?;
                Some(notify)
            } else {
                None
            };
            engine.SetSource(&BSTR::from(source_uri)).map_err(|error| {
                windows_backend_error("Media Foundation rejected the media source", error)
            })?;
            engine.Load().map_err(|error| {
                windows_backend_error("failed to load the Media Foundation source", error)
            })?;

            Ok(Self {
                engine,
                imaging_factory,
                native_events,
                output,
                media_info,
                timed_text,
                _timed_text_notify: timed_text_notify,
                timed_text_changes,
                bitmap: None,
                surface_handle: SurfaceHandle::new(),
                sequence: 1,
                com_initialized: true,
                mf_initialized: true,
            })
        })();

        if result.is_err() {
            unsafe {
                let _ = MFShutdown();
                CoUninitialize();
            }
        }
        result
    }

    fn run(&mut self, commands: Receiver<Command>) {
        loop {
            match commands.recv_timeout(FRAME_POLL_INTERVAL) {
                Ok(Command::Shutdown) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Ok(command) => self.handle_command(command),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }

            while let Ok(event) = self.native_events.try_recv() {
                self.handle_native_event(event);
            }
            while self.timed_text_changes.try_recv().is_ok() {
                if let Err(error) = self.refresh_media_info() {
                    log::warn!("failed to refresh Media Foundation stream metadata: {error}");
                }
            }
            if self.output.is_closed() {
                break;
            }
            match self.copy_current_frame() {
                Ok(Some(frame)) => {
                    self.output.publish_video_frame(Arc::new(frame));
                }
                Ok(None) => {}
                Err(error) => {
                    self.output.emit(MediaBackendEvent::Error(Arc::new(error)));
                    break;
                }
            }
        }
    }

    fn handle_command(&mut self, command: Command) {
        match command {
            Command::Play(response) => {
                let _ = response.send(unsafe { self.engine.Play() }.map_err(|error| {
                    windows_backend_error("failed to start Media Foundation playback", error)
                }));
            }
            Command::Pause(response) => {
                let _ = response.send(unsafe { self.engine.Pause() }.map_err(|error| {
                    windows_backend_error("failed to pause Media Foundation playback", error)
                }));
            }
            Command::Reload { autoplay, response } => {
                let result = (|| unsafe {
                    self.engine.SetCurrentTime(0.0).map_err(|error| {
                        windows_backend_error("failed to rewind Media Foundation playback", error)
                    })?;
                    if autoplay {
                        self.engine.Play()
                    } else {
                        self.engine.Pause()
                    }
                    .map_err(|error| {
                        windows_backend_error("failed to reload Media Foundation playback", error)
                    })
                })();
                let _ = response.send(result);
            }
            Command::Timeline(response) => {
                let _ = response.send(self.timeline());
            }
            Command::Seek { position, response } => {
                let result = unsafe { self.engine.SetCurrentTime(position.as_secs_f64()) }.map_err(
                    |error| {
                        windows_backend_error("failed to seek Media Foundation playback", error)
                    },
                );
                let _ = response.send(result);
            }
            Command::Step { frames, response } => {
                let result = if frames == 0 {
                    Err(MediaError::invalid_input(
                        "frame step amount must be greater than zero",
                    ))
                } else {
                    (|| unsafe {
                        self.engine.Pause().map_err(|error| {
                            windows_backend_error(
                                "failed to pause before Media Foundation frame step",
                                error,
                            )
                        })?;
                        let engine: IMFMediaEngineEx = self.engine.cast().map_err(|error| {
                            windows_backend_error(
                                "Media Foundation frame stepping is unavailable",
                                error,
                            )
                        })?;
                        for _ in 0..frames {
                            engine.FrameStep(true).map_err(|error| {
                                windows_backend_error(
                                    "Media Foundation rejected frame stepping",
                                    error,
                                )
                            })?;
                        }
                        Ok(())
                    })()
                };
                let _ = response.send(result);
            }
            Command::Rate { rate, response } => {
                let result = unsafe { self.engine.SetPlaybackRate(rate) }.map_err(|error| {
                    windows_backend_error("failed to change Media Foundation playback rate", error)
                });
                let _ = response.send(result);
            }
            Command::Volume(volume) => {
                let _ = unsafe { self.engine.SetVolume(volume) };
            }
            Command::Muted(muted) => {
                let _ = unsafe { self.engine.SetMuted(muted) };
            }
            Command::SelectAudio { id, response } => {
                let _ = response.send(self.select_audio_stream(&id));
            }
            Command::SelectSubtitle { id, response } => {
                let _ = response.send(self.select_subtitle_stream(id.as_ref()));
            }
            Command::Shutdown => {}
        }
    }

    fn timeline(&self) -> PlaybackTimeline {
        let position = unsafe { self.engine.GetCurrentTime() };
        let duration = unsafe { self.engine.GetDuration() };
        let position = seconds_duration(position).unwrap_or_default();
        let duration = seconds_duration(duration);
        let seekable = unsafe {
            self.engine
                .GetSeekable()
                .is_ok_and(|ranges| ranges.GetLength() > 0)
        };
        PlaybackTimeline::new(position, duration, seekable)
    }

    fn refresh_media_info(&mut self) -> MediaResult<()> {
        let engine: IMFMediaEngineEx = self.engine.cast().map_err(|error| {
            windows_backend_error("Media Foundation stream metadata is unavailable", error)
        })?;
        let mut info = MediaInfo::default();
        let stream_count = unsafe { engine.GetNumberOfStreams() }.map_err(|error| {
            windows_backend_error("failed to count Media Foundation streams", error)
        })?;
        for index in 0..stream_count {
            let Some(major_type) = media_foundation_stream_guid(&engine, index, &MF_MT_MAJOR_TYPE)
            else {
                continue;
            };
            let selected = unsafe { engine.GetStreamSelection(index) }
                .map(|selected| selected.as_bool())
                .unwrap_or(false);
            let id = MediaStreamId::new(format!("mf-stream-{index}"));
            let codec = media_foundation_stream_guid(&engine, index, &MF_MT_SUBTYPE)
                .map(|codec| format!("{codec:?}").into());
            let language =
                media_foundation_stream_string(&engine, index, &MF_SD_LANGUAGE).map(Into::into);
            let title =
                media_foundation_stream_string(&engine, index, &MF_SD_STREAM_NAME).map(Into::into);
            let bitrate =
                media_foundation_stream_u32(&engine, index, &MF_MT_AVG_BITRATE).map(u64::from);
            if major_type == MFMediaType_Audio {
                info.audio_streams.push(AudioStreamInfo {
                    id,
                    codec,
                    channels: media_foundation_stream_u32(
                        &engine,
                        index,
                        &MF_MT_AUDIO_NUM_CHANNELS,
                    ),
                    sample_rate: media_foundation_stream_u32(
                        &engine,
                        index,
                        &MF_MT_AUDIO_SAMPLES_PER_SECOND,
                    ),
                    bitrate,
                    language,
                    title,
                    selected,
                });
            } else if major_type == MFMediaType_Video {
                info.video_streams.push(VideoStreamInfo {
                    id,
                    codec,
                    coded_size: None,
                    display_size: None,
                    frame_rate: None,
                    bitrate,
                    language,
                    selected,
                });
            } else if major_type == MFMediaType_Subtitle {
                // Timed-text tracks below provide richer identifiers and labels.
            }
        }

        if let Some(timed_text) = &self.timed_text {
            let active_ids = unsafe { timed_text.GetActiveTracks() }
                .ok()
                .map(|tracks| timed_text_track_ids(&tracks))
                .unwrap_or_default();
            if let Ok(tracks) = unsafe { timed_text.GetTextTracks() } {
                for index in 0..unsafe { tracks.GetLength() } {
                    let Ok(track) = (unsafe { tracks.GetTrack(index) }) else {
                        continue;
                    };
                    let track_id = unsafe { track.GetId() };
                    info.subtitle_streams.push(SubtitleStreamInfo {
                        id: MediaStreamId::new(format!("mf-subtitle-{track_id}")),
                        codec: Some("timed-text".into()),
                        language: unsafe { track.GetLanguage() }
                            .ok()
                            .and_then(pwstr_to_string)
                            .map(Into::into),
                        title: unsafe { track.GetLabel() }
                            .ok()
                            .and_then(pwstr_to_string)
                            .map(Into::into),
                        forced: false,
                        selected: active_ids.contains(&track_id),
                    });
                }
            }
        }

        let info = Arc::new(info);
        if let Ok(mut current) = self.media_info.write() {
            *current = Some(info.clone());
        }
        self.output.emit(MediaBackendEvent::MediaInfoChanged(info));
        Ok(())
    }

    fn select_audio_stream(&mut self, id: &MediaStreamId) -> MediaResult<()> {
        let index = parse_media_foundation_stream_id(id, "mf-stream-")?;
        let engine: IMFMediaEngineEx = self.engine.cast().map_err(|error| {
            windows_backend_error("Media Foundation audio selection is unavailable", error)
        })?;
        let info = self
            .media_info
            .read()
            .ok()
            .and_then(|info| info.clone())
            .ok_or_else(|| MediaError::unsupported("audio streams are not available yet"))?;
        if !info.audio_streams.iter().any(|stream| &stream.id == id) {
            return Err(MediaError::invalid_input(format!(
                "unknown audio stream {id}"
            )));
        }
        for stream in &info.audio_streams {
            let stream_index = parse_media_foundation_stream_id(&stream.id, "mf-stream-")?;
            unsafe { engine.SetStreamSelection(stream_index, stream_index == index) }.map_err(
                |error| windows_backend_error("failed to select Media Foundation audio", error),
            )?;
        }
        unsafe { engine.ApplyStreamSelections() }.map_err(|error| {
            windows_backend_error("failed to apply Media Foundation audio selection", error)
        })?;
        self.refresh_media_info()
    }

    fn select_subtitle_stream(&mut self, id: Option<&MediaStreamId>) -> MediaResult<()> {
        let timed_text = self
            .timed_text
            .as_ref()
            .ok_or_else(|| MediaError::unsupported("Media Foundation timed text is unavailable"))?;
        let selected_id = id
            .map(|id| parse_media_foundation_stream_id(id, "mf-subtitle-"))
            .transpose()?;
        let tracks = unsafe { timed_text.GetTextTracks() }.map_err(|error| {
            windows_backend_error("failed to enumerate Media Foundation subtitles", error)
        })?;
        let mut found = selected_id.is_none();
        for index in 0..unsafe { tracks.GetLength() } {
            let track = unsafe { tracks.GetTrack(index) }.map_err(|error| {
                windows_backend_error("failed to read Media Foundation subtitle track", error)
            })?;
            let track_id = unsafe { track.GetId() };
            let selected = selected_id == Some(track_id);
            found |= selected;
            unsafe { timed_text.SelectTrack(track_id, selected) }.map_err(|error| {
                windows_backend_error("failed to select Media Foundation subtitle", error)
            })?;
        }
        if !found {
            return Err(MediaError::invalid_input(format!(
                "unknown subtitle stream {}",
                id.expect("a missing selection is always valid")
            )));
        }
        self.output
            .emit(MediaBackendEvent::Subtitle(SubtitleEvent::Reset));
        self.refresh_media_info()
    }

    fn handle_native_event(&mut self, event: NativeEvent) {
        let event_code = event.event as i32;
        if [
            MF_MEDIA_ENGINE_EVENT_LOADEDDATA.0,
            MF_MEDIA_ENGINE_EVENT_TRACKSCHANGE.0,
        ]
        .contains(&event_code)
            && let Err(error) = self.refresh_media_info()
        {
            log::warn!("failed to read Media Foundation stream metadata: {error}");
        }
        if [
            MF_MEDIA_ENGINE_EVENT_LOADEDDATA.0,
            MF_MEDIA_ENGINE_EVENT_CANPLAY.0,
            MF_MEDIA_ENGINE_EVENT_CANPLAYTHROUGH.0,
            MF_MEDIA_ENGINE_EVENT_FIRSTFRAMEREADY.0,
            MF_MEDIA_ENGINE_EVENT_SEEKED.0,
        ]
        .contains(&event_code)
        {
            self.output.emit(MediaBackendEvent::Ready);
        } else if [
            MF_MEDIA_ENGINE_EVENT_PLAYING.0,
            MF_MEDIA_ENGINE_EVENT_BUFFERINGENDED.0,
        ]
        .contains(&event_code)
        {
            self.output.emit(MediaBackendEvent::Buffering(100));
        } else if event_code == MF_MEDIA_ENGINE_EVENT_ENDED.0 {
            self.output.emit(MediaBackendEvent::Ended);
        } else if event_code == MF_MEDIA_ENGINE_EVENT_ERROR.0 {
            self.output
                .emit(MediaBackendEvent::Error(Arc::new(MediaError::new(
                    MediaErrorKind::Decode,
                    format!("Media Foundation playback error {:#010x}", event.param1),
                    MediaRecovery::ReloadSource,
                ))));
        }
    }

    fn copy_current_frame(&mut self) -> MediaResult<Option<VideoFrame>> {
        if !unsafe { self.engine.HasVideo() }.as_bool() {
            return Ok(None);
        }

        let mut presentation_ticks = 0i64;
        let status = unsafe {
            (Interface::vtable(&self.engine).OnVideoStreamTick)(
                Interface::as_raw(&self.engine),
                &mut presentation_ticks,
            )
        };
        if status == S_FALSE {
            return Ok(None);
        }
        status.ok().map_err(|error| {
            windows_backend_error("failed to poll Media Foundation video", error)
        })?;

        let mut width = 0;
        let mut height = 0;
        unsafe {
            self.engine
                .GetNativeVideoSize(Some(&mut width), Some(&mut height))
        }
        .map_err(|error| {
            windows_backend_error("failed to query Media Foundation video size", error)
        })?;
        if width == 0 || height == 0 {
            return Ok(None);
        }

        let recreate = self
            .bitmap
            .as_ref()
            .is_none_or(|(current_width, current_height, _)| {
                *current_width != width || *current_height != height
            });
        if recreate {
            let bitmap = unsafe {
                self.imaging_factory.CreateBitmap(
                    width,
                    height,
                    &GUID_WICPixelFormat32bppBGRA,
                    WICBitmapCacheOnLoad,
                )
            }
            .map_err(|error| {
                windows_backend_error("failed to allocate Media Foundation output bitmap", error)
            })?;
            self.bitmap = Some((width, height, bitmap));
        }
        let bitmap = &self
            .bitmap
            .as_ref()
            .expect("Media Foundation bitmap was initialized")
            .2;

        let source = MFVideoNormalizedRect {
            left: 0.0,
            top: 0.0,
            right: 1.0,
            bottom: 1.0,
        };
        let destination = RECT {
            left: 0,
            top: 0,
            right: i32::try_from(width)
                .map_err(|_| MediaError::backend("Media Foundation width is too large"))?,
            bottom: i32::try_from(height)
                .map_err(|_| MediaError::backend("Media Foundation height is too large"))?,
        };
        let border = MFARGB::default();
        unsafe {
            self.engine
                .TransferVideoFrame(bitmap, Some(&source), &destination, Some(&border))
        }
        .map_err(|error| {
            windows_backend_error("failed to transfer Media Foundation video frame", error)
        })?;

        let lock_rect = WICRect {
            X: 0,
            Y: 0,
            Width: destination.right,
            Height: destination.bottom,
        };
        let lock =
            unsafe { bitmap.Lock(&lock_rect, WICBitmapLockRead.0 as u32) }.map_err(|error| {
                windows_backend_error("failed to lock Media Foundation output bitmap", error)
            })?;
        let stride = unsafe { lock.GetStride() }.map_err(|error| {
            windows_backend_error("failed to query Media Foundation output stride", error)
        })?;
        let mut buffer_size = 0;
        let mut buffer = std::ptr::null_mut();
        unsafe { lock.GetDataPointer(&mut buffer_size, &mut buffer) }.map_err(|error| {
            windows_backend_error("failed to map Media Foundation output bitmap", error)
        })?;
        let required = usize::try_from(stride)
            .ok()
            .and_then(|stride| stride.checked_mul(height as usize))
            .ok_or_else(|| MediaError::backend("Media Foundation frame size overflow"))?;
        if buffer.is_null() || (buffer_size as usize) < required {
            return Err(MediaError::backend(
                "Media Foundation returned an incomplete output bitmap",
            ));
        }
        let bytes = unsafe { std::slice::from_raw_parts(buffer, required) }.to_vec();
        drop(lock);

        let frame_size = size(
            DevicePixels(destination.right),
            DevicePixels(destination.bottom),
        );
        let surface = SurfaceFrame::bgra(
            self.surface_handle.clone(),
            self.sequence,
            frame_size,
            bytes,
            stride,
        )
        .map_err(|error| {
            MediaError::from_error(
                MediaErrorKind::VideoOutput,
                MediaRecovery::None,
                "GPUI rejected a Media Foundation video frame",
                error,
            )
        })?;
        self.sequence = self.sequence.wrapping_add(1);
        let timestamp = u64::try_from(presentation_ticks)
            .ok()
            .map(|ticks| Duration::from_nanos(ticks.saturating_mul(100)));
        Ok(Some(VideoFrame::new(Arc::new(surface), timestamp, None)))
    }
}

fn media_foundation_stream_guid(engine: &IMFMediaEngineEx, index: u32, key: &GUID) -> Option<GUID> {
    let value = unsafe { engine.GetStreamAttribute(index, key) }.ok()?;
    if value.vt() != VT_CLSID {
        return None;
    }
    let pointer = unsafe { value.Anonymous.Anonymous.Anonymous.puuid };
    (!pointer.is_null()).then(|| unsafe { *pointer })
}

fn media_foundation_stream_string(
    engine: &IMFMediaEngineEx,
    index: u32,
    key: &GUID,
) -> Option<String> {
    let value = unsafe { engine.GetStreamAttribute(index, key) }.ok()?;
    BSTR::try_from(&value).ok().map(|value| value.to_string())
}

fn media_foundation_stream_u32(engine: &IMFMediaEngineEx, index: u32, key: &GUID) -> Option<u32> {
    let value = unsafe { engine.GetStreamAttribute(index, key) }.ok()?;
    u32::try_from(&value).ok()
}

fn timed_text_track_ids(tracks: &IMFTimedTextTrackList) -> Vec<u32> {
    (0..unsafe { tracks.GetLength() })
        .filter_map(|index| unsafe { tracks.GetTrack(index) }.ok())
        .map(|track| unsafe { track.GetId() })
        .collect()
}

fn parse_media_foundation_stream_id(id: &MediaStreamId, prefix: &str) -> MediaResult<u32> {
    id.as_str()
        .strip_prefix(prefix)
        .and_then(|index| index.parse().ok())
        .ok_or_else(|| MediaError::invalid_input(format!("invalid media stream identifier {id}")))
}

fn pwstr_to_string(value: PWSTR) -> Option<String> {
    if value.is_null() {
        return None;
    }
    let result = unsafe { value.to_string().ok() };
    unsafe { CoTaskMemFree(Some(value.0.cast())) };
    result
}

fn subtitle_cue_from_media_foundation(
    cue: &IMFTimedTextCue,
) -> MediaResult<Option<(MediaStreamId, SubtitleCue)>> {
    let mut lines = Vec::new();
    for index in 0..unsafe { cue.GetLineCount() } {
        let line = unsafe { cue.GetLine(index) }.map_err(|error| {
            windows_backend_error("failed to read Media Foundation subtitle text", error)
        })?;
        let text = unsafe { line.GetText() }
            .ok()
            .and_then(pwstr_to_string)
            .unwrap_or_default();
        lines.push(text);
    }
    let raw = lines.join("\n");
    if raw.trim().is_empty() {
        return Ok(None);
    }
    let start = seconds_duration(unsafe { cue.GetStartTime() }).unwrap_or_default();
    let duration = seconds_duration(unsafe { cue.GetDuration() }).unwrap_or_default();
    let end = start.checked_add(duration).unwrap_or(start);
    let cue_id = unsafe { cue.GetId() };
    let track_id = unsafe { cue.GetTrackId() };
    Ok(Some((
        MediaStreamId::new(format!("mf-subtitle-{track_id}")),
        SubtitleCue {
            id: Some(format!("mf-cue-{cue_id}").into()),
            start,
            end,
            text: raw.clone().into(),
            raw: raw.into(),
            settings: Some(format!("kind={:?}", unsafe { cue.GetCueKind() }).into()),
            format: SubtitleFormat::PlainText,
        },
    )))
}

impl Drop for MediaFoundationWorker {
    fn drop(&mut self) {
        unsafe {
            let _ = self.engine.Shutdown();
            if self.mf_initialized {
                let _ = MFShutdown();
            }
            if self.com_initialized {
                CoUninitialize();
            }
        }
    }
}

struct WindowsFrameExtractor {
    playback: WindowsPlayback,
    output: gpui_media::MediaOutput,
    timeout: Duration,
}

impl WindowsFrameExtractor {
    fn new(request: FrameExtractorBackendRequest) -> MediaResult<Self> {
        let (sink, output) = MediaOutputSink::channel();
        let playback = WindowsPlayback::new(
            MediaPlaybackRequest {
                source: request.source,
                gpu_specs: None,
            },
            sink,
        )?;
        let mut extractor = Self {
            playback,
            output,
            timeout: request.timeout,
        };
        extractor.playback.set_muted(true);
        extractor.playback.pause()?;
        Ok(extractor)
    }

    fn frame(&mut self, position: Duration, mode: SeekMode) -> MediaResult<Arc<VideoFrame>> {
        while self.output.video_frames.try_recv().is_ok() {}
        while self.output.events.try_recv().is_ok() {}
        self.playback.seek_to(position, mode)?;
        self.playback.play()?;

        let deadline = Instant::now() + self.timeout;
        let mut seek_ready = false;
        loop {
            while let Ok(event) = self.output.events.try_recv() {
                match event {
                    MediaBackendEvent::Ready => seek_ready = true,
                    MediaBackendEvent::Error(error) => return Err((*error).clone()),
                    _ => {}
                }
            }
            if let Ok(frame) = self.output.video_frames.try_recv()
                && (seek_ready || frame.timestamp().is_some())
            {
                self.playback.pause()?;
                return Ok(frame);
            }
            if Instant::now() >= deadline {
                self.playback.pause()?;
                return Err(MediaError::backend(
                    "timed out waiting for Media Foundation to decode a video frame",
                ));
            }
            std::thread::sleep(FRAME_POLL_INTERVAL);
        }
    }
}

impl FrameExtractionSession for WindowsFrameExtractor {
    fn initial_frame(&mut self) -> MediaResult<Arc<VideoFrame>> {
        self.frame(Duration::ZERO, SeekMode::KeyFrame)
    }

    fn frame_at(
        &mut self,
        position: Duration,
        seek_mode: SeekMode,
    ) -> MediaResult<Arc<VideoFrame>> {
        self.frame(position, seek_mode)
    }
}

fn seconds_duration(seconds: f64) -> Option<Duration> {
    (seconds.is_finite() && seconds >= 0.0).then(|| Duration::from_secs_f64(seconds))
}

fn windows_backend_error(
    context: impl std::fmt::Display,
    error: windows::core::Error,
) -> MediaError {
    MediaError::from_error(MediaErrorKind::Backend, MediaRecovery::None, context, error)
}

fn worker_channel_error(
    context: impl std::fmt::Display,
    error: impl std::fmt::Display,
) -> MediaError {
    MediaError::from_error(
        MediaErrorKind::Backend,
        MediaRecovery::Retry,
        context,
        error,
    )
}

fn reject_unsupported_network_options(request: &MediaPlaybackRequest) -> MediaResult<()> {
    if request.source.network_options() != &Default::default() {
        return Err(MediaError::unsupported(
            "Media Foundation SystemBackend does not support custom HTTP headers or proxy options",
        ));
    }
    Ok(())
}
