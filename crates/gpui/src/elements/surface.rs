use crate::{
    App, Bounds, DefiniteLength, DevicePixels, Element, ElementId, GlobalElementId, Hitbox,
    InspectorElementId, InteractiveElement, Interactivity, IntoElement, LayoutId, Length,
    ObjectFit, Pixels, Size, StyleRefinement, Styled, Window, px,
};
#[cfg(target_os = "macos")]
use core_video::pixel_buffer::CVPixelBuffer;
use smallvec::{SmallVec, smallvec};
use std::{
    fmt,
    sync::{
        Arc, Weak,
        atomic::{AtomicU64, Ordering},
    },
};
#[cfg(target_os = "linux")]
use std::{
    io,
    os::fd::{AsRawFd, OwnedFd},
};
use thiserror::Error;

/// A stable identifier for a dynamic surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SurfaceId(u64);

/// A stable identifier for one Linux DMA-BUF allocation.
#[cfg(target_os = "linux")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DmaBufId(u64);

/// Identifies one dynamic surface for the lifetime of a stream.
///
/// Reuse the same handle for every frame produced by a decoder. Renderers use
/// the handle to reuse GPU textures while [`SurfaceFrame::sequence`] changes.
#[derive(Clone, Debug)]
pub struct SurfaceHandle {
    inner: Arc<SurfaceHandleInner>,
}

#[derive(Debug)]
struct SurfaceHandleInner {
    id: SurfaceId,
}

/// A non-owning reference used by renderers to release cached GPU resources.
#[derive(Clone, Debug)]
pub struct WeakSurfaceHandle {
    inner: Weak<SurfaceHandleInner>,
}

impl SurfaceHandle {
    /// Creates a handle for a new dynamic surface.
    pub fn new() -> Self {
        static NEXT_SURFACE_ID: AtomicU64 = AtomicU64::new(1);

        Self {
            inner: Arc::new(SurfaceHandleInner {
                id: SurfaceId(NEXT_SURFACE_ID.fetch_add(1, Ordering::Relaxed)),
            }),
        }
    }

    /// Returns the stable identity used by renderer caches.
    pub fn id(&self) -> SurfaceId {
        self.inner.id
    }

    /// Creates a weak handle that does not keep the video stream alive.
    pub fn downgrade(&self) -> WeakSurfaceHandle {
        WeakSurfaceHandle {
            inner: Arc::downgrade(&self.inner),
        }
    }
}

impl Default for SurfaceHandle {
    fn default() -> Self {
        Self::new()
    }
}

impl WeakSurfaceHandle {
    /// Returns whether the owning [`SurfaceHandle`] still exists.
    pub fn is_alive(&self) -> bool {
        self.inner.strong_count() != 0
    }
}

/// One image plane stored in a Linux DMA-BUF object.
#[cfg(target_os = "linux")]
#[derive(Debug)]
pub struct DmaBufPlane {
    fd: OwnedFd,
    drm_modifier: u64,
    offset: u64,
    stride: u32,
}

#[cfg(target_os = "linux")]
impl DmaBufPlane {
    /// Describes one plane exported by a Linux graphics or video API.
    pub fn new(fd: OwnedFd, drm_modifier: u64, offset: u64, stride: u32) -> Self {
        Self {
            fd,
            drm_modifier,
            offset,
            stride,
        }
    }

    /// Returns the DRM format modifier supplied by the allocator.
    pub fn drm_modifier(&self) -> u64 {
        self.drm_modifier
    }

    /// Returns the first pixel's byte offset in the DMA-BUF object.
    pub fn offset(&self) -> u64 {
        self.offset
    }

    /// Returns the number of bytes between adjacent rows.
    pub fn stride(&self) -> u32 {
        self.stride
    }

    /// Duplicates the file descriptor for an importing graphics API.
    pub fn try_clone_fd(&self) -> io::Result<OwnedFd> {
        self.fd.try_clone()
    }
}

/// An owned reference to one Linux DMA-BUF video allocation.
///
/// Decoder buffer pools should create one handle per pool slot and reuse it
/// for every frame backed by that slot. The renderer uses this identity to
/// import each allocation into Vulkan only once.
#[cfg(target_os = "linux")]
#[derive(Clone, Debug)]
pub struct DmaBufHandle {
    inner: Arc<DmaBufHandleInner>,
}

#[cfg(target_os = "linux")]
struct DmaBufHandleInner {
    id: DmaBufId,
    coded_size: Size<DevicePixels>,
    format: SurfaceFormat,
    planes: SmallVec<[DmaBufPlane; 2]>,
    lifetime_guard: Option<Arc<dyn Send + Sync>>,
}

#[cfg(target_os = "linux")]
impl fmt::Debug for DmaBufHandleInner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DmaBufHandleInner")
            .field("id", &self.id)
            .field("coded_size", &self.coded_size)
            .field("format", &self.format)
            .field("planes", &self.planes)
            .field("has_lifetime_guard", &self.lifetime_guard.is_some())
            .finish()
    }
}

#[cfg(target_os = "linux")]
struct DmaBufAcquireFence {
    fd: parking_lot::Mutex<Option<OwnedFd>>,
}

#[cfg(target_os = "linux")]
impl fmt::Debug for DmaBufAcquireFence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DmaBufAcquireFence")
            .field("pending", &self.fd.lock().is_some())
            .finish()
    }
}

/// A non-owning reference used to discard an imported DMA-BUF texture after
/// the decoder pool releases the allocation.
#[cfg(target_os = "linux")]
#[derive(Clone, Debug)]
pub struct WeakDmaBufHandle {
    inner: Weak<DmaBufHandleInner>,
}

