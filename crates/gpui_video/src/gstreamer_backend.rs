use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread::JoinHandle,
    time::Duration,
};

use anyhow::{Context as _, Result, anyhow, bail};
use gpui::{
    Bounds, ColorRange, DevicePixels, GpuSpecs, SurfaceColorInfo, SurfaceFormat, SurfaceFrame,
    SurfaceHandle, SurfacePlane, YuvMatrix, point, size,
};
use gst::prelude::*;

use crate::{
    MediaSource, PlaybackTimeline, SeekMode, VideoFrame, VideoPlaybackStats,
    stats::PlaybackCounters,
};

#[cfg(target_os = "linux")]
mod dma_buf;

pub(crate) struct PlaybackOutput {
    pub frames: async_channel::Receiver<Arc<VideoFrame>>,
    pub events: async_channel::Receiver<BackendEvent>,
}

#[derive(Debug)]
pub(crate) enum BackendEvent {
    Ended,
    Error(String),
}

pub(crate) struct GstreamerPlayback {
    playbin: gst::Element,
    appsink: gst_app::AppSink,
    bus: gst::Bus,
    bus_thread: Option<JoinHandle<()>>,
    counters: Arc<PlaybackCounters>,
    using_cpu_fallback: AtomicBool,
}

impl GstreamerPlayback {
    pub fn new(
        source: &MediaSource,
        gpu_specs: Option<&GpuSpecs>,
    ) -> Result<(Self, PlaybackOutput)> {
        crate::init()?;

        let caps = appsink_caps(gpu_specs)?;
        let appsink = gst_app::AppSink::builder()
            .caps(&caps)
            .max_buffers(2)
            .drop(true)
            .wait_on_eos(false)
            .sync(true)
            .build();

        let (frame_tx, frame_rx) = async_channel::bounded(1);
        let frame_drop_rx = frame_rx.clone();
        let counters = Arc::new(PlaybackCounters::default());
        let counters_for_samples = counters.clone();
        let sequence = Arc::new(AtomicU64::new(1));
        let surface_handle = SurfaceHandle::new();
        #[cfg(target_os = "linux")]
        let producer_drm_device = Arc::new(std::sync::RwLock::new(None));
        #[cfg(target_os = "linux")]
        let producer_drm_device_for_samples = producer_drm_device.clone();

        appsink.set_callbacks(
            gst_app::AppSinkCallbacks::builder()
                .propose_allocation(|_, query| {
                    add_required_allocation_metas(query);
                    true
                })
                .new_sample(move |sink| {
                    let sample = sink.pull_sample().map_err(|_| gst::FlowError::Eos)?;
                    let next_sequence = sequence.fetch_add(1, Ordering::Relaxed);
                    let frame = sample_to_video_frame(
                        &sample,
                        surface_handle.clone(),
                        next_sequence,
                        #[cfg(target_os = "linux")]
                        producer_drm_device_for_samples
                            .read()
                            .ok()
                            .and_then(|device| *device),
                    )
                    .map_err(|error| {
                        eprintln!("cvi: discarded decoded frame: {error:#}");
                        gst::FlowError::Error
                    })?;

                    counters_for_samples.record_decoded_frame();
                    if publish_latest(&frame_tx, &frame_drop_rx, frame) {
                        counters_for_samples.record_dropped_frame();
                    }
                    Ok(gst::FlowSuccess::Ok)
                })
                .build(),
        );

        let playbin = gst::ElementFactory::make("playbin3")
            .build()
            .context("GStreamer element 'playbin3' is not installed")?;
        playbin.set_property("uri", source.uri());
        playbin.set_property("video-sink", &appsink);

        let bus = playbin
            .bus()
            .context("playbin3 did not provide a message bus")?;
        let bus_for_thread = bus.clone();
        #[cfg(target_os = "linux")]
        let producer_drm_device_for_bus = producer_drm_device;
        let (event_tx, event_rx) = async_channel::unbounded();
        let bus_thread = std::thread::Builder::new()
            .name("cvi-gstreamer-bus".into())
            .spawn(move || {
                for message in bus_for_thread.iter_timed(gst::ClockTime::NONE) {
                    #[cfg(target_os = "linux")]
                    if let gst::MessageView::HaveContext(have_context) = message.view()
                        && let Some(device) = drm_device_from_context(&have_context.context())
                        && let Ok(mut current_device) = producer_drm_device_for_bus.write()
                    {
                        *current_device = Some(device);
                    }

                    let event = match message.view() {
                        gst::MessageView::Eos(..) => Some(BackendEvent::Ended),
                        gst::MessageView::Error(error) => {
                            let mut message = error.error().to_string();
                            if let Some(debug) = error.debug() {
                                message.push_str(": ");
                                message.push_str(&debug);
                            }
                            Some(BackendEvent::Error(message))
                        }
                        _ => None,
                    };

                    if let Some(event) = event
                        && event_tx.send_blocking(event).is_err()
                    {
                        break;
                    }
                }
            })
            .context("failed to start GStreamer bus thread")?;

        Ok((
            Self {
                playbin,
                appsink,
                bus,
                bus_thread: Some(bus_thread),
                counters,
                using_cpu_fallback: AtomicBool::new(false),
            },
            PlaybackOutput {
                frames: frame_rx,
                events: event_rx,
            },
        ))
    }

