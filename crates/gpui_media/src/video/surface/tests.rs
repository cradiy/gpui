use super::*;
use gpui_media_core::{FrameColorInfo, FrameHandle, FramePlane, FramePoint, FrameRect};

#[test]
fn cpu_adapter_preserves_shared_planes_geometry_and_color() {
    let bytes: Arc<[u8]> = vec![128; 48].into();
    let buffer = FrameBuffer::new(
        FrameHandle::new(),
        7,
        FrameSize::new(8, 4),
        FrameRect {
            origin: FramePoint::new(2, 0),
            size: FrameSize::new(6, 4),
        },
        FrameSize::new(12, 4),
        PixelFormat::Nv12,
        [
            FramePlane::with_offset(bytes.clone(), 0, 8),
            FramePlane::with_offset(bytes.clone(), 32, 8),
        ],
        FrameColorInfo {
            matrix: gpui_media_core::YuvMatrix::Bt601,
            range: gpui_media_core::ColorRange::Full,
        },
    )
    .unwrap();
    let frame = VideoFrame::new(Arc::new(buffer), None, None);
    let mut adapter = VideoSurface::new();
    let surface = adapter.set_frame(&frame).unwrap();
    assert!(Arc::ptr_eq(&surface, &adapter.set_frame(&frame).unwrap()));
    assert_eq!(surface.sequence(), 7);
    assert_eq!(surface.visible_rect().origin.x, DevicePixels(2));
    assert_eq!(surface.display_size(), pixel_size(FrameSize::new(12, 4)));
    assert_eq!(surface.color().matrix, gpui::YuvMatrix::Bt601);
    assert_eq!(surface.color().range, gpui::ColorRange::Full);
    let gpui::SurfaceFrameBacking::Cpu(planes) = surface.backing() else {
        panic!("expected CPU frame");
    };
    assert_eq!(planes[0].bytes().as_ptr(), bytes.as_ptr());
    assert_eq!(planes[1].bytes().as_ptr(), bytes.as_ptr());
    assert_eq!(planes[1].offset(), 32);
    drop(frame);
    adapter.clear();
    assert_eq!(planes[1].bytes().len(), 48);
}

#[cfg(target_os = "linux")]
#[test]
fn native_adapter_retains_lease_and_preserves_import_state() {
    use gpui_media_core::{
        DRM_FORMAT_NV12, DmaBufImage, DmaBufObject, DmaBufPlaneLayout, DrmDevice,
    };
    for modifier in [0, 0x0200_0000_0840_1b04] {
        let lease = Arc::new(());
        let weak = Arc::downgrade(&lease);
        let size = FrameSize::new(16, 16);
        // SAFETY: This metadata-only test never samples the descriptor or performs GPU import.
        let image = unsafe {
            DmaBufImage::new(
                size,
                DRM_FORMAT_NV12,
                vec![DmaBufObject::new(
                    std::fs::File::open("/dev/zero").unwrap().into(),
                    modifier,
                )],
                vec![
                    DmaBufPlaneLayout::new(0, 0, 16),
                    DmaBufPlaneLayout::new(0, 256, 16),
                ],
                lease,
            )
        }
        .with_drm_device(DrmDevice {
            major: 226,
            minor: 128,
        });
        let frame = VideoFrame::new(
            Arc::new(
                FrameBuffer::with_backing(
                    FrameHandle::new(),
                    3,
                    size,
                    FrameRect {
                        origin: FramePoint::default(),
                        size,
                    },
                    size,
                    PixelFormat::Nv12,
                    FrameBacking::DmaBuf(Arc::new(image)),
                    FrameColorInfo::default(),
                )
                .unwrap(),
            ),
            None,
            None,
        );
        let mut adapter = VideoSurface::new();
        let surface = adapter.set_frame(&frame).unwrap();
        let gpui::SurfaceFrameBacking::DmaBuf(native) = surface.backing() else {
            panic!("expected DMA-BUF frame");
        };
        assert_eq!(native.drm_modifier(), modifier);
        if let Some(image) = native.image() {
            assert_eq!(image.planes()[1].offset(), 256);
            assert_eq!(
                image.drm_device(),
                Some(gpui::DrmDevice {
                    major: 226,
                    minor: 128
                })
            );
        } else {
            assert_eq!(native.planes()[1].offset(), 256);
        }
        native.report_import_failed("unsupported import");
        assert!(Arc::ptr_eq(&surface, &adapter.set_frame(&frame).unwrap()));
        assert!(matches!(
            native.import_status(),
            gpui::DmaBufImportStatus::Failed(_)
        ));
        drop(frame);
        adapter.clear();
        assert!(weak.upgrade().is_some());
        drop(surface);
        assert!(weak.upgrade().is_none());
    }
}
