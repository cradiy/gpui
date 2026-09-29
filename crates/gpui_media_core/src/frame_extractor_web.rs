pub use super::frame_extractor_options::*;
use crate::{MediaBackend, MediaError, MediaResult, MediaSource, SeekMode, VideoFrame};
use std::{sync::Arc, time::Duration};

/// Independent frame extraction is unavailable in the browser media backend.
#[derive(Clone)]
pub struct VideoFrameExtractor {
    _private: (),
}

fn unsupported<T>() -> MediaResult<T> {
    Err(MediaError::unsupported(
        "independent frame extraction is unavailable in browsers",
    ))
}
impl VideoFrameExtractor {
    pub fn new(_source: MediaSource, _backend: Arc<dyn MediaBackend>) -> MediaResult<Self> {
        unsupported()
    }
    pub fn with_options(
        _source: MediaSource,
        _options: VideoFrameExtractorOptions,
        _backend: Arc<dyn MediaBackend>,
    ) -> MediaResult<Self> {
        unsupported()
    }
    pub async fn frame_at(&self, _position: Duration) -> MediaResult<Arc<VideoFrame>> {
        unsupported()
    }
    pub async fn initial_frame(&self) -> MediaResult<Arc<VideoFrame>> {
        unsupported()
    }
    pub async fn frame_at_with_mode(
        &self,
        _position: Duration,
        _mode: SeekMode,
    ) -> MediaResult<Arc<VideoFrame>> {
        unsupported()
    }
    pub async fn frame_at_latest(&self, _position: Duration) -> MediaResult<Arc<VideoFrame>> {
        unsupported()
    }
    pub async fn frame_at_latest_with_mode(
        &self,
        _position: Duration,
        _mode: SeekMode,
    ) -> MediaResult<Arc<VideoFrame>> {
        unsupported()
    }
    pub fn frame_at_blocking(&self, _position: Duration) -> MediaResult<Arc<VideoFrame>> {
        unsupported()
    }
    pub fn initial_frame_blocking(&self) -> MediaResult<Arc<VideoFrame>> {
        unsupported()
    }
    pub fn frame_at_blocking_with_mode(
        &self,
        _position: Duration,
        _mode: SeekMode,
    ) -> MediaResult<Arc<VideoFrame>> {
        unsupported()
    }
}
