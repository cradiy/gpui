use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use crate::{MediaError, MediaResult};

#[cfg(target_os = "linux")]
mod dma_buf;
#[cfg(target_os = "linux")]
pub use dma_buf::*;
#[cfg(target_os = "macos")]
mod core_video;
#[cfg(target_os = "macos")]
pub use core_video::*;

/// Pixel dimensions of a decoded image or its intended display size.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FrameSize {
    pub width: i32,
    pub height: i32,
}

impl FrameSize {
    pub const fn new(width: i32, height: i32) -> Self {
        Self { width, height }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FramePoint {
    pub x: i32,
    pub y: i32,
}

impl FramePoint {
    pub const fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }
}

/// Visible region within the coded image, in pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FrameRect {
    pub origin: FramePoint,
    pub size: FrameSize,
}

/// Stable identity for a sequence of decoded frames.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FrameHandle(u64);

impl FrameHandle {
    pub fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Self(NEXT.fetch_add(1, Ordering::Relaxed))
    }
}

impl Default for FrameHandle {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PixelFormat {
    Bgra8,
    Rgba8,
    Nv12,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum YuvMatrix {
    Bt601,
    #[default]
    Bt709,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ColorRange {
    #[default]
    Limited,
    Full,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FrameColorInfo {
    pub matrix: YuvMatrix,
    pub range: ColorRange,
}

/// Native output layouts accepted by a frame consumer.
#[derive(Clone, Debug, Default)]
pub struct FrameOutputCapabilities {
    #[cfg(target_os = "linux")]
    pub native_nv12_dma_buf_modifiers: Vec<DmaBufModifier>,
}

/// Immutable CPU bytes with a row stride and an offset into the allocation.
#[derive(Clone, Debug)]
pub struct FramePlane {
    bytes: Arc<[u8]>,
    offset: usize,
    stride: u32,
}

impl FramePlane {
    pub fn new(bytes: impl Into<Arc<[u8]>>, stride: u32) -> Self {
        Self::with_offset(bytes, 0, stride)
    }

    pub fn with_offset(bytes: impl Into<Arc<[u8]>>, offset: usize, stride: u32) -> Self {
        Self {
            bytes: bytes.into(),
            offset,
            stride,
        }
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub fn shared_bytes(&self) -> Arc<[u8]> {
        self.bytes.clone()
    }
    pub fn offset(&self) -> usize {
        self.offset
    }
    pub fn stride(&self) -> u32 {
        self.stride
    }
}

#[derive(Clone, Debug)]
pub enum FrameBacking {
    Cpu(Vec<FramePlane>),
    #[cfg(target_os = "linux")]
    DmaBuf(Arc<DmaBufImage>),
    #[cfg(target_os = "macos")]
    CoreVideo(CoreVideoHandle),
}

/// A validated, immutable decoded image independent of a graphics API.
///
/// Within one `FrameHandle`, each new image must have a distinct sequence
/// number so consumers can cache uploaded pixels without comparing content.
#[derive(Clone, Debug)]
pub struct FrameBuffer {
    handle: FrameHandle,
    sequence: u64,
    coded_size: FrameSize,
    visible_rect: FrameRect,
    display_size: FrameSize,
    format: PixelFormat,
    backing: FrameBacking,
    color: FrameColorInfo,
}

impl FrameBuffer {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        handle: FrameHandle,
        sequence: u64,
        coded_size: FrameSize,
        visible_rect: FrameRect,
        display_size: FrameSize,
        format: PixelFormat,
        planes: impl IntoIterator<Item = FramePlane>,
        color: FrameColorInfo,
    ) -> MediaResult<Self> {
        Self::with_backing(
            handle,
            sequence,
            coded_size,
            visible_rect,
            display_size,
            format,
            FrameBacking::Cpu(planes.into_iter().collect()),
            color,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn with_backing(
        handle: FrameHandle,
        sequence: u64,
        coded_size: FrameSize,
        visible_rect: FrameRect,
        display_size: FrameSize,
        format: PixelFormat,
        backing: FrameBacking,
        color: FrameColorInfo,
    ) -> MediaResult<Self> {
        validate_geometry(coded_size, visible_rect, display_size, format)?;
        match &backing {
            FrameBacking::Cpu(planes) => {
                let layout = plane_layout(coded_size, format)?;
                if planes.len() != layout.len() {
                    return Err(MediaError::invalid_input(
                        "pixel format and plane count disagree",
                    ));
                }
                for (plane, (row_bytes, rows)) in planes.iter().zip(layout) {
                    if plane.stride < row_bytes {
                        return Err(MediaError::invalid_input("frame plane stride is too small"));
                    }
                    let end = (plane.stride as usize)
                        .checked_mul(rows as usize - 1)
                        .and_then(|value| value.checked_add(row_bytes as usize))
                        .and_then(|value| value.checked_add(plane.offset))
                        .ok_or_else(|| MediaError::invalid_input("frame plane layout overflow"))?;
                    if end > plane.bytes.len() {
                        return Err(MediaError::invalid_input("frame plane bytes are too short"));
                    }
                }
            }
            #[cfg(target_os = "linux")]
            FrameBacking::DmaBuf(image) => image.validate(coded_size, format)?,
            #[cfg(target_os = "macos")]
            FrameBacking::CoreVideo(buffer) => buffer.validate(coded_size, format)?,
        }
        Ok(Self {
            handle,
            sequence,
            coded_size,
            visible_rect,
            display_size,
            format,
            backing,
            color,
        })
    }

    pub fn rgba(
        handle: FrameHandle,
        sequence: u64,
        size: FrameSize,
        bytes: impl Into<Arc<[u8]>>,
        stride: u32,
    ) -> MediaResult<Self> {
        Self::new(
            handle,
            sequence,
            size,
            FrameRect {
                origin: FramePoint::default(),
                size,
            },
            size,
            PixelFormat::Rgba8,
            [FramePlane::new(bytes, stride)],
            FrameColorInfo::default(),
        )
    }

    pub fn bgra(
        handle: FrameHandle,
        sequence: u64,
        size: FrameSize,
        bytes: impl Into<Arc<[u8]>>,
        stride: u32,
    ) -> MediaResult<Self> {
        Self::new(
            handle,
            sequence,
            size,
            FrameRect {
                origin: FramePoint::default(),
                size,
            },
            size,
            PixelFormat::Bgra8,
            [FramePlane::new(bytes, stride)],
            FrameColorInfo::default(),
        )
    }

    pub fn handle(&self) -> FrameHandle {
        self.handle
    }
    pub fn sequence(&self) -> u64 {
        self.sequence
    }
    pub fn coded_size(&self) -> FrameSize {
        self.coded_size
    }
    pub fn visible_rect(&self) -> FrameRect {
        self.visible_rect
    }
    pub fn display_size(&self) -> FrameSize {
        self.display_size
    }
    pub fn format(&self) -> PixelFormat {
        self.format
    }
    pub fn color(&self) -> FrameColorInfo {
        self.color
    }
    pub fn backing(&self) -> &FrameBacking {
        &self.backing
    }
}

fn validate_geometry(
    coded: FrameSize,
    rect: FrameRect,
    display: FrameSize,
    format: PixelFormat,
) -> MediaResult<()> {
    if coded.width <= 0
        || coded.height <= 0
        || display.width <= 0
        || display.height <= 0
        || rect.size.width <= 0
        || rect.size.height <= 0
        || rect.origin.x < 0
        || rect.origin.y < 0
        || i64::from(rect.origin.x) + i64::from(rect.size.width) > i64::from(coded.width)
        || i64::from(rect.origin.y) + i64::from(rect.size.height) > i64::from(coded.height)
    {
        return Err(MediaError::invalid_input(
            "invalid frame dimensions or visible rectangle",
        ));
    }
    if format == PixelFormat::Nv12 && (rect.origin.x % 2 != 0 || rect.origin.y % 2 != 0) {
        return Err(MediaError::invalid_input("NV12 crop origin must be even"));
    }
    Ok(())
}

fn plane_layout(size: FrameSize, format: PixelFormat) -> MediaResult<Vec<(u32, u32)>> {
    let width = size.width as u32;
    let height = size.height as u32;
    match format {
        PixelFormat::Bgra8 | PixelFormat::Rgba8 => Ok(vec![(
            width
                .checked_mul(4)
                .ok_or_else(|| MediaError::invalid_input("frame row size overflow"))?,
            height,
        )]),
        PixelFormat::Nv12 => Ok(vec![
            (width, height),
            (width.div_ceil(2) * 2, height.div_ceil(2)),
        ]),
    }
}
