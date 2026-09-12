use gpui_media::{
    FrameExtractionSession, FrameExtractorBackendRequest, MediaError, MediaOutputSink,
    MediaPlaybackRequest, MediaPlaybackSession, MediaResult,
};

pub(super) fn initialize() -> MediaResult<()> {
    Err(unsupported())
}

pub(super) fn open_playback(
    _request: MediaPlaybackRequest,
    _output: MediaOutputSink,
) -> MediaResult<Box<dyn MediaPlaybackSession>> {
    Err(unsupported())
}

pub(super) fn open_frame_extractor(
    _request: FrameExtractorBackendRequest,
) -> MediaResult<Box<dyn FrameExtractionSession>> {
    Err(unsupported())
}

fn unsupported() -> MediaError {
    MediaError::unsupported("SystemBackend is not available on this operating system")
}
