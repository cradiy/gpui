use super::{
    ComputeKernel, GpuDeformationLimits, GpuTangentGroupsOutput, buffer, validate_storage,
};
use crate::Mesh;
use anyhow::{Result, ensure};
use bytemuck::{Pod, Zeroable};
use gpui_wgpu::{WgpuContext, wgpu};
use std::sync::Arc;

#[cfg(test)]
mod tests;

/// A 64-byte angle-weighted tangent frame in original triangle-corner order.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct GpuTangentFrame {
    /// Normalized normal-orthogonal direction and mean derivative magnitude.
    pub tangent: [f32; 4],
    /// Independently normalized bitangent and mean derivative magnitude.
    pub bitangent: [f32; 4],
    /// Original corner, regular group representative, and UV orientation (0 or 1).
    /// Nonregular corners use u32::MAX for group and orientation and have no frame.
    pub identity: [u32; 3],
    /// Sum of the accepted contributors' projected corner angles, in radians.
    pub angle_weight: f32,
    /// Preserved input status, or X = 1 for nonfinite arithmetic, X = 2 for an
    /// undefined regular frame. Nonregular corners retain their input status.
    pub status: [u32; 4],
}

/// Per-source uniforms and per-evaluation sorting/result payloads. Existing input
/// snapshots, CPU allocations, pipeline storage and driver overhead are excluded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GpuTangentFramesMemory {
    pub uniform_bytes: u64,
    pub scratch_bytes: u64,
    pub output_bytes: u64,
    pub padded_corners: u32,
    pub sort_passes: u32,
}
impl GpuTangentFramesMemory {
    /// Source admission covers pass uniforms. Output admission covers both sort
    /// buffers and the returned corner-frame buffer.
    pub fn plan(corners: usize, limits: GpuDeformationLimits) -> Result<Self> {
        ensure!(
            corners > 0 && corners <= (1usize << 30) && corners.is_multiple_of(3),
            "GPU tangent frames require a positive triangle-corner count no greater than 2^30"
        );
        let padded_corners = (corners as u32).next_power_of_two();
        let levels = padded_corners.ilog2();
        let sort_passes = levels * (levels + 1) / 2;
        let memory = Self {
            uniform_bytes: u64::from(sort_passes + 2) * 16,
            scratch_bytes: u64::from(padded_corners) * 128,
            output_bytes: corners as u64 * 64,
            padded_corners,
            sort_passes,
        };
        ensure!(
            memory.uniform_bytes <= limits.max_source_bytes,
            "GPU tangent frame uniforms exceed source payload budget"
        );
        ensure!(
            memory.scratch_bytes + memory.output_bytes <= limits.max_output_bytes,
            "GPU tangent frame scratch and output exceed payload budget"
        );
        Ok(memory)
    }
}

