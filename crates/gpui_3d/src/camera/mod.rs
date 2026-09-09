mod orbit;
pub use orbit::{OrbitController, OrbitError, OrbitSettings};

use crate::{
    Aabb,
    math::{Matrix, cross, dot, multiply, sub, transform},
};
use gpui::{Bounds, Pixels, Point, point, px};
use std::fmt;

/// Centered projection. Aspect ratio is supplied by the output viewport.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Projection {
    /// Vertical angle in radians, strictly between zero and pi.
    Perspective { vertical_fov: f32 },
    /// Positive full vertical span in scene units.
    Orthographic { vertical_size: f32 },
}

impl Default for Projection {
    fn default() -> Self {
        Self::Perspective {
            vertical_fov: std::f32::consts::FRAC_PI_4,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CameraError {
    InvalidView,
    InvalidProjection,
    InvalidViewport,
    InvalidPoint,
    InvalidFraming,
    Unrepresentable,
}
impl fmt::Display for CameraError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidView => {
                "camera requires finite distinct eye/target and a nonzero up vector"
            }
            Self::InvalidProjection => "invalid projection or clip range",
            Self::InvalidViewport => {
                "viewport requires finite origin and positive finite dimensions"
            }
            Self::InvalidPoint => "point coordinates must be finite",
            Self::InvalidFraming => "framing margin must be finite and at least one",
            Self::Unrepresentable => "camera calculation exceeds finite coordinate precision",
        })
    }
}
impl std::error::Error for CameraError {}

/// Invalid ray origin or zero/non-finite direction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RayError;
impl fmt::Display for RayError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ray requires a finite origin and finite nonzero direction")
    }
}
impl std::error::Error for RayError {}

/// World-space ray with normalized direction and nonnegative hit distances.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ray {
    origin: [f32; 3],
    direction: [f32; 3],
}
impl Ray {
    pub fn new(origin: [f32; 3], direction: [f32; 3]) -> Result<Self, RayError> {
        if !origin.iter().all(|v| v.is_finite()) {
            return Err(RayError);
        }
        Ok(Self {
            origin,
            direction: normalize(direction).ok_or(RayError)?,
        })
    }
    pub fn origin(self) -> [f32; 3] {
        self.origin
    }
    pub fn direction(self) -> [f32; 3] {
        self.direction
    }
    pub fn at(self, distance: f32) -> [f32; 3] {
        std::array::from_fn(|i| self.origin[i] + self.direction[i] * distance)
    }
}

/// Projection of a point in front of the camera, including points outside its frustum.
#[derive(Clone, Copy, Debug)]
pub struct ScreenPoint {
    pub position: Point<Pixels>,
    /// Linear forward distance in scene units, not ray distance or hardware depth.
    pub depth: f32,
    /// X/Y in -1..1 and hardware Z in 0..1 inside the clip volume.
    pub ndc: [f32; 3],
    /// Geometric clip-volume membership, not an occlusion result.
    pub in_frustum: bool,
}

/// Right-handed camera looking down local -Z, with configurable up and projection.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    pub eye: [f32; 3],
    pub target: [f32; 3],
    pub up: [f32; 3],
    pub projection: Projection,
    pub near: f32,
    pub far: f32,
}
impl Default for Camera {
    fn default() -> Self {
        Self {
            eye: [0., 0., 6.],
            target: [0.; 3],
            up: [0., 1., 0.],
            projection: Projection::default(),
            near: 0.05,
            far: 100.,
        }
    }
}

impl Camera {
    /// Orbits the origin. Angles are radians; pitch stays below the poles.
    pub fn orbit(yaw: f32, pitch: f32, distance: f32) -> Self {
        assert!(yaw.is_finite() && pitch.is_finite() && distance.is_finite() && distance > 0.);
        let pitch = pitch.clamp(-1.5, 1.5);
        Self {
            eye: [
                yaw.sin() * pitch.cos() * distance,
                pitch.sin() * distance,
                yaw.cos() * pitch.cos() * distance,
            ],
            ..Self::default()
        }
    }

