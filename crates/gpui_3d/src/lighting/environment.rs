use gpui::DiffuseEnvironment3d;
pub use gpui::{EnvironmentError3d as EnvironmentError, EnvironmentMap3d as EnvironmentMap};
use std::f64::consts::{PI, TAU};

/// Distant HDR background independent of scene illumination. None on Scene hides it.
#[derive(Clone, Debug)]
pub struct EnvironmentBackground {
    map: EnvironmentMap,
    intensity: f32,
    rotation_y: f32,
}
impl EnvironmentBackground {
    pub fn new(map: EnvironmentMap) -> Self {
        Self {
            map,
            intensity: 1.,
            rotation_y: 0.,
        }
    }
    /// Linear brightness multiplier in [0, 65504]. Zero draws opaque black.
    #[track_caller]
    pub fn intensity(mut self, intensity: f32) -> Self {
        assert!(intensity.is_finite() && (0. ..=65504.).contains(&intensity));
        self.intensity = intensity;
        self
    }
    /// Rotation around world +Y, in radians, independent of diffuse illumination.
    #[track_caller]
    pub fn rotation_y(mut self, radians: f32) -> Self {
        assert!(radians.is_finite());
        self.rotation_y = radians;
        self
    }
    pub(crate) fn prepare(
        &self,
        camera: crate::Camera,
        aspect: f32,
    ) -> Result<gpui::EnvironmentBackground3d, crate::CameraError> {
        let view = camera.view_matrix()?;
        let (x, y) = match camera.projection {
            crate::Projection::Perspective { vertical_fov } => {
                let y = (vertical_fov * 0.5).tan();
                (y * aspect, y)
            }
            crate::Projection::Orthographic { .. } => (0., 0.),
        };
        let rays = [
            [-view[0][2], -view[1][2], -view[2][2]],
            [view[0][0] * x, view[1][0] * x, view[2][0] * x],
            [view[0][1] * y, view[1][1] * y, view[2][1] * y],
        ];
        if !rays.iter().flatten().all(|v| v.is_finite()) {
            return Err(crate::CameraError::Unrepresentable);
        }
        Ok(gpui::EnvironmentBackground3d {
            map: self.map.clone(),
            intensity: self.intensity,
            rotation_y: self.rotation_y,
            rays,
        })
    }
}

/// Distant diffuse illumination represented by nine real spherical harmonics.
/// Independent of image formats, GPU resources, and background rendering.
#[derive(Clone, Copy, Debug)]
pub struct DiffuseEnvironment(pub(crate) DiffuseEnvironment3d);

impl DiffuseEnvironment {
    /// Projects a shared decoded radiance map into diffuse irradiance coefficients.
    pub fn from_map(map: &EnvironmentMap) -> Result<Self, EnvironmentError> {
        Self::from_equirectangular(map.size(), map.pixels())
    }
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
    fn background_rays_match_camera_queries_without_translation_parallax() {
        use crate::{Camera, Projection, Ray};
        use gpui::{Bounds, point, px, size};
        let map = EnvironmentMap::from_equirectangular([1, 1], vec![[2., 0.5, 1.]]).unwrap();
        let background = EnvironmentBackground::new(map);
        for projection in [
            Projection::Perspective { vertical_fov: 1.2 },
            Projection::Orthographic { vertical_size: 7. },
        ] {
            let camera = Camera {
                eye: [2., 3., 5.],
                target: [-1., 0.5, 1.],
                up: [0.2, 1., 0.1],
                projection,
                ..Default::default()
            };
            for (width, height) in [(800., 300.), (250., 700.)] {
                let viewport = Bounds::new(point(px(71.), px(43.)), size(px(width), px(height)));
                let prepared = background.prepare(camera, width / height).unwrap();
                for (u, v) in [(0., 0.), (0.5, 0.5), (1., 1.), (0.2, 0.75)] {
                    let pixel = viewport.origin + point(px(u * width), px(v * height));
                    let expected = camera.screen_to_ray(viewport, pixel).unwrap().direction();
                    let direction = std::array::from_fn(|i| {
                        prepared.rays[0][i]
                            + (u * 2. - 1.) * prepared.rays[1][i]
                            + (1. - v * 2.) * prepared.rays[2][i]
                    });
                    let actual = Ray::new([0.; 3], direction).unwrap().direction();
                    for (actual, expected) in actual.into_iter().zip(expected) {
                        assert!((actual - expected).abs() < 0.00001);
                    }
                }
                let moved = Camera {
                    eye: camera.eye.map(|v| v + 16.),
                    target: camera.target.map(|v| v + 16.),
                    ..camera
                };
                for (actual, expected) in background
                    .prepare(moved, width / height)
                    .unwrap()
                    .rays
                    .iter()
                    .flatten()
                    .zip(prepared.rays.iter().flatten())
                {
                    assert!((actual - expected).abs() < 0.00001);
                }
            }
        }
    }

    #[test]
    fn shared_maps_validate_pixels_and_preserve_diffuse_projection() {
        let pixels = vec![[4., 0.5, 0.2], [0.1, 2., 1.]];
        let expected = DiffuseEnvironment::from_equirectangular([2, 1], &pixels).unwrap();
        let map = EnvironmentMap::from_equirectangular([2, 1], pixels).unwrap();
        assert_eq!(map.pixels().as_ptr(), map.clone().pixels().as_ptr());
        assert_eq!(
            DiffuseEnvironment::from_map(&map).unwrap().coefficients(),
            expected.coefficients()
        );
        assert_eq!(
            EnvironmentMap::from_equirectangular([0, 1], vec![]).unwrap_err(),
            EnvironmentError::Dimensions
        );
        assert_eq!(
            EnvironmentMap::from_equirectangular([2, 1], vec![[0.; 3]]).unwrap_err(),
            EnvironmentError::PixelCount {
                expected: 2,
                actual: 1
            }
        );
        for invalid in [-1., f32::NAN, f32::INFINITY, 65505.] {
            assert_eq!(
                EnvironmentMap::from_equirectangular([1, 1], vec![[0., invalid, 0.]]).unwrap_err(),
                EnvironmentError::Radiance { pixel: 0 }
            );
        }
    }

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
