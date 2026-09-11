use super::gpu_deformation::{ComputeKernel, buffer, pack_mesh, pad, validate_storage};
use crate::{GpuDeformationLimits, GpuDeformationOutput, GpuDeformationVertex, MorphTargets};
use anyhow::{Context as _, Result, ensure};
use bytemuck::Zeroable;
use gpui_wgpu::{WgpuContext, wgpu};

#[cfg(test)]
mod tests;

/// Payload sizes, excluding driver overhead and optional readback staging.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GpuMorphMemory {
    pub base_bytes: u64,
    pub delta_bytes: u64,
    pub weight_bytes: u64,
    pub output_bytes: u64,
    pub uniform_bytes: u64,
}
impl GpuMorphMemory {
    /// Checked without an adapter. Empty target/weight bindings use one padded record.
    pub fn plan(vertices: usize, targets: usize, limits: GpuDeformationLimits) -> Result<Self> {
        ensure!(
            vertices > 0 && vertices <= u32::MAX as usize,
            "GPU Morph vertex count must fit positive u32"
        );
        ensure!(
            targets <= u32::MAX as usize,
            "GPU Morph target count exceeds u32"
        );
        let records = (vertices as u64)
            .checked_mul(targets as u64)
            .context("GPU Morph target size overflow")?;
        ensure!(
            records <= u64::from(u32::MAX),
            "GPU Morph target indexing exceeds u32"
        );
        let base_bytes = vertices as u64 * 64;
        let delta_bytes = records.max(1) * 64;
        let weight_bytes = (targets as u64).max(1) * 4;
        ensure!(
            base_bytes + delta_bytes + 16 <= limits.max_source_bytes,
            "GPU Morph source exceeds payload budget"
        );
        ensure!(
            base_bytes <= limits.max_output_bytes,
            "GPU Morph output exceeds payload budget"
        );
        Ok(Self {
            base_bytes,
            delta_bytes,
            weight_bytes,
            output_bytes: base_bytes,
            uniform_bytes: 16,
        })
    }

    fn validate_device(self, limits: &wgpu::Limits) -> Result<()> {
        validate_storage(
            limits,
            &[
                self.base_bytes,
                self.delta_bytes,
                self.weight_bytes,
                self.output_bytes,
            ],
            (self.output_bytes / 64) as usize,
        )
    }
}

/// Uploaded immutable Morph inputs and a reusable compute pipeline on one device.
pub struct GpuMorph {
    context: WgpuContext,
    source: MorphTargets,
    memory: GpuMorphMemory,
    base: wgpu::Buffer,
    deltas: wgpu::Buffer,
    params: wgpu::Buffer,
    kernel: ComputeKernel,
}

impl GpuMorph {
    pub fn new(
        context: WgpuContext,
        source: MorphTargets,
        limits: GpuDeformationLimits,
    ) -> Result<Self> {
        ensure!(!context.device_lost(), "GPU Morph device is lost");
        let device = &context.device;
        let capabilities = device.limits();
        ensure!(
            capabilities.max_storage_buffers_per_shader_stage >= 4
                && capabilities.max_compute_invocations_per_workgroup >= 64
                && capabilities.max_compute_workgroup_size_x >= 64,
            "GPU Morph compute limits are unsupported"
        );
        let memory = GpuMorphMemory::plan(
            source.base_mesh().vertex_count(),
            source.targets().len(),
            limits,
        )?;
        memory.validate_device(&capabilities)?;
        let mesh = source.base_mesh();
        let base = pack_mesh(mesh);
        let mut deltas = Vec::with_capacity((memory.delta_bytes / 64) as usize);
        for target in source.targets() {
            for vertex in 0..mesh.vertex_count() {
                deltas.push(GpuDeformationVertex {
                    position: pad(target.positions.as_ref().map_or([0.; 3], |v| v[vertex])),
                    normal: pad(target.normals.as_ref().map_or([0.; 3], |v| v[vertex])),
                    tangent: pad(target.tangents.as_ref().map_or([0.; 3], |v| v[vertex])),
                    status: [u32::from(target.normals.is_some()), 0, 0, 0],
                });
            }
        }
        if deltas.is_empty() {
            deltas.push(GpuDeformationVertex::zeroed());
        }
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let base = buffer(
            device,
            "gpui_3d.morph.base",
            bytemuck::cast_slice(&base),
            wgpu::BufferUsages::STORAGE,
        );
        let deltas = buffer(
            device,
            "gpui_3d.morph.deltas",
            bytemuck::cast_slice(&deltas),
            wgpu::BufferUsages::STORAGE,
        );
        let params = buffer(
            device,
            "gpui_3d.morph.params",
            bytemuck::cast_slice(&[
                mesh.vertex_count() as u32,
                source.targets().len() as u32,
                u32::from(mesh.tangents().is_some()),
                0,
            ]),
            wgpu::BufferUsages::UNIFORM,
        );
        if let Some(error) = gpui::block_on(scope.pop()) {
            anyhow::bail!("GPU Morph preparation: {error}");
        }
        let kernel =
            ComputeKernel::new(device, include_str!("gpu_morph.wgsl"), "morph", [64, 64, 4])?;
        Ok(Self {
            context,
            source,
            memory,
            base,
            deltas,
            params,
            kernel,
        })
    }

    pub fn source(&self) -> &MorphTargets {
        &self.source
    }
    pub fn memory(&self) -> GpuMorphMemory {
        self.memory
    }

    /// Submits a fresh output. Inputs are immutable and earlier outputs are never overwritten.
    /// Arithmetic is f32; vertex status and CPU mesh validity are checked on explicit readback.
    pub fn evaluate(&self, weights: &[f32]) -> Result<GpuDeformationOutput> {
        ensure!(!self.context.device_lost(), "GPU Morph device is lost");
        ensure!(
            weights.len() == self.source.targets().len(),
            "GPU Morph weight count mismatch"
        );
        ensure!(
            weights.iter().all(|weight| weight.is_finite()),
            "GPU Morph weights must be finite"
        );
        let device = &self.context.device;
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let weights = buffer(
            device,
            "gpui_3d.morph.weights",
            bytemuck::cast_slice(if weights.is_empty() { &[0.] } else { weights }),
            wgpu::BufferUsages::STORAGE,
        );
        if let Some(error) = gpui::block_on(scope.pop()) {
            anyhow::bail!("GPU Morph weights: {error}");
        }
        self.kernel.evaluate(
            &self.context,
            self.source.base_mesh().clone(),
            [&self.base, &self.deltas, &weights],
            &self.params,
        )
    }
}