#[cfg(target_os = "linux")]
impl DmaBufHandle {
    /// Creates a handle for a single-plane BGRA or RGBA DMA-BUF allocation.
    ///
    /// # Safety
    ///
    /// `fd` must refer to a valid DMA-BUF whose size, pixel format, DRM
    /// modifier, byte offset, and row stride match the supplied arguments.
    /// The producer must finish writing before publishing a frame and must not
    /// write the allocation while GPUI may be sampling it.
    pub unsafe fn new(
        fd: OwnedFd,
        coded_size: Size<DevicePixels>,
        format: SurfaceFormat,
        drm_modifier: u64,
        offset: u64,
        stride: u32,
    ) -> Result<Self, SurfaceFrameError> {
        Self::new_inner(
            coded_size,
            format,
            [DmaBufPlane::new(fd, drm_modifier, offset, stride)],
            None,
        )
    }

    /// Creates a DMA-BUF handle that retains a decoder-owned resource until
    /// GPUI's GPU submission has finished sampling the frame.
    ///
    /// Use this for decoder buffer pools whose slot becomes reusable when an
    /// `Arc`-backed frame or lease object is dropped.
    ///
    /// # Safety
    ///
    /// The DMA-BUF arguments have the same requirements as [`Self::new`]. The
    /// lifetime guard must keep that allocation unavailable for producer writes.
    #[allow(clippy::too_many_arguments)]
    pub unsafe fn new_with_lifetime_guard(
        fd: OwnedFd,
        coded_size: Size<DevicePixels>,
        format: SurfaceFormat,
        drm_modifier: u64,
        offset: u64,
        stride: u32,
        lifetime_guard: Arc<dyn Send + Sync>,
    ) -> Result<Self, SurfaceFrameError> {
        Self::new_inner(
            coded_size,
            format,
            [DmaBufPlane::new(fd, drm_modifier, offset, stride)],
            Some(lifetime_guard),
        )
    }

    /// Creates a handle for a two-plane NV12 DMA-BUF allocation.
    ///
    /// `y_plane` describes the full-resolution `R8` luma plane and `uv_plane`
    /// describes the half-resolution `RG8` chroma plane. Their descriptors may
    /// contain duplicate descriptors for one DMA-BUF object or descriptors for
    /// two separate objects.
    ///
    /// # Safety
    ///
    /// Both descriptors must refer to the stated NV12 allocation and their
    /// modifiers, offsets, and strides must match the producer's layout. The
    /// producer must not overwrite either plane while GPUI may sample it.
    pub unsafe fn new_nv12(
        coded_size: Size<DevicePixels>,
        y_plane: DmaBufPlane,
        uv_plane: DmaBufPlane,
    ) -> Result<Self, SurfaceFrameError> {
        Self::new_inner(coded_size, SurfaceFormat::Nv12, [y_plane, uv_plane], None)
    }

    /// Creates a two-plane NV12 handle that retains a decoder-owned resource.
    ///
    /// # Safety
    ///
    /// The plane descriptors have the same requirements as [`Self::new_nv12`].
    /// The lifetime guard must keep the allocation unavailable for writes.
    pub unsafe fn new_nv12_with_lifetime_guard(
        coded_size: Size<DevicePixels>,
        y_plane: DmaBufPlane,
        uv_plane: DmaBufPlane,
        lifetime_guard: Arc<dyn Send + Sync>,
    ) -> Result<Self, SurfaceFrameError> {
        Self::new_inner(
            coded_size,
            SurfaceFormat::Nv12,
            [y_plane, uv_plane],
            Some(lifetime_guard),
        )
    }

    fn new_inner(
        coded_size: Size<DevicePixels>,
        format: SurfaceFormat,
        planes: impl IntoIterator<Item = DmaBufPlane>,
        lifetime_guard: Option<Arc<dyn Send + Sync>>,
    ) -> Result<Self, SurfaceFrameError> {
        static NEXT_DMA_BUF_ID: AtomicU64 = AtomicU64::new(1);

        let planes = planes.into_iter().collect::<SmallVec<[_; 2]>>();
        validate_dma_buf_layout(coded_size, format, &planes)?;

        Ok(Self {
            inner: Arc::new(DmaBufHandleInner {
                id: DmaBufId(NEXT_DMA_BUF_ID.fetch_add(1, Ordering::Relaxed)),
                coded_size,
                format,
                planes,
                lifetime_guard,
            }),
        })
    }

    /// Returns the stable allocation identity used by renderer caches.
    pub fn id(&self) -> DmaBufId {
        self.inner.id
    }

    /// Returns the allocation dimensions.
    pub fn coded_size(&self) -> Size<DevicePixels> {
        self.inner.coded_size
    }

    /// Returns the allocation's pixel format.
    pub fn format(&self) -> SurfaceFormat {
        self.inner.format
    }

    /// Returns the DRM format modifier supplied by the allocator.
    pub fn drm_modifier(&self) -> u64 {
        self.inner.planes[0].drm_modifier()
    }

    /// Returns the first pixel's byte offset in the DMA-BUF allocation.
    pub fn offset(&self) -> u64 {
        self.inner.planes[0].offset()
    }

    /// Returns the number of bytes between adjacent rows.
    pub fn stride(&self) -> u32 {
        self.inner.planes[0].stride()
    }

    /// Returns all planes in image-format order.
    pub fn planes(&self) -> &[DmaBufPlane] {
        &self.inner.planes
    }

