use super::ReadFrame;
use crate::CameraError;
use gpui::{Bounds, point, px, size};
use std::fmt;

/// A world point's forward depth relative to one output pixel's nearest surface.
/// This is not continuous visibility, object membership, or a ray intersection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DepthRelation {
    Background,
    InFront,
    WithinTolerance,
    Behind,
}

/// Comparison using the containing physical pixel, with no depth interpolation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DepthComparison {
    pub pixel: [u32; 2],
    /// Nonnegative camera-forward depth in scene units, not ray distance.
    pub point_depth: f32,
    /// Nearest surviving surface depth; `None` denotes the frame's background sentinel.
    pub surface_depth: Option<f32>,
    pub relation: DepthRelation,
}

/// Invalid query parameters or unavailable/malformed frame depth data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DepthQueryError {
    InvalidTolerance,
    MissingDepth,
    InvalidSize,
    PixelCount { expected: u64, actual: usize },
    InvalidSample { pixel: [u32; 2] },
    Camera(CameraError),
}

impl fmt::Display for DepthQueryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidTolerance => f.write_str("depth tolerance must be finite and nonnegative"),
            Self::MissingDepth => f.write_str("frame has no linear-depth channel"),
            Self::InvalidSize => f.write_str("depth queries require nonzero frame dimensions"),
            Self::PixelCount { expected, actual } => write!(
                f,
                "linear-depth channel has {actual} pixels; expected {expected}"
            ),
            Self::InvalidSample { pixel } => write!(f, "invalid linear depth at pixel {pixel:?}"),
            Self::Camera(source) => write!(f, "depth query camera: {source}"),
        }
    }
}

impl std::error::Error for DepthQueryError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Camera(source) => Some(source),
            _ => None,
        }
    }
}

