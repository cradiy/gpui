//! Shared primitive shaders and their native resource contracts.

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

/// Appends additional WGSL primitives to the shared definitions.
pub fn compose_shader(primitives: &str) -> String {
    format!(
        "{QUAD_WGSL}\n{}\n{}\n{}\n{}\n{}\n{primitives}",
        include_str!("shadows.wgsl"),
        include_str!("underlines.wgsl"),
        include_str!("path_rasterization.wgsl"),
        include_str!("paths.wgsl"),
        include_str!("polychrome_sprites.wgsl")
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
/// Metal sampler slot for filtered sprite atlas reads.
pub const METAL_SAMPLER_SLOT: u64 = 0;

#[cfg(test)]
mod tests;
