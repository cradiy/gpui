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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Camera;
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
