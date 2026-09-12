//! GStreamer implementation used by the Linux and macOS `SystemBackend`.

use std::{
    collections::HashSet,
    sync::{
        Arc, RwLock,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread::JoinHandle,
    time::Duration,
};

use gpui_media_core::{
    ColorRange, FrameBuffer, FrameColorInfo, FrameHandle, FrameOutputCapabilities, FramePlane,
    FramePoint, FrameRect, FrameSize, PixelFormat, YuvMatrix,
};
use gst::prelude::*;

use gpui_media_core::normalize_subtitle_text;
use gpui_media_core::{
    AudioStreamInfo, FrameExtractionSession, FrameExtractorBackendRequest,
    FrameTransportPreference, MediaBackend, MediaBackendEvent, MediaCapabilities, MediaError,
    MediaErrorKind, MediaInfo, MediaOutputSink, MediaPlaybackRequest, MediaPlaybackSession,
    MediaRecovery, MediaResult, MediaSource, MediaStreamId, PlaybackTimeline, SeekMode,
    SubtitleCue, SubtitleEvent, SubtitleFormat, SubtitleStreamInfo, TransportChange, VideoFrame,
    VideoStreamInfo,
};
use network::{configure_playbin_network, configure_playbin_progressive_download};

#[cfg(target_os = "macos")]
mod core_video;
mod decoder;
#[cfg(target_os = "linux")]
mod dma_buf;
mod frame_extractor;
mod iso_bmff;
mod network;

/// Internal GStreamer system playback implementation.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct GstreamerSystemBackend;

pub(crate) fn initialize() -> MediaResult<()> {
    gst::init().map_err(|error| {
        MediaError::from_error(
            MediaErrorKind::Backend,
            MediaRecovery::None,
            "failed to initialize GStreamer",
            error,
        )
    })
}

pub(crate) struct GstreamerPlayback {
    playbin: gst::Element,
    #[cfg(target_os = "linux")]
    appsink: gst_app::AppSink,
    bus: gst::Bus,
    bus_shutdown: Arc<AtomicBool>,
    play_requested: Arc<AtomicBool>,
    bus_thread: Option<JoinHandle<()>>,
    container_duration: Option<Duration>,
    output: MediaOutputSink,
    media_info: Arc<RwLock<Option<Arc<MediaInfo>>>>,
    #[cfg(target_os = "linux")]
    using_cpu_fallback: AtomicBool,
}

impl GstreamerPlayback {
    pub fn new(
        source: &MediaSource,
        output_capabilities: Option<&FrameOutputCapabilities>,
        output: MediaOutputSink,
    ) -> MediaResult<Self> {
        initialize()?;
        let container_duration = iso_bmff::fragmented_duration(source);

        let caps = appsink_caps(output_capabilities)?;
        let appsink = gst_app::AppSink::builder()
            .caps(&caps)
            .max_buffers(2)
            .wait_on_eos(false)
            .sync(true)
            .build();
        #[cfg(feature = "v1_28")]
        appsink.set_leaky_type(gst_app::AppLeakyType::Downstream);
        #[cfg(not(feature = "v1_28"))]
        appsink.set_drop(true);

        let output_for_preroll = output.clone();
        let decoder_tracker = decoder::DecoderTracker::default();
        let decoder_for_preroll = decoder_tracker.clone();
        let decoder_for_samples = decoder_tracker.clone();
        let output_for_samples = output.clone();
        let media_info = Arc::new(RwLock::new(None));
        let selected_subtitle = Arc::new(RwLock::new(None));
        let sequence = Arc::new(AtomicU64::new(1));
        let sequence_for_preroll = sequence.clone();
        let surface_handle = FrameHandle::new();
        let surface_handle_for_preroll = surface_handle;
        #[cfg(target_os = "linux")]
        let producer_drm_device = Arc::new(std::sync::RwLock::new(None));
        #[cfg(target_os = "linux")]
        let producer_drm_device_for_preroll = producer_drm_device.clone();
        #[cfg(target_os = "linux")]
        let producer_drm_device_for_samples = producer_drm_device.clone();

        appsink.set_callbacks(
            gst_app::AppSinkCallbacks::builder()
                .propose_allocation(|_, query| {
                    add_required_allocation_metas(query);
                    true
                })
                .new_preroll(move |sink| {
                    let sample = sink.pull_preroll().map_err(|_| gst::FlowError::Eos)?;
                    publish_appsink_sample(
                        &sample,
                        decoder_for_preroll.info(sink),
                        &output_for_preroll,
                        &surface_handle_for_preroll,
                        &sequence_for_preroll,
                        #[cfg(target_os = "linux")]
                        producer_drm_device_for_preroll
                            .read()
                            .ok()
                            .and_then(|device| *device),
                    )
                })
                .new_sample(move |sink| {
                    let sample = sink.pull_sample().map_err(|_| gst::FlowError::Eos)?;
                    publish_appsink_sample(
                        &sample,
                        decoder_for_samples.info(sink),
                        &output_for_samples,
                        &surface_handle,
                        &sequence,
                        #[cfg(target_os = "linux")]
                        producer_drm_device_for_samples
                            .read()
                            .ok()
                            .and_then(|device| *device),
                    )
                })
                .build(),
        );

        let subtitle_sink = gst_app::AppSink::builder()
            .max_buffers(16)
            .wait_on_eos(false)
            .sync(true)
            .build();
        let output_for_subtitle_preroll = output.clone();
        let output_for_subtitle_samples = output.clone();
        let selected_subtitle_for_preroll = selected_subtitle.clone();
        let selected_subtitle_for_samples = selected_subtitle.clone();
        subtitle_sink.set_callbacks(
            gst_app::AppSinkCallbacks::builder()
                .new_preroll(move |sink| {
                    let sample = sink.pull_preroll().map_err(|_| gst::FlowError::Eos)?;
                    publish_subtitle_sample(
                        &sample,
                        &output_for_subtitle_preroll,
                        &selected_subtitle_for_preroll,
                    )
                })
                .new_sample(move |sink| {
                    let sample = sink.pull_sample().map_err(|_| gst::FlowError::Eos)?;
                    publish_subtitle_sample(
                        &sample,
                        &output_for_subtitle_samples,
                        &selected_subtitle_for_samples,
                    )
                })
                .build(),
        );

        let playbin = gst::ElementFactory::make("playbin3")
            .build()
            .map_err(|error| {
                gst_backend_error("GStreamer element 'playbin3' is not installed", error)
            })?;
        configure_playbin_network(&playbin, source.network_options());
        decoder_tracker.attach(&playbin);
        playbin.set_property("uri", source.uri());
        playbin.set_property("video-sink", &appsink);
        playbin.set_property("text-sink", &subtitle_sink);
        configure_playbin_progressive_download(&playbin, source.uri(), source.network_options());

        let bus = playbin
            .bus()
            .ok_or_else(|| MediaError::backend("playbin3 did not provide a message bus"))?;
        let bus_for_thread = bus.clone();
        let playbin_for_bus = playbin.clone();
        let bus_shutdown = Arc::new(AtomicBool::new(false));
        let bus_shutdown_for_thread = bus_shutdown.clone();
        let play_requested = Arc::new(AtomicBool::new(false));
        let play_requested_for_bus = play_requested.clone();
        #[cfg(target_os = "linux")]
        let producer_drm_device_for_bus = producer_drm_device;
        let output_for_bus = output.clone();
        let source_for_bus = source.clone();
        let media_info_for_bus = media_info.clone();
        let selected_subtitle_for_bus = selected_subtitle;
        let bus_thread = std::thread::Builder::new()
            .name("gpui-video-gstreamer-bus".into())
            .spawn(move || {
                while !bus_shutdown_for_thread.load(Ordering::Acquire)
                    && !output_for_bus.is_closed()
                {
                    let Some(message) =
                        bus_for_thread.timed_pop(gst::ClockTime::from_mseconds(100))
                    else {
                        continue;
                    };

                    #[cfg(target_os = "linux")]
                    if let gst::MessageView::HaveContext(have_context) = message.view()
                        && let Some(device) = drm_device_from_context(&have_context.context())
                        && let Ok(mut current_device) = producer_drm_device_for_bus.write()
                    {
                        *current_device = Some(device);
                    }

                    let event = match message.view() {
                        gst::MessageView::AsyncDone(..)
                            if message.src().is_some_and(|source| {
                                source == playbin_for_bus.upcast_ref::<gst::Object>()
                            }) =>
                        {
                            Some(MediaBackendEvent::Ready)
                        }
                        gst::MessageView::Buffering(buffering) => Some(
                            MediaBackendEvent::Buffering(buffering_percent(buffering.percent())),
                        ),
                        gst::MessageView::StreamCollection(streams) => {
                            let selected = selected_stream_ids(
                                media_info_for_bus
                                    .read()
                                    .ok()
                                    .and_then(|info| info.clone())
                                    .as_deref(),
                            );
                            let info = Arc::new(media_info_from_stream_collection(
                                &streams.stream_collection(),
                                &selected,
                            ));
                            replace_media_info(&media_info_for_bus, info.clone());
                            replace_selected_subtitle(&selected_subtitle_for_bus, &info);
                            Some(MediaBackendEvent::MediaInfoChanged(info))
                        }
                        gst::MessageView::StreamsSelected(streams) => {
                            let selected = streams
                                .streams()
                                .filter_map(|stream| stream.stream_id().map(|id| id.to_string()))
                                .collect::<HashSet<_>>();
                            let collection = streams.stream_collection();
                            let info =
                                Arc::new(media_info_from_stream_collection(&collection, &selected));
                            replace_media_info(&media_info_for_bus, info.clone());
                            replace_selected_subtitle(&selected_subtitle_for_bus, &info);
                            Some(MediaBackendEvent::MediaInfoChanged(info))
                        }
                        gst::MessageView::Eos(..) => {
                            play_requested_for_bus.store(false, Ordering::Release);
                            Some(MediaBackendEvent::Ended)
                        }
                        gst::MessageView::ClockLost(..)
                            if play_requested_for_bus.load(Ordering::Acquire) =>
                        {
                            match recover_lost_clock(&playbin_for_bus) {
                                Ok(()) => None,
                                Err(error) => {
                                    play_requested_for_bus.store(false, Ordering::Release);
                                    Some(MediaBackendEvent::Error(Arc::new(error)))
                                }
                            }
                        }
                        gst::MessageView::Error(error) => {
                            play_requested_for_bus.store(false, Ordering::Release);
                            Some(MediaBackendEvent::Error(Arc::new(
                                media_error_from_gstreamer_message(error, &source_for_bus),
                            )))
                        }
                        _ => None,
                    };

                    if let Some(event) = event
                        && !output_for_bus.emit(event)
                    {
                        break;
                    }
                }
            })
            .map_err(|error| MediaError::io("failed to start GStreamer bus thread", error))?;

        Ok(Self {
            playbin,
            #[cfg(target_os = "linux")]
            appsink,
            bus,
            bus_shutdown,
            play_requested,
            bus_thread: Some(bus_thread),
            container_duration,
            output,
            media_info,
            #[cfg(target_os = "linux")]
            using_cpu_fallback: AtomicBool::new(false),
        })
    }

    pub fn play(&self) -> MediaResult<()> {
        self.play_requested.store(true, Ordering::Release);
        let result = self
            .playbin
            .set_state(gst::State::Playing)
            .map(|_| ())
            .map_err(|error| gst_backend_error("failed to start playback", error));
        if result.is_err() {
            self.play_requested.store(false, Ordering::Release);
        }
        result
    }

    pub fn pause(&self) -> MediaResult<()> {
        self.play_requested.store(false, Ordering::Release);
        self.playbin
            .set_state(gst::State::Paused)
            .map(|_| ())
            .map_err(|error| gst_backend_error("failed to pause playback", error))
    }

    pub fn reload(&self, autoplay: bool) -> MediaResult<()> {
        self.play_requested.store(false, Ordering::Release);
        self.playbin
            .set_state(gst::State::Null)
            .map_err(|error| gst_backend_error("failed to reset playback pipeline", error))?;
        let target = if autoplay {
            gst::State::Playing
        } else {
            gst::State::Paused
        };
        let result = self
            .playbin
            .set_state(target)
            .map(|_| ())
            .map_err(|error| gst_backend_error("failed to reload playback pipeline", error));
        if result.is_ok() {
            self.play_requested.store(autoplay, Ordering::Release);
        }
        result
    }

    pub fn timeline(&self) -> PlaybackTimeline {
        let position = self
            .playbin
            .query_position::<gst::ClockTime>()
            .map(Duration::from)
            .unwrap_or_default();
        let queried_duration = self
            .playbin
            .query_duration::<gst::ClockTime>()
            .map(Duration::from);
        let duration = self.container_duration.max(queried_duration);
        let mut seeking = gst::query::Seeking::new(gst::Format::Time);
        let seekable = self.playbin.query(&mut seeking) && seeking.result().0;

        PlaybackTimeline::new(position, duration, seekable)
    }

    pub fn seek_to(&self, position: Duration, mode: SeekMode) -> MediaResult<()> {
        let position = clock_time(position)?;
        self.playbin
            .seek_simple(seek_flags(mode), position)
            .map_err(|error| gst_backend_error(format!("failed to seek to {position}"), error))
    }

    pub fn step_forward(&self, frames: u64) -> MediaResult<()> {
        if frames == 0 {
            return Err(MediaError::invalid_input(
                "frame step amount must be greater than zero",
            ));
        }
        let event = gst::event::Step::new(gst::format::Buffers::from_u64(frames), 1.0, true, false);
        if self.playbin.send_event(event) {
            Ok(())
        } else {
            Err(MediaError::unsupported(
                "the playback pipeline rejected frame stepping",
            ))
        }
    }

    pub fn set_playback_rate(&self, rate: f64) -> MediaResult<()> {
        if !rate.is_finite() || rate <= 0.0 {
            return Err(MediaError::invalid_input(
                "playback rate must be finite and greater than zero",
            ));
        }
        let position = self
            .playbin
            .query_position::<gst::ClockTime>()
            .unwrap_or(gst::ClockTime::ZERO);
        self.playbin
            .seek(
                rate,
                gst::SeekFlags::FLUSH | gst::SeekFlags::ACCURATE,
                gst::SeekType::Set,
                position,
                gst::SeekType::None,
                gst::ClockTime::NONE,
            )
            .map_err(|error| {
                gst_backend_error(format!("failed to change playback rate to {rate}"), error)
            })
    }

    pub fn set_volume(&self, volume: f64) {
        let volume = if volume.is_finite() {
            volume.clamp(0.0, 1.0)
        } else {
            1.0
        };
        self.playbin.set_property("volume", volume);
    }

    pub fn set_muted(&self, muted: bool) {
        self.playbin.set_property("mute", muted);
    }

    pub fn media_info(&self) -> Option<Arc<MediaInfo>> {
        self.media_info.read().ok().and_then(|info| info.clone())
    }

    pub fn select_audio_stream(&self, id: &MediaStreamId) -> MediaResult<()> {
        let mut info = self
            .media_info()
            .ok_or_else(|| MediaError::unsupported("audio streams are not available yet"))?;
        if !info.audio_streams.iter().any(|stream| &stream.id == id) {
            return Err(MediaError::invalid_input(format!(
                "unknown audio stream {id}"
            )));
        }
        let mut updated = (*info).clone();
        for stream in &mut updated.audio_streams {
            stream.selected = stream.id == *id;
        }
        info = Arc::new(updated);
        self.select_streams(&info)?;
        replace_media_info(&self.media_info, info.clone());
        self.output.emit(MediaBackendEvent::MediaInfoChanged(info));
        Ok(())
    }

    pub fn select_subtitle_stream(&self, id: Option<&MediaStreamId>) -> MediaResult<()> {
        let mut info = self
            .media_info()
            .ok_or_else(|| MediaError::unsupported("subtitle streams are not available yet"))?;
        if let Some(id) = id
            && !info.subtitle_streams.iter().any(|stream| &stream.id == id)
        {
            return Err(MediaError::invalid_input(format!(
                "unknown subtitle stream {id}"
            )));
        }
        let mut updated = (*info).clone();
        for stream in &mut updated.subtitle_streams {
            stream.selected = id.is_some_and(|id| stream.id == *id);
        }
        info = Arc::new(updated);
        self.select_streams(&info)?;
        replace_media_info(&self.media_info, info.clone());
        self.output.emit(MediaBackendEvent::MediaInfoChanged(info));
        Ok(())
    }

    fn select_streams(&self, info: &MediaInfo) -> MediaResult<()> {
        let stream_ids = selected_stream_ids(Some(info))
            .into_iter()
            .collect::<Vec<_>>();
        if stream_ids.is_empty() {
            return Err(MediaError::unsupported(
                "the playback pipeline has not selected any media streams",
            ));
        }
        let event = gst::event::SelectStreams::new(stream_ids.iter().map(String::as_str));
        if self.playbin.send_event(event) {
            Ok(())
        } else {
            Err(MediaError::unsupported(
                "the playback pipeline rejected stream selection",
            ))
        }
    }

    #[cfg(target_os = "linux")]
    pub fn switch_to_cpu_fallback(&self) -> MediaResult<bool> {
        if self
            .using_cpu_fallback
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Ok(false);
        }

        let result = (|| {
            let caps = cpu_appsink_caps()?;
            self.appsink.set_caps(Some(&caps));

            if let Some(sink_pad) = self.appsink.static_pad("sink") {
                sink_pad.send_event(gst::event::Reconfigure::new());
            }

            if let Some(position) = self.playbin.query_position::<gst::ClockTime>() {
                self.playbin
                    .seek_simple(gst::SeekFlags::FLUSH | gst::SeekFlags::KEY_UNIT, position)
                    .map_err(|error| {
                        gst_video_output_error(
                            format!("failed to renegotiate CPU video output at {position}"),
                            error,
                        )
                    })?;
            }
            Ok(true)
        })();

        if result.is_err() {
            self.using_cpu_fallback.store(false, Ordering::Release);
        }
        result
    }
}

