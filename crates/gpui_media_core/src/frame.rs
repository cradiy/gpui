use std::{fmt, sync::Arc, time::Duration};

mod buffer;
pub use buffer::*;

#[cfg(test)]
mod tests;

/// Describes how a decoded frame is stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameTransport {
    Cpu,
    CoreVideo,
    DmaBuf,
}

/// A decoded video frame together with its media timestamp.
#[derive(Clone)]
pub struct VideoFrame {
    buffer: Arc<FrameBuffer>,
    timestamp: Option<Duration>,
    duration: Option<Duration>,
    decoder_info: Option<Arc<crate::VideoDecoderInfo>>,
}

impl VideoFrame {
    pub fn new(
        buffer: Arc<FrameBuffer>,
        timestamp: Option<Duration>,
        duration: Option<Duration>,
    ) -> Self {
        Self {
            buffer,
            timestamp,
            duration,
            decoder_info: None,
        }
    }

    /// Attaches a snapshot of the backend's observed video decoder.
    pub fn with_decoder_info(mut self, info: Arc<crate::VideoDecoderInfo>) -> Self {
        self.decoder_info = Some(info);
        self
    }

    /// Returns decoder metadata, or `None` when the backend cannot identify it.
    pub fn decoder_info(&self) -> Option<&Arc<crate::VideoDecoderInfo>> {
        self.decoder_info.as_ref()
    }

    /// Returns the image buffer backing this decoded frame.
    pub fn buffer(&self) -> &Arc<FrameBuffer> {
        &self.buffer
    }

    /// Returns the frame presentation timestamp when supplied by the stream.
    pub fn timestamp(&self) -> Option<Duration> {
        self.timestamp
    }

    /// Returns the frame presentation duration when supplied by the stream.
    pub fn duration(&self) -> Option<Duration> {
        self.duration
    }

    pub fn coded_size(&self) -> FrameSize {
        self.buffer.coded_size()
    }

    /// Returns the displayable portion of the coded frame.
    pub fn visible_rect(&self) -> FrameRect {
        self.buffer.visible_rect()
    }

    /// Returns the intended presentation size after pixel-aspect correction.
    pub fn display_size(&self) -> FrameSize {
        self.buffer.display_size()
    }

    /// Returns the decoded pixel format.
    pub fn format(&self) -> PixelFormat {
        self.buffer.format()
    }

    /// Returns the YUV conversion metadata associated with this frame.
    pub fn color_info(&self) -> FrameColorInfo {
        self.buffer.color()
    }

    pub fn transport(&self) -> FrameTransport {
        match self.buffer.backing() {
            FrameBacking::Cpu(_) => FrameTransport::Cpu,
            #[cfg(target_os = "macos")]
            FrameBacking::CoreVideo(_) => FrameTransport::CoreVideo,
            #[cfg(target_os = "linux")]
            FrameBacking::DmaBuf(_) => FrameTransport::DmaBuf,
        }
    }
}

impl fmt::Debug for VideoFrame {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VideoFrame")
            .field("timestamp", &self.timestamp)
            .field("duration", &self.duration)
            .field("coded_size", &self.coded_size())
            .field("transport", &self.transport())
            .field("decoder_info", &self.decoder_info)
            .finish()
    }
}
