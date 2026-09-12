use super::*;
use gpui_3d::Scene3dDeviceCapabilities;

impl GpuSceneDeformation {
    /// Checks topology and enabled compute features/limits for all imported stages
    /// without creating a device, uploading buffers or submitting GPU work. Static
    /// primitives impose no deformation requirements. Direction sources are checked
    /// even for zero authored weights because later samples may activate them.
    /// Per-buffer sizes, payload budgets, source values, device health and shader
    /// execution are validated separately; success is not GPU allocation admission.
    pub fn check_support(
        asset: &SceneAsset,
        capabilities: &Scene3dDeviceCapabilities,
    ) -> Result<()> {
        Self::check_asset(asset)?;
        let morphs: HashMap<_, _> = asset
            .morphs()
            .iter()
            .map(|morph| (morph.primitive(), morph))
            .collect();
        let skins: std::collections::HashSet<_> =
            asset.skins().iter().map(|skin| skin.primitive()).collect();
        for &primitive in asset.primitives() {
            let check = (|| -> Result<()> {
                if let Some(morph) = morphs.get(&primitive.handle) {
                    GpuMorph::check_support(capabilities).context("Morph computation")?;
                    if morph.geometry().regenerates_normals() {
                        GpuFlatNormals::check_support(capabilities)
                            .context("flat normal reconstruction")?;
                    }
                    if morph.geometry().regenerates_tangents() {
                        GpuTangentGeneration::check_support(capabilities)
                            .context("tangent reconstruction")?;
                    }
                }
                if skins.contains(&primitive.handle) {
                    GpuSkin::check_support(capabilities).context("Skin computation")?;
                }
                Ok(())
            })();
            check.with_context(|| description(primitive))?;
        }
        Ok(())
    }
}
