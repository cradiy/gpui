use std::{fmt, sync::Arc, time::Duration};

use gpui::{
    Bounds, DevicePixels, Size, SurfaceColorInfo, SurfaceFormat, SurfaceFrame, SurfaceFrameBacking,
};

/// Describes how a decoded frame reaches GPUI.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameTransport {
    Cpu,
    CoreVideo,
    DmaBuf,
}

/// A decoded video frame together with its media timestamp.
#[derive(Clone)]
pub struct VideoFrame {
    surface: Arc<SurfaceFrame>,
    timestamp: Option<Duration>,
    duration: Option<Duration>,
}

impl VideoFrame {
    pub(crate) fn new(
        surface: Arc<SurfaceFrame>,
        timestamp: Option<Duration>,
        duration: Option<Duration>,
    ) -> Self {
        Self {
            surface,
            timestamp,
            duration,
        }
    }

    /// Returns the GPUI surface backing this decoded frame.
    pub fn surface(&self) -> &Arc<SurfaceFrame> {
        &self.surface
    }

    /// Returns the frame presentation timestamp when supplied by the stream.
    pub fn timestamp(&self) -> Option<Duration> {
        self.timestamp
    }

    /// Returns the frame presentation duration when supplied by the stream.
    pub fn duration(&self) -> Option<Duration> {
        self.duration
    }

    pub fn coded_size(&self) -> Size<DevicePixels> {
        self.surface.coded_size()
    }

    /// Returns the displayable portion of the coded frame.
    pub fn visible_rect(&self) -> Bounds<DevicePixels> {
        self.surface.visible_rect()
    }

    /// Returns the intended presentation size after pixel-aspect correction.
    pub fn display_size(&self) -> Size<DevicePixels> {
        self.surface.display_size()
    }

    /// Returns the decoded pixel format.
    pub fn format(&self) -> SurfaceFormat {
        self.surface.format()
    }

    /// Returns the YUV conversion metadata associated with this frame.
    pub fn color_info(&self) -> SurfaceColorInfo {
        self.surface.color()
    }

    pub fn transport(&self) -> FrameTransport {
        match self.surface.backing() {
            SurfaceFrameBacking::Cpu(_) => FrameTransport::Cpu,
            #[cfg(target_os = "macos")]
            SurfaceFrameBacking::CoreVideo(_) => FrameTransport::CoreVideo,
            #[cfg(target_os = "linux")]
            SurfaceFrameBacking::DmaBuf(_) => FrameTransport::DmaBuf,
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
            .finish()
    }
}