impl ReadFrame {
    /// Compares a world point against the containing pixel's linear depth, using
    /// this frame's retained camera and physical dimensions. `None` means outside
    /// the clip volume or half-open output rectangle. Tolerance is an absolute,
    /// finite, nonnegative forward distance in scene units; equality is included.
    /// Requires a complete depth channel even for clipped points. Only the selected
    /// sample is checked for finite nonnegative depth. No GPU work or allocation
    /// occurs. Pixel-center coverage and transparency follow the depth output,
    /// not color MSAA or alpha-weighted visibility.
    pub fn compare_depth(
        &self,
        world: [f32; 3],
        tolerance: f32,
    ) -> Result<Option<DepthComparison>, DepthQueryError> {
        if !tolerance.is_finite() || tolerance < 0. {
            return Err(DepthQueryError::InvalidTolerance);
        }
        let depths = self
            .pixels
            .linear_depth
            .as_ref()
            .ok_or(DepthQueryError::MissingDepth)?;
        let [width, height] = self.pixels.size;
        if width == 0 || height == 0 {
            return Err(DepthQueryError::InvalidSize);
        }
        let expected = u64::from(width) * u64::from(height);
        if depths.len() as u64 != expected {
            return Err(DepthQueryError::PixelCount {
                expected,
                actual: depths.len(),
            });
        }
        let viewport = Bounds::new(
            point(px(0.), px(0.)),
            size(px(width as f32), px(height as f32)),
        );
        let Some(projected) = self
            .camera
            .world_to_screen(viewport, world)
            .map_err(DepthQueryError::Camera)?
        else {
            return Ok(None);
        };
        let x = f64::from(projected.position.x);
        let y = f64::from(projected.position.y);
        if !projected.in_frustum
            || x < 0.
            || y < 0.
            || x >= f64::from(width)
            || y >= f64::from(height)
        {
            return Ok(None);
        }
        let pixel = [x.floor() as u32, y.floor() as u32];
        let index = (u64::from(pixel[1]) * u64::from(width) + u64::from(pixel[0])) as usize;
        let surface = depths[index];
        let background = self.pixels.depth_background.is_background(surface);
        if !background && (!surface.is_finite() || surface < 0.) {
            return Err(DepthQueryError::InvalidSample { pixel });
        }
        let delta = f64::from(projected.depth) - f64::from(surface);
        let relation = if background {
            DepthRelation::Background
        } else if delta.abs() <= f64::from(tolerance) {
            DepthRelation::WithinTolerance
        } else if delta < 0. {
            DepthRelation::InFront
        } else {
            DepthRelation::Behind
        };
        Ok(Some(DepthComparison {
            pixel,
            point_depth: projected.depth,
            surface_depth: (!background).then_some(surface),
            relation,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Camera, Projection, Scene3dPixels};
    use std::sync::Arc;

    #[test]
    fn zero_depth_surfaces_are_distinct_from_the_retained_background_sentinel() {
        let mut frame = frame(Projection::Orthographic { vertical_size: 4. });
        frame.camera.near = 0.;
        frame.pixels.depth_background = crate::DepthBackground::NegativeOne;
        frame.pixels.linear_depth = Some(vec![0., -1., 4., 8., 4., -1., 8., 4.]);
        let viewport = Bounds::new(point(px(0.), px(0.)), size(px(4.), px(2.)));
        let on_plane = frame
            .camera
            .screen_to_world(viewport, point(px(0.5), px(0.5)), 0.)
            .unwrap();
        assert_eq!(frame.world_position_at(0, 0).unwrap(), Some(on_plane));
        assert_eq!(frame.world_position_at(1, 0).unwrap(), None);
        let comparison = frame.compare_depth(on_plane, 0.).unwrap().unwrap();
        assert_eq!(comparison.relation, DepthRelation::WithinTolerance);
        assert_eq!(comparison.surface_depth, Some(0.));
        let behind = frame
            .camera
            .screen_to_world(viewport, point(px(0.5), px(0.5)), 1.)
            .unwrap();
        assert_eq!(
            frame.compare_depth(behind, 0.).unwrap().unwrap().relation,
            DepthRelation::Behind
        );
        let empty = frame
            .camera
            .screen_to_world(viewport, point(px(1.5), px(0.5)), 0.)
            .unwrap();
        let comparison = frame.compare_depth(empty, 0.).unwrap().unwrap();
        assert_eq!(comparison.relation, DepthRelation::Background);
        assert_eq!(comparison.surface_depth, None);
        for invalid in [-2., f32::NAN, f32::INFINITY] {
            frame.pixels.linear_depth.as_mut().unwrap()[0] = invalid;
            assert!(frame.world_position_at(0, 0).is_err());
            assert!(matches!(
                frame.compare_depth(on_plane, 0.),
                Err(DepthQueryError::InvalidSample { .. })
            ));
        }
        frame.pixels.depth_background = crate::DepthBackground::Zero;
        frame.pixels.linear_depth.as_mut().unwrap()[0] = 0.;
        assert_eq!(frame.world_position_at(0, 0).unwrap(), None);
        assert_eq!(
            frame.compare_depth(on_plane, 0.).unwrap().unwrap().relation,
            DepthRelation::Background
        );
    }

    fn frame(projection: Projection) -> ReadFrame {
        ReadFrame {
            pixels: Scene3dPixels {
                depth_background: Default::default(),
                size: [4, 2],
                rgba: None,
                linear_rgba: None,
                object_ids: None,
                linear_depth: Some(vec![0., 4., 4., 8., 4., 0., 8., 4.]),
                world_normals: None,
            },
            objects: Arc::from([]),
            camera: Camera {
                eye: [2., 3., 5.],
                target: [2., 3., 4.],
                projection,
                lens_shift: [0.25, -0.5],
                aspect_ratio: Some(1.5),
                near: 0.5,
                far: 10.,
                ..Default::default()
            },
        }
    }

    fn world(frame: &ReadFrame, pixel: [f32; 2], depth: f32) -> [f32; 3] {
        frame
            .camera
            .screen_to_world(
                Bounds::new(point(px(0.), px(0.)), size(px(4.), px(2.))),
                point(px(pixel[0]), px(pixel[1])),
                depth,
            )
            .unwrap()
    }

    #[test]
    fn compares_forward_depth_in_both_projections_without_ids() {
        for projection in [
            Projection::default(),
            Projection::Orthographic { vertical_size: 4. },
        ] {
            let frame = frame(projection);
            for (depth, tolerance, relation) in [
                (3., 0., DepthRelation::InFront),
                (4., 0., DepthRelation::WithinTolerance),
                (4.25, 0.25, DepthRelation::WithinTolerance),
                (3.75, 0.25, DepthRelation::WithinTolerance),
                (4.5, 0.25, DepthRelation::Behind),
            ] {
                let query = frame
                    .compare_depth(world(&frame, [2.75, 0.75], depth), tolerance)
                    .unwrap()
                    .unwrap();
                assert_eq!(query.pixel, [2, 0]);
                assert_eq!(query.surface_depth, Some(4.));
                assert_eq!(query.point_depth, depth);
                assert_eq!(query.relation, relation);
            }
            let query = frame
                .compare_depth(world(&frame, [0.5, 0.5], 9.), 0.)
                .unwrap()
                .unwrap();
            assert_eq!(query.relation, DepthRelation::Background);
            assert_eq!(query.surface_depth, None);
            for pixel in [[1, 0], [3, 0], [0, 1], [2, 1]] {
                let surface = frame
                    .world_position_at(pixel[0], pixel[1])
                    .unwrap()
                    .unwrap();
                let query = frame.compare_depth(surface, 1e-5).unwrap().unwrap();
                assert_eq!(query.pixel, pixel);
                assert_eq!(query.relation, DepthRelation::WithinTolerance);
            }
        }
    }

    #[test]
    fn rotated_infinite_camera_uses_forward_depth_not_ray_distance() {
        let mut frame = frame(Projection::Perspective { vertical_fov: 2. });
        frame.camera.target = [3., 5., 4.];
        frame.camera.far = f32::INFINITY;
        let point = world(&frame, [2.75, 0.75], 3.75);
        let distance = point
            .iter()
            .zip(frame.camera.eye)
            .map(|(value, eye)| (value - eye).powi(2))
            .sum::<f32>()
            .sqrt();
        assert!(distance > 4.);
        let query = frame.compare_depth(point, 0.).unwrap().unwrap();
        assert_eq!(query.pixel, [2, 0]);
        assert_eq!(query.relation, DepthRelation::InFront);
        assert!((query.point_depth - 3.75).abs() < 1e-5);
        assert_eq!(
            frame.compare_depth(point, 0.3).unwrap().unwrap().relation,
            DepthRelation::WithinTolerance
        );
    }

    #[test]
    fn distinguishes_clipping_background_and_invalid_frame_data() {
        let mut frame = frame(Projection::Orthographic { vertical_size: 4. });
        let point = world(&frame, [1.5, 0.5], 4.);
        let edge = frame
            .compare_depth(world(&frame, [0., 0.], 0.5), 0.)
            .unwrap()
            .unwrap();
        assert_eq!(edge.pixel, [0, 0]);
        assert_eq!(edge.relation, DepthRelation::Background);
        for (pixel, depth) in [
            ([-1., 0.5], 4.),
            ([4., 0.5], 4.),
            ([1.5, 2.], 4.),
            ([1.5, 0.5], 0.25),
            ([1.5, 0.5], 10.),
        ] {
            assert!(
                frame
                    .compare_depth(world(&frame, pixel, depth), 0.)
                    .unwrap()
                    .is_none()
            );
        }
        assert!(frame.compare_depth(frame.camera.eye, 0.).unwrap().is_none());
        for tolerance in [-1., f32::NAN, f32::INFINITY] {
            assert_eq!(
                frame.compare_depth(point, tolerance),
                Err(DepthQueryError::InvalidTolerance)
            );
        }
        assert_eq!(
            frame.compare_depth([f32::NAN; 3], 0.),
            Err(DepthQueryError::Camera(CameraError::InvalidPoint))
        );
        for sample in [-1., f32::NAN, f32::INFINITY] {
            frame.pixels.linear_depth.as_mut().unwrap()[1] = sample;
            assert_eq!(
                frame.compare_depth(point, 0.),
                Err(DepthQueryError::InvalidSample { pixel: [1, 0] })
            );
        }
        frame.pixels.linear_depth = Some(vec![0.; 7]);
        assert_eq!(
            frame.compare_depth(point, 0.),
            Err(DepthQueryError::PixelCount {
                expected: 8,
                actual: 7
            })
        );
        frame.pixels.size = [u32::MAX; 2];
        assert_eq!(
            frame.compare_depth(point, 0.),
            Err(DepthQueryError::PixelCount {
                expected: u64::from(u32::MAX).pow(2),
                actual: 7
            })
        );
        frame.pixels.size = [0, 2];
        assert_eq!(
            frame.compare_depth(point, 0.),
            Err(DepthQueryError::InvalidSize)
        );
        frame.pixels.linear_depth = None;
        assert_eq!(
            frame.compare_depth(point, 0.),
            Err(DepthQueryError::MissingDepth)
        );
        frame.pixels.size = [4, 2];
        assert_eq!(
            frame.compare_depth(frame.camera.eye, 0.),
            Err(DepthQueryError::MissingDepth)
        );
    }
}
