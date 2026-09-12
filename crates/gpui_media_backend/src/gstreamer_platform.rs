use gpui_media_core::{
    FrameExtractionSession, FrameExtractorBackendRequest, MediaBackend, MediaOutputSink,
    MediaPlaybackRequest, MediaPlaybackSession, MediaResult,
};

use super::gstreamer::{self, GstreamerSystemBackend};

pub(super) fn initialize() -> MediaResult<()> {
    gstreamer::initialize()
}

pub(super) fn open_playback(
    request: MediaPlaybackRequest,
    output: MediaOutputSink,
) -> MediaResult<Box<dyn MediaPlaybackSession>> {
    GstreamerSystemBackend.open_playback(request, output)
}

pub(super) fn open_frame_extractor(
    request: FrameExtractorBackendRequest,
) -> MediaResult<Box<dyn FrameExtractionSession>> {
    GstreamerSystemBackend.open_frame_extractor(request)
}