    /// Returns one plane by image-format index.
    pub fn plane(&self, index: usize) -> Option<&DmaBufPlane> {
        self.inner.planes.get(index)
    }

    /// Duplicates the file descriptor for an importing graphics API.
    ///
    /// Vulkan consumes the returned descriptor while this handle retains the
    /// decoder pool's original descriptor.
    pub fn try_clone_fd(&self) -> io::Result<OwnedFd> {
        self.inner.planes[0].try_clone_fd()
    }

    /// Creates a weak handle that does not keep the allocation alive.
    pub fn downgrade(&self) -> WeakDmaBufHandle {
        WeakDmaBufHandle {
            inner: Arc::downgrade(&self.inner),
        }
    }
}

#[cfg(target_os = "linux")]
impl WeakDmaBufHandle {
    /// Returns whether an owning [`DmaBufHandle`] still exists.
    pub fn is_alive(&self) -> bool {
        self.inner.strong_count() != 0
    }
}

/// Pixel format used by a dynamic surface frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SurfaceFormat {
    /// Four bytes per pixel in blue, green, red, alpha order.
    Bgra8,
    /// Four bytes per pixel in red, green, blue, alpha order.
    Rgba8,
    /// One full-resolution Y plane followed by one half-resolution interleaved UV plane.
    Nv12,
}

/// The YUV conversion matrix associated with a frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum YuvMatrix {
    /// Standard-definition BT.601 coefficients.
    Bt601,
    /// High-definition BT.709 coefficients.
    #[default]
    Bt709,
}

/// The encoded numeric range associated with a frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ColorRange {
    /// Video range: 16-235 for luma and 16-240 for chroma.
    #[default]
    Limited,
    /// Full 8-bit range.
    Full,
}

/// Color metadata needed to convert an SDR YUV frame to RGB.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SurfaceColorInfo {
    /// Matrix coefficients used by an NV12 frame.
    pub matrix: YuvMatrix,
    /// Numeric range used by an NV12 frame.
    pub range: ColorRange,
}

/// One CPU-backed image plane.
#[derive(Clone)]
pub struct SurfacePlane {
    bytes: Arc<[u8]>,
    stride: u32,
}

impl SurfacePlane {
    /// Creates a plane from shared bytes and a byte stride.
    pub fn new(bytes: impl Into<Arc<[u8]>>, stride: u32) -> Self {
        Self {
            bytes: bytes.into(),
            stride,
        }
    }

    /// Returns the plane bytes.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Returns the number of bytes between adjacent rows.
    pub fn stride(&self) -> u32 {
        self.stride
    }
}

impl fmt::Debug for SurfacePlane {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SurfacePlane")
            .field("len", &self.bytes.len())
            .field("stride", &self.stride)
            .finish()
    }
}

/// An immutable video frame selected by an upper-level playback engine.
#[derive(Clone, Debug)]
pub struct SurfaceFrame {
    handle: SurfaceHandle,
    sequence: u64,
    coded_size: Size<DevicePixels>,
    visible_rect: Bounds<DevicePixels>,
    display_size: Size<DevicePixels>,
    format: SurfaceFormat,
    backing: SurfaceFrameBackingData,
    color: SurfaceColorInfo,
}

#[derive(Clone, Debug)]
enum SurfaceFrameBackingData {
    Cpu(SmallVec<[SurfacePlane; 2]>),
    #[cfg(target_os = "linux")]
    DmaBuf {
        handle: DmaBufHandle,
        acquire_fence: Option<Arc<DmaBufAcquireFence>>,
    },
}

/// The storage backing an immutable surface frame.
#[derive(Clone, Copy, Debug)]
pub enum SurfaceFrameBacking<'a> {
    /// Portable CPU memory that must be uploaded to a GPU texture.
    Cpu(&'a [SurfacePlane]),
    /// A Linux DMA-BUF sampled without copying its pixels.
    #[cfg(target_os = "linux")]
    DmaBuf(&'a DmaBufHandle),
}

/// Invalid surface frame data.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum SurfaceFrameError {
    /// A frame dimension or display dimension was zero or negative.
    #[error("surface frame dimensions must be positive")]
    InvalidSize,
    /// The visible rectangle is empty or outside the coded image.
    #[error("visible rectangle must be non-empty and contained in the coded image")]
    InvalidVisibleRect,
    /// The number of planes does not match the pixel format.
    #[error("{format:?} requires {expected} plane(s), got {actual}")]
    InvalidPlaneCount {
        /// Frame format.
        format: SurfaceFormat,
        /// Required number of planes.
        expected: usize,
        /// Supplied number of planes.
        actual: usize,
    },
    /// A plane stride is smaller than one row of active pixels.
    #[error("plane {plane} stride {stride} is smaller than the required {minimum}")]
    InvalidStride {
        /// Plane index.
        plane: usize,
        /// Supplied byte stride.
        stride: u32,
        /// Minimum byte stride.
        minimum: u32,
    },
    /// A plane does not contain all rows described by its stride.
    #[error("plane {plane} contains {actual} bytes, but at least {minimum} are required")]
    PlaneTooShort {
        /// Plane index.
        plane: usize,
        /// Supplied byte length.
        actual: usize,
        /// Minimum byte length.
        minimum: usize,
    },
    /// A plane layout cannot be represented safely on this platform.
    #[error("plane layout is too large")]
    PlaneLayoutOverflow,
    /// NV12 crop origins must align to a two-pixel chroma sample.
    #[error("NV12 visible rectangle origin must be even")]
    UnalignedNv12VisibleRect,
}

