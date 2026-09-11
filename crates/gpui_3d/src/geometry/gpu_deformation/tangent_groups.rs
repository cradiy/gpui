use super::{
    ComputeKernel, GpuDeformationLimits, GpuTangentAdjacencyOutput, buffer, validate_storage,
};
use crate::Mesh;
use anyhow::{Result, ensure};
use bytemuck::{Pod, Zeroable};
use gpui_wgpu::{WgpuContext, wgpu};
use std::sync::Arc;

#[cfg(test)]
mod tests;

/// A 64-byte regular-face component record in original triangle-corner order.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct GpuTangentGroup {
    /// Original corner, welded vertex representative, connected group representative,
    /// and UV orientation. Nonregular corners use u32::MAX for group and orientation.
    pub identity: [u32; 4],
    /// Outgoing and incoming neighboring corners within the group, adjacency eligibility
    /// (0 regular, 1 needs inheritance, 2 coincident positions, 3 failed), and zero.
    /// Missing neighbors use u32::MAX.
    pub neighbors: [u32; 4],
    pub reserved: [u32; 4],
    /// Status from the paired face adjacency. No errors are repaired or suppressed.
    pub status: [u32; 4],
}

/// Payload admission for one source and one evaluation. Excludes retained input
/// snapshots, CPU data, and driver overhead. Two equal buffers alternate; one is returned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GpuTangentGroupsMemory {
    pub uniform_bytes: u64,
    pub scratch_bytes: u64,
    pub output_bytes: u64,
    pub propagation_passes: u32,
}
impl GpuTangentGroupsMemory {
    pub fn plan(corners: usize, limits: GpuDeformationLimits) -> Result<Self> {
        ensure!(
            corners > 0 && corners <= u32::MAX as usize && corners.is_multiple_of(3),
            "GPU tangent groups require a positive u32 triangle-corner count"
        );
        let memory = Self {
            uniform_bytes: 16,
            scratch_bytes: corners as u64 * 64,
            output_bytes: corners as u64 * 64,
            propagation_passes: u32::BITS - (corners as u32 - 1).leading_zeros(),
        };
        ensure!(
            memory.uniform_bytes <= limits.max_source_bytes,
            "GPU tangent group uniforms exceed source payload budget"
        );
        ensure!(
            memory.scratch_bytes + memory.output_bytes <= limits.max_output_bytes,
            "GPU tangent group scratch and output exceed payload budget"
        );
        Ok(memory)
    }
}

/// Connected groups of regular corners across orientation-compatible adjacency.
/// Pointer doubling follows the two directed neighbors with a fixed logarithmic
/// pass bound. Degenerate-frame inheritance and final tangent averaging are separate.
pub struct GpuTangentGroups {
    context: WgpuContext,
    base: Mesh,
    uv_set: u32,
    memory: GpuTangentGroupsMemory,
    params: wgpu::Buffer,
    initialize: ComputeKernel,
    propagate: ComputeKernel,
    finalize: ComputeKernel,
}
impl GpuTangentGroups {
    pub fn check_support(capabilities: &gpui_wgpu::Scene3dDeviceCapabilities) -> Result<()> {
        super::support::validate(capabilities, 4, 1, 0)
    }
    pub fn new(
        context: WgpuContext,
        base: Mesh,
        uv_set: u32,
        limits: GpuDeformationLimits,
    ) -> Result<Self> {
        ensure!(!context.device_lost(), "GPU deformation device is lost");
        Self::check_support(&gpui_wgpu::Scene3dDeviceCapabilities::query(&context))?;
        ensure!(
            base.uv_at(uv_set, 0).is_some(),
            "missing UV set {uv_set} for GPU tangent groups"
        );
        let memory = GpuTangentGroupsMemory::plan(base.index_count(), limits)?;
        validate_storage(
            &context.device.limits(),
            &[memory.output_bytes],
            base.index_count(),
        )?;
        let device = &context.device;
        let shader = include_str!("tangent_groups.wgsl");
        let initialize = ComputeKernel::new(device, shader, "initialize", [64; 3])?;
        let propagate = ComputeKernel::new(device, shader, "propagate", [64; 3])?;
        let finalize = ComputeKernel::new(device, shader, "finalize", [64; 3])?;
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let params = buffer(
            device,
            "gpui_3d.groups.params",
            bytemuck::cast_slice(&[base.index_count() as u32, 0, 0, 0]),
            wgpu::BufferUsages::UNIFORM,
        );
        if let Some(error) = gpui::block_on(scope.pop()) {
            anyhow::bail!("GPU tangent group preparation: {error}");
        }
        Ok(Self {
            context,
            base,
            uv_set,
            memory,
            params,
            initialize,
            propagate,
            finalize,
        })
    }
    pub fn memory(&self) -> GpuTangentGroupsMemory {
        self.memory
    }

    /// Returns immutable corner groups paired with the exact adjacency snapshot.
    /// A group's representative is its minimum original corner, not a compact index
    /// or a stable application identifier. CPU geometry and queries remain unchanged.
    pub fn evaluate(
        &self,
        adjacency: &GpuTangentAdjacencyOutput,
    ) -> Result<GpuTangentGroupsOutput> {
        let faces = adjacency.weld().derivatives();
        ensure!(
            !self.context.device_lost(),
            "GPU deformation device is lost"
        );
        ensure!(
            Arc::ptr_eq(&self.context.device, &faces.context().device),
            "GPU tangent groups input belongs to a different device"
        );
        ensure!(
            Arc::ptr_eq(&self.base.0, &faces.base_mesh().0),
            "GPU tangent groups source mesh mismatch"
        );
        ensure!(
            self.uv_set == faces.uv_set(),
            "GPU tangent groups coordinate set mismatch"
        );
        let device = &self.context.device;
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let allocate = |label| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: self.memory.output_bytes,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            })
        };
        let mut a = allocate("gpui_3d.groups.a");
        let mut b = allocate("gpui_3d.groups.b");
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("gpui_3d.groups"),
        });
        let corners = self.base.index_count() as u32;
        let edges = adjacency.buffer();
        let weld = adjacency.weld().buffer();
        self.initialize.encode(
            device,
            &mut encoder,
            corners,
            [edges, edges, weld, &a, &self.params],
        );
        for _ in 0..self.memory.propagation_passes {
            self.propagate.encode(
                device,
                &mut encoder,
                corners,
                [&a, edges, weld, &b, &self.params],
            );
            std::mem::swap(&mut a, &mut b);
        }
        self.finalize.encode(
            device,
            &mut encoder,
            corners,
            [&a, edges, weld, &b, &self.params],
        );
        self.context.queue.submit(Some(encoder.finish()));
        if let Some(error) = gpui::block_on(scope.pop()) {
            anyhow::bail!("GPU tangent group submission: {error}");
        }
        Ok(GpuTangentGroupsOutput {
            adjacency: adjacency.clone(),
            buffer: b,
        })
    }
}

/// Immutable regular-corner groups, retaining their adjacency and deformation inputs.
#[derive(Clone)]
pub struct GpuTangentGroupsOutput {
    adjacency: GpuTangentAdjacencyOutput,
    buffer: wgpu::Buffer,
}
impl GpuTangentGroupsOutput {
    pub fn adjacency(&self) -> &GpuTangentAdjacencyOutput {
        &self.adjacency
    }
    pub fn corner_count(&self) -> usize {
        self.adjacency.edge_count()
    }
    /// Read-only by contract; one GpuTangentGroup per original corner.
    pub fn buffer(&self) -> &wgpu::Buffer {
        &self.buffer
    }
}
