use crate::{
    EnvironmentError, EnvironmentMap,
    math::{cross, dot},
};
pub use gpui::SpecularEnvironmentMap3d as SpecularEnvironmentMap;
use std::f32::consts::{PI, TAU};

/// Explicit CPU prefilter quality. Processing is synchronous and independent of rendering.
#[derive(Clone, Copy, Debug)]
pub struct SpecularPrefilter {
    /// Power-of-two cube face size in 16..=512. Defaults to 128.
    pub resolution: u32,
    /// GGX samples per filtered texel in 16..=4096. Defaults to 256.
    /// The full filtered chain is limited to 64 million samples.
    pub samples: u32,
}
impl Default for SpecularPrefilter {
    fn default() -> Self {
        Self {
            resolution: 128,
            samples: 256,
        }
    }
}

/// Distant GGX specular illumination with independent intensity and rotation.
#[derive(Clone, Debug)]
pub struct SpecularEnvironment(pub(crate) gpui::SpecularEnvironment3d);
impl SpecularEnvironment {
    /// Prefilters decoded radiance on the calling thread; run during resource preparation.
    pub fn from_map(
        map: &EnvironmentMap,
        quality: SpecularPrefilter,
    ) -> Result<Self, EnvironmentError> {
        let edge = quality.resolution;
        if !edge.is_power_of_two()
            || !(16..=512).contains(&edge)
            || !(16..=4096).contains(&quality.samples)
            || 2 * u64::from(edge).pow(2) * u64::from(quality.samples) > 64_000_000
        {
            return Err(EnvironmentError::Prefilter);
        }
        let source = radiance_mips(map);
        let count = edge.ilog2() + 1;
        let mut levels = Vec::with_capacity(count as usize);
        for level in 0..count {
            let size = edge >> level;
            let roughness = level as f32 / (count - 1) as f32;
            let samples = if level == 0 {
                Vec::new()
            } else {
                ggx_samples(roughness, quality.samples)
            };
            let mut pixels = Vec::with_capacity(6 * (size * size) as usize);
            for face in 0..6 {
                for y in 0..size {
                    for x in 0..size {
                        let normal = cube_direction(face, x, y, size);
                        let color = if level == 0 {
                            sample_radiance(&source, normal, 0.).map(|v| v.clamp(0., 65504.))
                        } else {
                            let up = if normal[2].abs() < 0.999 {
                                [0., 0., 1.]
                            } else {
                                [1., 0., 0.]
                            };
                            let tangent = unit(cross(up, normal));
                            let bitangent = cross(normal, tangent);
                            let mut sum = [0_f64; 3];
                            let mut weight = 0_f64;
                            for (local, pdf) in &samples {
                                let direction = std::array::from_fn(|i| {
                                    tangent[i] * local[0]
                                        + bitangent[i] * local[1]
                                        + normal[i] * local[2]
                                });
                                let sin_theta = (1. - direction[1] * direction[1])
                                    .max(0.)
                                    .sqrt()
                                    .max(0.0001);
                                let texel_area = TAU * PI * sin_theta
                                    / (map.size()[0] as f32 * map.size()[1] as f32);
                                let lod = (0.5
                                    * (1. / (quality.samples as f32 * pdf * texel_area)).log2())
                                .max(0.);
                                let radiance = sample_radiance(&source, direction, lod);
                                for i in 0..3 {
                                    sum[i] += f64::from(radiance[i]) * f64::from(local[2]);
                                }
                                weight += f64::from(local[2]);
                            }
                            sum.map(|v| (v / weight).clamp(0., 65504.) as f32)
                        };
                        pixels.push(color);
                    }
                }
            }
            levels.push(pixels);
        }
        Ok(Self::from_prefiltered(
            SpecularEnvironmentMap::from_prefiltered(edge, levels)?,
        ))
    }
    /// Uses caller-supplied GGX-prefiltered cube levels without recomputing them.
    pub fn from_prefiltered(map: SpecularEnvironmentMap) -> Self {
        Self(gpui::SpecularEnvironment3d {
            map,
            intensity: 1.,
            rotation_y: 0.,
        })
    }
    pub fn map(&self) -> &SpecularEnvironmentMap {
        &self.0.map
    }
    /// Nonnegative linear multiplier in [0, 65504]. Zero disables reflections.
    #[track_caller]
    pub fn intensity(mut self, intensity: f32) -> Self {
        assert!(intensity.is_finite() && (0. ..=65504.).contains(&intensity));
        self.0.intensity = intensity;
        self
    }
    /// World +Y rotation in radians. Reuses prefiltered radiance.
    #[track_caller]
    pub fn rotation_y(mut self, radians: f32) -> Self {
        assert!(radians.is_finite());
        self.0.rotation_y = radians;
        self
    }
}

