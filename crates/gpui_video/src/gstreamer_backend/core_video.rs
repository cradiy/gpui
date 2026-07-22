use std::ffi::{c_char, c_void};

use anyhow::{Context as _, Result, bail};
use core_foundation::base::TCFType as _;
use core_video::pixel_buffer::{CVPixelBuffer, CVPixelBufferRef};
use gpui::{CoreVideoHandle, SurfaceFormat, SurfaceFrame, SurfaceHandle};

use super::{surface_color_info, video_frame_geometry};

#[repr(C)]
struct GstCoreVideoMeta {
    meta: gst::ffi::GstMeta,
    cvbuf: *const c_void,
    pixbuf: CVPixelBufferRef,
}

/// Retains the CVPixelBuffer carried by GStreamer's applemedia decoder.
///
/// GstCoreVideoMeta is an applemedia-private metadata layout, but it has been
/// stable across the GStreamer versions supported by this crate. Looking up
/// the registered meta API by name avoids linking against the plugin itself;
/// absence of the plugin or metadata cleanly falls back to mapped CPU frames.
fn core_video_pixel_buffer(buffer: &gst::BufferRef) -> Option<CVPixelBuffer> {
    const API_NAME: &[u8] = b"GstCoreVideoMetaAPI\0";
    let api_type =
        unsafe { gst::glib::gobject_ffi::g_type_from_name(API_NAME.as_ptr().cast::<c_char>()) };
    if api_type == 0 {
        return None;
    }

    let meta = unsafe {
        gst::ffi::gst_buffer_get_meta(buffer.as_ptr() as *mut gst::ffi::GstBuffer, api_type)
    };
    let base_meta = unsafe { meta.as_ref() }?;
    let meta_info = unsafe { base_meta.info.as_ref() }?;
    if meta_info.size < std::mem::size_of::<GstCoreVideoMeta>() {
        return None;
    }

    // SAFETY: The runtime API name and registered meta size match the
    // applemedia GstCoreVideoMeta layout declared above.
    let pixel_buffer = unsafe { &*meta.cast::<GstCoreVideoMeta>() }.pixbuf;
    if pixel_buffer.is_null() {
        return None;
    }

    Some(unsafe { CVPixelBuffer::wrap_under_get_rule(pixel_buffer) })
}

pub(super) fn sample_to_surface_frame(
    sample: &gst::Sample,
    handle: SurfaceHandle,
    sequence: u64,
) -> Result<Option<SurfaceFrame>> {
    let caps = sample.caps().context("decoded sample has no caps")?;
    let info = gst_video::VideoInfo::from_caps(caps).context("invalid decoded video caps")?;
    let Some(buffer) = sample.buffer() else {
        bail!("decoded sample has no buffer");
    };
    let Some(pixel_buffer) = core_video_pixel_buffer(buffer) else {
        return Ok(None);
    };

    let format = match info.format() {
        gst_video::VideoFormat::Bgra => SurfaceFormat::Bgra8,
        gst_video::VideoFormat::Rgba => SurfaceFormat::Rgba8,
        gst_video::VideoFormat::Nv12 => SurfaceFormat::Nv12,
        _ => return Ok(None),
    };
    if format == SurfaceFormat::Nv12 && pixel_buffer.get_plane_count() < 2 {
        return Ok(None);
    }
    if pixel_buffer.get_width() != info.width() as usize
        || pixel_buffer.get_height() != info.height() as usize
    {
        return Ok(None);
    }

    let (_, visible_rect, display_size) = video_frame_geometry(buffer, &info)?;
    let color = if format == SurfaceFormat::Nv12 {
        surface_color_info(&info)
    } else {
        Default::default()
    };
    let frame = SurfaceFrame::from_core_video(
        handle,
        sequence,
        visible_rect,
        display_size,
        format,
        // SAFETY: GStreamer owns the decoded buffer and publishes it only
        // after VideoToolbox has finished writing the frame. Retaining the
        // CVPixelBuffer keeps that immutable frame allocation alive.
        unsafe { CoreVideoHandle::new(pixel_buffer) },
        color,
    )
    .context("GPUI rejected CoreVideo frame")?;
    Ok(Some(frame))
}
