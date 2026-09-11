use super::{ComputeKernel, GpuDeformationLimits, GpuDeformationOutput, buffer, validate_storage};
use crate::Mesh;
use anyhow::{Result, ensure};
use gpui_wgpu::{Scene3dDeviceCapabilities, WgpuContext, wgpu};
use std::sync::Arc;

/// Retained mapping/uniform payload and one independent output. Input vertex
/// storage, CPU meshes, pipelines, driver overhead and readbacks are excluded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GpuDeformationRemapMemory {
    pub mapping_bytes: u64,
    pub uniform_bytes: u64,
    pub output_bytes: u64,
}

impl GpuDeformationRemapMemory {
    pub fn plan(
        source_vertices: usize,
        output_vertices: usize,
        limits: GpuDeformationLimits,
    ) -> Result<Self> {
        ensure!(
            source_vertices > 0 && source_vertices <= u32::MAX as usize,
            "GPU remap source vertex count must fit positive u32"
        );
        ensure!(
            output_vertices > 0 && output_vertices <= u32::MAX as usize,
            "GPU remap output vertex count must fit positive u32"
        );
        let memory = Self {
            mapping_bytes: output_vertices as u64 * 4,
            uniform_bytes: 16,
            output_bytes: output_vertices as u64 * 64,
        };
        ensure!(
            memory.mapping_bytes + memory.uniform_bytes <= limits.max_source_bytes,
            "GPU remap mapping exceeds source payload budget"
        );
        ensure!(
            memory.output_bytes <= limits.max_output_bytes,
            "GPU remap output exceeds payload budget"
        );
        Ok(memory)
    }
}

/// Reusable output-to-source mapping for evaluated GPU vertices. Copies canonical
/// records without floating-point arithmetic, preserving status and attribute bits.
/// The caller supplies destination topology, UVs and colors in the same coordinate
/// space. This does not interpolate, weld, transform or regenerate directions.
pub struct GpuDeformationRemap {
    context: WgpuContext,
    source: Mesh,
    destination: Mesh,
    memory: GpuDeformationRemapMemory,
    mapping: wgpu::Buffer,
    params: wgpu::Buffer,
    kernel: ComputeKernel<2>,
}

impl GpuDeformationRemap {
    pub fn check_support(capabilities: &Scene3dDeviceCapabilities) -> Result<()> {
        super::support::validate(capabilities, 3, 1, 0)
    }

    /// Validates the complete mapping and payload before uploading it. Duplicate
    /// source indices and subsets are allowed. Destination tangent metadata may
    /// be absent; when present, it must match the source's tangent coordinate set.
    pub fn new(
        context: WgpuContext,
        source: Mesh,
        destination: Mesh,
        source_vertices: &[u32],
        limits: GpuDeformationLimits,
    ) -> Result<Self> {
        ensure!(!context.device_lost(), "GPU deformation device is lost");
        let memory = GpuDeformationRemapMemory::plan(
            source.vertex_count(),
            destination.vertex_count(),
            limits,
        )?;
        validate_mapping(&source, &destination, source_vertices)?;
        Self::check_support(&Scene3dDeviceCapabilities::query(&context))?;
        validate_storage(
            &context.device.limits(),
            &[
                source.vertex_count() as u64 * 64,
                memory.mapping_bytes,
                memory.output_bytes,
            ],
            destination.vertex_count(),
        )?;
        let device = &context.device;
        let kernel = ComputeKernel::new(device, include_str!("remap.wgsl"), "remap", [64, 4])?;
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let mapping = buffer(
            device,
            "gpui_3d.remap.mapping",
            bytemuck::cast_slice(source_vertices),
            wgpu::BufferUsages::STORAGE,
        );
        let params = buffer(
            device,
            "gpui_3d.remap.params",
            bytemuck::cast_slice(&[destination.vertex_count() as u32, 0, 0, 0]),
            wgpu::BufferUsages::UNIFORM,
        );
        if let Some(error) = gpui::block_on(scope.pop()) {
            anyhow::bail!("GPU remap preparation: {error}");
        }
        Ok(Self {
            context,
            source,
            destination,
            memory,
            mapping,
            params,
            kernel,
        })
    }

    pub fn source_mesh(&self) -> &Mesh {
        &self.source
    }
    pub fn output_mesh(&self) -> &Mesh {
        &self.destination
    }
    pub fn memory(&self) -> GpuDeformationRemapMemory {
        self.memory
    }

    /// Creates an independent result with the destination mesh identity. Input
    /// device and source allocation must match. Previous results remain valid;
    /// CPU meshes, bounds and queries are unchanged. Invalid records are copied,
    /// not repaired; omitted source vertices do not affect the destination.
    pub fn evaluate(&self, input: &GpuDeformationOutput) -> Result<GpuDeformationOutput> {
        ensure!(
            Arc::ptr_eq(&self.context.device, &input.context.device),
            "GPU remap input belongs to a different device"
        );
        ensure!(
            self.source.ptr_eq(input.base_mesh()),
            "GPU remap source mesh mismatch"
        );
        self.kernel.evaluate(
            &self.context,
            self.destination.clone(),
            [&input.buffer, &self.mapping],
            &self.params,
        )
    }
}

fn validate_mapping(source: &Mesh, destination: &Mesh, mapping: &[u32]) -> Result<()> {
    ensure!(
        mapping.len() == destination.vertex_count(),
        "GPU remap requires one source index per output vertex"
    );
    for (output, &input) in mapping.iter().enumerate() {
        ensure!(
            (input as usize) < source.vertex_count(),
            "GPU remap output vertex {output} references missing source vertex {input}"
        );
    }
    ensure!(
        destination.tangent_uv_set().is_none()
            || destination.tangent_uv_set() == source.tangent_uv_set(),
        "GPU remap destination tangent coordinates do not match the source"
    );
    Ok(())
}

#[cfg(test)]
mod tests;