    /// Camera right, up, and backward unit vectors. A parallel up vector uses a
    /// deterministic world-axis fallback, allowing exact top and bottom views.
    pub fn axes(self) -> Result<[[f32; 3]; 3], CameraError> {
        if !self.eye.iter().chain(&self.target).all(|v| v.is_finite()) {
            return Err(CameraError::InvalidView);
        }
        let z = normalize(sub(self.eye, self.target)).ok_or(CameraError::InvalidView)?;
        let mut up = normalize(self.up).ok_or(CameraError::InvalidView)?;
        if dot(up, z).abs() > 0.999 {
            up = if z[1].abs() > 0.999 {
                [0., 0., 1.]
            } else {
                [0., 1., 0.]
            };
        }
        let x = normalize(cross(up, z)).ok_or(CameraError::InvalidView)?;
        let y = normalize(cross(z, x)).ok_or(CameraError::InvalidView)?;
        Ok([x, y, z])
    }

    pub fn view_matrix(self) -> Result<[[f32; 4]; 4], CameraError> {
        let [x, y, z] = self.axes()?;
        finite_matrix([
            [x[0], y[0], z[0], 0.],
            [x[1], y[1], z[1], 0.],
            [x[2], y[2], z[2], 0.],
            [-dot(x, self.eye), -dot(y, self.eye), -dot(z, self.eye), 1.],
        ])
    }

    pub fn projection_matrix(self, aspect: f32) -> Result<[[f32; 4]; 4], CameraError> {
        if !aspect.is_finite() || aspect <= 0. {
            return Err(CameraError::InvalidViewport);
        }
        if !self.near.is_finite()
            || !self.far.is_finite()
            || self.near <= 0.
            || self.far <= self.near
        {
            return Err(CameraError::InvalidProjection);
        }
        let range = self.near - self.far;
        finite_matrix(match self.projection {
            Projection::Perspective { vertical_fov } => {
                if !vertical_fov.is_finite()
                    || vertical_fov <= 0.
                    || vertical_fov >= std::f32::consts::PI
                {
                    return Err(CameraError::InvalidProjection);
                }
                let f = 1. / (vertical_fov * 0.5).tan();
                let z = self.far / range;
                [
                    [f / aspect, 0., 0., 0.],
                    [0., f, 0., 0.],
                    [0., 0., z, -1.],
                    [0., 0., z * self.near, 0.],
                ]
            }
            Projection::Orthographic { vertical_size } => {
                if !vertical_size.is_finite() || vertical_size <= 0. {
                    return Err(CameraError::InvalidProjection);
                }
                [
                    [2. / vertical_size / aspect, 0., 0., 0.],
                    [0., 2. / vertical_size, 0., 0.],
                    [0., 0., 1. / range, 0.],
                    [0., 0., self.near / range, 1.],
                ]
            }
        })
    }

    /// Column-major world-to-clip transform. Hardware depth runs from zero to one.
    pub fn view_projection(self, aspect: f32) -> Result<[[f32; 4]; 4], CameraError> {
        finite_matrix(multiply(
            self.projection_matrix(aspect)?,
            self.view_matrix()?,
        ))
    }

    pub fn world_to_view(self, world: [f32; 3]) -> Result<[f32; 3], CameraError> {
        if !world.iter().all(|v| v.is_finite()) {
            return Err(CameraError::InvalidPoint);
        }
        let [x, y, z] = self.axes()?;
        let offset = sub(world, self.eye);
        let p = [dot(offset, x), dot(offset, y), dot(offset, z)];
        if !p.iter().all(|v| v.is_finite()) {
            return Err(CameraError::Unrepresentable);
        }
        Ok(p)
    }

    /// Returns `None` on or behind the eye plane. Screen origin is top-left.
    /// Off-screen and near/far-clipped points in front retain coordinates and depth.
    pub fn world_to_screen(
        self,
        viewport: Bounds<Pixels>,
        world: [f32; 3],
    ) -> Result<Option<ScreenPoint>, CameraError> {
        let [width, height] = viewport_size(viewport)?;
        let projection = self.projection_matrix(width / height)?;
        let view = self.world_to_view(world)?;
        let depth = -view[2];
        if depth <= 0. {
            return Ok(None);
        }
        let clip = transform(projection, [view[0], view[1], view[2], 1.]);
        let ndc = [clip[0] / clip[3], clip[1] / clip[3], clip[2] / clip[3]];
        let position = viewport.origin
            + point(
                px((ndc[0] + 1.) * width * 0.5),
                px((1. - ndc[1]) * height * 0.5),
            );
        if !ndc
            .iter()
            .chain([f32::from(position.x), f32::from(position.y)].iter())
            .all(|v| v.is_finite())
        {
            return Err(CameraError::Unrepresentable);
        }
        Ok(Some(ScreenPoint {
            position,
            depth,
            ndc,
            in_frustum: depth >= self.near
                && depth < self.far
                && ndc[0].abs() <= 1.
                && ndc[1].abs() <= 1.,
        }))
    }

