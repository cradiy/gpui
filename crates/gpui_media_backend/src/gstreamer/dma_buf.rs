//! Linux DMA-BUF frame adaptation for the GStreamer system backend.

use std::{
    collections::HashMap,
    fs::File,
    os::fd::{BorrowedFd, OwnedFd},
    os::unix::fs::MetadataExt as _,
    sync::Arc,
};

use gpui_media_core::{
    DRM_FORMAT_NV12, DmaBufImage, DmaBufObject, DmaBufPlaneLayout, DrmDevice, FrameBacking,
    FrameBuffer, FrameHandle, FrameOutputCapabilities, PixelFormat,
};

use gpui_media_core::{MediaError, MediaErrorKind, MediaRecovery, MediaResult};

use super::{
    gst_video_output_error, gst_video_output_message, surface_color_info, video_frame_geometry,
};

pub(super) fn appsink_caps(
    output_capabilities: Option<&FrameOutputCapabilities>,
) -> MediaResult<gst::Caps> {
    let linear_drm_formats = [
        gst_video::VideoFormat::Nv12,
        gst_video::VideoFormat::Bgra,
        gst_video::VideoFormat::Rgba,
    ]
    .into_iter()
    .map(|format| {
        let fourcc = gst_video::dma_drm_fourcc_from_format(format).map_err(|error| {
            gst_video_output_error(format!("no DRM fourcc for {format:?}"), error)
        })?;
        Ok(gst_video::dma_drm_fourcc_to_string(fourcc, 0))
    })
    .collect::<MediaResult<Vec<_>>>()?;
    let linear_drm_formats = linear_drm_formats
        .iter()
        .map(|format| format.as_str())
        .collect::<Vec<_>>()
        .join(",");

    let mut native_nv12_formats = Vec::new();
    if let Some(output_capabilities) = output_capabilities {
        let nv12_fourcc = gst_video::dma_drm_fourcc_from_format(gst_video::VideoFormat::Nv12)
            .map_err(|error| gst_video_output_error("no DRM fourcc for NV12", error))?;
        for candidate in &output_capabilities.native_nv12_dma_buf_modifiers {
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
    .map_err(|error| gst_video_output_error("failed to construct appsink caps", error))
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
    handle: FrameHandle,
    sequence: u64,
    producer_drm_device: Option<DrmDevice>,
) -> MediaResult<FrameBuffer> {
    let caps = sample
        .caps()
        .ok_or_else(|| gst_video_output_message("decoded sample has no caps"))?;
    let buffer = sample
        .buffer_owned()
        .ok_or_else(|| gst_video_output_message("decoded sample has no buffer"))?;
    let (info, drm_fourcc, modifier) = video_info(caps)?;

    let format = surface_format(info.format())?;
    let (frame_size, visible_rect, display_size) = video_frame_geometry(buffer.as_ref(), &info)?;
    let (offsets, strides) = buffer
        .meta::<gst_video::VideoMeta>()
        .map(|meta| (meta.offset().to_vec(), meta.stride().to_vec()))
        .unwrap_or_else(|| (info.offset().to_vec(), info.stride().to_vec()));
    let expected_planes = match format {
        PixelFormat::Bgra8 | PixelFormat::Rgba8 => 1,
        PixelFormat::Nv12 => 2,
    };
    if offsets.len() < expected_planes || strides.len() < expected_planes {
        return Err(gst_video_output_message(format!(
            "DMA-BUF layout has {} offsets and {} strides, expected {expected_planes}",
            offsets.len(),
            strides.len()
        )));
    }

    let mut planes = Vec::with_capacity(expected_planes);
    for plane_index in 0..expected_planes {
        planes.push(import_plane(
            buffer.as_ref(),
            offsets[plane_index],
            strides[plane_index],
        )?);
    }

    let lifetime_guard: Arc<dyn Send + Sync> = Arc::new(buffer);
    if modifier != 0 && drm_fourcc != DRM_FORMAT_NV12 {
        return Err(gst_video_output_message(
            "non-linear RGB DMA-BUF is unsupported",
        ));
    }
    let (objects, layouts) = native_image_layout(planes, modifier)?;
    // SAFETY: Negotiated caps and VideoMeta describe these descriptors. Keeping
    // the GstBuffer leased prevents decoder pool reuse until consumers finish.
    let mut image =
        unsafe { DmaBufImage::new(frame_size, drm_fourcc, objects, layouts, lifetime_guard) };
    if let Some(device) = producer_drm_device {
        image = image.with_drm_device(device);
    }

    FrameBuffer::with_backing(
        handle,
        sequence,
        frame_size,
        visible_rect,
        display_size,
        format,
        FrameBacking::DmaBuf(Arc::new(image)),
        surface_color_info(&info),
    )
    .map_err(|error| gst_video_output_error("invalid DMA-BUF surface frame", error))
}

fn video_info(caps: &gst::CapsRef) -> MediaResult<(gst_video::VideoInfo, u32, u64)> {
    if gst_video::is_dma_drm_caps(caps) {
        let drm_info = gst_video::VideoInfoDmaDrm::from_caps(caps)
            .map_err(|error| gst_video_output_error("invalid DMA_DRM video caps", error))?;
        let modifier = drm_info.modifier();
        let info = drm_info
            .to_video_info()
            .map_err(|error| gst_video_output_error("unsupported DMA_DRM video format", error))?;
        Ok((info, drm_info.fourcc(), modifier))
    } else {
        let info = gst_video::VideoInfo::from_caps(caps)
            .map_err(|error| gst_video_output_error("invalid DMA-BUF video caps", error))?;
        let fourcc = gst_video::dma_drm_fourcc_from_format(info.format()).map_err(|error| {
            gst_video_output_error(format!("no DRM fourcc for {:?}", info.format()), error)
        })?;
        Ok((info, fourcc, 0))
    }
}

fn surface_format(format: gst_video::VideoFormat) -> MediaResult<PixelFormat> {
    match format {
        gst_video::VideoFormat::Bgra => Ok(PixelFormat::Bgra8),
        gst_video::VideoFormat::Rgba => Ok(PixelFormat::Rgba8),
        gst_video::VideoFormat::Nv12 => Ok(PixelFormat::Nv12),
        format => Err(MediaError::new(
            MediaErrorKind::UnsupportedCodec,
            format!("unsupported DMA-BUF video format: {format:?}"),
            MediaRecovery::None,
        )),
    }
}

struct ImportedPlane {
    fd: OwnedFd,
    object_key: DmaBufObjectKey,
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
) -> MediaResult<(Vec<DmaBufObject>, Vec<DmaBufPlaneLayout>)> {
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

fn import_plane(
    buffer: &gst::BufferRef,
    buffer_offset: usize,
    stride: i32,
) -> MediaResult<ImportedPlane> {
    if stride <= 0 {
        return Err(gst_video_output_message(format!(
            "negative DMA-BUF video stride is not supported: {stride}"
        )));
    }
    let end = buffer_offset
        .checked_add(1)
        .ok_or_else(|| gst_video_output_message("DMA-BUF plane offset overflow"))?;
    let (memory_range, skip) = buffer
        .find_memory(buffer_offset..end)
        .ok_or_else(|| gst_video_output_message("DMA-BUF plane offset is outside the GstBuffer"))?;
    if memory_range.len() != 1 {
        return Err(gst_video_output_message(
            "DMA-BUF plane spans multiple GstMemory objects",
        ));
    }
    let memory = buffer.peek_memory(memory_range.start);
    let dma_buf = memory
        .downcast_memory_ref::<gst_allocators::DmaBufMemory>()
        .ok_or_else(|| gst_video_output_message("video plane is not backed by DMA-BUF memory"))?;
    let fd = unsafe { BorrowedFd::borrow_raw(dma_buf.fd()) }
        .try_clone_to_owned()
        .map_err(|error| MediaError::io("failed to duplicate DMA-BUF fd", error))?;
    let file = File::from(fd);
    let metadata = file
        .metadata()
        .map_err(|error| MediaError::io("failed to identify DMA-BUF object", error))?;
    let object_key = DmaBufObjectKey {
        device: metadata.dev(),
        inode: metadata.ino(),
    };
    let fd = OwnedFd::from(file);
    let offset = memory
        .offset()
        .checked_add(skip)
        .and_then(|offset| u64::try_from(offset).ok())
        .ok_or_else(|| gst_video_output_message("DMA-BUF plane offset overflow"))?;

    Ok(ImportedPlane {
        fd,
        object_key,
        offset,
        stride: stride as u32,
    })
}
