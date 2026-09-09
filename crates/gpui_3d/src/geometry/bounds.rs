use crate::{AffineTransform, Mesh, TransformError};

/// Axis-aligned bounds. Empty groups have no bounds, represented by `Option<Aabb>`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Aabb {
    min: [f32; 3],
    max: [f32; 3],
}

impl Aabb {
    /// Returns `None` for non-finite or reversed limits. Zero extent is valid.
    pub fn new(min: [f32; 3], max: [f32; 3]) -> Option<Self> {
        (min.iter().chain(&max).all(|v| v.is_finite()) && (0..3).all(|i| min[i] <= max[i]))
            .then_some(Self { min, max })
    }

    pub fn min(self) -> [f32; 3] {
        self.min
    }
    pub fn max(self) -> [f32; 3] {
        self.max
    }

    pub fn union(self, other: Self) -> Self {
        Self {
            min: std::array::from_fn(|i| self.min[i].min(other.min[i])),
            max: std::array::from_fn(|i| self.max[i].max(other.max[i])),
        }
    }

    /// Conservative world-aligned bounds of all eight transformed corners.
    pub fn transformed(self, transform: AffineTransform) -> Result<Self, TransformError> {
        let mut min = [f32::INFINITY; 3];
        let mut max = [f32::NEG_INFINITY; 3];
        for corner in 0..8 {
            let p = transform.transform_point(std::array::from_fn(|i| {
                if corner & (1 << i) == 0 {
                    self.min[i]
                } else {
                    self.max[i]
                }
            }));
            if !p.iter().all(|v| v.is_finite()) {
                return Err(TransformError);
            }
            for i in 0..3 {
                min[i] = min[i].min(p[i]);
                max[i] = max[i].max(p[i]);
            }
        }
        Ok(Self { min, max })
    }
}

impl Mesh {
    /// Local bounds of all vertices, including vertices not referenced by triangles.
    pub fn bounds(&self) -> Aabb {
        let first = self.0.vertices()[0].position;
        self.0.vertices().iter().fold(
            Aabb {
                min: first,
                max: first,
            },
            |bounds, vertex| {
                bounds.union(Aabb {
                    min: vertex.position,
                    max: vertex.position,
                })
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn affine_bounds_include_sheared_and_reflected_corners() {
        let bounds = Aabb::new([-1., -2., -3.], [2., 4., 1.]).unwrap();
        let transform = AffineTransform::from_matrix([
            [-2., 0., 0., 0.],
            [0.5, 1., 0., 0.],
            [0., 0., 1., 0.],
            [3., 0., 0., 1.],
        ])
        .unwrap();
        assert_eq!(
            bounds.transformed(transform).unwrap(),
            Aabb::new([-2., -2., -3.], [7., 4., 1.]).unwrap(),
        );
        let plane = Mesh::plane().bounds().transformed(transform).unwrap();
        assert_eq!(plane.min(), [1.75, -0.5, 0.]);
        assert_eq!(plane.max(), [4.25, 0.5, 0.]);
        let large = Aabb::new([0.; 3], [f32::MAX; 3]).unwrap();
        assert!(large.transformed(transform).is_err());
    }
}