    /// Positions may lie outside the viewport for captured drags. Perspective
    /// rays start at the eye; orthographic rays start on the eye plane and are parallel.
    /// Rays do not themselves impose near/far clipping.
    pub fn screen_to_ray(
        self,
        viewport: Bounds<Pixels>,
        position: Point<Pixels>,
    ) -> Result<Ray, CameraError> {
        let [width, height] = viewport_size(viewport)?;
        if ![f32::from(position.x), f32::from(position.y)]
            .iter()
            .all(|v| v.is_finite())
        {
            return Err(CameraError::InvalidPoint);
        }
        self.projection_matrix(width / height)?;
        let [right, up, backward] = self.axes()?;
        let x = 2. * f32::from(position.x - viewport.origin.x) / width - 1.;
        let y = 1. - 2. * f32::from(position.y - viewport.origin.y) / height;
        let (origin, direction) = match self.projection {
            Projection::Perspective { vertical_fov } => {
                let extent = (vertical_fov * 0.5).tan();
                (
                    self.eye,
                    std::array::from_fn(|i| {
                        right[i] * x * width / height * extent + up[i] * y * extent - backward[i]
                    }),
                )
            }
            Projection::Orthographic { vertical_size } => (
                std::array::from_fn(|i| {
                    self.eye[i]
                        + right[i] * x * width / height * vertical_size * 0.5
                        + up[i] * y * vertical_size * 0.5
                }),
                backward.map(|v| -v),
            ),
        };
        Ray::new(origin, direction).map_err(|_| CameraError::Unrepresentable)
    }

    /// Frames all AABB corners while preserving viewing direction, up, and projection
    /// kind. Adjusts eye, target, clip planes, and orthographic size. Margin is a
    /// multiplicative screen-space factor >= 1. Does not change scene geometry.
    pub fn frame_bounds(
        mut self,
        bounds: Aabb,
        aspect: f32,
        margin: f32,
    ) -> Result<Self, CameraError> {
        self.projection_matrix(aspect)?;
        let [right, up, backward] = self.axes()?;
        if !margin.is_finite() || margin < 1. {
            return Err(CameraError::InvalidFraming);
        }
        let center = std::array::from_fn(|i| bounds.min()[i] * 0.5 + bounds.max()[i] * 0.5);
        let mut points = Vec::with_capacity(8);
        for corner in 0..8 {
            let p = std::array::from_fn(|i| {
                if corner & (1 << i) == 0 {
                    bounds.min()[i]
                } else {
                    bounds.max()[i]
                }
            });
            let p = sub(p, center);
            points.push([dot(p, right), dot(p, up), dot(p, backward)]);
        }
        if !points.iter().flatten().all(|v| v.is_finite()) {
            return Err(CameraError::Unrepresentable);
        }
        let radius = points
            .iter()
            .map(|p| p.iter().map(|v| f64::from(*v).powi(2)).sum::<f64>().sqrt() as f32)
            .fold(0., f32::max)
            .max(0.001);
        let mut distance = radius * 1.01;
        match self.projection {
            Projection::Perspective { vertical_fov } => {
                let tangent = (vertical_fov * 0.5).tan();
                for p in &points {
                    distance = distance
                        .max(p[2] + margin * (p[0].abs() / aspect).max(p[1].abs()) / tangent);
                }
            }
            Projection::Orthographic { .. } => {
                let half = points
                    .iter()
                    .map(|p| (p[0].abs() / aspect).max(p[1].abs()))
                    .fold(0., f32::max)
                    .max(0.001);
                self.projection = Projection::Orthographic {
                    vertical_size: half * 2. * margin,
                };
                distance = radius * 2.;
            }
        }
        self.target = center;
        self.eye = std::array::from_fn(|i| center[i] + backward[i] * distance);
        let min_depth = points
            .iter()
            .map(|p| distance - p[2])
            .fold(f32::INFINITY, f32::min);
        let max_depth = points.iter().map(|p| distance - p[2]).fold(0., f32::max);
        self.near = (min_depth * 0.5).max(f32::MIN_POSITIVE);
        self.far = max_depth * 1.5;
        self.view_projection(aspect)?;
        Ok(self)
    }

    #[cfg(test)]
    pub(crate) fn matrix(self, aspect: f32) -> Matrix {
        self.view_projection(aspect)
            .expect("valid camera and viewport")
    }
}

