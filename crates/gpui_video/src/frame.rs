use std::{fmt, sync::Arc, time::Duration};

use gpui::{DevicePixels, Size, SurfaceFrame, SurfaceFrameBacking};

/// Describes how a decoded frame reaches GPUI.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameTransport {
    Cpu,
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

    pub fn transport(&self) -> FrameTransport {
        match self.surface.backing() {
            SurfaceFrameBacking::Cpu(_) => FrameTransport::Cpu,
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