fn recover_lost_clock(playbin: &gst::Element) -> MediaResult<()> {
    log::warn!("GStreamer playback clock was lost; selecting a new clock");
    playbin.set_state(gst::State::Paused).map_err(|error| {
        gst_backend_error("failed to pause after losing the playback clock", error)
    })?;
    playbin
        .set_state(gst::State::Playing)
        .map(|_| ())
        .map_err(|error| gst_backend_error("failed to resume with a new playback clock", error))
}

fn replace_media_info(target: &RwLock<Option<Arc<MediaInfo>>>, info: Arc<MediaInfo>) {
    if let Ok(mut current) = target.write() {
        *current = Some(info);
    }
}

fn replace_selected_subtitle(target: &RwLock<Option<MediaStreamId>>, info: &MediaInfo) {
    if let Ok(mut current) = target.write() {
        *current = info
            .selected_subtitle_stream()
            .map(|stream| stream.id.clone());
    }
}

fn selected_stream_ids(info: Option<&MediaInfo>) -> HashSet<String> {
    let Some(info) = info else {
        return HashSet::new();
    };
    info.video_streams
        .iter()
        .filter(|stream| stream.selected)
        .map(|stream| stream.id.to_string())
        .chain(
            info.audio_streams
                .iter()
                .filter(|stream| stream.selected)
                .map(|stream| stream.id.to_string()),
        )
        .chain(
            info.subtitle_streams
                .iter()
                .filter(|stream| stream.selected)
                .map(|stream| stream.id.to_string()),
        )
        .collect()
}

