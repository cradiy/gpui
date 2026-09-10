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

    /// Closed-box overlap, including touching faces, edges, and zero extents.
    pub fn intersects(self, other: Self) -> bool {
        (0..3).all(|i| self.min[i] <= other.max[i] && other.min[i] <= self.max[i])
    }

    /// Shared closed volume, or `None` for disjoint boxes. Contact can have zero extent.
    pub fn intersection(self, other: Self) -> Option<Self> {
        Self::new(
            std::array::from_fn(|i| self.min[i].max(other.min[i])),
            std::array::from_fn(|i| self.max[i].min(other.max[i])),
        )
    }

    /// Minimum Euclidean distance between the closed boxes, zero for overlap or
    /// contact. The f64 result stays finite across all valid f32 bounds. This is
    /// a bounds distance, not a mesh distance or penetration depth.
    pub fn distance(self, other: Self) -> f64 {
        (0..3)
            .map(|i| {
                let gap = (f64::from(self.min[i]) - f64::from(other.max[i]))
                    .max(f64::from(other.min[i]) - f64::from(self.max[i]))
                    .max(0.);
                gap * gap
            })
            .sum::<f64>()
            .sqrt()
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
    fn closed_intersections_and_distances_include_contact_and_degenerate_boxes() {
        let a = Aabb::new([-1.; 3], [1.; 3]).unwrap();
        for b in [
            Aabb::new([1., -0.5, -0.5], [3., 0.5, 0.5]).unwrap(),
            Aabb::new([1., 1., -0.5], [3., 3., 0.5]).unwrap(),
            Aabb::new([1.; 3], [3.; 3]).unwrap(),
            Aabb::new([0.; 3], [0.; 3]).unwrap(),
        ] {
            assert!(a.intersects(b));
            assert!(b.intersects(a));
            let shared = a.intersection(b).unwrap();
            assert_eq!(shared, b.intersection(a).unwrap());
            assert_eq!(a.distance(b), 0.);
            assert_eq!(b.distance(a), 0.);
            assert_eq!(shared.intersection(a), Some(shared));
            assert_eq!(shared.intersection(b), Some(shared));
        }
        let diagonal = Aabb::new([4., 5., 13.], [6., 8., 15.]).unwrap();
        assert!(!a.intersects(diagonal));
        assert_eq!(a.intersection(diagonal), None);
        assert_eq!(a.distance(diagonal), 13.);
        assert_eq!(diagonal.distance(a), 13.);
        let low = Aabb::new([-f32::MAX; 3], [-f32::MAX; 3]).unwrap();
        let high = Aabb::new([f32::MAX; 3], [f32::MAX; 3]).unwrap();
        let expected = 2. * f64::from(f32::MAX) * 3_f64.sqrt();
        assert!(low.distance(high).is_finite());
        assert!((low.distance(high) / expected - 1.).abs() < 1e-15);
        let next = 1_f32.next_up();
        let separated = Aabb::new([next, 0., 0.], [next, 0., 0.]).unwrap();
        assert!(!a.intersects(separated));
        assert_eq!(a.distance(separated), f64::from(next) - 1.);
    }

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
