use std::{
    collections::HashMap,
    fs::File,
    os::fd::{BorrowedFd, OwnedFd},
    os::unix::fs::MetadataExt as _,
    sync::Arc,
};

use anyhow::{Context as _, Result, bail};
use gpui::{
    DRM_FORMAT_NV12, DmaBufHandle, DmaBufImage, DmaBufObject, DmaBufPlane, DmaBufPlaneLayout,
    DrmDevice, GpuSpecs, SurfaceFormat, SurfaceFrame, SurfaceHandle,
};

use super::{surface_color_info, video_frame_geometry};

pub(super) fn appsink_caps(gpu_specs: Option<&GpuSpecs>) -> Result<gst::Caps> {
    let linear_drm_formats = [
        gst_video::VideoFormat::Nv12,
        gst_video::VideoFormat::Bgra,
        gst_video::VideoFormat::Rgba,
    ]
    .into_iter()
    .map(|format| {
        let fourcc = gst_video::dma_drm_fourcc_from_format(format)
            .with_context(|| format!("no DRM fourcc for {format:?}"))?;
        Ok(gst_video::dma_drm_fourcc_to_string(fourcc, 0))
    })
    .collect::<Result<Vec<_>>>()?;
    let linear_drm_formats = linear_drm_formats
        .iter()
        .map(|format| format.as_str())
        .collect::<Vec<_>>()
        .join(",");

    let mut native_nv12_formats = Vec::new();
    if let Some(gpu_specs) = gpu_specs
        && gpu_specs.supports_native_nv12_dma_buf_import
    {
        let nv12_fourcc = gst_video::dma_drm_fourcc_from_format(gst_video::VideoFormat::Nv12)
            .context("no DRM fourcc for NV12")?;
        for candidate in &gpu_specs.native_nv12_dma_buf_modifiers {
            if candidate.plane_count == 2 {
                let format = gst_video::dma_drm_fourcc_to_string(nv12_fourcc, candidate.modifier)
                    .to_string();
                if !native_nv12_formats.contains(&format) {
                    native_nv12_formats.push(format);
                }
            }
        }
    }

    let native_caps = if native_nv12_formats.is_empty() {
        String::new()
    } else {
        format!(
            "video/x-raw(memory:DMABuf),format=(string)DMA_DRM,drm-format=(string){{{}}};",
            native_nv12_formats.join(",")
        )
    };

    format!(
        "{native_caps}\
         video/x-raw(memory:DMABuf),format=(string)DMA_DRM,drm-format=(string){{{linear_drm_formats}}};\
         video/x-raw(memory:DMABuf),format=(string){{NV12,BGRA,RGBA}};\
         video/x-raw,format=(string){{NV12,BGRA,RGBA}}"
    )
    .parse::<gst::Caps>()
    .context("failed to construct appsink caps")
}

pub(super) fn sample_uses_dma_buf(sample: &gst::Sample) -> bool {
    let Some(buffer) = sample.buffer() else {
        return false;
    };
    buffer.n_memory() > 0
        && buffer.iter_memories().all(|memory| {
            memory
                .downcast_memory_ref::<gst_allocators::DmaBufMemory>()
                .is_some()
        })
}