impl SurfaceFrame {
    /// Creates a validated CPU-backed surface frame.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        handle: SurfaceHandle,
        sequence: u64,
        coded_size: Size<DevicePixels>,
        visible_rect: Bounds<DevicePixels>,
        display_size: Size<DevicePixels>,
        format: SurfaceFormat,
        planes: impl IntoIterator<Item = SurfacePlane>,
        color: SurfaceColorInfo,
    ) -> Result<Self, SurfaceFrameError> {
        let planes = planes.into_iter().collect::<SmallVec<[_; 2]>>();
        validate_frame(coded_size, visible_rect, display_size, format, &planes)?;

        Ok(Self {
            handle,
            sequence,
            coded_size,
            visible_rect,
            display_size,
            format,
            backing: SurfaceFrameBackingData::Cpu(planes),
            color,
        })
    }

    /// Creates a validated frame backed by a Linux DMA-BUF allocation.
    ///
    /// The same [`DmaBufHandle`] should be reused whenever a decoder recycles
    /// the same pool allocation. This avoids a Vulkan import on every frame.
    #[cfg(target_os = "linux")]
    #[allow(clippy::too_many_arguments)]
    pub fn from_dma_buf(
        handle: SurfaceHandle,
        sequence: u64,
        visible_rect: Bounds<DevicePixels>,
        display_size: Size<DevicePixels>,
        dma_buf: DmaBufHandle,
    ) -> Result<Self, SurfaceFrameError> {
        Self::from_dma_buf_inner(
            handle,
            sequence,
            visible_rect,
            display_size,
            dma_buf,
            SurfaceColorInfo::default(),
            None,
        )
    }

    /// Creates a DMA-BUF frame with explicit YUV color metadata.
    #[cfg(target_os = "linux")]
    #[allow(clippy::too_many_arguments)]
    pub fn from_dma_buf_with_color(
        handle: SurfaceHandle,
        sequence: u64,
        visible_rect: Bounds<DevicePixels>,
        display_size: Size<DevicePixels>,
        dma_buf: DmaBufHandle,
        color: SurfaceColorInfo,
    ) -> Result<Self, SurfaceFrameError> {
        Self::from_dma_buf_inner(
            handle,
            sequence,
            visible_rect,
            display_size,
            dma_buf,
            color,
            None,
        )
    }

    /// Creates a DMA-BUF frame with a one-shot Linux `sync_file` acquire fence.
    ///
    /// GPUI waits for the fence before sampling this frame. Clones of the frame
    /// share the same fence state, so the descriptor is waited and closed once.
    #[cfg(target_os = "linux")]
    #[allow(clippy::too_many_arguments)]
    pub fn from_dma_buf_with_acquire_fence(
        handle: SurfaceHandle,
        sequence: u64,
        visible_rect: Bounds<DevicePixels>,
        display_size: Size<DevicePixels>,
        dma_buf: DmaBufHandle,
        color: SurfaceColorInfo,
        acquire_fence: OwnedFd,
    ) -> Result<Self, SurfaceFrameError> {
        Self::from_dma_buf_inner(
            handle,
            sequence,
            visible_rect,
            display_size,
            dma_buf,
            color,
            Some(acquire_fence),
        )
    }

    #[cfg(target_os = "linux")]
    #[allow(clippy::too_many_arguments)]
    fn from_dma_buf_inner(
        handle: SurfaceHandle,
        sequence: u64,
        visible_rect: Bounds<DevicePixels>,
        display_size: Size<DevicePixels>,
        dma_buf: DmaBufHandle,
        color: SurfaceColorInfo,
        acquire_fence: Option<OwnedFd>,
    ) -> Result<Self, SurfaceFrameError> {
        let coded_size = dma_buf.coded_size();
        let format = dma_buf.format();
        validate_frame_geometry(coded_size, visible_rect, display_size)?;
        if format == SurfaceFormat::Nv12
            && (visible_rect.origin.x.0 % 2 != 0 || visible_rect.origin.y.0 % 2 != 0)
        {
            return Err(SurfaceFrameError::UnalignedNv12VisibleRect);
        }

        Ok(Self {
            handle,
            sequence,
            coded_size,
            visible_rect,
            display_size,
            format,
            backing: SurfaceFrameBackingData::DmaBuf {
                handle: dma_buf,
                acquire_fence: acquire_fence.map(|fd| {
                    Arc::new(DmaBufAcquireFence {
                        fd: parking_lot::Mutex::new(Some(fd)),
                    })
                }),
            },
            color,
        })
    }

    /// Creates a tightly or loosely packed BGRA frame whose entire coded area is visible.
    pub fn bgra(
        handle: SurfaceHandle,
        sequence: u64,
        size: Size<DevicePixels>,
        bytes: impl Into<Arc<[u8]>>,
        stride: u32,
    ) -> Result<Self, SurfaceFrameError> {
        Self::new(
            handle,
            sequence,
            size,
            Bounds {
                origin: Default::default(),
                size,
            },
            size,
            SurfaceFormat::Bgra8,
            [SurfacePlane::new(bytes, stride)],
            SurfaceColorInfo::default(),
        )
    }

    /// Creates a tightly or loosely packed RGBA frame whose entire coded area is visible.
    pub fn rgba(
        handle: SurfaceHandle,
        sequence: u64,
        size: Size<DevicePixels>,
        bytes: impl Into<Arc<[u8]>>,
        stride: u32,
    ) -> Result<Self, SurfaceFrameError> {
        Self::new(
            handle,
            sequence,
            size,
            Bounds {
                origin: Default::default(),
                size,
            },
            size,
            SurfaceFormat::Rgba8,
            [SurfacePlane::new(bytes, stride)],
            SurfaceColorInfo::default(),
        )
    }

    /// Creates a validated NV12 frame whose entire coded area is visible.
    #[allow(clippy::too_many_arguments)]
    pub fn nv12(
        handle: SurfaceHandle,
        sequence: u64,
        size: Size<DevicePixels>,
        y_bytes: impl Into<Arc<[u8]>>,
        y_stride: u32,
        uv_bytes: impl Into<Arc<[u8]>>,
        uv_stride: u32,
        color: SurfaceColorInfo,
    ) -> Result<Self, SurfaceFrameError> {
        Self::new(
            handle,
            sequence,
            size,
            Bounds {
                origin: Default::default(),
                size,
            },
            size,
            SurfaceFormat::Nv12,
            [
                SurfacePlane::new(y_bytes, y_stride),
                SurfacePlane::new(uv_bytes, uv_stride),
            ],
            color,
        )
    }

    /// Returns the stream handle.
    pub fn handle(&self) -> &SurfaceHandle {
        &self.handle
    }

    /// Returns the content revision. A new selected frame must use a different sequence.
    pub fn sequence(&self) -> u64 {
        self.sequence
    }

    /// Returns the allocation size of the planes.
    pub fn coded_size(&self) -> Size<DevicePixels> {
        self.coded_size
    }

    /// Returns the portion of the coded frame that contains displayable pixels.
    pub fn visible_rect(&self) -> Bounds<DevicePixels> {
        self.visible_rect
    }

    /// Returns the intended display size used for aspect-ratio calculations.
    pub fn display_size(&self) -> Size<DevicePixels> {
        self.display_size
    }

    /// Returns the pixel format.
    pub fn format(&self) -> SurfaceFormat {
        self.format
    }

    /// Returns the storage backing this frame.
    pub fn backing(&self) -> SurfaceFrameBacking<'_> {
        match &self.backing {
            SurfaceFrameBackingData::Cpu(planes) => SurfaceFrameBacking::Cpu(planes),
            #[cfg(target_os = "linux")]
            SurfaceFrameBackingData::DmaBuf { handle, .. } => SurfaceFrameBacking::DmaBuf(handle),
        }
    }

    /// Returns the CPU planes, or `None` for a zero-copy native frame.
    pub fn cpu_planes(&self) -> Option<&[SurfacePlane]> {
        match &self.backing {
            SurfaceFrameBackingData::Cpu(planes) => Some(planes),
            #[cfg(target_os = "linux")]
            SurfaceFrameBackingData::DmaBuf { .. } => None,
        }
    }

    /// Returns the frame's SDR color metadata.
    pub fn color(&self) -> SurfaceColorInfo {
        self.color
    }

    /// Waits for this native frame's one-shot acquire fence, if present.
    ///
    /// This is idempotent across clones of the frame. It blocks only until the
    /// producer signals that its writes to the DMA-BUF are complete.
    #[cfg(target_os = "linux")]
    pub fn wait_for_dma_buf_acquire_fence(&self) -> io::Result<()> {
        let SurfaceFrameBackingData::DmaBuf {
            acquire_fence: Some(acquire_fence),
            ..
        } = &self.backing
        else {
            return Ok(());
        };

        let mut fence = acquire_fence.fd.lock();
        let Some(fd) = fence.as_ref() else {
            return Ok(());
        };
        let mut poll_fd = libc::pollfd {
            fd: fd.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        loop {
            let result = unsafe { libc::poll(&mut poll_fd, 1, -1) };
            if result > 0 {
                if poll_fd.revents & libc::POLLIN != 0 {
                    fence.take();
                    return Ok(());
                }
                return Err(io::Error::other(format!(
                    "acquire fence poll returned events {:#x}",
                    poll_fd.revents
                )));
            }
            if result == 0 {
                continue;
            }
            let error = io::Error::last_os_error();
            if error.kind() != io::ErrorKind::Interrupted {
                return Err(error);
            }
        }
    }
}