fn media_info_from_stream_collection(
    collection: &gst::StreamCollection,
    selected: &HashSet<String>,
) -> MediaInfo {
    let mut info = MediaInfo::default();
    for stream in collection.iter() {
        let Some(id) = stream.stream_id().map(|id| id.to_string()) else {
            continue;
        };
        let selected = selected.contains(&id)
            || (selected.is_empty() && stream.stream_flags().contains(gst::StreamFlags::SELECT));
        let tags = stream.tags();
        let caps = stream.caps();
        let structure = caps.as_ref().and_then(|caps| caps.structure(0));
        let codec_from_caps = structure.map(|structure| structure.name().to_string().into());
        let language = tags
            .as_ref()
            .and_then(|tags| tags.get::<gst::tags::LanguageCode>())
            .map(|tag| tag.get().to_owned().into());
        let title = tags
            .as_ref()
            .and_then(|tags| tags.get::<gst::tags::Title>())
            .map(|tag| tag.get().to_owned().into());
        let bitrate = tags
            .as_ref()
            .and_then(|tags| tags.get::<gst::tags::Bitrate>())
            .map(|tag| u64::from(tag.get()));
        let stream_type = stream.stream_type();
        if stream_type.contains(gst::StreamType::VIDEO) {
            let codec = tags
                .as_ref()
                .and_then(|tags| tags.get::<gst::tags::VideoCodec>())
                .or_else(|| {
                    tags.as_ref()
                        .and_then(|tags| tags.get::<gst::tags::Codec>())
                })
                .map(|tag| tag.get().to_owned().into())
                .or(codec_from_caps);
            let width = structure
                .and_then(|structure| structure.get::<i32>("width").ok())
                .and_then(|value| u32::try_from(value).ok());
            let height = structure
                .and_then(|structure| structure.get::<i32>("height").ok())
                .and_then(|value| u32::try_from(value).ok());
            let coded_size = width
                .zip(height)
                .map(|(width, height)| FrameSize::new(width as i32, height as i32));
            let frame_rate = structure
                .and_then(|structure| structure.get::<gst::Fraction>("framerate").ok())
                .and_then(|rate| {
                    (rate.denom() != 0).then(|| f64::from(rate.numer()) / f64::from(rate.denom()))
                });
            info.video_streams.push(VideoStreamInfo {
                id: MediaStreamId::new(id),
                codec,
                coded_size,
                display_size: coded_size,
                frame_rate,
                bitrate,
                language,
                selected,
            });
        } else if stream_type.contains(gst::StreamType::AUDIO) {
            let codec = tags
                .as_ref()
                .and_then(|tags| tags.get::<gst::tags::AudioCodec>())
                .or_else(|| {
                    tags.as_ref()
                        .and_then(|tags| tags.get::<gst::tags::Codec>())
                })
                .map(|tag| tag.get().to_owned().into())
                .or(codec_from_caps);
            let channels = structure
                .and_then(|structure| structure.get::<i32>("channels").ok())
                .and_then(|value| u32::try_from(value).ok());
            let sample_rate = structure
                .and_then(|structure| structure.get::<i32>("rate").ok())
                .and_then(|value| u32::try_from(value).ok());
            info.audio_streams.push(AudioStreamInfo {
                id: MediaStreamId::new(id),
                codec,
                channels,
                sample_rate,
                bitrate,
                language,
                title,
                selected,
            });
        } else if stream_type.contains(gst::StreamType::TEXT) {
            let codec = tags
                .as_ref()
                .and_then(|tags| tags.get::<gst::tags::SubtitleCodec>())
                .or_else(|| {
                    tags.as_ref()
                        .and_then(|tags| tags.get::<gst::tags::Codec>())
                })
                .map(|tag| tag.get().to_owned().into())
                .or(codec_from_caps);
            info.subtitle_streams.push(SubtitleStreamInfo {
                id: MediaStreamId::new(id),
                codec,
                language,
                title,
                forced: false,
                selected,
            });
        }
    }
    info
}

