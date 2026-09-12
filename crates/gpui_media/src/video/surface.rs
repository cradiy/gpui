use std::sync::Arc;

use gpui::{
    Bounds, DevicePixels, GpuSpecs, SurfaceColorInfo, SurfaceFormat, SurfaceFrame, SurfaceHandle,
    SurfacePlane, point, size,
};
use gpui_media_core::{
    FrameBacking, FrameBuffer, FrameOutputCapabilities, FrameSize, MediaError, MediaErrorKind,
    MediaRecovery, MediaResult, PixelFormat, VideoFrame,
};

#[cfg(test)]
mod tests;

/// Converts decoded frames to GPUI surfaces while retaining their allocations.
///
/// Keep one adapter per presentation stream. Repeated access to the same frame
/// preserves its surface and native import status. CPU planes share their bytes.
#[derive(Default)]
pub struct VideoSurface {
    source: Option<Arc<FrameBuffer>>,
    handle: SurfaceHandle,
    surface: Option<Arc<SurfaceFrame>>,
}

impl VideoSurface {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_frame(&mut self, frame: &VideoFrame) -> MediaResult<Arc<SurfaceFrame>> {
        let buffer = frame.buffer();
        if self
            .source
            .as_ref()
            .is_some_and(|source| Arc::ptr_eq(source, buffer))
        {
            return Ok(self
                .surface
                .as_ref()
                .expect("cached frame has a surface")
                .clone());
        }
        let handle = if self
            .source
            .as_ref()
            .is_some_and(|source| source.handle() != buffer.handle())
        {
            SurfaceHandle::new()
        } else {
            self.handle.clone()
        };
        let surface = Arc::new(convert_frame(buffer, handle.clone())?);
        self.source = Some(buffer.clone());
        self.handle = handle;
        self.surface = Some(surface.clone());
        Ok(surface)
    }

    pub fn surface(&self) -> Option<&Arc<SurfaceFrame>> {
        self.surface.as_ref()
    }

    pub fn clear(&mut self) {
        self.source = None;
        self.surface = None;
        self.handle = SurfaceHandle::new();
    }

    /// Native decoded layouts accepted by the renderer backing a window.
    pub fn output_capabilities(specs: &GpuSpecs) -> FrameOutputCapabilities {
        #[cfg(target_os = "linux")]
        {
            FrameOutputCapabilities {
                native_nv12_dma_buf_modifiers: if specs.supports_native_nv12_dma_buf_import {
                    specs
                        .native_nv12_dma_buf_modifiers
                        .iter()
                        .map(|modifier| gpui_media_core::DmaBufModifier {
                            modifier: modifier.modifier,
                            plane_count: modifier.plane_count,
                        })
                        .collect()
                } else {
                    Vec::new()
                },
            }
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = specs;
            FrameOutputCapabilities::default()
        }
    }
}

fn pixel_size(value: FrameSize) -> gpui::Size<DevicePixels> {
    size(DevicePixels(value.width), DevicePixels(value.height))
}

fn output_error(error: impl std::fmt::Display) -> MediaError {
    MediaError::from_error(
        MediaErrorKind::VideoOutput,
        MediaRecovery::Retry,
        "cannot adapt decoded frame to GPUI",
        error,
    )
}

