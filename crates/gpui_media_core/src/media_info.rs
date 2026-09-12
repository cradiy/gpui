use std::fmt;

use crate::FrameSize;
use std::sync::Arc;

/// Opaque identifier for one stream in an opened media source.
///
/// Identifiers only need to remain stable for the lifetime of the corresponding
/// source or playback session.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct MediaStreamId(Arc<str>);

impl MediaStreamId {
    pub fn new(id: impl Into<Arc<str>>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        self.0.as_ref()
    }
}

impl fmt::Display for MediaStreamId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl From<String> for MediaStreamId {
    fn from(id: String) -> Self {
        Self(id.into())
    }
}

impl From<&'static str> for MediaStreamId {
    fn from(id: &'static str) -> Self {
        Self(id.into())
    }
}

/// Metadata for one video stream.
#[derive(Clone, Debug)]
pub struct VideoStreamInfo {
    pub id: MediaStreamId,
    pub codec: Option<Arc<str>>,
    pub coded_size: Option<FrameSize>,
    pub display_size: Option<FrameSize>,
    pub frame_rate: Option<f64>,
    pub bitrate: Option<u64>,
    pub language: Option<Arc<str>>,
    pub selected: bool,
}

/// Metadata for one audio stream.
#[derive(Clone, Debug)]
pub struct AudioStreamInfo {
    pub id: MediaStreamId,
    pub codec: Option<Arc<str>>,
    pub channels: Option<u32>,
    pub sample_rate: Option<u32>,
    pub bitrate: Option<u64>,
    pub language: Option<Arc<str>>,
    pub title: Option<Arc<str>>,
    pub selected: bool,
}

/// Metadata for one embedded subtitle stream.
#[derive(Clone, Debug)]
pub struct SubtitleStreamInfo {
    pub id: MediaStreamId,
    pub codec: Option<Arc<str>>,
    pub language: Option<Arc<str>>,
    pub title: Option<Arc<str>>,
    pub forced: bool,
    pub selected: bool,
}

/// Stream metadata reported by an opened media session.
#[derive(Clone, Debug, Default)]
pub struct MediaInfo {
    pub video_streams: Vec<VideoStreamInfo>,
    pub audio_streams: Vec<AudioStreamInfo>,
    pub subtitle_streams: Vec<SubtitleStreamInfo>,
}

impl MediaInfo {
    pub fn selected_audio_stream(&self) -> Option<&AudioStreamInfo> {
        self.audio_streams.iter().find(|stream| stream.selected)
    }

    pub fn selected_subtitle_stream(&self) -> Option<&SubtitleStreamInfo> {
        self.subtitle_streams.iter().find(|stream| stream.selected)
    }
}

#[cfg(test)]
mod tests {
    use super::{AudioStreamInfo, MediaInfo, MediaStreamId, SubtitleStreamInfo};

    #[test]
    fn selected_stream_helpers_follow_backend_flags() {
        let info = MediaInfo {
            audio_streams: vec![AudioStreamInfo {
                id: MediaStreamId::new("audio-ja"),
                codec: None,
                channels: None,
                sample_rate: None,
                bitrate: None,
                language: Some("ja".into()),
                title: None,
                selected: true,
            }],
            subtitle_streams: vec![SubtitleStreamInfo {
                id: MediaStreamId::new("subtitle-en"),
                codec: None,
                language: Some("en".into()),
                title: None,
                forced: false,
                selected: false,
            }],
            ..Default::default()
        };

        assert_eq!(
            info.selected_audio_stream()
                .map(|stream| stream.id.as_str()),
            Some("audio-ja")
        );
        assert!(info.selected_subtitle_stream().is_none());
    }
}
