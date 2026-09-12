use super::{
    GpuDeformationLimits, GpuDeformationOutput, GpuTangentAdjacency, GpuTangentAdjacencyMemory,
    GpuTangentDerivatives, GpuTangentDerivativesMemory, GpuTangentFrames, GpuTangentFramesMemory,
    GpuTangentGroups, GpuTangentGroupsMemory, GpuTangentWeld, GpuTangentWeldMemory, GpuTangents,
    GpuTangentsMemory, GpuTangentsOutput,
};
use crate::{Mesh, TangentGenerationMode};
use anyhow::{Context, Result, ensure};
use gpui_wgpu::{Scene3dDeviceCapabilities, WgpuContext};

#[cfg(test)]
mod tests;

/// Combined GPU payload for all tangent generation stages. Excludes input snapshots,
/// CPU preparation, pipelines, driver overhead and other retained evaluations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GpuTangentGenerationMemory {
    /// Retained topology, coordinates and uniforms across all sources.
    pub source_bytes: u64,
    /// Conservative sum of every stage's scratch and output allocation for one call.
    pub evaluation_bytes: u64,
    /// Final vertex and repair buffers; included in evaluation_bytes.
    pub retained_output_bytes: u64,
}

impl GpuTangentGenerationMemory {
    /// Plans an unshared mesh with one vertex per indexed corner, without a device.
    /// Limits apply to combined source and evaluation payload, not to each stage alone.
    pub fn plan(corners: usize, limits: GpuDeformationLimits) -> Result<Self> {
        let derivatives = GpuTangentDerivativesMemory::plan(corners, corners, limits)?;
        let weld = GpuTangentWeldMemory::plan(corners, corners, limits)?;
        let adjacency = GpuTangentAdjacencyMemory::plan(corners, limits)?;
        let groups = GpuTangentGroupsMemory::plan(corners, limits)?;
        let frames = GpuTangentFramesMemory::plan(corners, limits)?;
        let publication = GpuTangentsMemory::plan(corners, limits)?;
        let retained_output_bytes = publication.vertex_bytes + publication.repair_bytes;
        let memory = Self {
            source_bytes: derivatives.uv_bytes
                + derivatives.index_bytes
                + derivatives.uniform_bytes
                + weld.uv_bytes
                + weld.index_bytes
                + weld.uniform_bytes
                + adjacency.uniform_bytes
                + groups.uniform_bytes
                + frames.uniform_bytes
                + publication.topology_bytes
                + publication.uniform_bytes,
            evaluation_bytes: derivatives.output_bytes
                + weld.scratch_bytes
                + weld.output_bytes
                + adjacency.scratch_bytes
                + adjacency.output_bytes
                + groups.scratch_bytes
                + groups.output_bytes
                + frames.scratch_bytes
                + frames.output_bytes
                + retained_output_bytes,
            retained_output_bytes,
        };
        ensure!(
            memory.source_bytes <= limits.max_source_bytes,
            "GPU tangent generation sources exceed combined payload budget"
        );
        ensure!(
            memory.evaluation_bytes <= limits.max_output_bytes,
            "GPU tangent generation evaluation exceeds combined payload budget"
        );
        Ok(memory)
    }
}

/// Reusable derivative, weld, adjacency, group, frame and publication sources.
/// Inputs require unshared triangle corners and matching immutable mesh identity.
/// Evaluation preserves the existing normals and returns canonical GPU vertices
/// with repair tags, without CPU vertex readback or CPU fallback.
pub struct GpuTangentGeneration {
    base: Mesh,
    uv_set: u32,
    memory: GpuTangentGenerationMemory,
    derivatives: GpuTangentDerivatives,
    weld: GpuTangentWeld,
    adjacency: GpuTangentAdjacency,
    groups: GpuTangentGroups,
    frames: GpuTangentFrames,
    publication: GpuTangents,
}

impl GpuTangentGeneration {
    pub fn check_support(capabilities: &Scene3dDeviceCapabilities) -> Result<()> {
        GpuTangentDerivatives::check_support(capabilities)?;
        GpuTangentWeld::check_support(capabilities)?;
        GpuTangentAdjacency::check_support(capabilities)?;
        GpuTangentGroups::check_support(capabilities)?;
        GpuTangentFrames::check_support(capabilities)?;
        GpuTangents::check_support(capabilities)
    }

    /// Checks aggregate budgets and capabilities before constructing stage sources.
    /// Initial CPU tangent preparation uses the selected policy. Vertex splitting
    /// and external attribute remapping remain explicit caller operations.
    pub fn new(
        context: WgpuContext,
        base: Mesh,
        uv_set: u32,
        mode: TangentGenerationMode,
        limits: GpuDeformationLimits,
    ) -> Result<Self> {
        let memory = GpuTangentGenerationMemory::plan(base.vertex_count(), limits)?;
        Self::check_support(&Scene3dDeviceCapabilities::query(&context))?;
        let publication = GpuTangents::new(context.clone(), base.clone(), uv_set, mode, limits)
            .context("tangent publication preparation")?;
        Ok(Self {
            derivatives: GpuTangentDerivatives::new(context.clone(), base.clone(), uv_set, limits)?,
            weld: GpuTangentWeld::new(context.clone(), base.clone(), uv_set, limits)?,
            adjacency: GpuTangentAdjacency::new(context.clone(), base.clone(), uv_set, limits)?,
            groups: GpuTangentGroups::new(context.clone(), base.clone(), uv_set, limits)?,
            frames: GpuTangentFrames::new(context, base.clone(), uv_set, limits)?,
            publication,
            base,
            uv_set,
            memory,
        })
    }

    pub fn base_mesh(&self) -> &Mesh {
        &self.base
    }
    pub fn uv_set(&self) -> u32 {
        self.uv_set
    }
    pub fn memory(&self) -> GpuTangentGenerationMemory {
        self.memory
    }

    /// Mesh identity with initial tangents, required for rendering generated outputs.
    pub fn output_mesh(&self) -> &Mesh {
        self.publication.output_mesh()
    }

    /// Generates tangents from this input snapshot independently of prior calls.
    /// GPU arithmetic failures remain in output status; submission success is not
    /// geometry validation. Use deformation readback or render packing status to
    /// inspect results. Returned buffers remain valid after source destruction.
    pub fn evaluate(&self, input: &GpuDeformationOutput) -> Result<GpuTangentsOutput> {
        let derivatives = self
            .derivatives
            .evaluate(input)
            .context("tangent derivatives")?;
        let weld = self
            .weld
            .evaluate(&derivatives)
            .context("tangent welding")?;
        let adjacency = self
            .adjacency
            .evaluate(&weld)
            .context("tangent adjacency")?;
        let groups = self.groups.evaluate(&adjacency).context("tangent groups")?;
        let frames = self.frames.evaluate(&groups).context("tangent frames")?;
        self.publication
            .evaluate(&frames)
            .context("tangent publication")
    }
}
