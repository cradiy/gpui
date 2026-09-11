pub use gpui::{
    MeshPassBlend3d as MeshPassBlend, MeshPassCull3d as MeshPassCull,
    MeshPassDepth3d as MeshPassDepth, MeshPassExpansion3d as MeshPassExpansion,
    MeshPassSpace3d as MeshPassSpace, MeshPassStage3d as MeshPassStage,
    MeshPassState3d as MeshPassState,
};

/// An additional color draw sharing its object's CPU or GPU geometry.
#[derive(Clone)]
pub struct MeshPass(pub(crate) gpui::MeshPass3d);

impl MeshPass {
    /// Uses independent material resources and custom vertex streams, with default state.
    #[cfg(all(feature = "wgpu", not(target_family = "wasm")))]
    pub fn new(material: crate::Scene3dMaterialSnapshot) -> Self {
        Self(gpui::MeshPass3d {
            expansion: None,
            material: gpui::MeshMaterial3d::new(std::sync::Arc::new(material)),
            state: Default::default(),
        })
    }

    /// Sets independent face visibility, depth, composition and scheduling controls.
    pub fn state(mut self, state: MeshPassState) -> Self {
        self.0.state = state;
        self
    }

    /// Displaces only the additional pass, without changing primary geometry or picking.
    pub fn expansion(mut self, expansion: MeshPassExpansion) -> Self {
        self.0.expansion = Some(expansion);
        self
    }
}
