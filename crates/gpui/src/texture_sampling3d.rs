/// Addressing of image coordinates outside the unit square.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u32)]
pub enum TextureAddressMode3d {
    /// Extend the edge texels.
    #[default]
    Clamp = 0,
    /// Repeat the image every unit interval.
    Repeat = 1,
    /// Alternate forward and reflected copies every unit interval.
    Mirror = 2,
}

/// Image filtering within a mip level.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u32)]
pub enum TextureFilter3d {
    /// Select the nearest texel.
    Nearest = 0,
    /// Interpolate the four neighboring texels.
    #[default]
    Linear = 1,
}

/// Selection and interpolation of image mip levels during minification.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum TextureMipFilter3d {
    /// Use only the original image.
    #[default]
    None,
    /// Select the nearest mip level.
    Nearest,
    /// Interpolate adjacent mip levels.
    Linear,
}

/// A non-finite image-coordinate transform.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("UV transform components must be finite")]
pub struct UvTransformError3d;

/// Affine image-coordinate transform, stored as two rows `[u, v, offset]`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UvTransform3d([[f32; 3]; 2]);

impl Default for UvTransform3d {
    fn default() -> Self {
        Self([[1., 0., 0.], [0., 1., 0.]])
    }
}

impl UvTransform3d {
    /// Accepts finite affine rows. Zero scale, reflection and shear are valid.
    pub fn from_rows(rows: [[f32; 3]; 2]) -> Result<Self, UvTransformError3d> {
        if !rows.iter().flatten().all(|value| value.is_finite()) {
            return Err(UvTransformError3d);
        }
        Ok(Self(rows))
    }

    /// Scales about UV origin, rotates, then translates. Rotation is in radians,
    /// positive clockwise in top-left-origin UV coordinates.
    pub fn from_scale_rotation_translation(
        scale: [f32; 2],
        rotation: f32,
        offset: [f32; 2],
    ) -> Result<Self, UvTransformError3d> {
        let (sin, cos) = rotation.sin_cos();
        Self::from_rows([
            [cos * scale[0], -sin * scale[1], offset[0]],
            [sin * scale[0], cos * scale[1], offset[1]],
        ])
    }

    /// The two affine rows, without padding.
    pub fn rows(self) -> [[f32; 3]; 2] {
        self.0
    }

    /// Returns `None` for non-finite input or an overflowing result.
    pub fn transform(self, uv: [f32; 2]) -> Option<[f32; 2]> {
        let result = self.0.map(|row| row[0] * uv[0] + row[1] * uv[1] + row[2]);
        result
            .iter()
            .all(|value| value.is_finite())
            .then_some(result)
    }
}

/// Image-coordinate transform, per-axis addressing and filtering.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextureSampling3d {
    /// Applied to mesh UVs before addressing and filtering.
    pub transform: UvTransform3d,
    /// Horizontal addressing.
    pub address_u: TextureAddressMode3d,
    /// Vertical addressing.
    pub address_v: TextureAddressMode3d,
    /// Minification texel filter, also used for magnification unless overridden.
    pub filter: TextureFilter3d,
    /// Optional magnification filter. `None` uses `filter`.
    pub mag_filter: Option<TextureFilter3d>,
    /// Defaults to `None`; other modes generate an independent image mip chain.
    pub mip_filter: TextureMipFilter3d,
    /// Maximum anisotropic sample ratio, from 1 through 16. Values above 1
    /// require linear texel and mip filtering. Defaults to 1 (isotropic).
    /// Backends without anisotropic filtering use isotropic sampling.
    pub max_anisotropy: u16,
}

impl Default for TextureSampling3d {
    fn default() -> Self {
        Self {
            transform: Default::default(),
            address_u: Default::default(),
            address_v: Default::default(),
            filter: Default::default(),
            mag_filter: None,
            mip_filter: Default::default(),
            max_anisotropy: 1,
        }
    }
}

impl TextureSampling3d {
    /// Magnification filter after applying the optional override.
    pub fn magnification_filter(self) -> TextureFilter3d {
        self.mag_filter.unwrap_or(self.filter)
    }
    /// Whether anisotropy and the selected filtering modes form a supported configuration.
    pub fn is_valid(self) -> bool {
        (1..=16).contains(&self.max_anisotropy)
            && (self.max_anisotropy == 1
                || self.filter == TextureFilter3d::Linear
                    && self.magnification_filter() == TextureFilter3d::Linear
                    && self.mip_filter == TextureMipFilter3d::Linear)
    }
}
