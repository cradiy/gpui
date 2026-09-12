use super::{
    ComputeKernel, GpuDeformationLimits, GpuTangentDerivativeOutput, buffer, validate_storage,
};
use crate::Mesh;
use anyhow::{Result, ensure};
use bytemuck::{Pod, Zeroable};
use gpui_wgpu::{WgpuContext, wgpu};
use std::sync::Arc;

#[cfg(test)]
mod tests;

const SHADER: &str = concat!(
    include_str!("precision.wgsl"),
    include_str!("tangent_weld.wgsl")
);

/// A 64-byte record in original triangle-corner order. Matching does not change
/// mesh topology or combine orientation groups.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct GpuTangentWeldRecord {
    /// Exact f32 bits: position XYZ, normalized normal XYZ, and selected UV XY.
    /// Signed zero is preserved; no positional tolerance is applied.
    pub key: [u32; 8],
    /// Original corner, source vertex, earliest matching corner, and failure flag.
    /// Failed corners represent themselves and are never welded to other corners.
    pub identity: [u32; 4],
    /// Source vertex status, [1, 0, 0, 0] for nonfinite arithmetic, or
    /// [2, 0, 0, 0] for a zero normal. Keys are unspecified when status is nonzero.
    pub status: [u32; 4],
}

/// Per-source and per-evaluation payload admission; excludes the retained input
/// and derivative buffers, CPU data, and driver overhead.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GpuTangentWeldMemory {
    pub uv_bytes: u64,
    pub index_bytes: u64,
    pub uniform_bytes: u64,
    /// Total for both ping-pong sort buffers.
    pub scratch_bytes: u64,
    pub output_bytes: u64,
    pub padded_corners: u32,
    pub sort_passes: u32,
}

impl GpuTangentWeldMemory {
    /// The source budget covers UVs, indices, and pass parameters. The output
    /// budget covers both scratch buffers plus one result, before allocation.
    pub fn plan(vertices: usize, indices: usize, limits: GpuDeformationLimits) -> Result<Self> {
        ensure!(
            vertices > 0
                && vertices <= u32::MAX as usize
                && indices > 0
                && indices <= (1usize << 30)
                && indices.is_multiple_of(3),
            "GPU tangent welding requires positive u32 vertices and at most 2^30 triangle corners"
        );
        let padded_corners = (indices as u32).next_power_of_two();
        let levels = padded_corners.ilog2();
        let sort_passes = levels * (levels + 1) / 2;
        let memory = Self {
            uv_bytes: vertices as u64 * 8,
            index_bytes: indices as u64 * 4,
            uniform_bytes: u64::from(sort_passes + 2) * 16,
            scratch_bytes: u64::from(padded_corners) * 128,
            output_bytes: indices as u64 * 64,
            padded_corners,
            sort_passes,
        };
        ensure!(
            memory.uv_bytes + memory.index_bytes + memory.uniform_bytes <= limits.max_source_bytes,
            "GPU tangent weld inputs exceed source payload budget"
        );
        ensure!(
            memory.scratch_bytes + memory.output_bytes <= limits.max_output_bytes,
            "GPU tangent weld scratch and output exceed payload budget"
        );
        Ok(memory)
    }
}

/// Dynamic exact-key welding for tangent processing. Uses deterministic bitonic
/// sorting with two scratch buffers; performs no CPU readback or all-pairs search.
/// Requires enabled SHADER_F64 for CPU-precision normalized normal keys.
pub struct GpuTangentWeld {
    context: WgpuContext,
    base: Mesh,
    uv_set: u32,
    memory: GpuTangentWeldMemory,
    uv: wgpu::Buffer,
    indices: wgpu::Buffer,
    params: Vec<wgpu::Buffer>,
    initialize: ComputeKernel,
    sort: ComputeKernel,
    resolve: ComputeKernel,
}

