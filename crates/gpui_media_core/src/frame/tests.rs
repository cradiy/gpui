use super::*;
use std::sync::Arc;

#[test]
fn planar_layout_validates_offsets_rows_and_chroma_alignment() {
    let size = FrameSize::new(5, 3);
    let bytes: Arc<[u8]> = vec![128; 40].into();
    let make = |uv_offset, uv_stride, rect| {
        FrameBuffer::new(
            FrameHandle::new(),
            1,
            size,
            rect,
            size,
            PixelFormat::Nv12,
            [
                FramePlane::with_offset(bytes.clone(), 0, 8),
                FramePlane::with_offset(bytes.clone(), uv_offset, uv_stride),
            ],
            FrameColorInfo::default(),
        )
    };
    let rect = FrameRect {
        origin: FramePoint::default(),
        size,
    };
    assert!(make(24, 8, rect).is_ok());
    assert!(make(27, 8, rect).is_err());
    assert!(make(24, 5, rect).is_err());
    assert!(
        make(
            24,
            8,
            FrameRect {
                origin: FramePoint::new(1, 0),
                size: FrameSize::new(4, 3)
            }
        )
        .is_err()
    );
    assert!(
        make(
            24,
            8,
            FrameRect {
                origin: FramePoint::new(4, 0),
                size: FrameSize::new(2, 3)
            }
        )
        .is_err()
    );
}

#[test]
fn frame_rejects_empty_or_overflowing_packed_geometry() {
    for size in [
        FrameSize::new(0, 1),
        FrameSize::new(-1, 1),
        FrameSize::new(i32::MAX, 1),
    ] {
        assert!(FrameBuffer::rgba(FrameHandle::new(), 1, size, Vec::new(), u32::MAX).is_err());
    }
}
