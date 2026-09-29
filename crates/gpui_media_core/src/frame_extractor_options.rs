use crate::{MediaError, MediaErrorKind, MediaRecovery, SeekMode};
use std::{fmt, time::Duration};

/// Indicates that a pending latest-only preview request was replaced by a
/// newer request before decoding started.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameExtractionSuperseded;

impl fmt::Display for FrameExtractionSuperseded {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("frame extraction request was superseded by a newer preview request")
    }
}

impl std::error::Error for FrameExtractionSuperseded {}

impl From<FrameExtractionSuperseded> for MediaError {
    fn from(error: FrameExtractionSuperseded) -> Self {
        Self::new(
            MediaErrorKind::Superseded,
            error.to_string(),
            MediaRecovery::None,
        )
    }
}

/// Configuration for an independent frame extraction pipeline.
#[derive(Clone, Copy, Debug)]
pub struct VideoFrameExtractorOptions {
    pub video_decoder: crate::VideoDecoderPolicy,
    pub timeout: Duration,
    pub seek_mode: SeekMode,
    /// Maximum number of extraction requests waiting behind the active seek.
    ///
    /// A bounded queue applies backpressure when a thumbnail or scrubber client
    /// submits requests faster than the decoder can satisfy them.
    pub request_queue_capacity: usize,
}

impl Default for VideoFrameExtractorOptions {
    fn default() -> Self {
        Self {
            video_decoder: crate::VideoDecoderPolicy::Auto,
            timeout: Duration::from_secs(10),
            seek_mode: SeekMode::Accurate,
            request_queue_capacity: 2,
        }
    }
}