fn normalize(v: [f32; 3]) -> Option<[f32; 3]> {
    if !v.iter().all(|v| v.is_finite()) {
        return None;
    }
    let length = v.iter().map(|v| f64::from(*v).powi(2)).sum::<f64>().sqrt();
    (length > 0.).then(|| v.map(|v| (f64::from(v) / length) as f32))
}
fn finite_matrix(matrix: Matrix) -> Result<Matrix, CameraError> {
    if matrix.iter().flatten().all(|v| v.is_finite()) {
        Ok(matrix)
    } else {
        Err(CameraError::Unrepresentable)
    }
}
fn viewport_size(bounds: Bounds<Pixels>) -> Result<[f32; 2], CameraError> {
    let [x, y, width, height] = [
        f32::from(bounds.origin.x),
        f32::from(bounds.origin.y),
        f32::from(bounds.size.width),
        f32::from(bounds.size.height),
    ];
    if ![x, y, width, height].iter().all(|v| v.is_finite()) || width <= 0. || height <= 0. {
        return Err(CameraError::InvalidViewport);
    }
    Ok([width, height])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Material, Mesh, Object, Scene};
    use gpui::{rgb, size};

    fn viewport() -> Bounds<Pixels> {
        Bounds::new(point(px(73.), px(51.)), size(px(800.), px(600.)))
    }
    fn projections() -> [Projection; 2] {
        [
            Projection::Perspective { vertical_fov: 0.9 },
            Projection::Orthographic { vertical_size: 5. },
        ]
    }
    fn close(a: [f32; 3], b: [f32; 3]) {
        for (a, b) in a.into_iter().zip(b) {
            assert!((a - b).abs() < 2e-4, "{a} != {b}");
        }
    }

    #[test]
    fn projection_rays_and_shader_matrix_agree_across_offsets_roll_and_scale() {
        for projection in projections() {
            for up in [[0., 1., 0.], [1., 0., 0.]] {
                let camera = Camera {
                    projection,
                    up,
                    ..Camera::orbit(0.35, -0.2, 8.)
                };
                let p = [0.4, -0.3, 0.2];
                let view = camera.world_to_view(p).unwrap();
                for scale in [1., 1.5, 2.] {
                    let bounds = Bounds::new(
                        viewport().origin * scale,
                        size(
                            viewport().size.width * scale,
                            viewport().size.height * scale,
                        ),
                    );
                    let screen = camera.world_to_screen(bounds, p).unwrap().unwrap();
                    assert!(screen.in_frustum);
                    assert!((screen.depth + view[2]).abs() < 1e-5);
                    let clip = transform(
                        camera.view_projection(4. / 3.).unwrap(),
                        [p[0], p[1], p[2], 1.],
                    );
                    close(
                        screen.ndc,
                        [clip[0] / clip[3], clip[1] / clip[3], clip[2] / clip[3]],
                    );
                    let ray = camera.screen_to_ray(bounds, screen.position).unwrap();
                    let distance = dot(sub(p, ray.origin()), ray.direction());
                    close(ray.at(distance), p);
                    let unscaled = camera.world_to_screen(viewport(), p).unwrap().unwrap();
                    assert!(
                        (f32::from(screen.position.x) / scale - f32::from(unscaled.position.x))
                            .abs()
                            < 1e-3
                    );
                }
            }
        }
    }

    #[test]
    fn orthographic_rays_shift_origin_and_pick_the_correct_surface() {
        let camera = Camera {
            projection: Projection::Orthographic { vertical_size: 4. },
            near: 1.,
            far: 9.,
            ..Default::default()
        };
        let bounds = viewport();
        let position = camera
            .world_to_screen(bounds, [1., 0., 0.])
            .unwrap()
            .unwrap()
            .position;
        let center = camera.screen_to_ray(bounds, bounds.center()).unwrap();
        let ray = camera.screen_to_ray(bounds, position).unwrap();
        close(ray.direction(), center.direction());
        close(ray.origin(), [1., 0., 6.]);
        let scene = Scene::new()
            .camera(camera)
            .object(Object::new(Mesh::plane(), Material::color(rgb(0xffffff))).id("center"))
            .object(
                Object::new(Mesh::plane(), Material::color(rgb(0xffffff)))
                    .id("side")
                    .position([1., 0., 0.]),
            )
            .object(
                Object::new(Mesh::plane(), Material::color(rgb(0xffffff)))
                    .id("near-clipped")
                    .position([1., 0., 5.5]),
            );
        let hit = scene.pick(bounds, position).unwrap();
        assert_eq!(hit.object_id, Some("side".into()));
        close(hit.position, [1., 0., 0.]);
        assert!((hit.distance - 6.).abs() < 1e-5);
        assert_eq!(
            scene.raycast(ray).unwrap().object_id,
            Some("near-clipped".into())
        );
        let arbitrary = Ray::new([1., 0., -2.], [0., 0., 50.]).unwrap();
        let hit = scene.raycast(arbitrary).unwrap();
        assert_eq!(hit.object_id, Some("side".into()));
        assert!((hit.distance - 2.).abs() < 1e-5);
    }

    #[test]
    fn clipping_and_invalid_queries_have_explicit_results() {
        for projection in projections() {
            let camera = Camera {
                projection,
                eye: [0.; 3],
                target: [0., 0., -1.],
                near: 1.,
                far: 10.,
                ..Default::default()
            };
            assert!(
                camera
                    .world_to_screen(viewport(), [0., 0., 1.])
                    .unwrap()
                    .is_none()
            );
            assert!(
                camera
                    .world_to_screen(viewport(), [0.; 3])
                    .unwrap()
                    .is_none()
            );
            for (depth, inside) in [
                (0.5, false),
                (1., true),
                (5., true),
                (10., false),
                (11., false),
            ] {
                let projected = camera
                    .world_to_screen(viewport(), [0., 0., -depth])
                    .unwrap()
                    .unwrap();
                assert_eq!(projected.in_frustum, inside);
                assert!((projected.depth - depth).abs() < 1e-5);
            }
            let outside = viewport().origin - point(px(200.), px(150.));
            assert!(camera.screen_to_ray(viewport(), outside).is_ok());
            assert!(matches!(
                camera.world_to_screen(Bounds::default(), [0., 0., -2.]),
                Err(CameraError::InvalidViewport)
            ));
            assert!(matches!(
                camera.world_to_screen(viewport(), [f32::NAN, 0., 0.]),
                Err(CameraError::InvalidPoint)
            ));
            for (depth, hardware) in [(1., 0.), (10., 1.)] {
                let clip = transform(camera.view_projection(1.).unwrap(), [0., 0., -depth, 1.]);
                assert!((clip[2] / clip[3] - hardware).abs() < 1e-6);
            }
        }
        let mut invalid = Camera::default();
        invalid.target = invalid.eye;
        assert_eq!(invalid.view_matrix(), Err(CameraError::InvalidView));
        invalid = Camera {
            projection: Projection::Orthographic { vertical_size: 0. },
            ..Default::default()
        };
        assert_eq!(
            invalid.projection_matrix(1.),
            Err(CameraError::InvalidProjection)
        );
        assert!(Ray::new([0.; 3], [0.; 3]).is_err());
        assert!(Ray::new([f32::INFINITY, 0., 0.], [1., 0., 0.]).is_err());
    }

    #[test]
    fn framing_contains_every_corner_and_preserves_view_direction() {
        let bounds = Aabb::new([-3., -1., -2.], [4., 2., 1.]).unwrap();
        for projection in projections() {
            for aspect in [0.25, 1., 3.] {
                for eye in [[4., 3., 6.], [0., 6., 0.]] {
                    let camera = Camera {
                        projection,
                        eye,
                        ..Default::default()
                    };
                    let framed = camera.frame_bounds(bounds, aspect, 1.2).unwrap();
                    close(camera.axes().unwrap()[2], framed.axes().unwrap()[2]);
                    close(framed.target, [0.5, 0.5, -0.5]);
                    let viewport =
                        Bounds::new(point(px(40.), px(50.)), size(px(600. * aspect), px(600.)));
                    for corner in 0..8 {
                        let p = std::array::from_fn(|i| {
                            if corner & (1 << i) == 0 {
                                bounds.min()[i]
                            } else {
                                bounds.max()[i]
                            }
                        });
                        let screen = framed.world_to_screen(viewport, p).unwrap().unwrap();
                        assert!(screen.in_frustum, "{framed:?}: {p:?}");
                        assert!(screen.ndc[0].abs() <= 1. / 1.2 + 1e-5);
                        assert!(screen.ndc[1].abs() <= 1. / 1.2 + 1e-5);
                    }
                }
            }
        }
        let point_bounds = Aabb::new([0.; 3], [0.; 3]).unwrap();
        assert!(
            Camera::default()
                .frame_bounds(point_bounds, 1., 1.1)
                .is_ok()
        );
        assert_eq!(
            Camera::default().frame_bounds(bounds, 1., 0.5),
            Err(CameraError::InvalidFraming)
        );
    }
}
