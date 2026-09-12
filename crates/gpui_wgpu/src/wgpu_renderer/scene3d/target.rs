#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct RenderRegion {
    pub size: [u32; 2],
    pub output_size: [u32; 2],
    pub origin: [u32; 2],
    pub rect: [f32; 4],
    pub source_rect: [f32; 4],
}

impl RenderRegion {
    pub(crate) fn full(size: [u32; 2]) -> Self {
        let rect = [0., 0., size[0] as f32, size[1] as f32];
        Self {
            size,
            output_size: size,
            origin: [0; 2],
            rect,
            source_rect: rect,
        }
    }

    pub(super) fn viewport(
        rect: [f32; 4],
        output: [u32; 2],
        scale: f32,
        max_dimension: u32,
    ) -> Option<Self> {
        if rect.iter().any(|value| !value.is_finite())
            || rect[2] <= 0.
            || rect[3] <= 0.
            || !scale.is_finite()
            || scale <= 0.
            || max_dimension == 0
        {
            return None;
        }
        let mut origin = [0; 2];
        let mut size = [0; 2];
        for axis in 0..2 {
            let start = f64::from(rect[axis])
                .floor()
                .clamp(0., f64::from(output[axis]));
            let end = (f64::from(rect[axis]) + f64::from(rect[axis + 2]))
                .ceil()
                .clamp(0., f64::from(output[axis]));
            if end <= start {
                return None;
            }
            origin[axis] = start as u32;
            size[axis] = (end - start) as u32;
        }
        let output_size = size;
        let scale =
            f64::from(scale).min(f64::from(max_dimension) / f64::from(size[0].max(size[1])));
        let size = size.map(|value| {
            (f64::from(value) * scale)
                .ceil()
                .clamp(1., f64::from(max_dimension)) as u32
        });
        let density = [
            size[0] as f64 / output_size[0] as f64,
            size[1] as f64 / output_size[1] as f64,
        ];
        let raster_rect = [
            ((f64::from(rect[0]) - f64::from(origin[0])) * density[0]) as f32,
            ((f64::from(rect[1]) - f64::from(origin[1])) * density[1]) as f32,
            (f64::from(rect[2]) * density[0]) as f32,
            (f64::from(rect[3]) * density[1]) as f32,
        ];
        if raster_rect.iter().any(|value| !value.is_finite()) {
            return None;
        }
        Some(Self {
            size,
            output_size,
            origin,
            rect: raster_rect,
            source_rect: rect,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scene3d_resolution_scales_target_density_without_changing_projection_space() {
        let bounds = [30.25, -10.75, 80.5, 60.5];
        let base = RenderRegion::viewport(bounds, [200, 150], 1., 16384).unwrap();
        for scale in [0.25, 0.7, 1., 1.5, 2., f32::MIN_POSITIVE, f32::MAX] {
            let region = RenderRegion::viewport(bounds, [200, 150], scale, 256).unwrap();
            assert_eq!(region.output_size, base.output_size);
            assert_eq!(region.origin, base.origin);
            assert_eq!(region.source_rect, bounds);
            for axis in 0..2 {
                assert!((1..=256).contains(&region.size[axis]));
                let density = region.size[axis] as f32 / region.output_size[axis] as f32;
                for pixel in 0..region.output_size[axis] {
                    let local = pixel as f32 + 0.5;
                    let source_uv = (local - base.rect[axis]) / base.rect[axis + 2];
                    let target_uv = (local * density - region.rect[axis]) / region.rect[axis + 2];
                    assert!((source_uv - target_uv).abs() < 1e-6);
                }
            }
        }
        assert_eq!(
            RenderRegion::viewport(bounds, [200, 150], 0.5, 256)
                .unwrap()
                .size,
            [41, 25]
        );
        assert_eq!(
            RenderRegion::viewport(bounds, [200, 150], 2., 256)
                .unwrap()
                .size,
            [162, 100]
        );
        assert_eq!(
            RenderRegion::viewport(bounds, [200, 150], f32::MAX, 256)
                .unwrap()
                .size,
            [256, 159]
        );
        for scale in [0., -1., f32::NAN, f32::INFINITY] {
            assert!(RenderRegion::viewport(bounds, [200, 150], scale, 256).is_none());
        }
        assert!(RenderRegion::viewport(bounds, [200, 150], 1., 0).is_none());
        assert!(
            RenderRegion::viewport([-f32::MAX / 2., 0., f32::MAX, 10.], [200, 150], 2., 256)
                .is_none()
        );
    }

    #[test]
    fn scene3d_target_regions_preserve_pixel_centers_and_source_coordinates() {
        for rect in [
            [30., 50., 80., 60.],
            [30.25, 50.75, 80.5, 60.5],
            [-10.25, -20.5, 80., 60.],
            [190.75, 140.25, 80., 60.],
            [-50., -50., 500., 500.],
        ] {
            let region = RenderRegion::viewport(rect, [200, 150], 1., 16384).unwrap();
            assert_eq!(region.source_rect, rect);
            for axis in 0..2 {
                assert!(region.origin[axis] + region.size[axis] <= [200, 150][axis]);
                for pixel in 0..region.size[axis] {
                    let local = pixel as f32 + 0.5;
                    let window = local + region.origin[axis] as f32;
                    let local_uv = (local - region.rect[axis]) / region.rect[axis + 2];
                    let window_uv = (window - rect[axis]) / rect[axis + 2];
                    assert!((local_uv - window_uv).abs() < 1e-6);
                }
            }
        }
        assert_eq!(
            RenderRegion::viewport([30.25, 50.75, 80.5, 60.5], [200, 150], 1., 16384)
                .unwrap()
                .size,
            [81, 62],
        );
        assert_eq!(
            RenderRegion::viewport([190.75, 140.25, 80., 60.], [200, 150], 1., 16384)
                .unwrap()
                .size,
            [10, 10],
        );
    }

    #[test]
    fn scene3d_target_regions_reject_empty_and_invalid_coverage() {
        for rect in [
            [200., 0., 10., 10.],
            [-10., 0., 10., 10.],
            [0., 150., 10., 10.],
            [0., -10., 10., 10.],
            [0., 0., 0., 10.],
            [0., 0., 10., -1.],
            [f32::NAN, 0., 10., 10.],
            [0., 0., f32::INFINITY, 10.],
        ] {
            assert!(RenderRegion::viewport(rect, [200, 150], 1., 16384).is_none());
        }
        assert!(RenderRegion::viewport([0., 0., 10., 10.], [0, 150], 1., 16384).is_none());
        let large =
            RenderRegion::viewport([-f32::MAX / 2., 0., f32::MAX, 10.], [200, 150], 1., 16384)
                .unwrap();
        assert_eq!(large.size, [200, 10]);
        assert!(large.rect.iter().all(|value| value.is_finite()));
    }
}