fn publish_subtitle_sample(
    sample: &gst::Sample,
    output: &MediaOutputSink,
    selected_subtitle: &RwLock<Option<MediaStreamId>>,
) -> std::result::Result<gst::FlowSuccess, gst::FlowError> {
    let stream_id = selected_subtitle
        .read()
        .ok()
        .and_then(|stream| stream.clone());
    let Some(stream_id) = stream_id else {
        return Ok(gst::FlowSuccess::Ok);
    };
    let buffer = sample.buffer().ok_or(gst::FlowError::Error)?;
    let mapping = buffer.map_readable().map_err(|_| gst::FlowError::Error)?;
    let raw = std::str::from_utf8(mapping.as_slice())
        .map_err(|_| gst::FlowError::Error)?
        .trim_matches('\0');
    if raw.trim().is_empty() {
        return Ok(gst::FlowSuccess::Ok);
    }
    let start = buffer.pts().map(Duration::from).unwrap_or_default();
    let end = buffer
        .duration()
        .map(Duration::from)
        .and_then(|duration| start.checked_add(duration))
        .unwrap_or(start);
    let cue = Arc::new(SubtitleCue {
        id: None,
        start,
        end,
        text: normalize_subtitle_text(raw).into(),
        raw: raw.to_owned().into(),
        settings: sample
            .caps()
            .and_then(|caps| caps.structure(0))
            .map(|structure| structure.to_string().into()),
        format: SubtitleFormat::PlainText,
    });
    output.emit(MediaBackendEvent::Subtitle(SubtitleEvent::Cue {
        stream_id,
        cue,
    }));
    Ok(gst::FlowSuccess::Ok)
}

fn publish_appsink_sample(
    sample: &gst::Sample,
    decoder_info: Option<Arc<gpui_media_core::VideoDecoderInfo>>,
    output: &MediaOutputSink,
    surface_handle: &FrameHandle,
    sequence: &AtomicU64,
    #[cfg(target_os = "linux")] producer_drm_device: Option<gpui_media_core::DrmDevice>,
) -> std::result::Result<gst::FlowSuccess, gst::FlowError> {
    let next_sequence = sequence.fetch_add(1, Ordering::Relaxed);
    let frame = sample_to_video_frame(
        sample,
        *surface_handle,
        next_sequence,
        #[cfg(target_os = "linux")]
        producer_drm_device,
    )
    .map_err(|error| {
        log::warn!("discarded decoded video frame: {error:#}");
        gst::FlowError::Error
    })?;

    let frame = match decoder_info {
        Some(info) => frame.with_decoder_info(info),
        None => frame,
    };
    output.publish_video_frame(Arc::new(frame));
    Ok(gst::FlowSuccess::Ok)
}

impl MediaBackend for GstreamerSystemBackend {
    fn name(&self) -> &'static str {
        "gstreamer"
    }

    fn open_playback(
        &self,
        request: MediaPlaybackRequest,
        output: MediaOutputSink,
    ) -> MediaResult<Box<dyn MediaPlaybackSession>> {
        GstreamerPlayback::new(
            &request.source,
            request.output_capabilities.as_ref(),
            output,
        )
        .map(|playback| Box::new(playback) as Box<dyn MediaPlaybackSession>)
        .map_err(|mut error| {
            error.message = request.source.redact_error_message(&error.message).into();
            error
        })
    }

    fn open_frame_extractor(
        &self,
        request: FrameExtractorBackendRequest,
    ) -> MediaResult<Box<dyn FrameExtractionSession>> {
        frame_extractor::GstreamerFrameExtractionSession::new(&request.source, request.timeout)
            .map(|session| Box::new(session) as Box<dyn FrameExtractionSession>)
            .map_err(|mut error| {
                error.message = request.source.redact_error_message(&error.message).into();
                error
            })
    }
}

impl MediaPlaybackSession for GstreamerPlayback {
    fn capabilities(&self) -> MediaCapabilities {
        MediaCapabilities {
            video: true,
            audio: true,
            subtitles: true,
            seeking: true,
            accurate_seeking: true,
            playback_rate: true,
            frame_stepping: true,
            frame_extraction: true,
            transport_switching: cfg!(target_os = "linux"),
        }
    }

    fn play(&mut self) -> MediaResult<()> {
        GstreamerPlayback::play(self)
    }

    fn pause(&mut self) -> MediaResult<()> {
        GstreamerPlayback::pause(self)
    }

    fn reload(&mut self, autoplay: bool) -> MediaResult<()> {
        GstreamerPlayback::reload(self, autoplay)
    }

    fn timeline(&self) -> PlaybackTimeline {
        GstreamerPlayback::timeline(self)
    }

