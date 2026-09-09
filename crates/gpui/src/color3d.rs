/// Transfer function of an image's RGB channels. Alpha is always linear.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u32)]
pub enum TextureColorSpace3d {
    /// Display-encoded color, decoded before image filtering and lighting.
    #[default]
    Srgb = 0,
    /// Linear values, sampled without a transfer-function conversion.
    Linear = 1,
}

/// Mapping of exposed linear HDR color into the display range.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u32)]
pub enum ToneMapping3d {
    /// Clamp to the display range without compressing highlights.
    #[default]
    None = 0,
    /// Compress each channel with `x / (1 + x)`.
    Reinhard = 1,
}

/// Display conversion applied to each linear HDR sample before color resolve.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ColorOutput3d {
    /// Exposure in stops, from -16 through 16. Zero leaves intensity unchanged.
    pub exposure: f32,
    /// Highlight mapping before sRGB encoding.
    pub tone_mapping: ToneMapping3d,
}

impl ColorOutput3d {
    /// Whether exposure is finite and within the supported range.
    pub fn is_valid(self) -> bool {
        self.exposure.is_finite() && (-16.0..=16.0).contains(&self.exposure)
    }
}
