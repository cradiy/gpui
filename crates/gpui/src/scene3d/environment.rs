use std::{fmt, sync::Arc};

/// Invalid environment dimensions, radiance, or irradiance coefficients.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnvironmentError3d {
    /// Invalid cube dimensions, roughness levels, or prefilter work budget.
    Prefilter,
    /// Empty or unaddressable dimensions.
    Dimensions,
    /// Pixel storage does not match the dimensions.
    PixelCount {
        /// Width times height.
        expected: usize,
        /// Number of supplied pixels.
        actual: usize,
    },
    /// A pixel has negative, non-finite, or out-of-range radiance.
    Radiance {
        /// Row-major pixel index.
        pixel: usize,
    },
    /// Irradiance coefficients exceed their finite range.
    Coefficients,
}
impl fmt::Display for EnvironmentError3d {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Prefilter => {
                f.write_str("invalid specular prefilter dimensions, levels, or work budget")
            }
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
impl std::error::Error for EnvironmentError3d {}

/// Shared immutable, top-left-origin equirectangular linear RGB radiance.
/// Direction is (sin(theta)*cos(phi), cos(theta), sin(theta)*sin(phi)),
/// where theta=pi*v and phi=2*pi*u-pi. No file decoding or GPU work is performed.
#[derive(Clone, Debug)]
pub struct EnvironmentMap3d {
    size: [u32; 2],
    pixels: Arc<[[f32; 3]]>,
}
impl EnvironmentMap3d {
    /// Validates and retains decoded linear radiance in [0, 65504].
    pub fn from_equirectangular(
        size: [u32; 2],
        pixels: Vec<[f32; 3]>,
    ) -> Result<Self, EnvironmentError3d> {
        let expected = (size[0] as usize)
            .checked_mul(size[1] as usize)
            .filter(|n| *n > 0 && *n <= isize::MAX as usize / std::mem::size_of::<[f32; 3]>())
            .ok_or(EnvironmentError3d::Dimensions)?;
        if pixels.len() != expected {
            return Err(EnvironmentError3d::PixelCount {
                expected,
                actual: pixels.len(),
            });
        }
        for (pixel, rgb) in pixels.iter().enumerate() {
            if rgb
                .iter()
                .any(|v| !v.is_finite() || !(0. ..=65504.).contains(v))
            {
                return Err(EnvironmentError3d::Radiance { pixel });
            }
        }
        Ok(Self {
            size,
            pixels: pixels.into(),
        })
    }
    /// Physical texel width and height.
    pub fn size(&self) -> [u32; 2] {
        self.size
    }
    /// Row-major linear RGB values. Clones share this allocation.
    pub fn pixels(&self) -> &[[f32; 3]] {
        &self.pixels
    }
}

/// Prepared distant environment background for one camera and viewport.
#[derive(Clone, Debug)]
pub struct EnvironmentBackground3d {
    /// Shared decoded radiance; GPU dimensions remain device-limited.
    pub map: EnvironmentMap3d,
    /// Linear multiplier in [0, 65504]. Zero draws opaque black.
    pub intensity: f32,
    /// Rotation about world +Y, in radians.
    pub rotation_y: f32,
    /// World direction = center + ndc.x * dx + ndc.y * dy, then normalized.
    /// Orthographic cameras have zero dx/dy and a constant forward direction.
    pub rays: [[f32; 3]; 3],
}
impl EnvironmentBackground3d {
    /// Checks finite background parameters and a nonzero central view direction.
    pub fn is_valid(&self) -> bool {
        self.intensity.is_finite()
            && (0. ..=65504.).contains(&self.intensity)
            && self.rotation_y.is_finite()
            && self.rays.iter().flatten().all(|v| v.is_finite())
            && self.rays[0].iter().any(|v| *v != 0.)
    }
}

/// Shared GGX-prefiltered cube radiance. Levels span perceptual roughness 0..1.
/// Each level stores +X, -X, +Y, -Y, +Z, -Z faces in top-left-origin row order.
#[derive(Clone, Debug)]
pub struct SpecularEnvironmentMap3d {
    size: u32,
    levels: Arc<[Vec<[f32; 3]>]>,
}
impl SpecularEnvironmentMap3d {
    /// Accepts a complete power-of-two mip chain of finite linear RGB radiance.
    /// Face size is at most 512; each level contains six square faces.
    pub fn from_prefiltered(
        size: u32,
        levels: Vec<Vec<[f32; 3]>>,
    ) -> Result<Self, EnvironmentError3d> {
        if !size.is_power_of_two() || size > 512 || levels.len() != size.ilog2() as usize + 1 {
            return Err(EnvironmentError3d::Prefilter);
        }
        for (level, pixels) in levels.iter().enumerate() {
            let edge = (size >> level) as usize;
            if pixels.len() != 6 * edge * edge {
                return Err(EnvironmentError3d::Prefilter);
            }
            for (pixel, rgb) in pixels.iter().enumerate() {
                if rgb
                    .iter()
                    .any(|v| !v.is_finite() || !(0. ..=65504.).contains(v))
                {
                    return Err(EnvironmentError3d::Radiance { pixel });
                }
            }
        }
        Ok(Self {
            size,
            levels: levels.into(),
        })
    }
    /// Base face edge in texels.
    pub fn size(&self) -> u32 {
        self.size
    }
    /// Complete roughness mip chain; clones share these allocations.
    pub fn levels(&self) -> &[Vec<[f32; 3]>] {
        &self.levels
    }
}

/// Distant specular illumination, independent of diffuse light and background.
#[derive(Clone, Debug)]
pub struct SpecularEnvironment3d {
    /// Shared prefiltered radiance.
    pub map: SpecularEnvironmentMap3d,
    /// Nonnegative linear radiance multiplier in [0, 65504].
    pub intensity: f32,
    /// Rotation around world +Y in radians.
    pub rotation_y: f32,
}
impl SpecularEnvironment3d {
    /// Checks finite, bounded illumination parameters.
    pub fn is_valid(&self) -> bool {
        self.intensity.is_finite()
            && (0. ..=65504.).contains(&self.intensity)
            && self.rotation_y.is_finite()
    }
}
