use std::{sync::Arc, time::Duration};

use gpui::{DevicePixels, SurfaceFrame, SurfaceHandle, size};
use gpui_media::{
    FrameExtractionSession, FrameExtractorBackendRequest, MediaBackend, MediaCapabilities,
    MediaError, MediaOutputSink, MediaPlaybackRequest, MediaPlaybackSession, MediaResult,
    MediaSource, PlaybackTimeline, SeekMode, VideoFrame, VideoFrameExtractor,
};

#[derive(Clone, Copy)]
struct CustomBackend;

impl MediaBackend for CustomBackend {
    fn name(&self) -> &'static str {
        "custom-test"
    }

    fn open_playback(
        &self,
        _request: MediaPlaybackRequest,
        output: MediaOutputSink,
    ) -> MediaResult<Box<dyn MediaPlaybackSession>> {
        output.publish_video_frame(test_frame(1, Duration::ZERO));
        Ok(Box::new(CustomPlayback))
    }

    fn open_frame_extractor(
        &self,
        _request: FrameExtractorBackendRequest,
    ) -> MediaResult<Box<dyn FrameExtractionSession>> {
        Ok(Box::new(CustomExtractor))
    }
}

struct CustomPlayback;

impl MediaPlaybackSession for CustomPlayback {
    fn capabilities(&self) -> MediaCapabilities {
        MediaCapabilities {
            video: true,
            seeking: true,
            frame_extraction: true,
            ..Default::default()
        }
    }

    fn play(&mut self) -> MediaResult<()> {
        Ok(())
    }

    fn pause(&mut self) -> MediaResult<()> {
        Ok(())
    }

    fn reload(&mut self, _autoplay: bool) -> MediaResult<()> {
        Ok(())
    }

    fn timeline(&self) -> PlaybackTimeline {
        PlaybackTimeline::new(Duration::ZERO, Some(Duration::from_secs(10)), true)
    }

    fn seek_to(&mut self, _position: Duration, _mode: SeekMode) -> MediaResult<()> {
        Ok(())
    }

    fn step_forward(&mut self, _frames: u64) -> MediaResult<()> {
        Err(MediaError::unsupported("frame stepping is unsupported"))
    }

    fn set_playback_rate(&mut self, _rate: f64) -> MediaResult<()> {
        Err(MediaError::unsupported(
            "playback rate changes are unsupported",
        ))
    }
}

struct CustomExtractor;

impl FrameExtractionSession for CustomExtractor {
    fn initial_frame(&mut self) -> MediaResult<Arc<VideoFrame>> {
        Ok(test_frame(1, Duration::ZERO))
    }

    fn frame_at(
        &mut self,
        position: Duration,
        _seek_mode: SeekMode,
    ) -> MediaResult<Arc<VideoFrame>> {
        Ok(test_frame(2, position))
    }
}

fn test_frame(sequence: u64, timestamp: Duration) -> Arc<VideoFrame> {
    let surface = SurfaceFrame::rgba(
        SurfaceHandle::new(),
        sequence,
        size(DevicePixels(1), DevicePixels(1)),
        vec![20, 40, 60, 255],
        4,
    )
    .unwrap();
    Arc::new(VideoFrame::new(
        Arc::new(surface),
        Some(timestamp),
        Some(Duration::from_millis(40)),
    ))
}

#[test]
fn external_backend_drives_the_generic_frame_extractor() {
    let source = MediaSource::from_uri("custom-test://video").unwrap();
    let backend: Arc<dyn MediaBackend> = Arc::new(CustomBackend);
    let extractor = VideoFrameExtractor::new(source, backend).unwrap();

    let initial = extractor.initial_frame_blocking().unwrap();
    assert_eq!(initial.timestamp(), Some(Duration::ZERO));

    let preview = extractor.frame_at_blocking(Duration::from_secs(3)).unwrap();
    assert_eq!(preview.timestamp(), Some(Duration::from_secs(3)));
    assert_eq!(preview.surface().sequence(), 2);
}
