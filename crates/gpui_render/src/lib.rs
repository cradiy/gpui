//! Shared primitive shaders and their native resource contracts.

/// Separable backdrop blur and composition using a hardware linear sampler.
pub const BACKDROP_BLUR_WGSL: &str = concat!(
    include_str!("backdrop_blur.wgsl"),
    include_str!("backdrop_sampling.wgsl")
);
/// Backdrop blur and composition using texture loads for bilinear interpolation.
/// This variant requires no sampler binding.
pub const BACKDROP_BLUR_MANUAL_WGSL: &str = concat!(
    include_str!("backdrop_blur.wgsl"),
    include_str!("backdrop_sampling_manual.wgsl")
);

/// Common types, color functions and rectangle entry points in WGSL.
pub const QUAD_WGSL: &str = concat!(include_str!("common.wgsl"), include_str!("quads.wgsl"));
/// Native rectangle shader for Metal, generated from [`QUAD_WGSL`].
pub const QUAD_MSL: &str = include_str!(concat!(env!("OUT_DIR"), "/quads.metal"));
/// Native rectangle shader for Direct3D 11, generated from [`QUAD_WGSL`].
pub const QUAD_HLSL: &str = include_str!(concat!(env!("OUT_DIR"), "/quads.hlsl"));

/// Common definitions and shadow entry points in WGSL.
pub const SHADOW_WGSL: &str = concat!(include_str!("common.wgsl"), include_str!("shadows.wgsl"));
/// Native shadow shader for Metal.
pub const SHADOW_MSL: &str = include_str!(concat!(env!("OUT_DIR"), "/shadows.metal"));
/// Native shadow shader for Direct3D 11.
pub const SHADOW_HLSL: &str = include_str!(concat!(env!("OUT_DIR"), "/shadows.hlsl"));
/// Common definitions and underline entry points in WGSL.
pub const UNDERLINE_WGSL: &str =
    concat!(include_str!("common.wgsl"), include_str!("underlines.wgsl"));
/// Native underline shader for Metal.
pub const UNDERLINE_MSL: &str = include_str!(concat!(env!("OUT_DIR"), "/underlines.metal"));
/// Native underline shader for Direct3D 11.
pub const UNDERLINE_HLSL: &str = include_str!(concat!(env!("OUT_DIR"), "/underlines.hlsl"));

/// Common definitions and path rasterization entry points in WGSL.
pub const PATH_RASTERIZATION_WGSL: &str = concat!(
    include_str!("common.wgsl"),
    include_str!("path_rasterization.wgsl")
);
/// Native path rasterization shader for Metal.
pub const PATH_RASTERIZATION_MSL: &str =
    include_str!(concat!(env!("OUT_DIR"), "/path_rasterization.metal"));
/// Native path rasterization shader for Direct3D 11.
pub const PATH_RASTERIZATION_HLSL: &str =
    include_str!(concat!(env!("OUT_DIR"), "/path_rasterization.hlsl"));

/// Common definitions and path composition entry points in WGSL.
pub const PATH_WGSL: &str = concat!(include_str!("common.wgsl"), include_str!("paths.wgsl"));
/// Native path composition shader for Metal.
pub const PATH_MSL: &str = include_str!(concat!(env!("OUT_DIR"), "/paths.metal"));
/// Native path composition shader for Direct3D 11.
pub const PATH_HLSL: &str = include_str!(concat!(env!("OUT_DIR"), "/paths.hlsl"));

/// Common definitions and color sprite entry points in WGSL.
pub const POLYCHROME_WGSL: &str = concat!(
    include_str!("common.wgsl"),
    include_str!("polychrome_sprites.wgsl")
);
/// Native color sprite shader for Metal.
pub const POLYCHROME_MSL: &str =
    include_str!(concat!(env!("OUT_DIR"), "/polychrome_sprites.metal"));
/// Native color sprite shader for Direct3D 11.
pub const POLYCHROME_HLSL: &str =
    include_str!(concat!(env!("OUT_DIR"), "/polychrome_sprites.hlsl"));

/// Common definitions and grayscale text and mask entry points.
pub const MONOCHROME_WGSL: &str = concat!(
    include_str!("common.wgsl"),
    include_str!("monochrome_sprites.wgsl")
);
/// Native grayscale text and mask shader for Metal.
pub const MONOCHROME_MSL: &str =
    include_str!(concat!(env!("OUT_DIR"), "/monochrome_sprites.metal"));
/// Native grayscale text and mask shader for Direct3D 11.
pub const MONOCHROME_HLSL: &str =
    include_str!(concat!(env!("OUT_DIR"), "/monochrome_sprites.hlsl"));
