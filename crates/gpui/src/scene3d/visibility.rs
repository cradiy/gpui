use super::{Mesh3d, MeshVertex3d};

pub(super) fn bounds(vertices: &[MeshVertex3d], indices: &[u32]) -> [[f32; 3]; 2] {
    let first = vertices[indices[0] as usize].position;
    let mut bounds = [first; 2];
    for &index in indices {
        for axis in 0..3 {
            let value = vertices[index as usize].position[axis];
            bounds[0][axis] = bounds[0][axis].min(value);
            bounds[1][axis] = bounds[1][axis].max(value);
        }
    }
    bounds
}

impl Mesh3d {
    /// Conservatively tests indexed geometry against a homogeneous clip volume.
    /// Matrices are column-major: local-to-world, then world-to-clip. Clip X/Y
    /// lie in `[-W, W]` and Z in `[0, W]`. Touching planes remain visible.
    /// Shear, reflections, and camera-plane crossings are supported. Non-finite
    /// matrices or numerically uncertain bounds return true, never a rejection.
    pub fn intersects_clip_volume(
        &self,
        model: [[f32; 4]; 4],
        view_projection: [[f32; 4]; 4],
    ) -> bool {
        Self::bounds_intersect_clip_volume(self.bounds, model, view_projection)
    }

    /// Conservatively tests finite ordered mesh-local bounds without changing mesh geometry.
    /// Invalid bounds or numerically uncertain transforms are treated as visible.
    pub fn bounds_intersect_clip_volume(
        bounds: [[f32; 3]; 2],
        model: [[f32; 4]; 4],
        view_projection: [[f32; 4]; 4],
    ) -> bool {
        Self::expanded_bounds_intersect_clip_volume(bounds, model, view_projection, 0., false)
    }