pub(super) fn sample_to_surface_frame(
    sample: &gst::Sample,
    handle: SurfaceHandle,
    sequence: u64,
    producer_drm_device: Option<DrmDevice>,
) -> Result<SurfaceFrame> {
    let caps = sample.caps().context("decoded sample has no caps")?;
    let buffer = sample
        .buffer_owned()
        .context("decoded sample has no buffer")?;
    let (info, drm_fourcc, modifier) = video_info(caps)?;

    let format = surface_format(info.format())?;
    let (frame_size, visible_rect, display_size) = video_frame_geometry(buffer.as_ref(), &info)?;
    let (offsets, strides) = buffer
        .meta::<gst_video::VideoMeta>()
        .map(|meta| (meta.offset().to_vec(), meta.stride().to_vec()))
        .unwrap_or_else(|| (info.offset().to_vec(), info.stride().to_vec()));
    let expected_planes = match format {
        SurfaceFormat::Bgra8 | SurfaceFormat::Rgba8 => 1,
        SurfaceFormat::Nv12 => 2,
    };
    if offsets.len() < expected_planes || strides.len() < expected_planes {
        bail!(
            "DMA-BUF layout has {} offsets and {} strides, expected {expected_planes}",
            offsets.len(),
            strides.len()
        );
    }

    let mut planes = Vec::with_capacity(expected_planes);
    for plane_index in 0..expected_planes {
        planes.push(import_plane(
            buffer.as_ref(),
            offsets[plane_index],
            strides[plane_index],
            modifier,
        )?);
    }

    let lifetime_guard: Arc<dyn Send + Sync> = Arc::new(buffer);
    let dma_buf = match format {
        SurfaceFormat::Bgra8 | SurfaceFormat::Rgba8 => {
            if modifier != 0 {
                bail!("non-linear RGB DMA-BUF modifier {modifier:#018x} is not supported");
            }
            let plane = planes.pop().expect("validated RGB plane");
            // SAFETY: The descriptor and layout come from GStreamer's negotiated
            // DMA-BUF caps and GstVideoMeta. The retained GstBuffer prevents the
            // decoder pool from reusing the allocation while GPUI samples it.
            unsafe {
                DmaBufHandle::new_with_lifetime_guard(
                    plane.fd,
                    frame_size,
                    format,
                    plane.modifier,
                    plane.offset,
                    plane.stride,
                    lifetime_guard,
                )
            }
        }
        SurfaceFormat::Nv12 => {
            if modifier == 0 {
                let uv = planes.pop().expect("validated NV12 UV plane");
                let y = planes.pop().expect("validated NV12 Y plane");
                // SAFETY: Both descriptors and their plane layouts are supplied by
                // GStreamer. Retaining the GstBuffer keeps both allocations leased.
                unsafe {
                    DmaBufHandle::new_nv12_with_lifetime_guard(
                        frame_size,
                        y.into_gpui(),
                        uv.into_gpui(),
                        lifetime_guard,
                    )
                }
            } else {
                if drm_fourcc != DRM_FORMAT_NV12 {
                    bail!("unexpected DRM fourcc for native NV12: {drm_fourcc:#010x}");
                }
                let (objects, layouts) = native_image_layout(planes, modifier)?;
                if objects.len() != 1 {
                    bail!(
                        "native NV12 import requires one DMA-BUF object, received {}",
                        objects.len()
                    );
                }
                let mut image = DmaBufImage::new(frame_size, drm_fourcc, objects, layouts);
                if let Some(device) = producer_drm_device {
                    image = image.with_drm_device(device);
                }
                // SAFETY: The object and image-plane mapping comes from the
                // negotiated DMA_DRM caps and GstVideoMeta. The retained buffer
                // keeps the decoder allocation leased while GPUI samples it.
                unsafe { DmaBufHandle::from_image_with_lifetime_guard(image, lifetime_guard) }
            }
        }
    }
    .context("GPUI rejected DMA-BUF frame layout")?;

    SurfaceFrame::from_dma_buf_with_color(
        handle,
        sequence,
        visible_rect,
        display_size,
        dma_buf,
        surface_color_info(&info),
    )
    .context("GPUI rejected DMA-BUF surface frame")
}