struct RadianceMip {
    size: [usize; 2],
    pixels: Vec<[f32; 3]>,
}
fn radiance_mips(map: &EnvironmentMap) -> Vec<RadianceMip> {
    let mut mips = vec![RadianceMip {
        size: map.size().map(|v| v as usize),
        pixels: map.pixels().to_vec(),
    }];
    while mips.last().unwrap().size != [1, 1] {
        let source = mips.last().unwrap();
        let [sw, sh] = source.size;
        let [width, height] = source.size.map(|v| (v / 2).max(1));
        let mut pixels = Vec::with_capacity(width * height);
        for y in 0..height {
            let y0 = y as f64 * sh as f64 / height as f64;
            let y1 = (y + 1) as f64 * sh as f64 / height as f64;
            for x in 0..width {
                let x0 = x as f64 * sw as f64 / width as f64;
                let x1 = (x + 1) as f64 * sw as f64 / width as f64;
                let mut sum = [0_f64; 3];
                let mut area = 0.;
                for sy in y0.floor() as usize..(y1.ceil() as usize).min(sh) {
                    let a = y0.max(sy as f64) * std::f64::consts::PI / sh as f64;
                    let b = y1.min((sy + 1) as f64) * std::f64::consts::PI / sh as f64;
                    for sx in x0.floor() as usize..(x1.ceil() as usize).min(sw) {
                        let weight =
                            (a.cos() - b.cos()) * (x1.min((sx + 1) as f64) - x0.max(sx as f64));
                        for (value, input) in sum.iter_mut().zip(source.pixels[sy * sw + sx]) {
                            *value += f64::from(input) * weight;
                        }
                        area += weight;
                    }
                }
                pixels.push(sum.map(|v| (v / area).clamp(0., 65504.) as f32));
            }
        }
        mips.push(RadianceMip {
            size: [width, height],
            pixels,
        });
    }
    mips
}
fn sample_radiance(mips: &[RadianceMip], direction: [f32; 3], lod: f32) -> [f32; 3] {
    let u = direction[2].atan2(direction[0]) / TAU + 0.5;
    let v = direction[1].clamp(-1., 1.).acos() / PI;
    let lod = lod.clamp(0., (mips.len() - 1) as f32);
    let a = lod.floor() as usize;
    let b = (a + 1).min(mips.len() - 1);
    let sample = |mip: &RadianceMip| {
        let x = u * mip.size[0] as f32 - 0.5;
        let y = v * mip.size[1] as f32 - 0.5;
        let ix = x.floor() as i32;
        let iy = y.floor() as i32;
        let tx = x - x.floor();
        let ty = y - y.floor();
        let texel = |x: i32, y: i32| {
            mip.pixels[y.clamp(0, mip.size[1] as i32 - 1) as usize * mip.size[0]
                + x.rem_euclid(mip.size[0] as i32) as usize]
        };
        let [p00, p10, p01, p11] = [
            texel(ix, iy),
            texel(ix + 1, iy),
            texel(ix, iy + 1),
            texel(ix + 1, iy + 1),
        ];
        std::array::from_fn::<_, 3, _>(|i| {
            (p00[i] * (1. - tx) + p10[i] * tx) * (1. - ty) + (p01[i] * (1. - tx) + p11[i] * tx) * ty
        })
    };
    let first = sample(&mips[a]);
    let second = sample(&mips[b]);
    std::array::from_fn(|i| first[i] * (1. - lod.fract()) + second[i] * lod.fract())
}
fn unit(v: [f32; 3]) -> [f32; 3] {
    let length = dot(v, v).sqrt();
    v.map(|v| v / length)
}
fn cube_direction(face: u32, x: u32, y: u32, size: u32) -> [f32; 3] {
    let u = 2. * (x as f32 + 0.5) / size as f32 - 1.;
    let v = 2. * (y as f32 + 0.5) / size as f32 - 1.;
    unit(match face {
        0 => [1., -v, -u],
        1 => [-1., -v, u],
        2 => [u, 1., v],
        3 => [u, -1., -v],
        4 => [u, -v, 1.],
        _ => [-u, -v, -1.],
    })
}
fn ggx_samples(roughness: f32, count: u32) -> Vec<([f32; 3], f32)> {
    let a2 = roughness.powi(4);
    (0..count)
        .filter_map(|i| {
            let u = i as f32 / count as f32;
            let phi = TAU * i.reverse_bits() as f32 * 2_f32.powi(-32);
            let hz = ((1. - u) / (1. + (a2 - 1.) * u)).sqrt();
            let r = (1. - hz * hz).max(0.).sqrt();
            let local = [
                2. * hz * r * phi.cos(),
                2. * hz * r * phi.sin(),
                2. * hz * hz - 1.,
            ];
            let d = (1. - hz * hz) + a2 * hz * hz;
            (local[2] > 0.).then_some((local, (a2 / (4. * PI * d * d)).max(1e-10)))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn prefilter_preserves_uniform_hdr_radiance_at_every_face_and_roughness() {
        let expected = [4., 0.25, 65504.];
        let map = EnvironmentMap::from_equirectangular([5, 3], vec![expected; 15]).unwrap();
        let environment = SpecularEnvironment::from_map(
            &map,
            SpecularPrefilter {
                resolution: 16,
                samples: 64,
            },
        )
        .unwrap();
        for pixel in environment.map().levels().iter().flatten() {
            for (actual, expected) in pixel.iter().zip(expected) {
                assert!((actual - expected).abs() < 0.02, "{actual} != {expected}");
            }
        }
        assert_eq!(
            environment.map().levels().as_ptr(),
            environment.clone().map().levels().as_ptr()
        );
    }
    #[test]
    fn spherical_source_reduction_keeps_odd_edges_and_polar_solid_angles() {
        let pixels: Vec<_> = (0..3)
            .flat_map(|y| (0..3).map(move |x| [if y == 0 { 16. } else { 0. }, [1., 2., 6.][x], 1.]))
            .collect();
        let map = EnvironmentMap::from_equirectangular([3, 3], pixels).unwrap();
        assert_eq!(radiance_mips(&map).last().unwrap().pixels, [[4., 3., 1.]]);
    }
    #[test]
    fn roughness_broadens_directional_radiance_and_cube_faces_follow_world_axes() {
        let pixels: Vec<_> = (0..32)
            .flat_map(|y| {
                (0..64).map(move |x| {
                    let t = (y as f32 + 0.5) * PI / 32.;
                    let p = (x as f32 + 0.5) * TAU / 64. - PI;
                    let n = [t.sin() * p.cos(), t.cos(), t.sin() * p.sin()];
                    [
                        8. * n[0].max(0.).powi(32),
                        n[1] * 0.5 + 0.5,
                        n[2] * 0.5 + 0.5,
                    ]
                })
            })
            .collect();
        let map = EnvironmentMap::from_equirectangular([64, 32], pixels).unwrap();
        let result = SpecularEnvironment::from_map(
            &map,
            SpecularPrefilter {
                resolution: 16,
                samples: 256,
            },
        )
        .unwrap();
        let levels = result.map().levels();
        assert!(levels[0][2 * 256 + 8 * 16 + 8][1] > 0.99);
        assert!(levels[0][3 * 256 + 8 * 16 + 8][1] < 0.01);
        assert!(levels[0][4 * 256 + 8 * 16 + 8][2] > 0.99);
        assert!(levels[0][5 * 256 + 8 * 16 + 8][2] < 0.01);
        let peak = |level: usize| levels[level].iter().map(|v| v[0]).fold(0_f32, f32::max);
        assert!(peak(0) > peak(2) * 2.);
        assert!(peak(2) > peak(4));
        let illuminated_fraction = |level: usize| {
            levels[level].iter().filter(|v| v[0] > 0.01).count() as f32 / levels[level].len() as f32
        };
        assert!(illuminated_fraction(3) > illuminated_fraction(0));
    }
    #[test]
    fn invalid_prefilter_work_and_external_cube_chains_are_rejected() {
        let map = EnvironmentMap::from_equirectangular([1, 1], vec![[1.; 3]]).unwrap();
        for quality in [
            SpecularPrefilter {
                resolution: 0,
                samples: 64,
            },
            SpecularPrefilter {
                resolution: 17,
                samples: 64,
            },
            SpecularPrefilter {
                resolution: 16,
                samples: 0,
            },
            SpecularPrefilter {
                resolution: 512,
                samples: 4096,
            },
        ] {
            assert_eq!(
                SpecularEnvironment::from_map(&map, quality).unwrap_err(),
                EnvironmentError::Prefilter
            );
        }
        assert!(SpecularEnvironmentMap::from_prefiltered(2, vec![vec![[1.; 3]; 24]]).is_err());
        assert!(SpecularEnvironmentMap::from_prefiltered(1, vec![vec![[1.; 3]; 5]]).is_err());
        assert!(SpecularEnvironmentMap::from_prefiltered(1, vec![vec![[f32::NAN; 3]; 6]]).is_err());
    }
}