    pub fn play(&self) -> Result<()> {
        self.playbin
            .set_state(gst::State::Playing)
            .map(|_| ())
            .map_err(|error| anyhow!("failed to start playback: {error:?}"))
    }

    pub fn pause(&self) -> Result<()> {
        self.playbin
            .set_state(gst::State::Paused)
            .map(|_| ())
            .map_err(|error| anyhow!("failed to pause playback: {error:?}"))
    }

    pub fn restart(&self) -> Result<()> {
        self.seek_to(Duration::ZERO, SeekMode::KeyFrame)?;
        self.play()
    }

    pub fn timeline(&self) -> PlaybackTimeline {
        let position = self
            .playbin
            .query_position::<gst::ClockTime>()
            .map(Duration::from)
            .unwrap_or_default();
        let duration = self
            .playbin
            .query_duration::<gst::ClockTime>()
            .map(Duration::from);
        let mut seeking = gst::query::Seeking::new(gst::Format::Time);
        let seekable = self.playbin.query(&mut seeking) && seeking.result().0;

        PlaybackTimeline::new(position, duration, seekable)
    }

    pub fn seek_to(&self, position: Duration, mode: SeekMode) -> Result<()> {
        let position = clock_time(position)?;
        self.playbin
            .seek_simple(seek_flags(mode), position)
            .with_context(|| format!("failed to seek to {position}"))
    }

    pub fn step_forward(&self, frames: u64) -> Result<()> {
        if frames == 0 {
            bail!("frame step amount must be greater than zero");
        }
        let event = gst::event::Step::new(gst::format::Buffers::from_u64(frames), 1.0, true, false);
        if self.playbin.send_event(event) {
            Ok(())
        } else {
            bail!("the playback pipeline rejected frame stepping")
        }
    }