    fn seek_to(&mut self, position: Duration, mode: SeekMode) -> MediaResult<()> {
        GstreamerPlayback::seek_to(self, position, mode)
    }

    fn step_forward(&mut self, frames: u64) -> MediaResult<()> {
        GstreamerPlayback::step_forward(self, frames)
    }

    fn set_playback_rate(&mut self, rate: f64) -> MediaResult<()> {
        GstreamerPlayback::set_playback_rate(self, rate)
    }

    fn set_volume(&mut self, volume: f64) {
        GstreamerPlayback::set_volume(self, volume);
    }

    fn set_muted(&mut self, muted: bool) {
        GstreamerPlayback::set_muted(self, muted);
    }

    fn media_info(&self) -> Option<Arc<MediaInfo>> {
        GstreamerPlayback::media_info(self)
    }

    fn select_audio_stream(&mut self, id: &MediaStreamId) -> MediaResult<()> {
        GstreamerPlayback::select_audio_stream(self, id)
    }

    fn select_subtitle_stream(&mut self, id: Option<&MediaStreamId>) -> MediaResult<()> {
        GstreamerPlayback::select_subtitle_stream(self, id)
    }

    fn set_frame_transport_preference(
        &mut self,
        preference: FrameTransportPreference,
    ) -> MediaResult<TransportChange> {
        match preference {
            FrameTransportPreference::Auto | FrameTransportPreference::PreferNative => {
                Ok(TransportChange::Unchanged)
            }
            FrameTransportPreference::CpuOnly => {
                #[cfg(target_os = "linux")]
                {
                    self.switch_to_cpu_fallback().map(|changed| {
                        if changed {
                            TransportChange::Reconfigured
                        } else {
                            TransportChange::Unchanged
                        }
                    })
                }

                #[cfg(not(target_os = "linux"))]
                {
                    Ok(TransportChange::Unchanged)
                }
            }
        }
    }
}

fn gst_backend_error(context: impl std::fmt::Display, error: impl std::fmt::Debug) -> MediaError {
    MediaError::new(
        MediaErrorKind::Backend,
        format!("{context}: {error:?}"),
        MediaRecovery::None,
    )
}

fn gst_video_output_error(
    context: impl std::fmt::Display,
    error: impl std::fmt::Debug,
) -> MediaError {
    MediaError::new(
        MediaErrorKind::VideoOutput,
        format!("{context}: {error:?}"),
        MediaRecovery::Retry,
    )
}

#[cfg(target_os = "linux")]
fn gst_video_output_message(message: impl Into<std::sync::Arc<str>>) -> MediaError {
    MediaError::new(MediaErrorKind::VideoOutput, message, MediaRecovery::Retry)
}

fn gst_decode_error(context: impl std::fmt::Display, error: impl std::fmt::Debug) -> MediaError {
    MediaError::new(
        MediaErrorKind::Decode,
        format!("{context}: {error:?}"),
        MediaRecovery::None,
    )
}

fn gst_decode_message(message: impl Into<std::sync::Arc<str>>) -> MediaError {
    MediaError::new(MediaErrorKind::Decode, message, MediaRecovery::None)
}

fn media_error_from_gstreamer_message(
    error: &gst::message::Error,
    source: &MediaSource,
) -> MediaError {
    let glib_error = error.error();
    let status = error.details().and_then(|details| {
        ["http-status-code", "status-code"]
            .into_iter()
            .find_map(|name| {
                details
                    .get::<i32>(name)
                    .ok()
                    .and_then(|value| u16::try_from(value).ok())
            })
    });
    let (kind, recovery) = if matches!(status, Some(401 | 403)) {
        (MediaErrorKind::Authentication, MediaRecovery::ReloadSource)
    } else if status == Some(404) {
        (MediaErrorKind::SourceNotFound, MediaRecovery::None)
    } else if let Some(status) = status {
        (
            MediaErrorKind::Network {
                status: Some(status),
            },
            if status == 408 || status == 429 || status >= 500 {
                MediaRecovery::Retry
            } else {
                MediaRecovery::None
            },
        )
    } else if let Some(resource) = glib_error.kind::<gst::ResourceError>() {
        match resource {
            gst::ResourceError::NotAuthorized => {
                (MediaErrorKind::Authentication, MediaRecovery::ReloadSource)
            }
            gst::ResourceError::NotFound => (MediaErrorKind::SourceNotFound, MediaRecovery::None),
            gst::ResourceError::OpenRead | gst::ResourceError::Read | gst::ResourceError::Busy
                if source.uri().starts_with("http://") || source.uri().starts_with("https://") =>
            {
                (
                    MediaErrorKind::Network { status: None },
                    MediaRecovery::Retry,
                )
            }
            _ => (MediaErrorKind::Backend, MediaRecovery::Retry),
        }
    } else if let Some(stream) = glib_error.kind::<gst::StreamError>() {
        match stream {
            gst::StreamError::CodecNotFound => {
                (MediaErrorKind::UnsupportedCodec, MediaRecovery::None)
            }
            gst::StreamError::TypeNotFound
            | gst::StreamError::WrongType
            | gst::StreamError::Demux
            | gst::StreamError::Format => {
                (MediaErrorKind::UnsupportedContainer, MediaRecovery::None)
            }
            gst::StreamError::Decode => (MediaErrorKind::Decode, MediaRecovery::None),
            _ => (MediaErrorKind::Backend, MediaRecovery::None),
        }
    } else {
        let element_name = error
            .src()
            .and_then(|source| source.downcast_ref::<gst::Element>())
            .map(|element| element.name().to_ascii_lowercase());
        match element_name.as_deref() {
            Some(name) if name.contains("audio") && name.contains("sink") => {
                (MediaErrorKind::AudioOutput, MediaRecovery::Retry)
            }
            Some(name) if name.contains("video") && name.contains("sink") => {
                (MediaErrorKind::VideoOutput, MediaRecovery::Retry)
            }
            _ => (MediaErrorKind::Backend, MediaRecovery::Retry),
        }
    };

    let mut message = glib_error.to_string();
    if let Some(debug) = error.debug() {
        message.push_str(": ");
        message.push_str(&debug);
    }
    MediaError::new(kind, source.redact_error_message(&message), recovery)
}

fn buffering_percent(percent: i32) -> u8 {
    percent.clamp(0, 100) as u8
}

impl Drop for GstreamerPlayback {
    fn drop(&mut self) {
        self.play_requested.store(false, Ordering::Release);
        self.bus_shutdown.store(true, Ordering::Release);
        self.bus.set_flushing(true);
        let _ = self.playbin.set_state(gst::State::Null);
        if let Some(thread) = self.bus_thread.take() {
            let _ = thread.join();
        }
    }
}

pub(crate) fn add_required_allocation_metas(query: &mut gst::query::Allocation) {
    if query
        .find_allocation_meta::<gst_video::VideoMeta>()
        .is_none()
    {
        query.add_allocation_meta::<gst_video::VideoMeta>(None);
    }
}

