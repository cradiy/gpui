use gpui::DiffuseEnvironment3d;
use std::{
    f64::consts::{PI, TAU},
    fmt,
};

/// Invalid environment dimensions, linear radiance, or precomputed coefficients.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnvironmentError {
    Dimensions,
    PixelCount { expected: usize, actual: usize },
    Radiance { pixel: usize },
    Coefficients,
}
impl fmt::Display for EnvironmentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Dimensions => {
                f.write_str("environment dimensions must be positive and fit addressable memory")
            }
            Self::PixelCount { expected, actual } => write!(
                f,
                "expected {expected} environment pixels, received {actual}"
            ),
            Self::Radiance { pixel } => write!(
                f,
                "environment pixel {pixel} requires finite linear RGB in [0, 65504]"
            ),
            Self::Coefficients => {
                f.write_str("environment coefficients require finite RGB in [-262016, 262016]")
            }
        }
    }
}
impl std::error::Error for EnvironmentError {}

/// Distant diffuse illumination represented by nine real spherical harmonics.
/// Independent of image formats, GPU resources, and background rendering.
#[derive(Clone, Copy, Debug)]
pub struct DiffuseEnvironment(pub(crate) DiffuseEnvironment3d);

impl DiffuseEnvironment {
    /// Accepts L2 coefficients of irradiance/pi, not unconvolved radiance.
    /// Order: Y00, Y1-1, Y10, Y11, Y2-2, Y2-1, Y20, Y21, Y22.
    /// The basis uses polynomials 1, y, z, x, xy, yz, 3z²-1, xz, x²-y².
    pub fn from_coefficients(coefficients: [[f32; 3]; 9]) -> Result<Self, EnvironmentError> {
        let value = DiffuseEnvironment3d {
            coefficients,
            intensity: 1.,
            rotation_y: 0.,
        };
        if !value.is_valid() {
            return Err(EnvironmentError::Coefficients);
        }
        Ok(Self(value))
    }

    /// Projects top-left-origin, row-major linear RGB radiance onto L2 SH.
    /// Direction is (sin(theta)*cos(phi), cos(theta), sin(theta)*sin(phi)),
    /// where theta=pi*v and phi=2*pi*u-pi. Values above one are retained.
    /// Integrates piecewise-constant texels over their spherical areas, then
    /// convolves with the cosine kernel/pi. Run once when the source changes.
    pub fn from_equirectangular(
        size: [u32; 2],
        pixels: &[[f32; 3]],
    ) -> Result<Self, EnvironmentError> {
        let [width, height] = size.map(|v| v as usize);
        let expected = width
            .checked_mul(height)
            .filter(|v| *v > 0)
            .ok_or(EnvironmentError::Dimensions)?;
        if pixels.len() != expected {
            return Err(EnvironmentError::PixelCount {
                expected,
                actual: pixels.len(),
            });
        }
        let phi: Vec<_> = (0..width)
            .map(|x| {
                let p0 = x as f64 * TAU / width as f64 - PI;
                let p1 = (x + 1) as f64 * TAU / width as f64 - PI;
                let delta = p1 - p0;
                let s2 = ((2. * p1).sin() - (2. * p0).sin()) / 4.;
                [
                    delta,
                    p1.sin() - p0.sin(),
                    p0.cos() - p1.cos(),
                    delta / 2. + s2,
                    delta / 2. - s2,
                    (p1.sin().powi(2) - p0.sin().powi(2)) / 2.,
                ]
            })
            .collect();
        let mut coefficients = [[0_f64; 3]; 9];
        for (y, row) in pixels.chunks_exact(width).enumerate() {
            let t0 = y as f64 * PI / height as f64;
            let t1 = (y + 1) as f64 * PI / height as f64;
            let area = t0.cos() - t1.cos();
            let sin2 = (t1 - t0) / 2. - ((2. * t1).sin() - (2. * t0).sin()) / 4.;
            let cos_sin = (t1.sin().powi(2) - t0.sin().powi(2)) / 2.;
            let sin2_cos = (t1.sin().powi(3) - t0.sin().powi(3)) / 3.;
            let cos2_sin = (t0.cos().powi(3) - t1.cos().powi(3)) / 3.;
            let sin3 = area - cos2_sin;
            for (x, radiance) in row.iter().enumerate() {
                if radiance
                    .iter()
                    .any(|v| !v.is_finite() || !(0. ..=65504.).contains(v))
                {
                    return Err(EnvironmentError::Radiance {
                        pixel: y * width + x,
                    });
                }
                let [dp, cp, sp, cc, ss, sc] = phi[x];
                let integral = [
                    SH[0] * area * dp,
                    SH[1] * cos_sin * dp,
                    SH[1] * sin2 * sp,
                    SH[1] * sin2 * cp,
                    SH[2] * sin2_cos * cp,
                    SH[2] * sin2_cos * sp,
                    SH[3] * (3. * sin3 * ss - area * dp),
                    SH[2] * sin3 * sc,
                    SH[4] * (sin3 * cc - cos2_sin * dp),
                ];
                for (i, coefficient) in coefficients.iter_mut().enumerate() {
                    let convolution = if i == 0 {
                        1.
                    } else if i < 4 {
                        2. / 3.
                    } else {
                        0.25
                    };
                    for (channel, value) in coefficient.iter_mut().zip(radiance) {
                        *channel += f64::from(*value) * integral[i] * convolution;
                    }
                }
            }
        }
        Self::from_coefficients(coefficients.map(|coefficient| coefficient.map(|v| v as f32)))
    }