    pub fn set_playback_rate(&self, rate: f64) -> Result<()> {
        if !rate.is_finite() || rate <= 0.0 {
            bail!("playback rate must be finite and greater than zero");
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
            .with_context(|| format!("failed to change playback rate to {rate}"))
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

    pub fn stats(&self, delivered_frames: u64) -> VideoPlaybackStats {
        self.counters.snapshot(delivered_frames)
    }

    pub fn switch_to_cpu_fallback(&self) -> Result<bool> {
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
                    .with_context(|| {
                        format!("failed to renegotiate CPU video output at {position}")
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

impl Drop for GstreamerPlayback {
    fn drop(&mut self) {
        let _ = self.playbin.set_state(gst::State::Null);
        self.bus.set_flushing(true);
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

fn publish_latest<T>(
    sender: &async_channel::Sender<T>,
    drain: &async_channel::Receiver<T>,
    value: T,
) -> bool {
    match sender.try_send(value) {
        Ok(()) | Err(async_channel::TrySendError::Closed(_)) => false,
        Err(async_channel::TrySendError::Full(value)) => {
            let dropped = drain.try_recv().is_ok();
            let _ = sender.try_send(value);
            dropped
        }
    }
}

pub(crate) fn seek_flags(mode: SeekMode) -> gst::SeekFlags {
    match mode {
        SeekMode::Accurate => gst::SeekFlags::FLUSH | gst::SeekFlags::ACCURATE,
        SeekMode::KeyFrame => gst::SeekFlags::FLUSH | gst::SeekFlags::KEY_UNIT,
    }
}

pub(crate) fn clock_time(duration: Duration) -> Result<gst::ClockTime> {
    gst::ClockTime::try_from(duration).context("media timestamp exceeds the GStreamer time range")
}

pub(crate) fn appsink_caps(gpu_specs: Option<&GpuSpecs>) -> Result<gst::Caps> {
    #[cfg(target_os = "linux")]
    {
        dma_buf::appsink_caps(gpu_specs)
    }

    #[cfg(not(target_os = "linux"))]
    {
        "video/x-raw,format=(string){NV12,BGRA,RGBA}"
            .parse::<gst::Caps>()
            .context("failed to construct appsink caps")
    }
}

fn cpu_appsink_caps() -> Result<gst::Caps> {
    "video/x-raw,format=(string){NV12,BGRA,RGBA}"
        .parse::<gst::Caps>()
        .context("failed to construct CPU appsink caps")
}

fn sample_to_surface_frame(
    sample: &gst::Sample,
    handle: SurfaceHandle,
    sequence: u64,
    #[cfg(target_os = "linux")] producer_drm_device: Option<gpui::DrmDevice>,
) -> Result<SurfaceFrame> {
    #[cfg(target_os = "linux")]
    if dma_buf::sample_uses_dma_buf(sample) {
        return dma_buf::sample_to_surface_frame(sample, handle, sequence, producer_drm_device);
    }

    sample_to_cpu_surface_frame(sample, handle, sequence)
}

pub(crate) fn sample_to_video_frame(
    sample: &gst::Sample,
    handle: SurfaceHandle,
    sequence: u64,
    #[cfg(target_os = "linux")] producer_drm_device: Option<gpui::DrmDevice>,
) -> Result<Arc<VideoFrame>> {
    let buffer = sample.buffer().context("decoded sample has no buffer")?;
    let timestamp = buffer.pts().map(Duration::from);
    let duration = buffer.duration().map(Duration::from);
    let surface = Arc::new(sample_to_surface_frame(
        sample,
        handle,
        sequence,
        #[cfg(target_os = "linux")]
        producer_drm_device,
    )?);

    Ok(Arc::new(VideoFrame::new(surface, timestamp, duration)))
}

#[cfg(target_os = "linux")]
fn drm_device_from_context(context: &gst::Context) -> Option<gpui::DrmDevice> {
    use std::os::unix::fs::MetadataExt as _;

    let path = context.structure().get::<String>("path").ok()?;
    let metadata = std::fs::metadata(path).ok()?;
    let device = metadata.rdev();
    Some(gpui::DrmDevice {
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
    handle: SurfaceHandle,
    sequence: u64,
) -> Result<SurfaceFrame> {
    let caps = sample.caps().context("decoded sample has no caps")?;
    let info = gst_video::VideoInfo::from_caps(caps).context("invalid decoded video caps")?;
    let buffer = sample
        .buffer_owned()
        .context("decoded sample has no buffer")?;
    let (coded_size, visible_rect, display_size) = video_frame_geometry(buffer.as_ref(), &info)?;
    let frame = gst_video::VideoFrame::from_buffer_readable(buffer, &info)
        .map_err(|_| anyhow!("failed to map decoded video buffer"))?;

    let stride = |plane: usize| -> Result<u32> {
        let stride = *info
            .stride()
            .get(plane)
            .context("decoded video plane has no stride")?;
        if stride <= 0 {
            bail!("negative decoded video stride is not supported: {stride}");
        }
        Ok(stride as u32)
    };

    match info.format() {
        gst_video::VideoFormat::Bgra => SurfaceFrame::new(
            handle,
            sequence,
            coded_size,
            visible_rect,
            display_size,
            SurfaceFormat::Bgra8,
            [SurfacePlane::new(
                frame
                    .plane_data(0)
                    .context("failed to read BGRA plane")?
                    .to_vec(),
                stride(0)?,
            )],
            SurfaceColorInfo::default(),
        )
        .context("GPUI rejected BGRA frame"),
        gst_video::VideoFormat::Rgba => SurfaceFrame::new(
            handle,
            sequence,
            coded_size,
            visible_rect,
            display_size,
            SurfaceFormat::Rgba8,
            [SurfacePlane::new(
                frame
                    .plane_data(0)
                    .context("failed to read RGBA plane")?
                    .to_vec(),
                stride(0)?,
            )],
            SurfaceColorInfo::default(),
        )
        .context("GPUI rejected RGBA frame"),
        gst_video::VideoFormat::Nv12 => SurfaceFrame::new(
            handle,
            sequence,
            coded_size,
            visible_rect,
            display_size,
            SurfaceFormat::Nv12,
            [
                SurfacePlane::new(
                    frame
                        .plane_data(0)
                        .context("failed to read NV12 Y plane")?
                        .to_vec(),
                    stride(0)?,
                ),
                SurfacePlane::new(
                    frame
                        .plane_data(1)
                        .context("failed to read NV12 UV plane")?
                        .to_vec(),
                    stride(1)?,
                ),
            ],
            surface_color_info(&info),
        )
        .context("GPUI rejected NV12 frame"),
        format => bail!("unsupported decoded video format: {format:?}"),
    }
}

pub(super) fn video_frame_geometry(
    buffer: &gst::BufferRef,
    info: &gst_video::VideoInfo,
) -> Result<(
    gpui::Size<DevicePixels>,
    Bounds<DevicePixels>,
    gpui::Size<DevicePixels>,
)> {
    let coded_size = size(
        DevicePixels(i32::try_from(info.width()).context("video width is too large")?),
        DevicePixels(i32::try_from(info.height()).context("video height is too large")?),
    );
    let (x, y, width, height) = buffer
        .meta::<gst_video::VideoCropMeta>()
        .map(|crop| crop.rect())
        .unwrap_or((0, 0, info.width(), info.height()));
    let visible_rect = Bounds {
        origin: point(
            DevicePixels(i32::try_from(x).context("video crop x is too large")?),
            DevicePixels(i32::try_from(y).context("video crop y is too large")?),
        ),
        size: size(
            DevicePixels(i32::try_from(width).context("video crop width is too large")?),
            DevicePixels(i32::try_from(height).context("video crop height is too large")?),
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
        .context("video display width overflow")?;
    let display_size = size(
        DevicePixels(i32::try_from(display_width).context("video display width is too large")?),
        DevicePixels(i32::try_from(height).context("video display height is too large")?),
    );

    Ok((coded_size, visible_rect, display_size))
}

fn surface_color_info(info: &gst_video::VideoInfo) -> SurfaceColorInfo {
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

    SurfaceColorInfo { matrix, range }
}

#[cfg(test)]
mod tests {
    use std::fs::File;

    use gpui::{SurfaceFrameBacking, SurfaceHandle};

    use super::{
        add_required_allocation_metas, appsink_caps, publish_latest, sample_to_surface_frame,
        video_frame_geometry,
    };

    #[test]
    fn latest_frame_queue_replaces_stale_value() {
        let (sender, receiver) = async_channel::bounded(1);

        assert!(!publish_latest(&sender, &receiver, 1));
        assert!(publish_latest(&sender, &receiver, 2));
        assert_eq!(receiver.try_recv(), Ok(2));
    }

    #[test]
    fn video_geometry_preserves_crop_and_pixel_aspect_ratio() {
        crate::init().unwrap();
        let caps = "video/x-raw,format=BGRA,width=100,height=60,pixel-aspect-ratio=2/1"
            .parse::<gst::Caps>()
            .unwrap();
        let info = gst_video::VideoInfo::from_caps(&caps).unwrap();
        let mut buffer = gst::Buffer::new();
        gst_video::VideoCropMeta::add(buffer.get_mut().unwrap(), (10, 5, 80, 50));

        let (coded_size, visible_rect, display_size) =
            video_frame_geometry(buffer.as_ref(), &info).unwrap();

        assert_eq!(coded_size.width.0, 100);
        assert_eq!(coded_size.height.0, 60);
        assert_eq!(visible_rect.origin.x.0, 10);
        assert_eq!(visible_rect.origin.y.0, 5);
        assert_eq!(visible_rect.size.width.0, 80);
        assert_eq!(visible_rect.size.height.0, 50);
        assert_eq!(display_size.width.0, 160);
        assert_eq!(display_size.height.0, 50);
    }

    #[test]
    fn appsink_allocation_supports_video_meta() {
        crate::init().unwrap();
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
        crate::init().unwrap();
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
        crate::init().unwrap();
        let modifier = 0x0200_0000_0840_1b04;
        let gpu_specs = gpui::GpuSpecs {
            supports_native_nv12_dma_buf_import: true,
            native_nv12_dma_buf_modifiers: vec![
                gpui::DmaBufModifier {
                    modifier,
                    plane_count: 2,
                },
                gpui::DmaBufModifier {
                    modifier: 0x1234,
                    plane_count: 3,
                },
            ],
            ..Default::default()
        };
        let serialized = appsink_caps(Some(&gpu_specs)).unwrap().to_string();

        assert!(serialized.contains("NV12:0x0200000008401b04"));
        assert!(!serialized.contains("NV12:0x0000000000001234"));
    }

    #[test]
    fn cpu_sample_uses_cpu_surface_backing() {
        crate::init().unwrap();
        let caps = "video/x-raw,format=BGRA,width=2,height=2,framerate=1/1"
            .parse::<gst::Caps>()
            .unwrap();
        let buffer = gst::Buffer::from_mut_slice(vec![0_u8; 16]);
        let sample = gst::Sample::builder().caps(&caps).buffer(&buffer).build();
        let frame = sample_to_surface_frame(
            &sample,
            SurfaceHandle::new(),
            1,
            #[cfg(target_os = "linux")]
            None,
        )
        .unwrap();

        assert!(matches!(frame.backing(), SurfaceFrameBacking::Cpu(_)));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn dma_buf_sample_uses_dma_buf_surface_backing() {
        use gst_allocators::prelude::DmaBufAllocatorExtManual as _;

        crate::init().unwrap();
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
        let frame = sample_to_surface_frame(&sample, SurfaceHandle::new(), 1, None).unwrap();

        assert!(matches!(frame.backing(), SurfaceFrameBacking::DmaBuf(_)));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn native_nv12_sample_preserves_one_object_two_plane_layout() {
        use gst_allocators::prelude::DmaBufAllocatorExtManual as _;

        crate::init().unwrap();
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
        let producer = gpui::DrmDevice {
            major: 226,
            minor: 128,
        };
        let frame =
            sample_to_surface_frame(&sample, SurfaceHandle::new(), 1, Some(producer)).unwrap();

        let SurfaceFrameBacking::DmaBuf(dma_buf) = frame.backing() else {
            panic!("expected DMA-BUF surface backing");
        };
        let image = dma_buf.image().expect("expected native DMA-BUF image");
        assert_eq!(image.objects().len(), 1);
        assert_eq!(image.planes().len(), 2);
        assert_eq!(image.planes()[0].object_index(), 0);
        assert_eq!(image.planes()[0].offset(), 0);
        assert_eq!(image.planes()[1].object_index(), 0);
        assert_eq!(image.planes()[1].offset(), 256);
        assert_eq!(image.drm_device(), Some(producer));
    }
}