fn video_info(caps: &gst::CapsRef) -> Result<(gst_video::VideoInfo, u32, u64)> {
    if gst_video::is_dma_drm_caps(caps) {
        let drm_info =
            gst_video::VideoInfoDmaDrm::from_caps(caps).context("invalid DMA_DRM video caps")?;
        let modifier = drm_info.modifier();
        let info = drm_info
            .to_video_info()
            .context("unsupported DMA_DRM video format")?;
        Ok((info, drm_info.fourcc(), modifier))
    } else {
        let info = gst_video::VideoInfo::from_caps(caps).context("invalid DMA-BUF video caps")?;
        let fourcc = gst_video::dma_drm_fourcc_from_format(info.format())
            .with_context(|| format!("no DRM fourcc for {:?}", info.format()))?;
        Ok((info, fourcc, 0))
    }
}

fn surface_format(format: gst_video::VideoFormat) -> Result<SurfaceFormat> {
    match format {
        gst_video::VideoFormat::Bgra => Ok(SurfaceFormat::Bgra8),
        gst_video::VideoFormat::Rgba => Ok(SurfaceFormat::Rgba8),
        gst_video::VideoFormat::Nv12 => Ok(SurfaceFormat::Nv12),
        format => bail!("unsupported DMA-BUF video format: {format:?}"),
    }
}

struct ImportedPlane {
    fd: OwnedFd,
    object_key: DmaBufObjectKey,
    modifier: u64,
    offset: u64,
    stride: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct DmaBufObjectKey {
    device: u64,
    inode: u64,
}

fn native_image_layout(
    planes: Vec<ImportedPlane>,
    modifier: u64,
) -> Result<(Vec<DmaBufObject>, Vec<DmaBufPlaneLayout>)> {
    let mut object_indices = HashMap::new();
    let mut objects = Vec::new();
    let mut layouts = Vec::with_capacity(planes.len());

    for plane in planes {
        let object_index = if let Some(index) = object_indices.get(&plane.object_key) {
            *index
        } else {
            let index = objects.len();
            object_indices.insert(plane.object_key, index);
            objects.push(DmaBufObject::new(plane.fd, modifier));
            layouts.push(DmaBufPlaneLayout::new(index, plane.offset, plane.stride));
            continue;
        };
        layouts.push(DmaBufPlaneLayout::new(
            object_index,
            plane.offset,
            plane.stride,
        ));
    }

    Ok((objects, layouts))
}

impl ImportedPlane {
    fn into_gpui(self) -> DmaBufPlane {
        DmaBufPlane::new(self.fd, self.modifier, self.offset, self.stride)
    }
}

fn import_plane(
    buffer: &gst::BufferRef,
    buffer_offset: usize,
    stride: i32,
    modifier: u64,
) -> Result<ImportedPlane> {
    if stride <= 0 {
        bail!("negative DMA-BUF video stride is not supported: {stride}");
    }
    let end = buffer_offset
        .checked_add(1)
        .context("DMA-BUF plane offset overflow")?;
    let (memory_range, skip) = buffer
        .find_memory(buffer_offset..end)
        .context("DMA-BUF plane offset is outside the GstBuffer")?;
    if memory_range.len() != 1 {
        bail!("DMA-BUF plane spans multiple GstMemory objects");
    }
    let memory = buffer.peek_memory(memory_range.start);
    let dma_buf = memory
        .downcast_memory_ref::<gst_allocators::DmaBufMemory>()
        .context("video plane is not backed by DMA-BUF memory")?;
    let fd = unsafe { BorrowedFd::borrow_raw(dma_buf.fd()) }
        .try_clone_to_owned()
        .context("failed to duplicate DMA-BUF fd")?;
    let file = File::from(fd);
    let metadata = file
        .metadata()
        .context("failed to identify DMA-BUF object")?;
    let object_key = DmaBufObjectKey {
        device: metadata.dev(),
        inode: metadata.ino(),
    };
    let fd = OwnedFd::from(file);
    let offset = memory
        .offset()
        .checked_add(skip)
        .and_then(|offset| u64::try_from(offset).ok())
        .context("DMA-BUF plane offset overflow")?;

    Ok(ImportedPlane {
        fd,
        object_key,
        modifier,
        offset,
        stride: stride as u32,
    })
}
