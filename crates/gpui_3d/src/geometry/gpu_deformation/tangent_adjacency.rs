use super::{ComputeKernel, GpuDeformationLimits, GpuTangentWeldOutput, buffer, validate_storage};
use crate::Mesh;
use anyhow::{Result, ensure};
use bytemuck::{Pod, Zeroable};
use gpui_wgpu::{WgpuContext, wgpu};
use std::sync::Arc;

#[cfg(test)]
mod tests;

/// A 64-byte directed edge record in original triangle-corner order.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct GpuTangentEdge {
    /// Edge's starting corner, welded start, welded end, and opposite edge's
    /// starting corner (u32::MAX when unpaired).
    pub edge: [u32; 4],
    /// Neighbor corners matching start/end, regular-face orientation compatibility,
    /// and face eligibility (0 regular, 1 needs inheritance, 2 coincident positions,
    /// 3 failed). Missing neighbor corners are u32::MAX.
    pub adjacency: [u32; 4],
    /// Original face derivative classification, without selecting an inherited orientation.
    pub classification: [u32; 4],
    /// First failed corner in the face, otherwise the face derivative status.
    /// Nonzero status excludes every edge of this face from pairing.
    pub status: [u32; 4],
}

/// Per-source uniforms and per-evaluation sort/result payloads. Excludes retained
/// weld, derivative, and input vertex buffers, CPU objects, and driver overhead.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GpuTangentAdjacencyMemory {
    pub uniform_bytes: u64,
    pub scratch_bytes: u64,
    pub output_bytes: u64,
    pub padded_edges: u32,
    pub sort_passes: u32,
}
impl GpuTangentAdjacencyMemory {
    /// Source admission covers all pass uniforms; output admission covers both
    /// scratch buffers plus one final edge buffer.
    pub fn plan(corners: usize, limits: GpuDeformationLimits) -> Result<Self> {
        ensure!(
            corners > 0 && corners <= (1usize << 30) && corners.is_multiple_of(3),
            "GPU tangent adjacency requires a positive triangle-corner count no greater than 2^30"
        );
        let padded_edges = (corners as u32).next_power_of_two();
        let levels = padded_edges.ilog2();
        let sort_passes = levels * (levels + 1) / 2;
        let memory = Self {
            uniform_bytes: u64::from(sort_passes + 2) * 16,
            scratch_bytes: u64::from(padded_edges) * 128,
            output_bytes: corners as u64 * 64,
            padded_edges,
            sort_passes,
        };
        ensure!(
            memory.uniform_bytes <= limits.max_source_bytes,
            "GPU tangent adjacency uniforms exceed source payload budget"
        );
        ensure!(
            memory.scratch_bytes + memory.output_bytes <= limits.max_output_bytes,
            "GPU tangent adjacency scratch and output exceed payload budget"
        );
        Ok(memory)
    }
}

