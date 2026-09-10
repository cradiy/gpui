use super::{Camera, CameraError, viewport_size};
use crate::Aabb;
use gpui::{Bounds, Pixels, point, px, size};

type Vector = [f64; 4];
type Matrix = [Vector; 4];
const CLIP_PLANES: [Vector; 6] = [
    [1., 0., 0., 1.],
    [-1., 0., 0., 1.],
    [0., 1., 0., 1.],
    [0., -1., 0., 1.],
    [0., 0., 1., 0.],
    [0., 0., -1., 1.],
];

/// Owned camera clip volume for repeated world-AABB queries without a window or GPU.
/// Uses the camera's rendered view-projection matrix and its closed clip planes.
/// These are bounding-volume queries, not mesh intersections or occlusion results.
#[derive(Clone, Debug)]
pub struct Frustum {
    matrix: Matrix,
    inverse: Matrix,
    planes: [Vector; 6],
}

impl Camera {
    /// Prepares a clip-volume snapshot at this aspect ratio. Later camera changes
    /// do not alter it. Numerically singular projection matrices are rejected.
    pub fn frustum(self, aspect: f32) -> Result<Frustum, CameraError> {
        let matrix = self
            .view_projection(aspect)?
            .map(|column| column.map(f64::from));
        let inverse = inverse(matrix).ok_or(CameraError::Unrepresentable)?;
        let planes = CLIP_PLANES.map(|plane| std::array::from_fn(|c| dot(plane, matrix[c])));
        Ok(Frustum {
            matrix,
            inverse,
            planes,
        })
    }

    /// Screen rectangle of a world AABB after clipping against all six camera planes.
    /// Uses the viewport's aspect ratio and top-left origin. `None` means the bounds
    /// miss the clip volume; boundary contact may return a zero-area rectangle.
    /// This does not establish that the bounded geometry is unoccluded or draws pixels.
    pub fn project_bounds(
        self,
        viewport: Bounds<Pixels>,
        bounds: Aabb,
    ) -> Result<Option<Bounds<Pixels>>, CameraError> {
        let [width, height] = viewport_size(viewport)?;
        self.frustum(width / height)?
            .project_bounds(viewport, bounds)
    }
}

impl Frustum {
    /// Conservatively tests a world AABB against six clip planes. Boundary contact
    /// and numerically uncertain separation are retained. A `true` result can be
    /// a false positive near frustum corners; it is not proof of visible geometry.
    pub fn intersects(&self, bounds: Aabb) -> bool {
        self.planes.iter().all(|&plane| {
            let support = std::array::from_fn(|i| {
                if i == 3 {
                    1.
                } else if plane[i] >= 0. {
                    f64::from(bounds.max()[i])
                } else {
                    f64::from(bounds.min()[i])
                }
            });
            distance(plane, support) >= 0.
        })
    }