pub(crate) fn seek_flags(mode: SeekMode) -> gst::SeekFlags {
    match mode {
        SeekMode::Accurate => gst::SeekFlags::FLUSH | gst::SeekFlags::ACCURATE,
        SeekMode::Interactive | SeekMode::KeyFrame => {
            gst::SeekFlags::FLUSH | gst::SeekFlags::KEY_UNIT
        }
    }
}

pub(crate) fn clock_time(duration: Duration) -> MediaResult<gst::ClockTime> {
    gst::ClockTime::try_from(duration).map_err(|error| {
        MediaError::from_error(
            MediaErrorKind::InvalidInput,
            MediaRecovery::None,
            "media timestamp exceeds the GStreamer time range",
            error,
        )
    })
}

pub(crate) fn appsink_caps(
    _output_capabilities: Option<&FrameOutputCapabilities>,
) -> MediaResult<gst::Caps> {
    #[cfg(target_os = "linux")]
    {
        dma_buf::appsink_caps(_output_capabilities)
    }

    #[cfg(not(target_os = "linux"))]
    {
        "video/x-raw,format=(string){NV12,BGRA,RGBA}"
            .parse::<gst::Caps>()
            .map_err(|error| gst_video_output_error("failed to construct appsink caps", error))
    }
}

#[cfg(target_os = "linux")]
fn cpu_appsink_caps() -> MediaResult<gst::Caps> {
    "video/x-raw,format=(string){NV12,BGRA,RGBA}"
        .parse::<gst::Caps>()
        .map_err(|error| gst_video_output_error("failed to construct CPU appsink caps", error))
}

fn sample_to_surface_frame(
    sample: &gst::Sample,
    handle: FrameHandle,
    sequence: u64,
    #[cfg(target_os = "linux")] producer_drm_device: Option<gpui_media_core::DrmDevice>,
) -> MediaResult<FrameBuffer> {
    #[cfg(target_os = "macos")]
    if let Some(frame) = core_video::sample_to_surface_frame(sample, handle, sequence)? {
        return Ok(frame);
    }

    #[cfg(target_os = "linux")]
    if dma_buf::sample_uses_dma_buf(sample) {
        return dma_buf::sample_to_surface_frame(sample, handle, sequence, producer_drm_device);
    }

    sample_to_cpu_surface_frame(sample, handle, sequence)
}

pub(crate) fn sample_to_video_frame(
    sample: &gst::Sample,
    handle: FrameHandle,
    sequence: u64,
    #[cfg(target_os = "linux")] producer_drm_device: Option<gpui_media_core::DrmDevice>,
) -> MediaResult<VideoFrame> {
    let buffer = sample
        .buffer()
        .ok_or_else(|| gst_decode_message("decoded sample has no buffer"))?;
    let timestamp = buffer.pts().map(Duration::from);
    let duration = buffer.duration().map(Duration::from);
    let surface = Arc::new(sample_to_surface_frame(
        sample,
        handle,
        sequence,
        #[cfg(target_os = "linux")]
        producer_drm_device,
    )?);

    Ok(VideoFrame::new(surface, timestamp, duration))
}

#[cfg(target_os = "linux")]
fn drm_device_from_context(context: &gst::Context) -> Option<gpui_media_core::DrmDevice> {
    use std::os::unix::fs::MetadataExt as _;

    let path = context.structure().get::<String>("path").ok()?;
    let metadata = std::fs::metadata(path).ok()?;
    let device = metadata.rdev();
    Some(gpui_media_core::DrmDevice {
        major: linux_device_major(device),
        minor: linux_device_minor(device),
    })
}

#[cfg(target_os = "linux")]
fn linux_device_major(device: u64) -> u32 {
    (((device >> 8) & 0x0000_0fff) | ((device >> 32) & 0xffff_f000)) as u32
}

#[cfg(target_os = "linux")]
fn linux_device_minor(device: u64) -> u32 {
    ((device & 0x0000_00ff) | ((device >> 12) & 0xffff_ff00)) as u32
}

fn sample_to_cpu_surface_frame(
    sample: &gst::Sample,
    handle: FrameHandle,
    sequence: u64,
) -> MediaResult<FrameBuffer> {
    let caps = sample
        .caps()
        .ok_or_else(|| gst_decode_message("decoded sample has no caps"))?;
    let info = gst_video::VideoInfo::from_caps(caps)
        .map_err(|error| gst_decode_error("invalid decoded video caps", error))?;
    let buffer = sample
        .buffer_owned()
        .ok_or_else(|| gst_decode_message("decoded sample has no buffer"))?;
    let (coded_size, visible_rect, display_size) = video_frame_geometry(buffer.as_ref(), &info)?;
    let frame = gst_video::VideoFrame::from_buffer_readable(buffer, &info)
        .map_err(|_| gst_decode_message("failed to map decoded video buffer"))?;

    let stride = |plane: usize| -> MediaResult<u32> {
        let stride = *info
            .stride()
            .get(plane)
            .ok_or_else(|| gst_decode_message("decoded video plane has no stride"))?;
        if stride <= 0 {
            return Err(gst_decode_message(format!(
                "negative decoded video stride is not supported: {stride}"
            )));
        }
        Ok(stride as u32)
    };

    match info.format() {
        gst_video::VideoFormat::Bgra => FrameBuffer::new(
            handle,
            sequence,
            coded_size,
            visible_rect,
            display_size,
            PixelFormat::Bgra8,
            [FramePlane::new(
                frame
                    .plane_data(0)
                    .map_err(|error| gst_decode_error("failed to read BGRA plane", error))?
                    .to_vec(),
                stride(0)?,
            )],
            FrameColorInfo::default(),
        )
        .map_err(|error| gst_video_output_error("invalid BGRA frame", error)),
        gst_video::VideoFormat::Rgba => FrameBuffer::new(
            handle,
            sequence,
            coded_size,
            visible_rect,
            display_size,
            PixelFormat::Rgba8,
            [FramePlane::new(
                frame
                    .plane_data(0)
                    .map_err(|error| gst_decode_error("failed to read RGBA plane", error))?
                    .to_vec(),
                stride(0)?,
            )],
            FrameColorInfo::default(),
        )
        .map_err(|error| gst_video_output_error("invalid RGBA frame", error)),
        gst_video::VideoFormat::Nv12 => FrameBuffer::new(
            handle,
            sequence,
            coded_size,
            visible_rect,
            display_size,
            PixelFormat::Nv12,
            [
                FramePlane::new(
                    frame
                        .plane_data(0)
                        .map_err(|error| gst_decode_error("failed to read NV12 Y plane", error))?
                        .to_vec(),
                    stride(0)?,
                ),
                FramePlane::new(
                    frame
                        .plane_data(1)
                        .map_err(|error| gst_decode_error("failed to read NV12 UV plane", error))?
                        .to_vec(),
                    stride(1)?,
                ),
            ],
            surface_color_info(&info),
        )
        .map_err(|error| gst_video_output_error("invalid NV12 frame", error)),
        format => Err(MediaError::new(
            MediaErrorKind::UnsupportedCodec,
            format!("unsupported decoded video format: {format:?}"),
            MediaRecovery::None,
        )),
    }
}