/// Pairs oppositely directed edges by welded endpoints before applying orientation
/// constraints. Edges within each direction are ranked by original corner; equal
/// ranks in opposite directions pair deterministically, including non-manifold edges.
/// This stage does not build connected groups.
pub struct GpuTangentAdjacency {
    context: WgpuContext,
    base: Mesh,
    uv_set: u32,
    memory: GpuTangentAdjacencyMemory,
    params: Vec<wgpu::Buffer>,
    initialize: ComputeKernel,
    sort: ComputeKernel,
    resolve: ComputeKernel,
}
impl GpuTangentAdjacency {
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
            "missing UV set {uv_set} for GPU tangent adjacency"
        );
        let memory = GpuTangentAdjacencyMemory::plan(base.index_count(), limits)?;
        validate_storage(
            &context.device.limits(),
            &[
                memory.scratch_bytes / 2,
                memory.output_bytes,
                base.triangle_count() as u64 * 64,
            ],
            memory.padded_edges as usize,
        )?;
        let device = &context.device;
        let shader = include_str!("tangent_adjacency.wgsl");
        let initialize = ComputeKernel::new(device, shader, "initialize", [64; 3])?;
        let sort = ComputeKernel::new(device, shader, "sort_pairs", [64; 3])?;
        let resolve = ComputeKernel::new(device, shader, "resolve", [64; 3])?;
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let params = super::tangent_weld::passes(base.index_count() as u32)
            .into_iter()
            .map(|params| {
                buffer(
                    device,
                    "gpui_3d.adjacency.params",
                    bytemuck::cast_slice(&params),
                    wgpu::BufferUsages::UNIFORM,
                )
            })
            .collect();
        if let Some(error) = gpui::block_on(scope.pop()) {
            anyhow::bail!("GPU tangent adjacency preparation: {error}");
        }
        Ok(Self {
            context,
            base,
            uv_set,
            memory,
            params,
            initialize,
            sort,
            resolve,
        })
    }
    pub fn memory(&self) -> GpuTangentAdjacencyMemory {
        self.memory
    }

    /// Uses face derivatives and corner matches from one retained input snapshot.
    /// Faces with coincident positions and failed faces remain in the output but
    /// are excluded from pairing. Distinct collinear positions and undefined UV
    /// frames retain neighbors for caller-owned inheritance.
    pub fn evaluate(&self, weld: &GpuTangentWeldOutput) -> Result<GpuTangentAdjacencyOutput> {
        let faces = weld.derivatives();
        ensure!(
            !self.context.device_lost(),
            "GPU deformation device is lost"
        );
        ensure!(
            Arc::ptr_eq(&self.context.device, &faces.context().device),
            "GPU tangent adjacency input belongs to a different device"
        );
        ensure!(
            Arc::ptr_eq(&self.base.0, &faces.base_mesh().0),
            "GPU tangent adjacency source mesh mismatch"
        );
        ensure!(
            self.uv_set == faces.uv_set(),
            "GPU tangent adjacency coordinate set mismatch"
        );
        let device = &self.context.device;
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let allocate = |size, label| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            })
        };
        let mut a = allocate(self.memory.scratch_bytes / 2, "gpui_3d.adjacency.sort_a");
        let mut b = allocate(self.memory.scratch_bytes / 2, "gpui_3d.adjacency.sort_b");
        let output = allocate(self.memory.output_bytes, "gpui_3d.adjacency.output");
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("gpui_3d.adjacency"),
        });
        self.initialize.encode(
            device,
            &mut encoder,
            self.memory.padded_edges,
            [
                weld.buffer(),
                faces.buffer(),
                weld.buffer(),
                &a,
                &self.params[0],
            ],
        );
        for params in &self.params[1..self.params.len() - 1] {
            self.sort.encode(
                device,
                &mut encoder,
                self.memory.padded_edges,
                [&a, faces.buffer(), weld.buffer(), &b, params],
            );
            std::mem::swap(&mut a, &mut b);
        }
        self.resolve.encode(
            device,
            &mut encoder,
            self.base.index_count() as u32,
            [
                &a,
                faces.buffer(),
                weld.buffer(),
                &output,
                self.params.last().unwrap(),
            ],
        );
        self.context.queue.submit(Some(encoder.finish()));
        if let Some(error) = gpui::block_on(scope.pop()) {
            anyhow::bail!("GPU tangent adjacency submission: {error}");
        }
        Ok(GpuTangentAdjacencyOutput {
            weld: weld.clone(),
            buffer: output,
        })
    }
}

/// Immutable edge relationships paired with their original corner and face data.
#[derive(Clone)]
pub struct GpuTangentAdjacencyOutput {
    weld: GpuTangentWeldOutput,
    buffer: wgpu::Buffer,
}
impl GpuTangentAdjacencyOutput {
    pub fn weld(&self) -> &GpuTangentWeldOutput {
        &self.weld
    }
    pub fn edge_count(&self) -> usize {
        self.weld.corner_count()
    }
    /// Read-only by contract; one GpuTangentEdge per original triangle corner.
    pub fn buffer(&self) -> &wgpu::Buffer {
        &self.buffer
    }
}