    /// Projects the clipped intersection, including box/frustum edge crossings and
    /// camera-containing bounds. Coordinates are conservatively rounded to pixels.
    /// The viewport scales this snapshot's normalized image; it does not change the
    /// aspect ratio used to create the frustum. Empty intersections return `None`.
    pub fn project_bounds(
        &self,
        viewport: Bounds<Pixels>,
        bounds: Aabb,
    ) -> Result<Option<Bounds<Pixels>>, CameraError> {
        let [width, height] = viewport_size(viewport)?;
        if !self.intersects(bounds) {
            return Ok(None);
        }
        let box_vertices: [Vector; 8] = std::array::from_fn(|corner| {
            transform(
                self.matrix,
                std::array::from_fn(|i| {
                    if i == 3 {
                        1.
                    } else if corner & (1 << i) == 0 {
                        f64::from(bounds.min()[i])
                    } else {
                        f64::from(bounds.max()[i])
                    }
                }),
            )
        });
        let clip_vertices: [Vector; 8] = std::array::from_fn(|corner| {
            [
                if corner & 1 == 0 { -1. } else { 1. },
                if corner & 2 == 0 { -1. } else { 1. },
                if corner & 4 == 0 { 0. } else { 1. },
                1.,
            ]
        });
        let box_planes: [Vector; 6] = std::array::from_fn(|index| {
            let axis = index / 2;
            let (sign, bound) = if index % 2 == 0 {
                (1., bounds.min()[axis])
            } else {
                (-1., bounds.max()[axis])
            };
            std::array::from_fn(|c| {
                sign * (self.inverse[c][axis] - f64::from(bound) * self.inverse[c][3])
            })
        });
        let mut min = [f64::INFINITY; 2];
        let mut max = [f64::NEG_INFINITY; 2];
        let mut include = |p: Vector| {
            if p[3] <= 0. {
                min = [-1.; 2];
                max = [1.; 2];
            } else {
                for axis in 0..2 {
                    let value = (p[axis] / p[3]).clamp(-1., 1.);
                    min[axis] = min[axis].min(value);
                    max[axis] = max[axis].max(value);
                }
            }
        };
        // Every intersection vertex lies on an edge of at least one polyhedron.
        // Clipping homogeneous frustum edges also handles far vertices at infinity.
        for corner in 0..8 {
            for axis in 0..3 {
                let other = corner | (1 << axis);
                if corner == other {
                    continue;
                }
                for (vertices, planes) in
                    [(&box_vertices, &CLIP_PLANES), (&clip_vertices, &box_planes)]
                {
                    if let Some([a, b]) = clip_edge(vertices[corner], vertices[other], planes) {
                        include(a);
                        include(b);
                    }
                }
            }
        }
        if min[0] == f64::INFINITY {
            return Ok(None);
        }
        let origin = [
            f64::from(f32::from(viewport.origin.x)),
            f64::from(f32::from(viewport.origin.y)),
        ];
        let lower = [
            origin[0] + (min[0] + 1.) * f64::from(width) * 0.5,
            origin[1] + (1. - max[1]) * f64::from(height) * 0.5,
        ];
        let upper = [
            origin[0] + (max[0] + 1.) * f64::from(width) * 0.5,
            origin[1] + (1. - min[1]) * f64::from(height) * 0.5,
        ];
        let lower = lower.map(round_down);
        let upper = upper.map(round_up);
        let extent =
            std::array::from_fn::<_, 2, _>(|i| round_up(f64::from(upper[i]) - f64::from(lower[i])));
        if !lower
            .iter()
            .chain(&upper)
            .chain(&extent)
            .all(|v| v.is_finite())
            || (0..2).any(|i| !(lower[i] + extent[i]).is_finite())
        {
            return Err(CameraError::Unrepresentable);
        }
        Ok(Some(Bounds::new(
            point(px(lower[0]), px(lower[1])),
            size(px(extent[0]), px(extent[1])),
        )))
    }
}

fn dot(a: Vector, b: Vector) -> f64 {
    a.into_iter().zip(b).map(|(a, b)| a * b).sum()
}
fn transform(matrix: Matrix, p: Vector) -> Vector {
    std::array::from_fn(|r| (0..4).map(|c| matrix[c][r] * p[c]).sum())
}
fn distance(plane: Vector, p: Vector) -> f64 {
    let value = dot(plane, p);
    let magnitude: f64 = plane.into_iter().zip(p).map(|(a, b)| (a * b).abs()).sum();
    if value.abs() <= magnitude * 8. * f64::from(f32::EPSILON) {
        0.
    } else {
        value
    }
}
fn clip_edge(a: Vector, b: Vector, planes: &[Vector; 6]) -> Option<[Vector; 2]> {
    let (mut lower, mut upper) = (0_f64, 1_f64);
    for &plane in planes {
        let da = distance(plane, a);
        let db = distance(plane, b);
        if da < 0. && db < 0. {
            return None;
        }
        if da < 0. {
            lower = lower.max(da / (da - db));
        }
        if db < 0. {
            upper = upper.min(da / (da - db));
        }
        if lower > upper {
            return None;
        }
    }
    Some([lower, upper].map(|t| std::array::from_fn(|i| a[i] * (1. - t) + b[i] * t)))
}
fn round_down(value: f64) -> f32 {
    let rounded = value as f32;
    if f64::from(rounded) > value {
        rounded.next_down()
    } else {
        rounded
    }
}
fn round_up(value: f64) -> f32 {
    let rounded = value as f32;
    if f64::from(rounded) < value {
        rounded.next_up()
    } else {
        rounded
    }
}
fn inverse(matrix: Matrix) -> Option<Matrix> {
    let mut rows: [[f64; 8]; 4] = std::array::from_fn(|r| {
        std::array::from_fn(|c| {
            if c < 4 {
                matrix[c][r]
            } else {
                f64::from(c - 4 == r)
            }
        })
    });
    for column in 0..4 {
        let pivot =
            (column..4).max_by(|&a, &b| rows[a][column].abs().total_cmp(&rows[b][column].abs()))?;
        rows.swap(column, pivot);
        let divisor = rows[column][column];
        if divisor == 0. {
            return None;
        }
        rows[column] = rows[column].map(|value| value / divisor);
        let pivot = rows[column];
        for (r, row) in rows.iter_mut().enumerate() {
            if r != column {
                let scale = row[column];
                for c in 0..8 {
                    row[c] -= scale * pivot[c];
                }
            }
        }
    }
    let inverse: Matrix = std::array::from_fn(|c| std::array::from_fn(|r| rows[r][c + 4]));
    inverse
        .iter()
        .flatten()
        .all(|v| v.is_finite())
        .then_some(inverse)
}