fn validate_frame(
    coded_size: Size<DevicePixels>,
    visible_rect: Bounds<DevicePixels>,
    display_size: Size<DevicePixels>,
    format: SurfaceFormat,
    planes: &[SurfacePlane],
) -> Result<(), SurfaceFrameError> {
    validate_frame_geometry(coded_size, visible_rect, display_size)?;

    let coded_width = coded_size.width.0;
    let coded_height = coded_size.height.0;

    let expected_planes = match format {
        SurfaceFormat::Bgra8 | SurfaceFormat::Rgba8 => 1,
        SurfaceFormat::Nv12 => 2,
    };
    if planes.len() != expected_planes {
        return Err(SurfaceFrameError::InvalidPlaneCount {
            format,
            expected: expected_planes,
            actual: planes.len(),
        });
    }

    if format == SurfaceFormat::Nv12
        && (visible_rect.origin.x.0 % 2 != 0 || visible_rect.origin.y.0 % 2 != 0)
    {
        return Err(SurfaceFrameError::UnalignedNv12VisibleRect);
    }

    let width = coded_width as u32;
    let height = coded_height as u32;
    let plane_shapes: SmallVec<[(u32, u32); 2]> = match format {
        SurfaceFormat::Bgra8 | SurfaceFormat::Rgba8 => smallvec![(
            width
                .checked_mul(4)
                .ok_or(SurfaceFrameError::PlaneLayoutOverflow)?,
            height
        )],
        SurfaceFormat::Nv12 => {
            smallvec![(width, height), (width.div_ceil(2) * 2, height.div_ceil(2))]
        }
    };

    for (plane_index, (plane, (minimum_stride, rows))) in
        planes.iter().zip(plane_shapes).enumerate()
    {
        if plane.stride < minimum_stride {
            return Err(SurfaceFrameError::InvalidStride {
                plane: plane_index,
                stride: plane.stride,
                minimum: minimum_stride,
            });
        }
        let minimum_len = (rows.saturating_sub(1) as usize)
            .checked_mul(plane.stride as usize)
            .and_then(|preceding_rows| preceding_rows.checked_add(minimum_stride as usize))
            .ok_or(SurfaceFrameError::PlaneLayoutOverflow)?;
        if plane.bytes.len() < minimum_len {
            return Err(SurfaceFrameError::PlaneTooShort {
                plane: plane_index,
                actual: plane.bytes.len(),
                minimum: minimum_len,
            });
        }
    }

    Ok(())
}

