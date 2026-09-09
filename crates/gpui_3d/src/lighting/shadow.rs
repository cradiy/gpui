use anyhow::{Result, ensure};

/// One directional shadow map with a camera-independent, light-aligned volume.
#[derive(Clone, Copy, Debug)]
pub struct DirectionalShadow {
    /// Index into Scene::lights, or zero for Scene::light.
    pub light_index: u32,
    /// World-space center of the covered volume.
    pub center: [f32; 3],
    /// Half-width, half-height and half-depth along the light's local axes.
    pub half_extent: [f32; 3],
    /// Power-of-two side length in [256, 4096]. Defaults to 2048.
    pub resolution: u32,
    /// Normalized light-depth offset in [0, 0.05]. Defaults to 0.0005.
    pub depth_bias: f32,
    /// Nonnegative world-space geometric-normal offset, weighted by surface slope. Defaults to 0.01.
    pub normal_bias: f32,
    /// PCF radius in texels in [0, 4]. Zero gives hard shadows; defaults to 1.5.
    pub softness: f32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::transform;

    #[test]
    fn light_volume_projects_center_and_source_side_depth_for_oblique_and_vertical_lights() {
        let settings = DirectionalShadow::new([12., -4., 7.], [3., 2., 5.]);
        for direction in [[0., 0., 1.], [0., 1., 0.], [1., -2., 3.], [1e30, 0., 0.]] {
            let frame = settings.prepare(direction).unwrap();
            let center = settings.center;
            let length = direction
                .iter()
                .map(|v| f64::from(*v).powi(2))
                .sum::<f64>()
                .sqrt();
            let z = direction.map(|v| (f64::from(v) / length) as f32);
            for (distance, expected_depth) in [(0., 0.5), (5., 0.), (-5., 1.)] {
                let point = std::array::from_fn(|i| {
                    if i == 3 {
                        1.
                    } else {
                        center[i] + z[i] * distance
                    }
                });
                let clip = transform(frame.view_projection, point);
                assert!(clip[0].abs() < 1e-5 && clip[1].abs() < 1e-5);
                assert!((clip[2] - expected_depth).abs() < 1e-5);
                assert_eq!(clip[3], 1.);
            }
            for (row, extent) in [(0, 3.), (1, 2.)] {
                let point = std::array::from_fn(|i| {
                    if i == 3 {
                        1.
                    } else {
                        center[i] + frame.view_projection[i][row] * extent * extent
                    }
                });
                let clip = transform(frame.view_projection, point);
                assert!((clip[row] - 1.).abs() < 1e-5);
                assert!((clip[2] - 0.5).abs() < 1e-5);
            }
        }
    }
}

impl DirectionalShadow {
    /// Covers a fixed light-aligned volume. Half-extents must be finite and at least 0.0001.
    pub fn new(center: [f32; 3], half_extent: [f32; 3]) -> Self {
        Self {
            light_index: 0,
            center,
            half_extent,
            resolution: 2048,
            depth_bias: 0.0005,
            normal_bias: 0.01,
            softness: 1.5,
        }
    }

    pub(crate) fn prepare(self, direction: [f32; 3]) -> Result<gpui::DirectionalShadow3d> {
        ensure!(
            self.center.iter().all(|v| v.is_finite())
                && self
                    .half_extent
                    .iter()
                    .all(|v| v.is_finite() && *v >= 0.0001),
            "invalid directional shadow volume"
        );
        ensure!(
            direction.iter().all(|v| v.is_finite()) && direction.iter().any(|v| *v != 0.),
            "directional shadow requires a nonzero finite light direction"
        );
        let normalize = |v: [f64; 3]| {
            let length = v.iter().map(|x| x * x).sum::<f64>().sqrt();
            v.map(|x| x / length)
        };
        let cross = |a: [f64; 3], b: [f64; 3]| {
            [
                a[1] * b[2] - a[2] * b[1],
                a[2] * b[0] - a[0] * b[2],
                a[0] * b[1] - a[1] * b[0],
            ]
        };
        let z = normalize(direction.map(f64::from));
        let up = if z[1].abs() > 0.95 {
            [0., 0., 1.]
        } else {
            [0., 1., 0.]
        };
        let x = normalize(cross(up, z));
        let y = cross(z, x);
        let axes = [x, y, z];
        let scale = [
            1. / f64::from(self.half_extent[0]),
            1. / f64::from(self.half_extent[1]),
            -0.5 / f64::from(self.half_extent[2]),
        ];
        let mut matrix = [[0.; 4]; 4];
        for row in 0..3 {
            for column in 0..3 {
                matrix[column][row] = (axes[row][column] * scale[row]) as f32;
            }
            let offset = -(0..3)
                .map(|i| axes[row][i] * f64::from(self.center[i]))
                .sum::<f64>()
                * scale[row];
            matrix[3][row] = (offset + if row == 2 { 0.5 } else { 0. }) as f32;
        }
        matrix[3][3] = 1.;
        let shadow = gpui::DirectionalShadow3d {
            light_index: self.light_index,
            view_projection: matrix,
            resolution: self.resolution,
            depth_bias: self.depth_bias,
            normal_bias: self.normal_bias,
            softness: self.softness,
        };
        ensure!(shadow.is_valid(), "invalid directional shadow parameters");
        Ok(shadow)
    }
}
