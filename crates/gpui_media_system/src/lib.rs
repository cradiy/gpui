//! Platform-selected system media backend.

pub use gpui_media_core::*;

#[cfg(all(any(target_os = "linux", target_os = "macos"), not(feature = "v1_24")))]
compile_error!("enable a GStreamer version feature: v1_24, v1_26, or v1_28");

#[cfg(all(any(target_os = "linux", target_os = "macos"), feature = "v1_24"))]
mod gstreamer;
#[cfg(all(any(target_os = "linux", target_os = "macos"), feature = "v1_24"))]
mod gstreamer_platform;
#[cfg(not(any(
    all(any(target_os = "linux", target_os = "macos"), feature = "v1_24"),
    target_os = "windows"
)))]
mod unsupported;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(all(any(target_os = "linux", target_os = "macos"), feature = "v1_24"))]
use gstreamer_platform as platform;
#[cfg(not(any(
    all(any(target_os = "linux", target_os = "macos"), feature = "v1_24"),
    target_os = "windows"
)))]
use unsupported as platform;
#[cfg(target_os = "windows")]
use windows as platform;

/// A media backend implemented by the system media stack for the current
/// operating system.
///
/// Linux and macOS use the system GStreamer registry, while Windows uses Media
/// Foundation.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemBackend;

impl SystemBackend {
    pub fn new() -> Self {
        Self
    }

    pub fn initialize() -> MediaResult<()> {
        platform::initialize()
    }
}

impl MediaBackend for SystemBackend {
    fn name(&self) -> &'static str {
        "system"
    }

    fn open_playback(
        &self,
        request: MediaPlaybackRequest,
        output: MediaOutputSink,
    ) -> MediaResult<Box<dyn MediaPlaybackSession>> {
        platform::open_playback(request, output)
    }

    fn open_frame_extractor(
        &self,
        request: FrameExtractorBackendRequest,
    ) -> MediaResult<Box<dyn FrameExtractionSession>> {
        platform::open_frame_extractor(request)
    }
}