fn validate_frame_geometry(
    coded_size: Size<DevicePixels>,
    visible_rect: Bounds<DevicePixels>,
    display_size: Size<DevicePixels>,
) -> Result<(), SurfaceFrameError> {
    let coded_width = coded_size.width.0;
    let coded_height = coded_size.height.0;
    if coded_width <= 0
        || coded_height <= 0
        || display_size.width.0 <= 0
        || display_size.height.0 <= 0
    {
        return Err(SurfaceFrameError::InvalidSize);
    }

    let visible = visible_rect;
    if visible.size.width.0 <= 0
        || visible.size.height.0 <= 0
        || visible.origin.x.0 < 0
        || visible.origin.y.0 < 0
        || visible.origin.x.0 + visible.size.width.0 > coded_width
        || visible.origin.y.0 + visible.size.height.0 > coded_height
    {
        return Err(SurfaceFrameError::InvalidVisibleRect);
    }

    Ok(())
}

#[cfg(target_os = "linux")]
fn validate_dma_buf_layout(
    coded_size: Size<DevicePixels>,
    format: SurfaceFormat,
    planes: &[DmaBufPlane],
) -> Result<(), SurfaceFrameError> {
    if coded_size.width.0 <= 0 || coded_size.height.0 <= 0 {
        return Err(SurfaceFrameError::InvalidSize);
    }
    let width = coded_size.width.0 as u32;
    let expected_planes = match format {
        SurfaceFormat::Bgra8 | SurfaceFormat::Rgba8 => 1,
        SurfaceFormat::Nv12 => 2,
    };
    if planes.len() != expected_planes {
        return Err(SurfaceFrameError::InvalidPlaneCount {
            format,
            expected: expected_planes,
            actual: planes.len(),
        });
    }
    let minimum_strides: SmallVec<[u32; 2]> = match format {
        SurfaceFormat::Bgra8 | SurfaceFormat::Rgba8 => smallvec![
            width
                .checked_mul(4)
                .ok_or(SurfaceFrameError::PlaneLayoutOverflow)?
        ],
        SurfaceFormat::Nv12 => smallvec![width, width.div_ceil(2) * 2],
    };
    for (plane_index, (plane, minimum_stride)) in planes.iter().zip(minimum_strides).enumerate() {
        if plane.stride() < minimum_stride {
            return Err(SurfaceFrameError::InvalidStride {
                plane: plane_index,
                stride: plane.stride(),
                minimum: minimum_stride,
            });
        }
    }

    Ok(())
}

/// A source of a surface's content.
#[derive(Clone, Debug)]
pub enum SurfaceSource {
    /// A portable CPU-backed frame.
    Frame(Arc<SurfaceFrame>),
    /// A macOS image buffer from CoreVideo.
    #[cfg(target_os = "macos")]
    Surface(CVPixelBuffer),
}

impl From<SurfaceFrame> for SurfaceSource {
    fn from(value: SurfaceFrame) -> Self {
        Self::Frame(Arc::new(value))
    }
}

impl From<Arc<SurfaceFrame>> for SurfaceSource {
    fn from(value: Arc<SurfaceFrame>) -> Self {
        Self::Frame(value)
    }
}

#[cfg(target_os = "macos")]
impl From<CVPixelBuffer> for SurfaceSource {
    fn from(value: CVPixelBuffer) -> Self {
        SurfaceSource::Surface(value)
    }
}

impl SurfaceSource {
    /// Returns the portable frame when this source is CPU-backed.
    pub fn frame(&self) -> Option<&Arc<SurfaceFrame>> {
        match self {
            Self::Frame(frame) => Some(frame),
            #[cfg(target_os = "macos")]
            Self::Surface(_) => None,
        }
    }

    fn display_size(&self) -> Size<DevicePixels> {
        match self {
            Self::Frame(frame) => frame.display_size(),
            #[cfg(target_os = "macos")]
            Self::Surface(surface) => {
                crate::size(surface.get_width().into(), surface.get_height().into())
            }
        }
    }
}

/// A dynamic surface element.
pub struct Surface {
    source: SurfaceSource,
    object_fit: ObjectFit,
    interactivity: Interactivity,
}

/// Creates a dynamic surface element.
#[track_caller]
pub fn surface(source: impl Into<SurfaceSource>) -> Surface {
    Surface {
        source: source.into(),
        object_fit: ObjectFit::Contain,
        interactivity: Interactivity::new(),
    }
}