pub(super) fn video_frame_geometry(
    buffer: &gst::BufferRef,
    info: &gst_video::VideoInfo,
) -> MediaResult<(FrameSize, FrameRect, FrameSize)> {
    let coded_size = FrameSize::new(
        i32::try_from(info.width())
            .map_err(|error| gst_decode_error("video width is too large", error))?,
        i32::try_from(info.height())
            .map_err(|error| gst_decode_error("video height is too large", error))?,
    );
    let (x, y, width, height) = buffer
        .meta::<gst_video::VideoCropMeta>()
        .map(|crop| crop.rect())
        .unwrap_or((0, 0, info.width(), info.height()));
    let visible_rect = FrameRect {
        origin: FramePoint::new(
            i32::try_from(x)
                .map_err(|error| gst_decode_error("video crop x is too large", error))?,
            i32::try_from(y)
                .map_err(|error| gst_decode_error("video crop y is too large", error))?,
        ),
        size: FrameSize::new(
            i32::try_from(width)
                .map_err(|error| gst_decode_error("video crop width is too large", error))?,
            i32::try_from(height)
                .map_err(|error| gst_decode_error("video crop height is too large", error))?,
        ),
    };

    let pixel_aspect_ratio = info.par();
    let (numerator, denominator) = match (pixel_aspect_ratio.numer(), pixel_aspect_ratio.denom()) {
        (numerator, denominator) if numerator > 0 && denominator > 0 => {
            (numerator as u64, denominator as u64)
        }
        _ => (1, 1),
    };
    let display_width = u64::from(width)
        .checked_mul(numerator)
        .and_then(|scaled| scaled.checked_add(denominator / 2))
        .map(|scaled| scaled / denominator)
        .ok_or_else(|| gst_decode_message("video display width overflow"))?;
    let display_size = FrameSize::new(
        i32::try_from(display_width)
            .map_err(|error| gst_decode_error("video display width is too large", error))?,
        i32::try_from(height)
            .map_err(|error| gst_decode_error("video display height is too large", error))?,
    );

    Ok((coded_size, visible_rect, display_size))
}