/// Subpixel entry points appended after shared definitions with dual-source blending enabled.
pub const SUBPIXEL_WGSL: &str = include_str!("subpixel_sprites.wgsl");
/// Native subpixel text shader for Direct3D 11.
pub const SUBPIXEL_HLSL: &str = include_str!(concat!(env!("OUT_DIR"), "/subpixel_sprites.hlsl"));

/// Common definitions and RGBA/NV12 surface entry points.
pub const SURFACE_WGSL: &str = concat!(include_str!("common.wgsl"), include_str!("surfaces.wgsl"));
/// Native RGBA and NV12 surface shader for Metal.
pub const SURFACE_MSL: &str = include_str!(concat!(env!("OUT_DIR"), "/surfaces.metal"));
/// Native RGBA and NV12 surface shader for Direct3D 11.
pub const SURFACE_HLSL: &str = include_str!(concat!(env!("OUT_DIR"), "/surfaces.hlsl"));

/// Appends additional WGSL primitives to the shared definitions.
pub fn compose_shader(primitives: &str) -> String {
    format!(
        "{QUAD_WGSL}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{primitives}",
        include_str!("shadows.wgsl"),
        include_str!("underlines.wgsl"),
        include_str!("path_rasterization.wgsl"),
        include_str!("paths.wgsl"),
        include_str!("polychrome_sprites.wgsl"),
        include_str!("monochrome_sprites.wgsl"),
        include_str!("surfaces.wgsl")
    )
}

/// Uniforms for shared primitive shaders. Native renderers use straight alpha
/// and sRGB colors; WGPU selects alpha mode to match its render target.
#[derive(Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
pub struct PrimitiveGlobals {
    /// Render target dimensions in physical pixels.
    pub viewport_size: [f32; 2],
    /// Whether shader output must be premultiplied.
    pub premultiplied_alpha: u32,
    /// Reserved, must be zero.
    pub pad: u32,
    /// Window-space origin of the render target.
    pub viewport_origin: [f32; 2],
    /// Reserved, must be zero.
    pub origin_pad: [u32; 2],
}

/// Metal buffer slot for [`PrimitiveGlobals`].
pub const METAL_GLOBALS_SLOT: u64 = 0;
/// Metal buffer slot for primitive instances.
pub const METAL_INSTANCES_SLOT: u64 = 1;
/// Metal buffer slot for Naga's runtime array lengths, expressed in bytes.
pub const METAL_SIZES_SLOT: u64 = 3;
/// Metal texture slot for sprite atlases and resolved path images.
pub const METAL_TEXTURE_SLOT: u64 = 0;
/// Metal texture slot for the interleaved chroma plane of an NV12 surface.
pub const METAL_CHROMA_TEXTURE_SLOT: u64 = 1;
/// Metal sampler slot for filtered sprite atlas reads.
pub const METAL_SAMPLER_SLOT: u64 = 0;

/// Font coverage correction parameters. Zero values preserve uncorrected coverage.
#[derive(Clone, Copy, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
pub struct GammaParams {
    /// Polynomial coefficients for coverage correction.
    pub gamma_ratios: [f32; 4],
    /// Contrast adjustment for grayscale glyphs.
    pub grayscale_enhanced_contrast: f32,
    /// Contrast adjustment for subpixel glyphs.
    pub subpixel_enhanced_contrast: f32,
    /// Nonzero when the display uses BGR subpixel order.
    pub is_bgr: u32,
    /// Reserved, must be zero.
    pub _pad: u32,
}

/// Parameters for RGBA and NV12 surface shaders.
/// Rectangles store `[origin_x, origin_y, width, height]`.
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
pub struct SurfaceParams {
    /// Surface rectangle in physical window pixels.
    pub bounds: [f32; 4],
    /// Rectangle used for rounded clipping, in physical window pixels.
    pub clip_bounds: [f32; 4],
    /// Content clipping rectangle in physical window pixels.
    pub content_mask: [f32; 4],
    /// Visible rectangle in normalized texture coordinates.
    pub uv_bounds: [f32; 4],
    /// Corner radii in top-left, top-right, bottom-right, bottom-left order.
    pub corner_radii: [f32; 4],
    /// Matrix rows converting `[Y, Cb, Cr, 1]` to RGB.
    pub color_rows: [[f32; 4]; 3],
    /// Opacity multiplier.
    pub opacity: f32,
    /// Reserved, must be zero.
    pub _pad: [f32; 3],
}

/// Metal buffer slot for font coverage correction parameters.
pub const METAL_GAMMA_SLOT: u64 = 2;

#[cfg(test)]
mod tests;