    /// Convolved linear RGB coefficients, excluding intensity and rotation.
    pub fn coefficients(&self) -> &[[f32; 3]; 9] {
        &self.0.coefficients
    }

    /// Nonnegative linear multiplier in [0, 65504]. Zero disables illumination.
    #[track_caller]
    pub fn intensity(mut self, intensity: f32) -> Self {
        assert!(intensity.is_finite() && (0. ..=65504.).contains(&intensity));
        self.0.intensity = intensity;
        self
    }

    /// Rotates the environment around world +Y, in radians, without reprojection.
    #[track_caller]
    pub fn rotation_y(mut self, radians: f32) -> Self {
        assert!(radians.is_finite());
        self.0.rotation_y = radians;
        self
    }
}

const SH: [f64; 5] = [
    0.28209479177387814,
    0.4886025119029199,
    1.0925484305920792,
    0.31539156525252005,
    0.5462742152960396,
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uniform_radiance_preserves_hdr_energy_at_any_resolution() {
        let radiance = [4., 0.5, 64.];
        for size in [[1, 1], [2, 3], [16, 8]] {
            let pixels = vec![radiance; (size[0] * size[1]) as usize];
            let environment = DiffuseEnvironment::from_equirectangular(size, &pixels).unwrap();
            for (i, coefficient) in environment.coefficients().iter().enumerate() {
                for (actual, radiance) in coefficient.iter().zip(radiance) {
                    let expected = if i == 0 { radiance / SH[0] as f32 } else { 0. };
                    assert!(
                        (actual - expected).abs() < 0.0001,
                        "{size:?}: coefficient {i}"
                    );
                }
            }
        }
    }

    #[test]
    fn axis_hemispheres_integrate_to_half_lambert_irradiance() {
        let pixels: Vec<_> = (0..2)
            .flat_map(|y| {
                (0..4).map(move |x| {
                    [
                        f32::from(y == 0),
                        f32::from(x == 1 || x == 2),
                        f32::from(x >= 2),
                    ]
                })
            })
            .collect();
        let environment = DiffuseEnvironment::from_equirectangular([4, 2], &pixels).unwrap();
        let mut expected = [[0.; 3]; 9];
        expected[0] = [0.5 / SH[0] as f32; 3];
        expected[1][0] = 0.5 / SH[1] as f32;
        expected[3][1] = 0.5 / SH[1] as f32;
        expected[2][2] = 0.5 / SH[1] as f32;
        for (actual, expected) in environment
            .coefficients()
            .iter()
            .flatten()
            .zip(expected.iter().flatten())
        {
            assert!(
                (actual - expected).abs() < 0.00001,
                "{actual} != {expected}"
            );
        }
    }

    #[test]
    fn subdividing_texels_preserves_all_coefficients() {
        let pixels = [[3., 0., 1.], [0., 7., 0.], [0.2, 0.5, 4.], [2., 1., 0.]];
        let original = DiffuseEnvironment::from_equirectangular([2, 2], &pixels).unwrap();
        let subdivided: Vec<_> = (0..10)
            .flat_map(|y| (0..10).map(move |x| pixels[y / 5 * 2 + x / 5]))
            .collect();
        let refined = DiffuseEnvironment::from_equirectangular([10, 10], &subdivided).unwrap();
        for (actual, expected) in refined
            .coefficients()
            .iter()
            .flatten()
            .zip(original.coefficients().iter().flatten())
        {
            assert!(
                (actual - expected).abs() < 0.00001,
                "{actual} != {expected}"
            );
        }
    }

    #[test]
    fn invalid_sources_are_rejected_before_gpu_submission() {
        assert_eq!(
            DiffuseEnvironment::from_equirectangular([0, 1], &[]).unwrap_err(),
            EnvironmentError::Dimensions
        );
        assert_eq!(
            DiffuseEnvironment::from_equirectangular([2, 1], &[[1.; 3]]).unwrap_err(),
            EnvironmentError::PixelCount {
                expected: 2,
                actual: 1
            }
        );
        for value in [f32::NAN, f32::INFINITY, -0.1, 65505.] {
            assert_eq!(
                DiffuseEnvironment::from_equirectangular([2, 1], &[[1.; 3], [value, 0., 0.]])
                    .unwrap_err(),
                EnvironmentError::Radiance { pixel: 1 }
            );
        }
        for value in [f32::NAN, f32::INFINITY, 262017., -262017.] {
            assert_eq!(
                DiffuseEnvironment::from_coefficients([[value; 3]; 9]).unwrap_err(),
                EnvironmentError::Coefficients
            );
        }
    }
}