impl Surface {
    /// Sets how the frame is fitted into the element bounds.
    pub fn object_fit(mut self, object_fit: ObjectFit) -> Self {
        self.object_fit = object_fit;
        self
    }
}

impl Element for Surface {
    type RequestLayoutState = ();
    type PrepaintState = Option<Hitbox>;

    fn id(&self) -> Option<ElementId> {
        self.interactivity.element_id.clone()
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        self.interactivity.source_location()
    }

    fn request_layout(
        &mut self,
        global_id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let display_size = self.source.display_size().map(|value| px(value.0 as f32));
        let layout_id = self.interactivity.request_layout(
            global_id,
            inspector_id,
            window,
            cx,
            |mut style, window, cx| {
                style.aspect_ratio = Some(display_size.width / display_size.height);
                if let Length::Auto = style.size.width {
                    style.size.width = match style.size.height {
                        Length::Definite(DefiniteLength::Absolute(height)) => {
                            let height = height.to_pixels(window.rem_size());
                            Length::Definite(
                                px(display_size.width.0 * height.0 / display_size.height.0).into(),
                            )
                        }
                        _ => Length::Definite(display_size.width.into()),
                    };
                }
                if let Length::Auto = style.size.height {
                    style.size.height = match style.size.width {
                        Length::Definite(DefiniteLength::Absolute(width)) => {
                            let width = width.to_pixels(window.rem_size());
                            Length::Definite(
                                px(display_size.height.0 * width.0 / display_size.width.0).into(),
                            )
                        }
                        _ => Length::Definite(display_size.height.into()),
                    };
                }
                window.request_layout(style, [], cx)
            },
        );
        (layout_id, ())
    }

    fn prepaint(
        &mut self,
        global_id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        self.interactivity.prepaint(
            global_id,
            inspector_id,
            bounds,
            bounds.size,
            window,
            cx,
            |_, _, hitbox, _, _| hitbox,
        )
    }

    fn paint(
        &mut self,
        global_id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        hitbox: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let source = self.source.clone();
        let display_size = source.display_size();
        self.interactivity.paint(
            global_id,
            inspector_id,
            bounds,
            hitbox.as_ref(),
            window,
            cx,
            |style, window, _| {
                let fitted_bounds = self.object_fit.get_bounds(bounds, display_size);
                let painted_bounds = fitted_bounds.intersect(&bounds);
                let corner_radii = style
                    .corner_radii
                    .to_pixels(window.rem_size())
                    .clamp_radii_for_quad_size(painted_bounds.size);
                window.paint_surface(fitted_bounds, painted_bounds, corner_radii, source);
            },
        );
    }
}

impl IntoElement for Surface {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Styled for Surface {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.interactivity.base_style
    }
}

