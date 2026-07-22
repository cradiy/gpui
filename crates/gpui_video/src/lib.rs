//! Reusable GPUI video playback component.
//!
//! GStreamer owns demuxing, decoding, audio output and the playback clock.
//! [`VideoPlayer`] receives decoded video frames through an appsink and renders
//! only the newest frame with GPUI's dynamic `surface` element. It deliberately
//! contains no controls, pointer behavior, status overlay or fullscreen policy;
//! host applications build those features from the exported state and events.

mod container;
mod frame;
mod frame_extractor;
mod gstreamer_backend;
mod network;
mod player;
mod source;
mod stats;
mod timeline;

pub use container::{VideoContainer, video_container};
pub use frame::{FrameTransport, VideoFrame};
pub use frame_extractor::{
    FrameExtractionSuperseded, VideoFrameExtractor, VideoFrameExtractorOptions,
};
pub use player::{PlaybackState, VideoPlayer, VideoPlayerEvent, VideoPlayerOptions};
pub use source::{MediaSource, NetworkSourceOptions};
pub use stats::VideoPlaybackStats;
pub use timeline::{PlaybackTimeline, SeekMode};

/// Initializes GStreamer for the current process.
///
/// Applications may call this during startup. [`VideoPlayer::new`] also calls
/// it, so embedding the component does not require a separate initialization
/// step.
pub fn init() -> anyhow::Result<()> {
    gst::init().map_err(|error| anyhow::anyhow!("failed to initialize GStreamer: {error}"))
}
