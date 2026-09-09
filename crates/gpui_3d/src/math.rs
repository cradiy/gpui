pub(crate) type Matrix = [[f32; 4]; 4];
pub(crate) fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|i| a[i] - b[i])
}
pub(crate) fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}
pub(crate) fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
pub(crate) fn unit(v: [f32; 3]) -> [f32; 3] {
    let n = dot(v, v).sqrt().max(0.000001);
    v.map(|x| x / n)
}
pub(crate) fn multiply(a: Matrix, b: Matrix) -> Matrix {
    std::array::from_fn(|c| std::array::from_fn(|r| (0..4).map(|k| a[k][r] * b[c][k]).sum()))
}
#[cfg(test)]
pub(crate) fn transform(m: Matrix, p: [f32; 4]) -> [f32; 4] {
    std::array::from_fn(|r| (0..4).map(|c| m[c][r] * p[c]).sum())
}

/// Translation, XYZ Euler rotation and nonzero scale in right-handed world space.
#[derive(Clone, Copy, Debug)]
pub struct Transform {
    /// World position.
    pub position: [f32; 3],
    /// Euler angles in radians, applied X, then Y, then Z.
    pub rotation: [f32; 3],
    /// Per-axis scale. Negative values reflect the mesh.
    pub scale: [f32; 3],
}
impl Default for Transform {
    fn default() -> Self {
        Self {
            position: [0.; 3],
            rotation: [0.; 3],
            scale: [1.; 3],
        }
    }
}
impl Transform {
    pub(crate) fn matrices(self) -> (Matrix, Matrix) {
        assert!(
            self.position
                .iter()
                .chain(&self.rotation)
                .chain(&self.scale)
                .all(|x| x.is_finite())
        );
        assert!(self.scale.iter().all(|x| x.abs() >= 0.0001));
        let [x, y, z] = self.rotation;
        let rx = [
            [1., 0., 0., 0.],
            [0., x.cos(), x.sin(), 0.],
            [0., -x.sin(), x.cos(), 0.],
            [0., 0., 0., 1.],
        ];
        let ry = [
            [y.cos(), 0., -y.sin(), 0.],
            [0., 1., 0., 0.],
            [y.sin(), 0., y.cos(), 0.],
            [0., 0., 0., 1.],
        ];
        let rz = [
            [z.cos(), z.sin(), 0., 0.],
            [-z.sin(), z.cos(), 0., 0.],
            [0., 0., 1., 0.],
            [0., 0., 0., 1.],
        ];
        let mut model = multiply(rz, multiply(ry, rx));
        let mut normal = model;
        for c in 0..3 {
            for r in 0..3 {
                model[c][r] *= self.scale[c];
                normal[c][r] /= self.scale[c];
            }
        }
        model[3] = [self.position[0], self.position[1], self.position[2], 1.];
        (model, normal)
    }
}

/// Perspective camera looking at a target with world-up along positive Y.
#[derive(Clone, Copy, Debug)]
pub struct Camera {
    /// Camera position.
    pub eye: [f32; 3],
    /// Look-at target, distinct from the eye.
    pub target: [f32; 3],
    /// Vertical field of view in radians.
    pub fov: f32,
    /// Positive near clip distance.
    pub near: f32,
    /// Far clip distance, greater than near.
    pub far: f32,
}
impl Default for Camera {
    fn default() -> Self {
        Self {
            eye: [0., 0., 6.],
            target: [0.; 3],
            fov: std::f32::consts::FRAC_PI_4,
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
            ..Default::default()
        }
    }
    pub(crate) fn matrix(self, aspect: f32) -> Matrix {
        assert!(self.eye.iter().chain(&self.target).all(|x| x.is_finite()));
        assert!(self.fov.is_finite() && self.fov > 0. && self.fov < std::f32::consts::PI);
        assert!(
            self.near.is_finite() && self.far.is_finite() && self.near > 0. && self.far > self.near
        );
        let backward = sub(self.eye, self.target);
        assert!(dot(backward, backward) > 0.000001);
        let z = unit(backward);
        let up = if z[1].abs() > 0.999 {
            [0., 0., 1.]
        } else {
            [0., 1., 0.]
        };
        let x = unit(cross(up, z));
        let y = cross(z, x);
        let view = [
            [x[0], y[0], z[0], 0.],
            [x[1], y[1], z[1], 0.],
            [x[2], y[2], z[2], 0.],
            [-dot(x, self.eye), -dot(y, self.eye), -dot(z, self.eye), 1.],
        ];
        let f = 1. / (self.fov * 0.5).tan();
        let z = self.far / (self.near - self.far);
        let projection = [
            [f / aspect.max(0.001), 0., 0., 0.],
            [0., f, 0., 0.],
            [0., 0., z, -1.],
            [0., 0., z * self.near, 0.],
        ];
        multiply(projection, view)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn perspective_maps_clip_planes_and_preserves_camera_target() {
        let camera = Camera::default();
        let m = camera.matrix(2.);
        let center = transform(m, [0., 0., 0., 1.]);
        assert!(center[0].abs() < 0.00001 && center[1].abs() < 0.00001 && center[3] > 0.);
        for (distance, depth) in [(camera.near, 0.), (camera.far, 1.)] {
            let p = transform(m, [0., 0., camera.eye[2] - distance, 1.]);
            assert!((p[2] / p[3] - depth).abs() < 0.0001);
        }
    }
    #[test]
    fn normal_transform_remains_orthogonal_under_nonuniform_scale() {
        let (model, normal) = Transform {
            rotation: [0.3, 0.6, -0.4],
            scale: [2., 0.5, 3.],
            ..Default::default()
        }
        .matrices();
        let t = transform(model, [1., 1., 0., 0.]);
        let n = transform(normal, [1., -1., 0., 0.]);
        assert!((t[0] * n[0] + t[1] * n[1] + t[2] * n[2]).abs() < 0.00001);
    }
}
