use crate::math::{Matrix, multiply, transform};
use std::fmt;

const IDENTITY: Matrix = [
    [1., 0., 0., 0.],
    [0., 1., 0., 0.],
    [0., 0., 1., 0.],
    [0., 0., 0., 1.],
];

/// Invalid, singular, or numerically unrepresentable affine transform.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TransformError;

impl fmt::Display for TransformError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("transform must be finite, affine, and invertible at f32 precision")
    }
}
impl std::error::Error for TransformError {}

/// Validated column-major affine transform, including shear and reflections.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AffineTransform {
    matrix: Matrix,
    inverse: Matrix,
}

impl Default for AffineTransform {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl AffineTransform {
    pub const IDENTITY: Self = Self {
        matrix: IDENTITY,
        inverse: IDENTITY,
    };

    /// Accepts a finite, invertible matrix with last row `[0, 0, 0, 1]`.
    pub fn from_matrix(matrix: [[f32; 4]; 4]) -> Result<Self, TransformError> {
        Ok(Self {
            matrix,
            inverse: inverse_affine(matrix)?,
        })
    }

    /// Builds `translation * rotation * scale`. Quaternion order is `[x, y, z, w]`.
    /// Finite nonzero quaternions are normalized. Scale may be negative, but not zero.
    pub fn from_trs(
        translation: [f32; 3],
        rotation: [f32; 4],
        scale: [f32; 3],
    ) -> Result<Self, TransformError> {
        if !rotation.iter().all(|v| v.is_finite()) {
            return Err(TransformError);
        }
        let length = rotation
            .iter()
            .map(|v| f64::from(*v).powi(2))
            .sum::<f64>()
            .sqrt();
        if length == 0. {
            return Err(TransformError);
        }
        let [x, y, z, w] = rotation.map(|v| (f64::from(v) / length) as f32);
        let mut matrix = [
            [
                1. - 2. * (y * y + z * z),
                2. * (x * y + z * w),
                2. * (x * z - y * w),
                0.,
            ],
            [
                2. * (x * y - z * w),
                1. - 2. * (x * x + z * z),
                2. * (y * z + x * w),
                0.,
            ],
            [
                2. * (x * z + y * w),
                2. * (y * z - x * w),
                1. - 2. * (x * x + y * y),
                0.,
            ],
            [translation[0], translation[1], translation[2], 1.],
        ];
        for c in 0..3 {
            for r in 0..3 {
                matrix[c][r] *= scale[c];
            }
        }
        Self::from_matrix(matrix)
    }

    pub fn from_translation(translation: [f32; 3]) -> Result<Self, TransformError> {
        Self::from_trs(translation, [0., 0., 0., 1.], [1.; 3])
    }

    /// Returns the column-major local-to-parent matrix.
    pub fn matrix(self) -> [[f32; 4]; 4] {
        self.matrix
    }

    /// Applies `local` first, then this transform.
    pub fn compose(self, local: Self) -> Result<Self, TransformError> {
        Self::from_matrix(multiply(self.matrix, local.matrix))
    }

    pub fn inverse(self) -> Self {
        Self {
            matrix: self.inverse,
            inverse: self.matrix,
        }
    }

    pub fn transform_point(self, point: [f32; 3]) -> [f32; 3] {
        let p = transform(self.matrix, [point[0], point[1], point[2], 1.]);
        [p[0], p[1], p[2]]
    }

    /// Inverse-transpose linear transform, without translation.
    pub fn normal_matrix(self) -> [[f32; 4]; 4] {
        let mut normal = IDENTITY;
        for (c, column) in normal.iter_mut().enumerate().take(3) {
            for (r, value) in column.iter_mut().enumerate().take(3) {
                *value = self.inverse[r][c];
            }
        }
        normal
    }
}

fn inverse_affine(matrix: Matrix) -> Result<Matrix, TransformError> {
    if !matrix.iter().flatten().all(|v| v.is_finite())
        || [matrix[0][3], matrix[1][3], matrix[2][3], matrix[3][3]] != [0., 0., 0., 1.]
    {
        return Err(TransformError);
    }
    let column = |c: usize| [matrix[c][0], matrix[c][1], matrix[c][2]].map(f64::from);
    let cross = |a: [f64; 3], b: [f64; 3]| {
        [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ]
    };
    let dot = |a: [f64; 3], b: [f64; 3]| a.iter().zip(b).map(|(x, y)| x * y).sum::<f64>();
    let [a, b, c] = [column(0), column(1), column(2)];
    let determinant = dot(a, cross(b, c));
    let volume = (dot(a, a) * dot(b, b) * dot(c, c)).sqrt();
    if determinant.abs() <= volume * 1e-8 || volume == 0. {
        return Err(TransformError);
    }
    let rows = [cross(b, c), cross(c, a), cross(a, b)].map(|row| row.map(|v| v / determinant));
    let mut inverse = IDENTITY;
    for (c, column) in inverse.iter_mut().enumerate().take(3) {
        for r in 0..3 {
            column[r] = rows[r][c] as f32;
        }
    }
    for r in 0..3 {
        inverse[3][r] = -dot(rows[r], column(3)) as f32;
    }
    if !inverse.iter().flatten().all(|v| v.is_finite()) {
        return Err(TransformError);
    }
    Ok(inverse)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: [f32; 3], b: [f32; 3]) {
        for (a, b) in a.into_iter().zip(b) {
            assert!((a - b).abs() < 2e-5, "{a} != {b}");
        }
    }

    #[test]
    fn quaternion_normalization_composition_and_reflected_normals() {
        let root =
            AffineTransform::from_trs([3., -2., 1.], [0., 0., 5., 5.], [-2., 3., 0.5]).unwrap();
        close(root.transform_point([1., 0., 0.]), [3., -4., 1.]);
        let child =
            AffineTransform::from_trs([1., 2., -1.], [0.3, 0.2, 0.1, 0.8], [2., 1., 3.]).unwrap();
        let world = root.compose(child).unwrap();
        let p = [0.2, -0.8, 1.3];
        close(
            world.transform_point(p),
            root.transform_point(child.transform_point(p)),
        );
        close(world.inverse().transform_point(world.transform_point(p)), p);
        let tangent = transform(world.matrix(), [1., 1., 0., 0.]);
        let normal = transform(world.normal_matrix(), [1., -1., 0., 0.]);
        assert!((0..3).map(|i| tangent[i] * normal[i]).sum::<f32>().abs() < 2e-5);
    }

    #[test]
    fn invalid_inputs_do_not_produce_nonfinite_or_projective_transforms() {
        assert!(AffineTransform::from_trs([0.; 3], [0.; 4], [1.; 3]).is_err());
        assert!(AffineTransform::from_trs([0.; 3], [0., 0., 0., 1.], [1., 0., 1.]).is_err());
        assert!(AffineTransform::from_translation([f32::NAN, 0., 0.]).is_err());
        let mut matrix = IDENTITY;
        matrix[0][3] = 0.1;
        assert!(AffineTransform::from_matrix(matrix).is_err());
        matrix = IDENTITY;
        matrix[1] = matrix[0];
        assert!(AffineTransform::from_matrix(matrix).is_err());
        matrix[1][1] = 1e-10;
        assert!(AffineTransform::from_matrix(matrix).is_err());
        let huge = AffineTransform::from_trs([0.; 3], [0., 0., 0., 1.], [1e30; 3]).unwrap();
        assert!(huge.compose(huge).is_err());
    }
}