    pub(super) fn expanded_bounds_intersect_clip_volume(
        bounds: [[f32; 3]; 2],
        model: [[f32; 4]; 4],
        view_projection: [[f32; 4]; 4],
        world_radius: f64,
        screen_expansion: bool,
    ) -> bool {
        if !world_radius.is_finite() || world_radius < 0. {
            return true;
        }
        if !bounds.iter().flatten().all(|v| v.is_finite())
            || (0..3).any(|axis| bounds[0][axis] > bounds[1][axis])
        {
            return true;
        }
        if !model
            .iter()
            .flatten()
            .chain(view_projection.iter().flatten())
            .all(|v| v.is_finite())
        {
            return true;
        }
        let model = model.map(|column| column.map(f64::from));
        let camera = view_projection.map(|column| column.map(f64::from));
        if (0..4).any(|c| {
            (0..4).any(|r| {
                (0..4)
                    .map(|k| camera[k][r].abs() * model[c][k].abs())
                    .sum::<f64>()
                    > f64::from(f32::MAX) / 8.
            })
        }) {
            return true;
        }
        let local_extent: [f64; 4] = std::array::from_fn(|i| {
            if i == 3 {
                1.
            } else {
                f64::from(bounds[0][i])
                    .abs()
                    .max(f64::from(bounds[1][i]).abs())
            }
        });
        let world_magnitude: [f64; 4] = std::array::from_fn(|r| {
            (0..4)
                .map(|c| model[c][r].abs() * local_extent[c])
                .sum::<f64>()
                + if r < 3 { world_radius } else { 0. }
        });
        let clip_magnitude: [f64; 4] = std::array::from_fn(|r| {
            (0..4)
                .map(|c| camera[c][r].abs() * world_magnitude[c])
                .sum()
        });
        if world_magnitude
            .iter()
            .chain(&clip_magnitude)
            .any(|v| *v > f64::from(f32::MAX) / 8.)
        {
            return true;
        }
        for (axis, sign, include_w) in [
            (0, 1., true),
            (0, -1., true),
            (1, 1., true),
            (1, -1., true),
            (2, 1., false),
            (2, -1., true),
        ] {
            if screen_expansion && axis < 2 {
                continue;
            }
            let world_plane: [f64; 4] = std::array::from_fn(|r| {
                camera[r][axis] * sign + if include_w { camera[r][3] } else { 0. }
            });
            let plane: [f64; 4] =
                std::array::from_fn(|c| (0..4).map(|r| world_plane[r] * model[c][r]).sum());
            let maximum = plane[3]
                + world_radius * world_plane[..3].iter().map(|v| v * v).sum::<f64>().sqrt()
                + (0..3)
                    .map(|i| plane[i] * f64::from(bounds[usize::from(plane[i] >= 0.)][i]))
                    .sum::<f64>();
            let magnitude = clip_magnitude[axis] + if include_w { clip_magnitude[3] } else { 0. };
            let tolerance =
                64. * f64::from(f32::EPSILON) * magnitude + f64::from(f32::MIN_POSITIVE);
            if maximum < -tolerance {
                return false;
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const IDENTITY: [[f32; 4]; 4] = [
        [1., 0., 0., 0.],
        [0., 1., 0., 0.],
        [0., 0., 1., 0.],
        [0., 0., 0., 1.],
    ];

    fn cube() -> std::sync::Arc<Mesh3d> {
        Mesh3d::new(
            (0..8)
                .map(|corner| MeshVertex3d {
                    position: std::array::from_fn(|axis| {
                        if corner & (1 << axis) == 0 { -0.5 } else { 0.5 }
                    }),
                    normal: [0., 0., 1.],
                    uv: [0.; 2],
                })
                .collect(),
            vec![0, 1, 2, 0, 2, 3, 4, 5, 6, 4, 6, 7],
        )
    }

    #[test]
    fn scene3d_clip_planes_keep_contacts_and_reject_separated_bounds() {
        let mesh = cube();
        assert!(mesh.intersects_clip_volume(IDENTITY, IDENTITY));
        for (axis, contact) in [
            (0, -1.5),
            (0, 1.5),
            (1, -1.5),
            (1, 1.5),
            (2, -0.5),
            (2, 1.5),
        ] {
            let mut model = IDENTITY;
            model[3][axis] = contact;
            assert!(mesh.intersects_clip_volume(model, IDENTITY));
            model[3][axis] += contact.signum() * 0.001;
            assert!(!mesh.intersects_clip_volume(model, IDENTITY));
        }
        let mut enclosing = IDENTITY;
        for (i, column) in enclosing.iter_mut().enumerate().take(3) {
            column[i] = 10.;
        }
        assert!(mesh.intersects_clip_volume(enclosing, IDENTITY));
        let mut model = IDENTITY;
        model[3][0] = 1.5 + f32::EPSILON;
        assert!(mesh.intersects_clip_volume(model, IDENTITY));
    }

    #[test]
    fn scene3d_clip_tests_preserve_visible_samples_under_shear_reflection_and_perspective() {
        let mesh = cube();
        let camera = [
            [1., 0., 0., 0.],
            [0., 1., 0., 0.],
            [0., 0., -1.01, -1.],
            [0., 0., -0.101, 0.],
        ];
        for z in [-20., -3., -0.1, 0.1, 2.] {
            for x in [-4., 0., 4.] {
                for reflection in [-1., 1.] {
                    let model = [
                        [reflection, 0.2, 0., 0.],
                        [0.6, 0.8, 0.4, 0.],
                        [0., 0., 1., 0.],
                        [x, 0., z, 1.],
                    ];
                    for corner in 0..27 {
                        let p = [
                            (corner % 3) as f32 * 0.5 - 0.5,
                            ((corner / 3) % 3) as f32 * 0.5 - 0.5,
                            (corner / 9) as f32 * 0.5 - 0.5,
                            1.,
                        ];
                        let world: [f32; 4] =
                            std::array::from_fn(|r| (0..4).map(|c| model[c][r] * p[c]).sum());
                        let clip: [f32; 4] =
                            std::array::from_fn(|r| (0..4).map(|c| camera[c][r] * world[c]).sum());
                        if clip[0].abs() <= clip[3]
                            && clip[1].abs() <= clip[3]
                            && clip[2] >= 0.
                            && clip[2] <= clip[3]
                        {
                            assert!(mesh.intersects_clip_volume(model, camera), "{model:?}");
                        }
                    }
                    if z == 2. || z == -20. {
                        assert!(!mesh.intersects_clip_volume(model, camera));
                    }
                }
            }
        }
        let mut behind = IDENTITY;
        behind[3][2] = 2.;
        assert!(!mesh.intersects_clip_volume(behind, camera));
        behind[3][2] = -0.1;
        assert!(mesh.intersects_clip_volume(behind, camera));
        let mut invalid = IDENTITY;
        invalid[0][0] = f32::NAN;
        assert!(mesh.intersects_clip_volume(invalid, camera));
        invalid[0][0] = f32::MAX;
        assert!(mesh.intersects_clip_volume(invalid, camera));
    }

    #[test]
    fn scene3d_clip_bounds_follow_vertex_snapshots_and_ignore_unused_vertices() {
        let source = Mesh3d::new(
            [[3., 0., 0.5], [4., 0., 0.5], [3., 1., 0.5], [0.; 3]]
                .map(|position| MeshVertex3d {
                    position,
                    normal: [0., 0., 1.],
                    uv: [0.; 2],
                })
                .to_vec(),
            vec![0, 1, 2],
        );
        assert!(!source.intersects_clip_volume(IDENTITY, IDENTITY));
        let mut vertices = source.vertices().to_vec();
        for vertex in vertices.iter_mut().take(3) {
            vertex.position[0] -= 3.;
        }
        let moved = source.with_vertices(vertices, None).unwrap();
        assert!(moved.intersects_clip_volume(IDENTITY, IDENTITY));
        assert!(!source.intersects_clip_volume(IDENTITY, IDENTITY));
        let tangent_copy = source.with_tangents(vec![[1., 0., 0., 1.]; 4]).unwrap();
        assert!(!tangent_copy.intersects_clip_volume(IDENTITY, IDENTITY));
    }
}
