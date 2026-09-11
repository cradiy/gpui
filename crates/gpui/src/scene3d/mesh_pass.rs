use super::{AlphaMode3d, MeshMaterial3d};
mod expansion;
pub use expansion::*;

/// Maximum additional color draws per mesh.
pub const MAX_MESH_PASSES_3D: usize = 8;

/// Additional draws run in object submission order, then per-object declaration order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MeshPassStage3d {
    /// After opaque/masked surfaces and before sorted transparent surfaces.
    #[default]
    AfterOpaque,
    /// After all primary surfaces and AfterOpaque draws.
    AfterTransparent,
}

/// Local triangle faces to discard. Mirrored object transforms preserve local winding.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u32)]
pub enum MeshPassCull3d {
    /// Retain both faces.
    #[default]
    None,
    /// Discard local counterclockwise faces.
    Front,
    /// Discard local clockwise faces.
    Back,
}

/// Comparison against the shared camera depth attachment.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MeshPassDepth3d {
    /// Reject every fragment.
    Never,
    /// Accept smaller depth.
    Less,
    /// Accept equal depth.
    Equal,
    /// Accept smaller or equal depth.
    #[default]
    LessEqual,
    /// Accept greater depth.
    Greater,
    /// Accept unequal depth.
    NotEqual,
    /// Accept greater or equal depth.
    GreaterEqual,
    /// Disable depth rejection.
    Always,
}

/// Linear premultiplied color composition before scene exposure and output encoding.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MeshPassBlend3d {
    /// Replace destination color and alpha.
    Replace,
    /// Premultiplied source-over composition.
    #[default]
    SourceOver,
    /// Adds RGB; alpha uses source-over accumulation.
    Additive,
}

/// Independent color-pass state; does not change primary shadow or data outputs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MeshPassState3d {
    /// Scheduling point relative to primary surfaces.
    pub stage: MeshPassStage3d,
    /// Local face visibility, independent of the primary material.
    pub cull: MeshPassCull3d,
    /// Test against the shared camera depth attachment.
    pub depth_compare: MeshPassDepth3d,
    /// Update camera depth for subsequent color draws.
    pub depth_write: bool,
    /// Constant raster depth bias, in backend depth units.
    pub depth_bias: i32,
    /// Finite scale applied to the maximum depth slope.
    pub depth_slope_bias: f32,
    /// Nonnegative finite maximum bias magnitude; zero means unclamped.
    pub depth_bias_clamp: f32,
    /// Composition into the linear HDR color attachment.
    pub blend: MeshPassBlend3d,
    /// Surface-alpha interpretation; depth and blending remain independently controlled.
    pub alpha_mode: AlphaMode3d,
    /// Finite value in [0, 1], used by Mask coverage.
    pub alpha_cutoff: f32,
}

impl Default for MeshPassState3d {
    fn default() -> Self {
        Self {
            stage: MeshPassStage3d::AfterOpaque,
            cull: MeshPassCull3d::None,
            depth_compare: MeshPassDepth3d::LessEqual,
            depth_write: false,
            depth_bias: 0,
            depth_slope_bias: 0.,
            depth_bias_clamp: 0.,
            blend: MeshPassBlend3d::SourceOver,
            alpha_mode: AlphaMode3d::Blend,
            alpha_cutoff: 0.5,
        }
    }
}

impl MeshPassState3d {
    /// Whether floating-point controls are finite and within their supported ranges.
    pub fn is_valid(&self) -> bool {
        self.depth_slope_bias.is_finite()
            && self.depth_bias_clamp.is_finite()
            && self.depth_bias_clamp >= 0.
            && self.alpha_cutoff.is_finite()
            && (0. ..=1.).contains(&self.alpha_cutoff)
    }
}

/// A material draw reusing its owner's geometry, transform and standard material inputs.
/// Does not cast shadows or contribute to ID, depth or normal output channels.
#[derive(Clone, Debug)]
pub struct MeshPass3d {
    /// Optional bounded normal displacement, applied only to this color pass.
    pub expansion: Option<MeshPassExpansion3d>,
    /// Retained backend material resources and custom vertex streams.
    pub material: MeshMaterial3d,
    /// Color-pass raster and composition controls.
    pub state: MeshPassState3d,
}