/// Angle-weighted regular corner frames. Contributions are ordered by original
/// corner within each connected group; exactly opposing projected tangent or
/// bitangent directions are excluded per destination corner. No float atomics are used.
/// Degenerate-frame inheritance, repair, and vertex publication are separate stages.
pub struct GpuTangentFrames {
    context: WgpuContext,
    base: Mesh,
    uv_set: u32,
    memory: GpuTangentFramesMemory,
    params: Vec<wgpu::Buffer>,
    initialize: ComputeKernel,
    sort: ComputeKernel,
    accumulate: ComputeKernel,
}
impl GpuTangentFrames {
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
            "missing UV set {uv_set} for GPU tangent frames"
        );
        let memory = GpuTangentFramesMemory::plan(base.index_count(), limits)?;
        validate_storage(
            &context.device.limits(),
            &[memory.scratch_bytes / 2, memory.output_bytes],
            memory.padded_corners as usize,
        )?;
        let device = &context.device;
        let shader = include_str!("tangent_frames.wgsl");
        let initialize = ComputeKernel::new(device, shader, "initialize", [64; 3])?;
        let sort = ComputeKernel::new(device, shader, "sort_pairs", [64; 3])?;
        let accumulate = ComputeKernel::new(device, shader, "accumulate", [64; 3])?;
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let params = super::tangent_weld::passes(base.index_count() as u32)
            .into_iter()
            .map(|params| {
                buffer(
                    device,
                    "gpui_3d.frames.params",
                    bytemuck::cast_slice(&params),
                    wgpu::BufferUsages::UNIFORM,
                )
            })
            .collect();
        if let Some(error) = gpui::block_on(scope.pop()) {
            anyhow::bail!("GPU tangent frame preparation: {error}");
        }
        Ok(Self {
            context,
            base,
            uv_set,
            memory,
            params,
            initialize,
            sort,
            accumulate,
        })
    }
    pub fn memory(&self) -> GpuTangentFramesMemory {
        self.memory
    }

    /// Retains groups, derivatives and deformed inputs from one evaluation.
    /// CPU meshes, bounds and queries are unchanged. Each regular corner scans its
    /// sorted group; a group of k corners requires O(k^2) contribution comparisons.
    pub fn evaluate(&self, groups: &GpuTangentGroupsOutput) -> Result<GpuTangentFramesOutput> {
        let weld = groups.adjacency().weld();
        let faces = weld.derivatives();
        ensure!(
            !self.context.device_lost(),
            "GPU deformation device is lost"
        );
        ensure!(
            Arc::ptr_eq(&self.context.device, &faces.context().device),
            "GPU tangent frames input belongs to a different device"
        );
        ensure!(
            Arc::ptr_eq(&self.base.0, &faces.base_mesh().0),
            "GPU tangent frames source mesh mismatch"
        );
        ensure!(
            self.uv_set == faces.uv_set(),
            "GPU tangent frames coordinate set mismatch"
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
        let mut a = allocate(self.memory.scratch_bytes / 2, "gpui_3d.frames.sort_a");
        let mut b = allocate(self.memory.scratch_bytes / 2, "gpui_3d.frames.sort_b");
        let output = allocate(self.memory.output_bytes, "gpui_3d.frames.output");
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("gpui_3d.frames"),
        });
        self.initialize.encode(
            device,
            &mut encoder,
            self.memory.padded_corners,
            [
                faces.buffer(),
                groups.buffer(),
                weld.buffer(),
                &a,
                &self.params[0],
            ],
        );
        for params in &self.params[1..self.params.len() - 1] {
            self.sort.encode(
                device,
                &mut encoder,
                self.memory.padded_corners,
                [&a, groups.buffer(), weld.buffer(), &b, params],
            );
            std::mem::swap(&mut a, &mut b);
        }
        self.accumulate.encode(
            device,
            &mut encoder,
            self.base.index_count() as u32,
            [
                &a,
                groups.buffer(),
                weld.buffer(),
                &output,
                self.params.last().unwrap(),
            ],
        );
        self.context.queue.submit(Some(encoder.finish()));
        if let Some(error) = gpui::block_on(scope.pop()) {
            anyhow::bail!("GPU tangent frame submission: {error}");
        }
        Ok(GpuTangentFramesOutput {
            groups: groups.clone(),
            buffer: output,
        })
    }
}

/// Immutable corner frames paired with their connected groups and deformation input.
#[derive(Clone)]
pub struct GpuTangentFramesOutput {
    groups: GpuTangentGroupsOutput,
    buffer: wgpu::Buffer,
}
impl GpuTangentFramesOutput {
    pub fn groups(&self) -> &GpuTangentGroupsOutput {
        &self.groups
    }
    pub fn corner_count(&self) -> usize {
        self.groups.corner_count()
    }
    /// Read-only by contract; one GpuTangentFrame per original triangle corner.
    pub fn buffer(&self) -> &wgpu::Buffer {
        &self.buffer
    }
}