impl GpuTangentWeld {
    pub fn check_support(capabilities: &gpui_wgpu::Scene3dDeviceCapabilities) -> Result<()> {
        ensure!(
            capabilities
                .enabled_features
                .contains(wgpu::Features::SHADER_F64),
            "GPU tangent welding requires enabled SHADER_F64; adapter support: {}",
            capabilities
                .adapter_features
                .contains(wgpu::Features::SHADER_F64)
        );
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
        let memory = GpuTangentWeldMemory::plan(base.vertex_count(), base.index_count(), limits)?;
        validate_storage(
            &context.device.limits(),
            &[
                base.vertex_count() as u64 * 64,
                memory.uv_bytes,
                memory.index_bytes,
                memory.scratch_bytes / 2,
                memory.output_bytes,
            ],
            memory.padded_corners as usize,
        )?;
        let uv = super::tangent_derivatives::coordinates(&base, uv_set)?;
        let device = &context.device;
        let shader = SHADER;
        let initialize = ComputeKernel::new(device, shader, "initialize", [64, 8, 4])?;
        let sort = ComputeKernel::new(device, shader, "sort_pairs", [64, 8, 4])?;
        let resolve = ComputeKernel::new(device, shader, "resolve", [64, 8, 4])?;
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let uv = buffer(
            device,
            "gpui_3d.weld.uv",
            bytemuck::cast_slice(&uv),
            wgpu::BufferUsages::STORAGE,
        );
        let indices = buffer(
            device,
            "gpui_3d.weld.indices",
            bytemuck::cast_slice(base.indices()),
            wgpu::BufferUsages::STORAGE,
        );
        let params = passes(base.index_count() as u32)
            .into_iter()
            .map(|params| {
                buffer(
                    device,
                    "gpui_3d.weld.params",
                    bytemuck::cast_slice(&params),
                    wgpu::BufferUsages::UNIFORM,
                )
            })
            .collect();
        if let Some(error) = gpui::block_on(scope.pop()) {
            anyhow::bail!("GPU tangent weld preparation: {error}");
        }
        Ok(Self {
            context,
            base,
            uv_set,
            memory,
            uv,
            indices,
            params,
            initialize,
            sort,
            resolve,
        })
    }

    pub fn memory(&self) -> GpuTangentWeldMemory {
        self.memory
    }

    /// Welds the exact vertex snapshot paired with the face derivatives. Requires
    /// matching device, base mesh allocation, and UV set. Output is immutable and
    /// uses original corner order with earliest-corner representatives.
    pub fn evaluate(
        &self,
        derivatives: &GpuTangentDerivativeOutput,
    ) -> Result<GpuTangentWeldOutput> {
        ensure!(
            !self.context.device_lost(),
            "GPU deformation device is lost"
        );
        ensure!(
            Arc::ptr_eq(&self.context.device, &derivatives.context().device),
            "GPU tangent weld input belongs to a different device"
        );
        ensure!(
            Arc::ptr_eq(&self.base.0, &derivatives.base_mesh().0),
            "GPU tangent weld source mesh mismatch"
        );
        ensure!(
            self.uv_set == derivatives.uv_set(),
            "GPU tangent weld coordinate set mismatch"
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
        let mut a = allocate(self.memory.scratch_bytes / 2, "gpui_3d.weld.sort_a");
        let mut b = allocate(self.memory.scratch_bytes / 2, "gpui_3d.weld.sort_b");
        let output = allocate(self.memory.output_bytes, "gpui_3d.weld.output");
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("gpui_3d.weld"),
        });
        self.initialize.encode(
            device,
            &mut encoder,
            self.memory.padded_corners,
            [
                derivatives.input_buffer(),
                &self.uv,
                &self.indices,
                &a,
                &self.params[0],
            ],
        );
        for params in &self.params[1..self.params.len() - 1] {
            self.sort.encode(
                device,
                &mut encoder,
                self.memory.padded_corners,
                [&a, &self.uv, &self.indices, &b, params],
            );
            std::mem::swap(&mut a, &mut b);
        }
        self.resolve.encode(
            device,
            &mut encoder,
            self.base.index_count() as u32,
            [
                &a,
                &self.uv,
                &self.indices,
                &output,
                self.params.last().unwrap(),
            ],
        );
        self.context.queue.submit(Some(encoder.finish()));
        if let Some(error) = gpui::block_on(scope.pop()) {
            anyhow::bail!("GPU tangent weld submission: {error}");
        }
        Ok(GpuTangentWeldOutput {
            derivatives: derivatives.clone(),
            buffer: output,
        })
    }
}

/// Original-corner records paired with retained derivatives and input vertices.
/// This is not a final tangent buffer or a topology replacement.
#[derive(Clone)]
pub struct GpuTangentWeldOutput {
    derivatives: GpuTangentDerivativeOutput,
    buffer: wgpu::Buffer,
}
impl GpuTangentWeldOutput {
    pub fn derivatives(&self) -> &GpuTangentDerivativeOutput {
        &self.derivatives
    }
    pub fn corner_count(&self) -> usize {
        self.derivatives.base_mesh().index_count()
    }
    /// Read-only by contract; one GpuTangentWeldRecord per original corner.
    pub fn buffer(&self) -> &wgpu::Buffer {
        &self.buffer
    }
}

pub(super) fn passes(corners: u32) -> Vec<[u32; 4]> {
    let capacity = corners.next_power_of_two();
    let mut params = vec![[corners, 0, 0, capacity]];
    let mut width = 2;
    while width <= capacity {
        let mut distance = width / 2;
        while distance > 0 {
            params.push([corners, distance, width, capacity]);
            distance /= 2;
        }
        width *= 2;
    }
    params.push([corners, 0, 0, capacity]);
    params
}