fn convert_frame(buffer: &FrameBuffer, handle: SurfaceHandle) -> MediaResult<SurfaceFrame> {
    let coded = pixel_size(buffer.coded_size());
    let display = pixel_size(buffer.display_size());
    let rect = buffer.visible_rect();
    let visible = Bounds {
        origin: point(DevicePixels(rect.origin.x), DevicePixels(rect.origin.y)),
        size: pixel_size(rect.size),
    };
    let format = match buffer.format() {
        PixelFormat::Bgra8 => SurfaceFormat::Bgra8,
        PixelFormat::Rgba8 => SurfaceFormat::Rgba8,
        PixelFormat::Nv12 => SurfaceFormat::Nv12,
    };
    let color = SurfaceColorInfo {
        matrix: match buffer.color().matrix {
            gpui_media_core::YuvMatrix::Bt601 => gpui::YuvMatrix::Bt601,
            gpui_media_core::YuvMatrix::Bt709 => gpui::YuvMatrix::Bt709,
        },
        range: match buffer.color().range {
            gpui_media_core::ColorRange::Limited => gpui::ColorRange::Limited,
            gpui_media_core::ColorRange::Full => gpui::ColorRange::Full,
        },
    };
    match buffer.backing() {
        FrameBacking::Cpu(planes) => SurfaceFrame::new(
            handle,
            buffer.sequence(),
            coded,
            visible,
            display,
            format,
            planes.iter().map(|plane| {
                SurfacePlane::with_offset(plane.shared_bytes(), plane.offset(), plane.stride())
            }),
            color,
        )
        .map_err(output_error),
        #[cfg(target_os = "linux")]
        FrameBacking::DmaBuf(image) => {
            let modifier = image.objects()[0].modifier();
            let lease: Arc<dyn Send + Sync> = image.clone();
            let dma_buf = if modifier == 0 {
                if format == SurfaceFormat::Nv12 {
                    let plane = |index: usize| -> MediaResult<gpui::DmaBufPlane> {
                        let layout = image.planes()[index];
                        let fd = image.objects()[layout.object_index()]
                            .try_clone_fd()
                            .map_err(output_error)?;
                        Ok(gpui::DmaBufPlane::new(
                            fd,
                            modifier,
                            layout.offset(),
                            layout.stride(),
                        ))
                    };
                    let y = plane(0)?;
                    let uv = plane(1)?;
                    // SAFETY: The image owns valid layouts and keeps the producer leased.
                    unsafe { gpui::DmaBufHandle::new_nv12_with_lifetime_guard(coded, y, uv, lease) }
                } else {
                    let plane = image.planes()[0];
                    // SAFETY: The image lease prevents producer writes during sampling.
                    unsafe {
                        gpui::DmaBufHandle::new_with_lifetime_guard(
                            image.objects()[plane.object_index()]
                                .try_clone_fd()
                                .map_err(output_error)?,
                            coded,
                            format,
                            modifier,
                            plane.offset(),
                            plane.stride(),
                            lease,
                        )
                    }
                }
            } else {
                let objects = image
                    .objects()
                    .iter()
                    .map(|object| {
                        object
                            .try_clone_fd()
                            .map(|fd| gpui::DmaBufObject::new(fd, object.modifier()))
                            .map_err(output_error)
                    })
                    .collect::<MediaResult<Vec<_>>>()?;
                let planes = image
                    .planes()
                    .iter()
                    .map(|plane| {
                        gpui::DmaBufPlaneLayout::new(
                            plane.object_index(),
                            plane.offset(),
                            plane.stride(),
                        )
                    })
                    .collect();
                let mut native = gpui::DmaBufImage::new(coded, image.drm_fourcc(), objects, planes);
                if let Some(device) = image.drm_device() {
                    native = native.with_drm_device(gpui::DrmDevice {
                        major: device.major,
                        minor: device.minor,
                    });
                }
                // SAFETY: Descriptor duplication preserves the image; lease retains the producer allocation.
                unsafe { gpui::DmaBufHandle::from_image_with_lifetime_guard(native, lease) }
            }
            .map_err(output_error)?;
            SurfaceFrame::from_dma_buf_with_color(
                handle,
                buffer.sequence(),
                visible,
                display,
                dma_buf,
                color,
            )
            .map_err(output_error)
        }
        #[cfg(target_os = "macos")]
        FrameBacking::CoreVideo(core_video) => {
            // SAFETY: Both wrappers enforce immutable access and retain the same CVPixelBuffer.
            let native = unsafe { gpui::CoreVideoHandle::new(core_video.pixel_buffer().clone()) };
            SurfaceFrame::from_core_video(
                handle,
                buffer.sequence(),
                visible,
                display,
                format,
                native,
                color,
            )
            .map_err(output_error)
        }
    }
}
