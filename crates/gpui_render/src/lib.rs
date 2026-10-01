//! Shared primitive shaders and their native resource contracts.

/// Common types, color functions and rectangle entry points in WGSL.
pub const QUAD_WGSL: &str = concat!(include_str!("common.wgsl"), include_str!("quads.wgsl"));
/// Native rectangle shader for Metal, generated from [`QUAD_WGSL`].
pub const QUAD_MSL: &str = include_str!(concat!(env!("OUT_DIR"), "/quads.metal"));
/// Native rectangle shader for Direct3D 11, generated from [`QUAD_WGSL`].
pub const QUAD_HLSL: &str = include_str!(concat!(env!("OUT_DIR"), "/quads.hlsl"));

/// Appends additional WGSL primitives to the shared definitions.
pub fn compose_shader(primitives: &str) -> String {
    format!("{QUAD_WGSL}\n{primitives}")
}

/// Uniforms for shared rectangle shaders. Native renderers use straight alpha
/// and sRGB colors; WGPU selects alpha mode to match its render target.
#[derive(Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
pub struct QuadGlobals {
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

/// Metal buffer slot for [`QuadGlobals`].
pub const METAL_GLOBALS_SLOT: u64 = 0;
/// Metal buffer slot for rectangle instances.
pub const METAL_QUADS_SLOT: u64 = 1;
/// Metal buffer slot for Naga's runtime array lengths, expressed in bytes.
pub const METAL_SIZES_SLOT: u64 = 3;

#[cfg(test)]
mod tests;