fn surface_color_info(info: &gst_video::VideoInfo) -> FrameColorInfo {
    let colorimetry = info.colorimetry();
    let matrix = match colorimetry.matrix() {
        gst_video::VideoColorMatrix::Bt601 => YuvMatrix::Bt601,
        gst_video::VideoColorMatrix::Bt709 => YuvMatrix::Bt709,
        _ if info.height() <= 576 => YuvMatrix::Bt601,
        _ => YuvMatrix::Bt709,
    };
    let range = match colorimetry.range() {
        gst_video::VideoColorRange::Range0_255 => ColorRange::Full,
        _ => ColorRange::Limited,
    };

    FrameColorInfo { matrix, range }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    #[cfg(target_os = "linux")]
    use std::fs::File;

    use gpui_media_core::{FrameBacking, FrameHandle};

    use gpui_media_core::{MediaErrorKind, MediaRecovery, MediaSource, NetworkSourceOptions};

    use super::{
        add_required_allocation_metas, appsink_caps, buffering_percent,
        media_error_from_gstreamer_message, media_info_from_stream_collection,
        sample_to_surface_frame, video_frame_geometry,
    };

    #[test]
    fn buffering_percentage_is_clamped() {
        assert_eq!(buffering_percent(-1), 0);
        assert_eq!(buffering_percent(42), 42);
        assert_eq!(buffering_percent(101), 100);
    }

    #[test]
    fn stream_collection_maps_audio_and_subtitle_metadata() {
        super::initialize().unwrap();
        let audio_caps = "audio/x-raw,channels=6,rate=48000"
            .parse::<gst::Caps>()
            .unwrap();
        let audio = gst::Stream::new(
            Some("audio-en"),
            Some(&audio_caps),
            gst::StreamType::AUDIO,
            gst::StreamFlags::SELECT,
        );
        let mut audio_tags = gst::TagList::new();
        {
            let tags = audio_tags.get_mut().unwrap();
            tags.add::<gst::tags::LanguageCode>(&"en", gst::TagMergeMode::Replace);
            tags.add::<gst::tags::Title>(&"English 5.1", gst::TagMergeMode::Replace);
            tags.add::<gst::tags::AudioCodec>(&"PCM", gst::TagMergeMode::Replace);
        }
        audio.set_tags(Some(&audio_tags));

        let subtitle_caps = "text/x-raw,format=utf8".parse::<gst::Caps>().unwrap();
        let subtitle = gst::Stream::new(
            Some("subtitle-zh"),
            Some(&subtitle_caps),
            gst::StreamType::TEXT,
            gst::StreamFlags::empty(),
        );
        let mut subtitle_tags = gst::TagList::new();
        subtitle_tags
            .get_mut()
            .unwrap()
            .add::<gst::tags::LanguageCode>(&"zh", gst::TagMergeMode::Replace);
        subtitle.set_tags(Some(&subtitle_tags));

        let collection = gst::StreamCollection::builder(Some("test"))
            .streams([audio, subtitle])
            .build();
        let info = media_info_from_stream_collection(&collection, &HashSet::new());

        assert_eq!(info.audio_streams.len(), 1);
        assert_eq!(info.audio_streams[0].id.as_str(), "audio-en");
        assert_eq!(info.audio_streams[0].channels, Some(6));
        assert_eq!(info.audio_streams[0].sample_rate, Some(48_000));
        assert_eq!(info.audio_streams[0].language.as_deref(), Some("en"));
        assert_eq!(info.audio_streams[0].title.as_deref(), Some("English 5.1"));
        assert!(info.audio_streams[0].selected);
        assert_eq!(info.subtitle_streams.len(), 1);
        assert_eq!(info.subtitle_streams[0].id.as_str(), "subtitle-zh");
        assert_eq!(info.subtitle_streams[0].language.as_deref(), Some("zh"));
    }

    #[test]
    fn classifies_http_authentication_and_redacts_diagnostics() {
        super::initialize().unwrap();
        let source = MediaSource::from_uri("https://example.com/video?token=signed-secret")
            .unwrap()
            .with_network_options(
                NetworkSourceOptions::default()
                    .with_bearer_token("bearer-secret")
                    .unwrap(),
            );
        let details = gst::Structure::builder("http-error-details")
            .field("http-status-code", 401i32)
            .build();
        let message = gst::message::Error::builder(
            gst::ResourceError::NotAuthorized,
            "authorization failed",
        )
        .debug("request https://example.com/video?token=signed-secret used Bearer bearer-secret")
        .details(details)
        .build();
        let gst::MessageView::Error(error) = message.view() else {
            panic!("expected a GStreamer error message");
        };

        let error = media_error_from_gstreamer_message(error, &source);
        assert_eq!(error.kind, MediaErrorKind::Authentication);
        assert_eq!(error.recovery, MediaRecovery::ReloadSource);
        assert!(!error.message.contains("signed-secret"));
        assert!(!error.message.contains("bearer-secret"));
    }

    #[test]
    fn maps_missing_resources_without_retry() {
        super::initialize().unwrap();
        let source = MediaSource::from_uri("file:///missing.mp4").unwrap();
        let message =
            gst::message::Error::new(gst::ResourceError::NotFound, "resource was not found");
        let gst::MessageView::Error(error) = message.view() else {
            panic!("expected a GStreamer error message");
        };

        let error = media_error_from_gstreamer_message(error, &source);
        assert_eq!(error.kind, MediaErrorKind::SourceNotFound);
        assert!(!error.retryable());
    }

    #[test]
    fn video_geometry_preserves_crop_and_pixel_aspect_ratio() {
        crate::SystemBackend::initialize().unwrap();
        let caps = "video/x-raw,format=BGRA,width=100,height=60,pixel-aspect-ratio=2/1"
            .parse::<gst::Caps>()
            .unwrap();
        let info = gst_video::VideoInfo::from_caps(&caps).unwrap();
        let mut buffer = gst::Buffer::new();
        gst_video::VideoCropMeta::add(buffer.get_mut().unwrap(), (10, 5, 80, 50));

        let (coded_size, visible_rect, display_size) =
            video_frame_geometry(buffer.as_ref(), &info).unwrap();

        assert_eq!(coded_size.width, 100);
        assert_eq!(coded_size.height, 60);
        assert_eq!(visible_rect.origin.x, 10);
        assert_eq!(visible_rect.origin.y, 5);
        assert_eq!(visible_rect.size.width, 80);
        assert_eq!(visible_rect.size.height, 50);
        assert_eq!(display_size.width, 160);
        assert_eq!(display_size.height, 50);
    }

    #[test]
    fn appsink_allocation_supports_video_meta() {
        crate::SystemBackend::initialize().unwrap();
        let mut query = gst::query::Allocation::new(None, false);
        add_required_allocation_metas(&mut query);

        assert!(
            query
                .find_allocation_meta::<gst_video::VideoMeta>()
                .is_some()
        );
    }

    #[test]
    fn appsink_caps_prioritize_linear_dma_buf() {
        crate::SystemBackend::initialize().unwrap();
        let caps = appsink_caps(None).unwrap();
        let serialized = caps.to_string();

        #[cfg(target_os = "linux")]
        {
            assert!(serialized.starts_with("video/x-raw(memory:DMABuf)"));
            assert!(serialized.contains("format=(string)DMA_DRM"));
            assert!(serialized.contains("drm-format=(string){ NV12, AR24, AB24 }"));
        }
        assert!(serialized.contains("video/x-raw, format=(string){ NV12, BGRA, RGBA }"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn appsink_caps_advertise_only_supported_native_nv12_layouts() {
        crate::SystemBackend::initialize().unwrap();
        let modifier = 0x0200_0000_0840_1b04;
        let output_capabilities = gpui_media_core::FrameOutputCapabilities {
            native_nv12_dma_buf_modifiers: vec![
                gpui_media_core::DmaBufModifier {
                    modifier,
                    plane_count: 2,
                },
                gpui_media_core::DmaBufModifier {
                    modifier: 0x1234,
                    plane_count: 3,
                },
            ],
        };
        let serialized = appsink_caps(Some(&output_capabilities))
            .unwrap()
            .to_string();

        assert!(serialized.contains("NV12:0x0200000008401b04"));
        assert!(!serialized.contains("NV12:0x0000000000001234"));
    }

    #[test]
    fn cpu_sample_uses_cpu_surface_backing() {
        crate::SystemBackend::initialize().unwrap();
        let caps = "video/x-raw,format=BGRA,width=2,height=2,framerate=1/1"
            .parse::<gst::Caps>()
            .unwrap();
        let buffer = gst::Buffer::from_mut_slice(vec![0_u8; 16]);
        let sample = gst::Sample::builder().caps(&caps).buffer(&buffer).build();
        let frame = sample_to_surface_frame(
            &sample,
            FrameHandle::new(),
            1,
            #[cfg(target_os = "linux")]
            None,
        )
        .unwrap();

        assert!(matches!(frame.backing(), FrameBacking::Cpu(_)));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn dma_buf_sample_uses_dma_buf_surface_backing() {
        use gst_allocators::prelude::DmaBufAllocatorExtManual as _;

        crate::SystemBackend::initialize().unwrap();
        let caps = "video/x-raw(memory:DMABuf),format=NV12,width=16,height=16,framerate=1/1"
            .parse::<gst::Caps>()
            .unwrap();
        let allocator = gst_allocators::DmaBufAllocator::new();
        let memory = unsafe {
            allocator
                .alloc_dmabuf(File::open("/dev/zero").unwrap(), 384)
                .unwrap()
        };
        let mut buffer = gst::Buffer::new();
        buffer.get_mut().unwrap().append_memory(memory);
        let sample = gst::Sample::builder().caps(&caps).buffer(&buffer).build();
        let frame = sample_to_surface_frame(&sample, FrameHandle::new(), 1, None).unwrap();

        assert!(matches!(frame.backing(), FrameBacking::DmaBuf(_)));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn native_nv12_sample_preserves_one_object_two_plane_layout() {
        use gst_allocators::prelude::DmaBufAllocatorExtManual as _;

        crate::SystemBackend::initialize().unwrap();
        let modifier = 0x0200_0000_0840_1b04;
        let fourcc = gst_video::dma_drm_fourcc_from_format(gst_video::VideoFormat::Nv12).unwrap();
        let drm_format = gst_video::dma_drm_fourcc_to_string(fourcc, modifier);
        let caps = format!(
            "video/x-raw(memory:DMABuf),format=DMA_DRM,drm-format={drm_format},width=16,height=16,framerate=1/1"
        )
        .parse::<gst::Caps>()
        .unwrap();
        let allocator = gst_allocators::DmaBufAllocator::new();
        let memory = unsafe {
            allocator
                .alloc_dmabuf(File::open("/dev/zero").unwrap(), 384)
                .unwrap()
        };
        let mut buffer = gst::Buffer::new();
        buffer.get_mut().unwrap().append_memory(memory);
        gst_video::VideoMeta::add_full(
            buffer.get_mut().unwrap(),
            gst_video::VideoFrameFlags::empty(),
            gst_video::VideoFormat::DmaDrm,
            16,
            16,
            &[0, 256],
            &[16, 16],
        )
        .unwrap();
        let sample = gst::Sample::builder().caps(&caps).buffer(&buffer).build();
        let producer = gpui_media_core::DrmDevice {
            major: 226,
            minor: 128,
        };
        let frame =
            sample_to_surface_frame(&sample, FrameHandle::new(), 1, Some(producer)).unwrap();

        let FrameBacking::DmaBuf(dma_buf) = frame.backing() else {
            panic!("expected DMA-BUF surface backing");
        };
        let image = dma_buf;
        assert_eq!(image.objects().len(), 1);
        assert_eq!(image.planes().len(), 2);
        assert_eq!(image.planes()[0].object_index(), 0);
        assert_eq!(image.planes()[0].offset(), 0);
        assert_eq!(image.planes()[1].object_index(), 0);
        assert_eq!(image.planes()[1].offset(), 256);
        assert_eq!(image.drm_device(), Some(producer));
    }
}
