use super::{Camera, OrbitError, Projection, distance, rotate, validate_camera};
use crate::math::{cross, dot};

#[derive(Clone, Copy)]
pub(super) struct Motion {
    start: Camera,
    pub current: Camera,
    pub elapsed_nanos: u128,
}

impl Motion {
    pub fn new(camera: Camera) -> Self {
        Self {
            start: camera,
            current: camera,
            elapsed_nanos: 0,
        }
    }

    pub fn sample(self, target: Camera, t: f64) -> Result<Camera, OrbitError> {
        let start = self.start;
        let lerp = |a: f32, b: f32| (f64::from(a) * (1. - t) + f64::from(b) * t) as f32;
        let log_lerp = |a: f64, b: f64| (a.ln() * (1. - t) + b.ln() * t).exp();
        let a = start.axes()?[2];
        let b = target.axes()?[2];
        let direction = if a == b {
            a
        } else {
            let up = crate::Ray::new([0.; 3], start.up)
                .map_err(|_| crate::CameraError::InvalidView)?
                .direction();
            let horizontal = |v: [f32; 3]| {
                let height = dot(v, up);
                let flat = std::array::from_fn(|i| v[i] - height * up[i]);
                (dot(flat, flat) > 1e-12)
                    .then(|| crate::Ray::new([0.; 3], flat).map(|ray| ray.direction()))
                    .transpose()
            };
            let flat_a = horizontal(a).map_err(|_| crate::CameraError::InvalidView)?;
            let flat_b = horizontal(b).map_err(|_| crate::CameraError::InvalidView)?;
            let fallback = cross(start.axes()?[0], up);
            let flat_a = flat_a.or(flat_b).unwrap_or(fallback);
            let flat_b = flat_b.unwrap_or(flat_a);
            let yaw = dot(cross(flat_a, flat_b), up).atan2(dot(flat_a, flat_b));
            let pitch = lerp(
                dot(a, up).clamp(-1., 1.).asin(),
                dot(b, up).clamp(-1., 1.).asin(),
            );
            let flat = rotate(flat_a, up, (f64::from(yaw) * t) as f32);
            std::array::from_fn(|i| flat[i] * pitch.cos() + up[i] * pitch.sin())
        };
        let radius = log_lerp(f64::from(distance(start)), f64::from(distance(target)));
        let mut camera = Camera {
            target: std::array::from_fn(|i| lerp(start.target[i], target.target[i])),
            ..target
        };
        camera.eye = std::array::from_fn(|i| {
            (f64::from(camera.target[i]) + f64::from(direction[i]) * radius) as f32
        });
        if start.eye == target.eye && start.target == target.target {
            camera.eye = start.eye;
        }
        camera.projection = match (start.projection, target.projection) {
            (
                Projection::Perspective { vertical_fov: a },
                Projection::Perspective { vertical_fov: b },
            ) => Projection::Perspective {
                vertical_fov: (2.
                    * log_lerp((f64::from(a) * 0.5).tan(), (f64::from(b) * 0.5).tan()).atan())
                    as f32,
            },
            (
                Projection::Orthographic { vertical_size: a },
                Projection::Orthographic { vertical_size: b },
            ) => Projection::Orthographic {
                vertical_size: log_lerp(f64::from(a), f64::from(b)) as f32,
            },
            _ => unreachable!("projection changes cancel damping"),
        };
        validate_camera(camera)?;
        Ok(camera)
    }
}
