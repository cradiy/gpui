use super::*;
use gpui_3d::{
    GpuDeformationVertex, GpuFlatNormalsMemory, GpuMorphMemory, GpuSkinMemory,
    GpuTangentGenerationMemory,
};

/// Retained imported deformation buffer payload, excluding CPU data, pipelines,
/// driver overhead, evaluation allocations, render sources and readbacks.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GpuSceneSourceMemory {
    pub source_bytes: u64,
    /// Deformable primitive occurrences, without deduplication across nodes.
    pub primitive_count: usize,
}

impl GpuSceneSourceMemory {
    /// Checks imported direction policies, core payload limits and the optional
    /// aggregate source budget without GPU allocation. Device support and device
    /// limits are checked separately during construction.
    pub fn plan(
        asset: &SceneAsset,
        limits: GpuDeformationLimits,
        max_source_bytes: Option<u64>,
    ) -> Result<Self> {
        GpuSceneDeformation::check_asset(asset)?;
        let morphs: HashMap<_, _> = asset
            .morphs()
            .iter()
            .map(|source| (source.primitive(), source))
            .collect();
        let skins: HashMap<_, _> = asset
            .skins()
            .iter()
            .map(|source| (source.primitive(), source))
            .collect();
        let mut memory = Self::default();
        for &primitive in asset.primitives() {
            let morph = morphs.get(&primitive.handle);
            let skin = skins.get(&primitive.handle);
            if morph.is_none() && skin.is_none() {
                continue;
            }
            let plan = (|| -> Result<()> {
                if let Some(morph) = morph {
                    let geometry = morph.geometry();
                    let targets = geometry.attribute_targets();
                    let vertices = targets.base_mesh().vertex_count();
                    let source = GpuMorphMemory::plan(vertices, targets.targets().len(), limits)?;
                    memory.add(source.base_bytes)?;
                    memory.add(source.delta_bytes)?;
                    memory.add(source.uniform_bytes)?;
                    if geometry.regenerates_normals() {
                        let source = GpuFlatNormalsMemory::plan(vertices, limits)?;
                        memory.add(source.face_offsets_bytes)?;
                        memory.add(source.index_bytes)?;
                        memory.add(source.uniform_bytes)?;
                    }
                    if geometry.regenerates_tangents() {
                        memory.add(
                            GpuTangentGenerationMemory::plan(vertices, limits)?.source_bytes,
                        )?;
                        memory.add_bind(geometry.base_mesh().vertex_count(), limits)?;
                    }
                }
                if let Some(skin) = skin {
                    let binding = skin.binding();
                    let influences =
                        (0..binding.vertex_count()).try_fold(0_usize, |count, vertex| {
                            count
                                .checked_add(binding.vertex_influences(vertex)?.len())
                                .context("GPU Skin influence count overflow")
                        })?;
                    let source = GpuSkinMemory::plan(
                        binding.vertex_count(),
                        binding.joint_count(),
                        influences,
                        limits,
                    )?;
                    memory.add(source.binding_bytes)?;
                    memory.add(source.uniform_bytes)?;
                    if morph.is_none() {
                        memory.add_bind(skin.base_mesh().vertex_count(), limits)?;
                    }
                }
                Ok(())
            })();
            plan.with_context(|| description(primitive))?;
            memory.primitive_count += 1;
        }
        ensure!(
            max_source_bytes.is_none_or(|limit| memory.source_bytes <= limit),
            "GPU scene sources require {} bytes, exceeding the configured budget",
            memory.source_bytes,
        );
        Ok(memory)
    }

    fn add(&mut self, bytes: u64) -> Result<()> {
        self.source_bytes = self
            .source_bytes
            .checked_add(bytes)
            .context("GPU scene source payload overflow")?;
        Ok(())
    }

    fn add_bind(&mut self, vertices: usize, limits: GpuDeformationLimits) -> Result<()> {
        let bytes = (vertices as u64)
            .checked_mul(std::mem::size_of::<GpuDeformationVertex>() as u64)
            .context("GPU bind payload overflow")?;
        ensure!(
            bytes <= limits.max_source_bytes && bytes <= limits.max_output_bytes,
            "GPU bind mesh exceeds payload budget"
        );
        self.add(bytes)
    }
}
