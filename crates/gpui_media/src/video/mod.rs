mod container;
mod frame;
mod frame_extractor;
mod media_backend;
mod media_info;
mod player;
mod stats;
mod subtitles;
mod window_sizing;

pub use container::{VideoContainer, video_container};
pub use frame::{FrameTransport, VideoFrame};
pub use frame_extractor::{
    FrameExtractionSuperseded, VideoFrameExtractor, VideoFrameExtractorOptions,
};
pub use media_backend::{
    FrameExtractionSession, FrameExtractorBackendRequest, FrameTransportPreference, MediaBackend,
    MediaBackendEvent, MediaCapabilities, MediaOutput, MediaOutputSink, MediaPlaybackRequest,
    MediaPlaybackSession, TransportChange,
};
pub use media_info::{
    AudioStreamInfo, MediaInfo, MediaStreamId, SubtitleStreamInfo, VideoStreamInfo,
};
pub use player::{VideoPlayer, VideoPlayerBuilder, VideoPlayerEvent, VideoPlayerOptions};
pub use stats::VideoPlaybackStats;
pub use subtitles::{
    ParsedSubtitles, SubtitleCue, SubtitleEvent, SubtitleFormat, detect_subtitle_format,
    normalize_subtitle_text, parse_subtitles,
};
pub use window_sizing::{fit_video_window_bounds, fit_video_window_size};
