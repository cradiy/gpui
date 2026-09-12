use std::{fmt, io, os::fd::OwnedFd, sync::Arc};

use super::{FrameSize, PixelFormat, plane_layout};
use crate::{MediaError, MediaResult};

pub const DRM_FORMAT_NV12: u32 = u32::from_le_bytes(*b"NV12");
pub const DRM_FORMAT_ARGB8888: u32 = u32::from_le_bytes(*b"AR24");
pub const DRM_FORMAT_ABGR8888: u32 = u32::from_le_bytes(*b"AB24");

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DrmDevice {
    pub major: u32,
    pub minor: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DmaBufModifier {
    pub modifier: u64,
    pub plane_count: u32,
}

#[derive(Debug)]
pub struct DmaBufObject {
    fd: OwnedFd,
    modifier: u64,
}

impl DmaBufObject {
    pub fn new(fd: OwnedFd, modifier: u64) -> Self {
        Self { fd, modifier }
    }
    pub fn modifier(&self) -> u64 {
        self.modifier
    }
    pub fn try_clone_fd(&self) -> io::Result<OwnedFd> {
        self.fd.try_clone()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DmaBufPlaneLayout {
    object_index: usize,
    offset: u64,
    stride: u32,
}

impl DmaBufPlaneLayout {
    pub fn new(object_index: usize, offset: u64, stride: u32) -> Self {
        Self {
            object_index,
            offset,
            stride,
        }
    }
    pub fn object_index(&self) -> usize {
        self.object_index
    }
    pub fn offset(&self) -> u64 {
        self.offset
    }
    pub fn stride(&self) -> u32 {
        self.stride
    }
}

/// Owned DMA-BUF descriptors and the producer lease for one immutable image.
pub struct DmaBufImage {
    coded_size: FrameSize,
    drm_fourcc: u32,
    objects: Vec<DmaBufObject>,
    planes: Vec<DmaBufPlaneLayout>,
    drm_device: Option<DrmDevice>,
    _lease: Arc<dyn Send + Sync>,
}

impl DmaBufImage {
    /// Retains an exported image until all frame consumers release it.
    ///
    /// # Safety
    /// Descriptors and layouts must describe a completed, readable image. The
    /// lease must prevent producer writes and pool reuse for its entire lifetime.
    pub unsafe fn new(
        coded_size: FrameSize,
        drm_fourcc: u32,
        objects: Vec<DmaBufObject>,
        planes: Vec<DmaBufPlaneLayout>,
        lease: Arc<dyn Send + Sync>,
    ) -> Self {
        Self {
            coded_size,
            drm_fourcc,
            objects,
            planes,
            drm_device: None,
            _lease: lease,
        }
    }
    pub fn with_drm_device(mut self, device: DrmDevice) -> Self {
        self.drm_device = Some(device);
        self
    }
    pub fn coded_size(&self) -> FrameSize {
        self.coded_size
    }
    pub fn drm_fourcc(&self) -> u32 {
        self.drm_fourcc
    }
    pub fn objects(&self) -> &[DmaBufObject] {
        &self.objects
    }
    pub fn planes(&self) -> &[DmaBufPlaneLayout] {
        &self.planes
    }
    pub fn drm_device(&self) -> Option<DrmDevice> {
        self.drm_device
    }

    pub(super) fn validate(&self, size: FrameSize, format: PixelFormat) -> MediaResult<()> {
        let fourcc = match format {
            PixelFormat::Bgra8 => DRM_FORMAT_ARGB8888,
            PixelFormat::Rgba8 => DRM_FORMAT_ABGR8888,
            PixelFormat::Nv12 => DRM_FORMAT_NV12,
        };
        let layout = plane_layout(size, format)?;
        if self.coded_size != size
            || self.drm_fourcc != fourcc
            || self.objects.is_empty()
            || self.planes.len() != layout.len()
        {
            return Err(MediaError::invalid_input(
                "DMA-BUF image geometry or format mismatch",
            ));
        }
        let modifier = self.objects[0].modifier;
        if self
            .objects
            .iter()
            .any(|object| object.modifier != modifier)
        {
            return Err(MediaError::invalid_input(
                "DMA-BUF image modifiers disagree",
            ));
        }
        for (plane, (row_bytes, rows)) in self.planes.iter().zip(layout) {
            if plane.object_index >= self.objects.len()
                || plane.stride == 0
                || (modifier == 0 && plane.stride < row_bytes)
                || plane
                    .offset
                    .checked_add(u64::from(plane.stride) * u64::from(rows))
                    .is_none()
            {
                return Err(MediaError::invalid_input("invalid DMA-BUF plane layout"));
            }
        }
        Ok(())
    }
}

impl fmt::Debug for DmaBufImage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DmaBufImage")
            .field("coded_size", &self.coded_size)
            .field("drm_fourcc", &self.drm_fourcc)
            .field("objects", &self.objects)
            .field("planes", &self.planes)
            .field("drm_device", &self.drm_device)
            .finish_non_exhaustive()
    }
}
