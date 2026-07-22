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

#[cfg(target_os = "linux")]
static NEXT_DMA_BUF_ID: AtomicU64 = AtomicU64::new(1);

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

/// DRM fourcc for an 8-bit NV12 image.
#[cfg(target_os = "linux")]
pub const DRM_FORMAT_NV12: u32 =
    b'N' as u32 | (b'V' as u32) << 8 | (b'1' as u32) << 16 | (b'2' as u32) << 24;

/// Identifies a Linux DRM render device by its character-device numbers.
#[cfg(target_os = "linux")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DrmDevice {
    /// Character-device major number.
    pub major: u32,
    /// Character-device minor number.
    pub minor: u32,
}

/// One sampleable DRM modifier advertised for native NV12 Vulkan images.
#[cfg(target_os = "linux")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DmaBufModifier {
    /// Runtime DRM format modifier value.
    pub modifier: u64,
    /// Number of memory-plane layouts Vulkan requires for this modifier.
    pub plane_count: u32,
}

/// One memory object referenced by a native DMA-BUF image.
#[cfg(target_os = "linux")]
#[derive(Debug)]
pub struct DmaBufObject {
    fd: OwnedFd,
    modifier: u64,
}

#[cfg(target_os = "linux")]
impl DmaBufObject {
    /// Creates an object descriptor from an owned DMA-BUF fd and DRM modifier.
    pub fn new(fd: OwnedFd, modifier: u64) -> Self {
        Self { fd, modifier }
    }

    /// Returns this object's DRM modifier.
    pub fn modifier(&self) -> u64 {
        self.modifier
    }

    /// Duplicates the object descriptor for a graphics API import operation.
    pub fn try_clone_fd(&self) -> io::Result<OwnedFd> {
        self.fd.try_clone()
    }
}

/// Maps one image-format plane into a DMA-BUF object.
#[cfg(target_os = "linux")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DmaBufPlaneLayout {
    object_index: usize,
    offset: u64,
    stride: u32,
}

#[cfg(target_os = "linux")]
impl DmaBufPlaneLayout {
    /// Creates a plane layout referencing `objects[object_index]`.
    pub fn new(object_index: usize, offset: u64, stride: u32) -> Self {
        Self {
            object_index,
            offset,
            stride,
        }
    }

    /// Returns the index of the backing object.
    pub fn object_index(&self) -> usize {
        self.object_index
    }

    /// Returns this plane's byte offset within its object.
    pub fn offset(&self) -> u64 {
        self.offset
    }

    /// Returns the number of bytes between adjacent rows.
    pub fn stride(&self) -> u32 {
        self.stride
    }
}

/// A native DRM image preserving object ownership and per-plane layouts.
#[cfg(target_os = "linux")]
#[derive(Debug)]
pub struct DmaBufImage {
    coded_size: Size<DevicePixels>,
    drm_fourcc: u32,
    objects: Vec<DmaBufObject>,
    planes: Vec<DmaBufPlaneLayout>,
    drm_device: Option<DrmDevice>,
}

/// Runtime state of a Linux DMA-BUF import attempt.
///
/// Applications can inspect this after presenting a frame and switch to a CPU
/// fallback if the active renderer rejects its format, modifier, or device.
#[cfg(target_os = "linux")]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum DmaBufImportStatus {
    /// The renderer has not attempted this allocation yet.
    #[default]
    Pending,
    /// The allocation was imported and is available for sampling.
    Ready,
    /// The renderer rejected the allocation.
    Failed(Arc<str>),
}

#[cfg(target_os = "linux")]
impl DmaBufImage {
    /// Describes a native DRM image exactly as exported by the producer.
    pub fn new(
        coded_size: Size<DevicePixels>,
        drm_fourcc: u32,
        objects: Vec<DmaBufObject>,
        planes: Vec<DmaBufPlaneLayout>,
    ) -> Self {
        Self {
            coded_size,
            drm_fourcc,
            objects,
            planes,
            drm_device: None,
        }
    }

    /// Records the DRM render device which produced the allocation.
    pub fn with_drm_device(mut self, drm_device: DrmDevice) -> Self {
        self.drm_device = Some(drm_device);
        self
    }

    /// Returns the coded image dimensions.
    pub fn coded_size(&self) -> Size<DevicePixels> {
        self.coded_size
    }

    /// Returns the DRM fourcc describing the complete image.
    pub fn drm_fourcc(&self) -> u32 {
        self.drm_fourcc
    }

    /// Returns all memory objects referenced by the image.
    pub fn objects(&self) -> &[DmaBufObject] {
        &self.objects
    }