impl InteractiveElement for Surface {
    fn interactivity(&mut self) -> &mut Interactivity {
        &mut self.interactivity
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{bounds, point, size};

    #[test]
    fn validates_bgra_stride_and_length() {
        let handle = SurfaceHandle::new();
        let size = size(DevicePixels(4), DevicePixels(2));

        assert!(matches!(
            SurfaceFrame::bgra(handle.clone(), 0, size, vec![0; 32], 15),
            Err(SurfaceFrameError::InvalidStride { .. })
        ));
        assert!(matches!(
            SurfaceFrame::bgra(handle, 0, size, vec![0; 31], 16),
            Err(SurfaceFrameError::PlaneTooShort { .. })
        ));
    }

    #[test]
    fn accepts_non_tightly_packed_nv12() {
        let size = size(DevicePixels(4), DevicePixels(4));
        let frame = SurfaceFrame::nv12(
            SurfaceHandle::new(),
            7,
            size,
            vec![0; 8 * 4],
            8,
            vec![0; 8 * 2],
            8,
            SurfaceColorInfo {
                matrix: YuvMatrix::Bt709,
                range: ColorRange::Limited,
            },
        )
        .unwrap();

        assert_eq!(frame.sequence(), 7);
        assert_eq!(frame.format(), SurfaceFormat::Nv12);
        assert_eq!(frame.cpu_planes().unwrap()[1].stride(), 8);
    }

    #[test]
    fn rejects_invalid_or_unaligned_visible_rect() {
        let coded_size = size(DevicePixels(8), DevicePixels(8));
        let display_size = size(DevicePixels(6), DevicePixels(6));
        let planes = [
            SurfacePlane::new(vec![0; 64], 8),
            SurfacePlane::new(vec![0; 32], 8),
        ];

        let result = SurfaceFrame::new(
            SurfaceHandle::new(),
            0,
            coded_size,
            bounds(
                point(DevicePixels(1), DevicePixels(0)),
                size(DevicePixels(6), DevicePixels(6)),
            ),
            display_size,
            SurfaceFormat::Nv12,
            planes,
            SurfaceColorInfo::default(),
        );
        assert_eq!(
            result.unwrap_err(),
            SurfaceFrameError::UnalignedNv12VisibleRect
        );
    }

    #[test]
    fn weak_handle_expires_with_stream() {
        let handle = SurfaceHandle::new();
        let weak = handle.downgrade();
        assert!(weak.is_alive());
        drop(handle);
        assert!(!weak.is_alive());
    }

    #[cfg(target_os = "linux")]
    fn test_dma_buf(
        coded_size: Size<DevicePixels>,
        format: SurfaceFormat,
        stride: u32,
    ) -> Result<DmaBufHandle, SurfaceFrameError> {
        let fd: OwnedFd = std::fs::File::open("/dev/null").unwrap().into();
        // This test exercises metadata and ownership only; the descriptor is never imported.
        unsafe { DmaBufHandle::new(fd, coded_size, format, 0, 64, stride) }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn dma_buf_frame_preserves_allocation_identity() {
        let coded_size = size(DevicePixels(8), DevicePixels(4));
        let dma_buf = test_dma_buf(coded_size, SurfaceFormat::Bgra8, 32).unwrap();
        let id = dma_buf.id();
        let weak = dma_buf.downgrade();
        let imported_fd = dma_buf.try_clone_fd().unwrap();
        let frame = SurfaceFrame::from_dma_buf(
            SurfaceHandle::new(),
            11,
            bounds(Default::default(), coded_size),
            coded_size,
            dma_buf,
        )
        .unwrap();

        assert!(frame.cpu_planes().is_none());
        assert!(matches!(
            frame.backing(),
            SurfaceFrameBacking::DmaBuf(buffer) if buffer.id() == id
        ));
        assert!(weak.is_alive());
        drop(frame);
        assert!(!weak.is_alive());
        assert!(std::fs::File::from(imported_fd).metadata().is_ok());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn dma_buf_frame_accepts_nv12_planes_and_rejects_short_strides() {
        let coded_size = size(DevicePixels(8), DevicePixels(4));
        let y_fd: OwnedFd = std::fs::File::open("/dev/null").unwrap().into();
        let uv_fd: OwnedFd = std::fs::File::open("/dev/null").unwrap().into();
        let dma_buf = unsafe {
            DmaBufHandle::new_nv12(
                coded_size,
                DmaBufPlane::new(y_fd, 0, 0, 8),
                DmaBufPlane::new(uv_fd, 0, 32, 8),
            )
        }
        .unwrap();
        let color = SurfaceColorInfo {
            matrix: YuvMatrix::Bt601,
            range: ColorRange::Full,
        };
        let frame = SurfaceFrame::from_dma_buf_with_color(
            SurfaceHandle::new(),
            12,
            bounds(Default::default(), coded_size),
            coded_size,
            dma_buf,
            color,
        )
        .unwrap();

        assert_eq!(frame.format(), SurfaceFormat::Nv12);
        assert_eq!(frame.color(), color);
        let SurfaceFrameBacking::DmaBuf(dma_buf) = frame.backing() else {
            panic!("expected DMA-BUF backing");
        };
        assert_eq!(dma_buf.planes().len(), 2);
        assert_eq!(dma_buf.plane(1).unwrap().offset(), 32);

        let short_stride = test_dma_buf(coded_size, SurfaceFormat::Rgba8, 31);
        assert!(matches!(
            short_stride,
            Err(SurfaceFrameError::InvalidStride {
                plane: 0,
                stride: 31,
                minimum: 32,
            })
        ));

        let y_fd: OwnedFd = std::fs::File::open("/dev/null").unwrap().into();
        let uv_fd: OwnedFd = std::fs::File::open("/dev/null").unwrap().into();
        let short_uv_stride = unsafe {
            DmaBufHandle::new_nv12(
                coded_size,
                DmaBufPlane::new(y_fd, 0, 0, 8),
                DmaBufPlane::new(uv_fd, 0, 32, 7),
            )
        };
        assert!(matches!(
            short_uv_stride,
            Err(SurfaceFrameError::InvalidStride {
                plane: 1,
                stride: 7,
                minimum: 8,
            })
        ));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn dma_buf_acquire_fence_is_waited_once_across_frame_clones() {
        use std::os::fd::FromRawFd;

        let mut pipe_fds = [-1; 2];
        assert_eq!(
            unsafe { libc::pipe2(pipe_fds.as_mut_ptr(), libc::O_CLOEXEC) },
            0
        );
        let read_fd = unsafe { OwnedFd::from_raw_fd(pipe_fds[0]) };
        let write_fd = unsafe { OwnedFd::from_raw_fd(pipe_fds[1]) };
        let coded_size = size(DevicePixels(8), DevicePixels(4));
        let dma_buf = test_dma_buf(coded_size, SurfaceFormat::Bgra8, 32).unwrap();
        let frame = SurfaceFrame::from_dma_buf_with_acquire_fence(
            SurfaceHandle::new(),
            13,
            bounds(Default::default(), coded_size),
            coded_size,
            dma_buf,
            SurfaceColorInfo::default(),
            read_fd,
        )
        .unwrap();
        let cloned = frame.clone();
        let byte = [1_u8];
        assert_eq!(
            unsafe { libc::write(write_fd.as_raw_fd(), byte.as_ptr().cast(), byte.len()) },
            1
        );

        frame.wait_for_dma_buf_acquire_fence().unwrap();
        cloned.wait_for_dma_buf_acquire_fence().unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn dma_buf_retains_optional_lifetime_guard() {
        struct DropGuard(Arc<std::sync::atomic::AtomicBool>);

        impl Drop for DropGuard {
            fn drop(&mut self) {
                self.0.store(true, Ordering::Relaxed);
            }
        }

        let dropped = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let coded_size = size(DevicePixels(8), DevicePixels(4));
        let fd: OwnedFd = std::fs::File::open("/dev/null").unwrap().into();
        let dma_buf = unsafe {
            DmaBufHandle::new_with_lifetime_guard(
                fd,
                coded_size,
                SurfaceFormat::Bgra8,
                0,
                0,
                32,
                Arc::new(DropGuard(dropped.clone())),
            )
        }
        .unwrap();

        assert!(!dropped.load(Ordering::Relaxed));
        drop(dma_buf);
        assert!(dropped.load(Ordering::Relaxed));
    }
}