    /// Returns image-format plane layouts in plane order.
    pub fn planes(&self) -> &[DmaBufPlaneLayout] {
        &self.planes
    }

    /// Returns the producer DRM device, when supplied by the caller.
    pub fn drm_device(&self) -> Option<DrmDevice> {
        self.drm_device
    }
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
    storage: DmaBufStorage,
    lifetime_guard: Option<Arc<dyn Send + Sync>>,
    import_status: parking_lot::RwLock<DmaBufImportStatus>,
}

#[cfg(target_os = "linux")]
#[derive(Debug)]
enum DmaBufStorage {
    SeparatePlanes(SmallVec<[DmaBufPlane; 2]>),
    NativeImage(DmaBufImage),
}

#[cfg(target_os = "linux")]
impl fmt::Debug for DmaBufHandleInner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DmaBufHandleInner")
            .field("id", &self.id)
            .field("coded_size", &self.coded_size)
            .field("format", &self.format)
            .field("storage", &self.storage)
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

    /// Creates a handle for a native DRM image whose object-to-plane mapping
    /// must be preserved during import.
    ///
    /// # Safety
    ///
    /// Every object and plane layout must match the producer's exported DRM
    /// descriptor. The producer must not overwrite the allocation while GPUI
    /// may sample it.
    pub unsafe fn from_image(image: DmaBufImage) -> Result<Self, SurfaceFrameError> {
        Self::from_image_inner(image, None)
    }

    /// Creates a native DRM image handle retaining its decoder-owned resource.
    ///
    /// # Safety
    ///
    /// The image has the same requirements as [`Self::from_image`]. The guard
    /// must keep all referenced objects unavailable for producer writes.
    pub unsafe fn from_image_with_lifetime_guard(
        image: DmaBufImage,
        lifetime_guard: Arc<dyn Send + Sync>,
    ) -> Result<Self, SurfaceFrameError> {
        Self::from_image_inner(image, Some(lifetime_guard))
    }

    fn new_inner(
        coded_size: Size<DevicePixels>,
        format: SurfaceFormat,
        planes: impl IntoIterator<Item = DmaBufPlane>,
        lifetime_guard: Option<Arc<dyn Send + Sync>>,
    ) -> Result<Self, SurfaceFrameError> {
        let planes = planes.into_iter().collect::<SmallVec<[_; 2]>>();
        validate_dma_buf_layout(coded_size, format, &planes)?;

        Ok(Self {
            inner: Arc::new(DmaBufHandleInner {
                id: DmaBufId(NEXT_DMA_BUF_ID.fetch_add(1, Ordering::Relaxed)),
                coded_size,
                format,
                storage: DmaBufStorage::SeparatePlanes(planes),
                lifetime_guard,
                import_status: parking_lot::RwLock::new(DmaBufImportStatus::Pending),
            }),
        })
    }

    fn from_image_inner(
        image: DmaBufImage,
        lifetime_guard: Option<Arc<dyn Send + Sync>>,
    ) -> Result<Self, SurfaceFrameError> {
        let format = surface_format_from_drm_fourcc(image.drm_fourcc())?;
        validate_dma_buf_image(&image, format)?;
        let coded_size = image.coded_size();
        Ok(Self {
            inner: Arc::new(DmaBufHandleInner {
                id: DmaBufId(NEXT_DMA_BUF_ID.fetch_add(1, Ordering::Relaxed)),
                coded_size,
                format,
                storage: DmaBufStorage::NativeImage(image),
                lifetime_guard,
                import_status: parking_lot::RwLock::new(DmaBufImportStatus::Pending),
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
        match &self.inner.storage {
            DmaBufStorage::SeparatePlanes(planes) => planes[0].drm_modifier(),
            DmaBufStorage::NativeImage(image) => image.objects()[0].modifier(),
        }
    }

    /// Returns the first pixel's byte offset in the DMA-BUF allocation.
    pub fn offset(&self) -> u64 {
        match &self.inner.storage {
            DmaBufStorage::SeparatePlanes(planes) => planes[0].offset(),
            DmaBufStorage::NativeImage(image) => image.planes()[0].offset(),
        }
    }

    /// Returns the number of bytes between adjacent rows.
    pub fn stride(&self) -> u32 {
        match &self.inner.storage {
            DmaBufStorage::SeparatePlanes(planes) => planes[0].stride(),
            DmaBufStorage::NativeImage(image) => image.planes()[0].stride(),
        }
    }

    /// Returns legacy independently imported planes in image-format order.
    ///
    /// Native multi-plane images instead expose their layouts through [`Self::image`].
    pub fn planes(&self) -> &[DmaBufPlane] {
        match &self.inner.storage {
            DmaBufStorage::SeparatePlanes(planes) => planes,
            DmaBufStorage::NativeImage(_) => &[],
        }
    }

    /// Returns one plane by image-format index.
    pub fn plane(&self, index: usize) -> Option<&DmaBufPlane> {
        self.planes().get(index)
    }

    /// Returns the native object/plane image descriptor, when present.
    pub fn image(&self) -> Option<&DmaBufImage> {
        match &self.inner.storage {
            DmaBufStorage::SeparatePlanes(_) => None,
            DmaBufStorage::NativeImage(image) => Some(image),
        }
    }

    /// Returns the result of this allocation's renderer import attempt.
    pub fn import_status(&self) -> DmaBufImportStatus {
        self.inner.import_status.read().clone()
    }

    /// Records a successful renderer import.
    #[doc(hidden)]
    pub fn report_import_ready(&self) {
        *self.inner.import_status.write() = DmaBufImportStatus::Ready;
    }

    /// Records a renderer import failure for application fallback handling.
    #[doc(hidden)]
    pub fn report_import_failed(&self, error: impl Into<Arc<str>>) {
        *self.inner.import_status.write() = DmaBufImportStatus::Failed(error.into());
    }

    /// Duplicates the file descriptor for an importing graphics API.
    ///
    /// Vulkan consumes the returned descriptor while this handle retains the
    /// decoder pool's original descriptor.
    pub fn try_clone_fd(&self) -> io::Result<OwnedFd> {
        match &self.inner.storage {
            DmaBufStorage::SeparatePlanes(planes) => planes[0].try_clone_fd(),
            DmaBufStorage::NativeImage(image) => image.objects()[0].try_clone_fd(),
        }
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

/// An owned reference to a macOS CoreVideo allocation.
///
/// CoreVideo pixel buffers are reference-counted objects whose backing
/// IOSurface can be sampled by Metal without copying its pixels. The wrapper
/// makes that cross-thread ownership explicit for decoder callbacks, which
/// commonly publish frames from a GStreamer or VideoToolbox worker thread.
#[cfg(target_os = "macos")]
#[derive(Clone)]
pub struct CoreVideoHandle {
    pixel_buffer: CVPixelBuffer,
}

#[cfg(target_os = "macos")]
impl CoreVideoHandle {
    /// Retains a CoreVideo pixel buffer for use by a surface frame.
    ///
    /// # Safety
    ///
    /// The producer must not mutate the pixel buffer or its backing storage
    /// after publishing this handle. It must publish a new frame instead.
    pub unsafe fn new(pixel_buffer: CVPixelBuffer) -> Self {
        Self { pixel_buffer }
    }

    /// Returns the retained CoreVideo pixel buffer.
    ///
    /// # Safety
    ///
    /// The returned buffer may only be inspected or sampled. Its base address
    /// must not be mutated while any clone of this handle exists.
    pub unsafe fn pixel_buffer(&self) -> &CVPixelBuffer {
        &self.pixel_buffer
    }
}

#[cfg(target_os = "macos")]
impl fmt::Debug for CoreVideoHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CoreVideoHandle")
            .field("width", &self.pixel_buffer.get_width())
            .field("height", &self.pixel_buffer.get_height())
            .field("pixel_format", &self.pixel_buffer.get_pixel_format())
            .finish()
    }
}

// SAFETY: CVPixelBuffer is an immutable, reference-counted CVBuffer while it
// is held by GPUI. CoreVideo permits retaining, releasing, and using image
// buffers across threads. Producers must publish a new frame rather than
// mutate a buffer while GPUI may sample it.
#[cfg(target_os = "macos")]
unsafe impl Send for CoreVideoHandle {}

// SAFETY: See the Send implementation. GPUI exposes only shared access to the
// retained pixel buffer and never locks or mutates its base address.
#[cfg(target_os = "macos")]
unsafe impl Sync for CoreVideoHandle {}

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
    #[cfg(target_os = "macos")]
    CoreVideo(CoreVideoHandle),
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
    /// A macOS CoreVideo pixel buffer sampled through Metal.
    #[cfg(target_os = "macos")]
    CoreVideo(&'a CoreVideoHandle),
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
    /// The native DMA-BUF image format is not supported yet.
    #[cfg(target_os = "linux")]
    #[error("unsupported DMA-BUF DRM fourcc {fourcc:#010x}")]
    UnsupportedDmaBufFourcc {
        /// Runtime DRM fourcc value.
        fourcc: u32,
    },
    /// A native image does not reference any DMA-BUF memory object.
    #[cfg(target_os = "linux")]
    #[error("DMA-BUF image requires at least one memory object")]
    MissingDmaBufObjects,
    /// A plane refers to an object index not present in the image.
    #[cfg(target_os = "linux")]
    #[error("DMA-BUF plane {plane} references missing object {object_index}")]
    InvalidDmaBufObjectIndex {
        /// Plane index in image-format order.
        plane: usize,
        /// Invalid object index.
        object_index: usize,
    },
    /// Objects forming one native image disagree about its DRM modifier.
    #[cfg(target_os = "linux")]
    #[error("all DMA-BUF objects in one native image must use the same DRM modifier")]
    MismatchedDmaBufModifiers,
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

    /// Creates a frame backed by a macOS CoreVideo pixel buffer.
    #[cfg(target_os = "macos")]
    #[allow(clippy::too_many_arguments)]
    pub fn from_core_video(
        handle: SurfaceHandle,
        sequence: u64,
        visible_rect: Bounds<DevicePixels>,
        display_size: Size<DevicePixels>,
        format: SurfaceFormat,
        core_video: CoreVideoHandle,
        color: SurfaceColorInfo,
    ) -> Result<Self, SurfaceFrameError> {
        let coded_size = Size {
            width: DevicePixels(
                i32::try_from(core_video.pixel_buffer.get_width())
                    .map_err(|_| SurfaceFrameError::PlaneLayoutOverflow)?,
            ),
            height: DevicePixels(
                i32::try_from(core_video.pixel_buffer.get_height())
                    .map_err(|_| SurfaceFrameError::PlaneLayoutOverflow)?,
            ),
        };
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
            backing: SurfaceFrameBackingData::CoreVideo(core_video),
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
            #[cfg(target_os = "macos")]
            SurfaceFrameBackingData::CoreVideo(core_video) => {
                SurfaceFrameBacking::CoreVideo(core_video)
            }
            #[cfg(target_os = "linux")]
            SurfaceFrameBackingData::DmaBuf { handle, .. } => SurfaceFrameBacking::DmaBuf(handle),
        }
    }

    /// Returns the CPU planes, or `None` for a zero-copy native frame.
    pub fn cpu_planes(&self) -> Option<&[SurfacePlane]> {
        match &self.backing {
            SurfaceFrameBackingData::Cpu(planes) => Some(planes),
            #[cfg(target_os = "macos")]
            SurfaceFrameBackingData::CoreVideo(_) => None,
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

#[cfg(target_os = "linux")]
fn surface_format_from_drm_fourcc(fourcc: u32) -> Result<SurfaceFormat, SurfaceFrameError> {
    match fourcc {
        DRM_FORMAT_NV12 => Ok(SurfaceFormat::Nv12),
        _ => Err(SurfaceFrameError::UnsupportedDmaBufFourcc { fourcc }),
    }
}

#[cfg(target_os = "linux")]
fn validate_dma_buf_image(
    image: &DmaBufImage,
    format: SurfaceFormat,
) -> Result<(), SurfaceFrameError> {
    if image.objects().is_empty() {
        return Err(SurfaceFrameError::MissingDmaBufObjects);
    }
    let modifier = image.objects()[0].modifier();
    if image
        .objects()
        .iter()
        .any(|object| object.modifier() != modifier)
    {
        return Err(SurfaceFrameError::MismatchedDmaBufModifiers);
    }
    for (plane, layout) in image.planes().iter().enumerate() {
        if layout.object_index() >= image.objects().len() {
            return Err(SurfaceFrameError::InvalidDmaBufObjectIndex {
                plane,
                object_index: layout.object_index(),
            });
        }
    }

    let metadata_planes = image
        .planes()
        .iter()
        .map(|layout| DmaBufPlaneMetadata {
            stride: layout.stride(),
        })
        .collect::<SmallVec<[_; 2]>>();
    validate_dma_buf_plane_metadata(image.coded_size(), format, &metadata_planes)
}

#[cfg(target_os = "linux")]
struct DmaBufPlaneMetadata {
    stride: u32,
}

#[cfg(target_os = "linux")]
fn validate_dma_buf_plane_metadata(
    coded_size: Size<DevicePixels>,
    format: SurfaceFormat,
    planes: &[DmaBufPlaneMetadata],
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
        if plane.stride < minimum_stride {
            return Err(SurfaceFrameError::InvalidStride {
                plane: plane_index,
                stride: plane.stride,
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

    #[cfg(target_os = "macos")]
    #[test]
    fn accepts_core_video_backing() {
        use core_video::pixel_buffer::kCVPixelFormatType_32BGRA;

        let pixel_buffer = CVPixelBuffer::new(kCVPixelFormatType_32BGRA, 8, 4, None).unwrap();
        let frame = SurfaceFrame::from_core_video(
            SurfaceHandle::new(),
            9,
            bounds(
                point(DevicePixels(2), DevicePixels(1)),
                size(DevicePixels(4), DevicePixels(2)),
            ),
            size(DevicePixels(4), DevicePixels(2)),
            SurfaceFormat::Bgra8,
            // SAFETY: The test never mutates the pixel buffer after publishing it.
            unsafe { CoreVideoHandle::new(pixel_buffer) },
            SurfaceColorInfo::default(),
        )
        .unwrap();

        assert_eq!(frame.coded_size(), size(DevicePixels(8), DevicePixels(4)));
        assert_eq!(frame.sequence(), 9);
        assert!(matches!(frame.backing(), SurfaceFrameBacking::CoreVideo(_)));
        assert!(frame.cpu_planes().is_none());
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
    fn native_dma_buf_preserves_object_to_plane_layout() {
        let coded_size = size(DevicePixels(3840), DevicePixels(2160));
        let modifier = 0x0200_0000_0840_1b04;
        let fd: OwnedFd = std::fs::File::open("/dev/null").unwrap().into();
        let image = DmaBufImage::new(
            coded_size,
            DRM_FORMAT_NV12,
            vec![DmaBufObject::new(fd, modifier)],
            vec![
                DmaBufPlaneLayout::new(0, 0, 4096),
                DmaBufPlaneLayout::new(0, 8_847_360, 4096),
            ],
        )
        .with_drm_device(DrmDevice {
            major: 226,
            minor: 128,
        });

        let dma_buf = unsafe { DmaBufHandle::from_image(image) }.unwrap();
        let native = dma_buf.image().expect("native image descriptor");

        assert_eq!(dma_buf.format(), SurfaceFormat::Nv12);
        assert_eq!(dma_buf.import_status(), DmaBufImportStatus::Pending);
        assert!(dma_buf.planes().is_empty());
        assert_eq!(native.objects().len(), 1);
        assert_eq!(native.objects()[0].modifier(), modifier);
        assert_eq!(native.planes().len(), 2);
        assert_eq!(native.planes()[0], DmaBufPlaneLayout::new(0, 0, 4096));
        assert_eq!(native.planes()[1].object_index(), 0);
        assert_eq!(native.planes()[1].offset(), 8_847_360);
        assert_eq!(
            native.drm_device(),
            Some(DrmDevice {
                major: 226,
                minor: 128
            })
        );

        dma_buf.report_import_ready();
        assert_eq!(dma_buf.import_status(), DmaBufImportStatus::Ready);
        dma_buf.report_import_failed("unsupported test modifier");
        assert_eq!(
            dma_buf.import_status(),
            DmaBufImportStatus::Failed(Arc::from("unsupported test modifier"))
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn native_dma_buf_rejects_invalid_object_mapping() {
        let coded_size = size(DevicePixels(8), DevicePixels(4));
        let fd: OwnedFd = std::fs::File::open("/dev/null").unwrap().into();
        let image = DmaBufImage::new(
            coded_size,
            DRM_FORMAT_NV12,
            vec![DmaBufObject::new(fd, 0)],
            vec![
                DmaBufPlaneLayout::new(0, 0, 8),
                DmaBufPlaneLayout::new(1, 32, 8),
            ],
        );

        let result = unsafe { DmaBufHandle::from_image(image) };
        assert!(matches!(
            result,
            Err(SurfaceFrameError::InvalidDmaBufObjectIndex {
                plane: 1,
                object_index: 1,
            })
        ));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn native_dma_buf_rejects_inconsistent_object_modifiers() {
        let coded_size = size(DevicePixels(8), DevicePixels(4));
        let first: OwnedFd = std::fs::File::open("/dev/null").unwrap().into();
        let second: OwnedFd = std::fs::File::open("/dev/null").unwrap().into();
        let image = DmaBufImage::new(
            coded_size,
            DRM_FORMAT_NV12,
            vec![DmaBufObject::new(first, 1), DmaBufObject::new(second, 2)],
            vec![
                DmaBufPlaneLayout::new(0, 0, 8),
                DmaBufPlaneLayout::new(1, 0, 8),
            ],
        );

        let result = unsafe { DmaBufHandle::from_image(image) };
        assert!(matches!(
            result,
            Err(SurfaceFrameError::MismatchedDmaBufModifiers)
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
