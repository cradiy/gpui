use super::{ComputeKernel, GpuDeformationLimits, GpuDeformationOutput, buffer, validate_storage};
use crate::Mesh;
use anyhow::{Result, ensure};
use bytemuck::{Pod, Zeroable};
use gpui_wgpu::{WgpuContext, wgpu};
use std::sync::Arc;

#[cfg(test)]
mod tests;

/// One 64-byte face record, in source triangle order. These are unprojected
/// surface derivatives, not smoothed or normal-orthogonal vertex tangents.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct GpuTangentDerivative {
    /// XYZ: unit dP/du direction. W: dP/du magnitude.
    pub tangent: [f32; 4],
    /// XYZ: unit dP/dv direction. W: dP/dv magnitude.
    pub bitangent: [f32; 4],
    /// Boolean lanes: zero geometric area, zero UV determinant, positive UV
    /// orientation, and undefined derivative pair. Undefined pairs have zero vectors.
    /// Regular pairs require absolute UV determinant and both derivative magnitudes
    /// strictly above f32::MIN_POSITIVE.
    pub classification: [u32; 4],
    /// First failing input corner's status, or [1, 0, 0, 0] for detected nonfinite
    /// arithmetic. Inspect this before using other fields. Zero means no detected error;
    /// degeneracy is classified separately, not repaired or discarded.
    pub status: [u32; 4],
}

/// Per-source and per-result payloads, excluding the input and driver overhead.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GpuTangentDerivativesMemory {
    pub uv_bytes: u64,
    pub index_bytes: u64,
    pub uniform_bytes: u64,
    pub output_bytes: u64,
}

impl GpuTangentDerivativesMemory {
    pub fn plan(vertices: usize, indices: usize, limits: GpuDeformationLimits) -> Result<Self> {
        ensure!(
            vertices > 0
                && vertices <= u32::MAX as usize
                && indices > 0
                && indices <= u32::MAX as usize
                && indices.is_multiple_of(3),
            "GPU tangent derivatives require positive u32 vertex and triangle index counts"
        );
        let memory = Self {
            uv_bytes: vertices as u64 * 8,
            index_bytes: indices as u64 * 4,
            uniform_bytes: 16,
            output_bytes: (indices / 3) as u64 * 64,
        };
        ensure!(
            memory.uv_bytes + memory.index_bytes + memory.uniform_bytes <= limits.max_source_bytes,
            "GPU tangent derivative inputs exceed source payload budget"
        );
        ensure!(
            memory.output_bytes <= limits.max_output_bytes,
            "GPU tangent derivatives exceed output payload budget"
        );
        Ok(memory)
    }
}

/// Retained triangle indices and one UV set for derivative evaluation after deformation.
/// Shared vertices and arbitrary triangle order are supported. This stage performs no
/// welding, corner grouping, smoothing, normal projection, or degenerate-frame repair.
pub struct GpuTangentDerivatives {
    context: WgpuContext,
    base: Mesh,
    uv_set: u32,
    memory: GpuTangentDerivativesMemory,
    uv: wgpu::Buffer,
    indices: wgpu::Buffer,
    params: wgpu::Buffer,
    kernel: ComputeKernel,
}

impl GpuTangentDerivatives {
    /// Checks compute requirements without creating GPU resources.
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
        let memory =
            GpuTangentDerivativesMemory::plan(base.vertex_count(), base.index_count(), limits)?;
        validate_storage(
            &context.device.limits(),
            &[
                base.vertex_count() as u64 * 64,
                memory.uv_bytes,
                memory.index_bytes,
                memory.output_bytes,
            ],
            base.triangle_count(),
        )?;
        let uv = coordinates(&base, uv_set)?;
        let device = &context.device;
        let kernel = ComputeKernel::new(
            device,
            include_str!("tangent_derivatives.wgsl"),
            "tangent_derivatives",
            [64, 8, 4],
        )?;
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let uv = buffer(
            device,
            "gpui_3d.tangent.uv",
            bytemuck::cast_slice(&uv),
            wgpu::BufferUsages::STORAGE,
        );
        let indices = buffer(
            device,
            "gpui_3d.tangent.indices",
            bytemuck::cast_slice(base.indices()),
            wgpu::BufferUsages::STORAGE,
        );
        let params = buffer(
            device,
            "gpui_3d.tangent.params",
            bytemuck::cast_slice(&[base.triangle_count() as u32, 0, 0, 0]),
            wgpu::BufferUsages::UNIFORM,
        );
        if let Some(error) = gpui::block_on(scope.pop()) {
            anyhow::bail!("GPU tangent derivative preparation: {error}");
        }
        Ok(Self {
            context,
            base,
            uv_set,
            memory,
            uv,
            indices,
            params,
            kernel,
        })
    }

    pub fn memory(&self) -> GpuTangentDerivativesMemory {
        self.memory
    }

    /// Submits an immutable face buffer using positions from the same source mesh
    /// allocation and device. Does not read vertices or alter the input's tangent data.
    pub fn evaluate(&self, input: &GpuDeformationOutput) -> Result<GpuTangentDerivativeOutput> {
        ensure!(
            Arc::ptr_eq(&self.context.device, &input.context.device),
            "GPU tangent derivative input belongs to a different device"
        );
        ensure!(
            Arc::ptr_eq(&self.base.0, &input.base.0),
            "GPU tangent derivative source mesh mismatch"
        );
        let buffer = self.kernel.evaluate_records(
            &self.context,
            self.base.triangle_count(),
            [&input.buffer, &self.uv, &self.indices],
            &self.params,
        )?;
        Ok(GpuTangentDerivativeOutput {
            context: self.context.clone(),
            base: self.base.clone(),
            uv_set: self.uv_set,
            input: input.buffer.clone(),
            buffer,
        })
    }
}

/// Immutable intermediate records. Retains the input vertex buffer and device; remains
/// valid after later evaluations or source destruction. Not a renderable vertex buffer.
#[derive(Clone)]
pub struct GpuTangentDerivativeOutput {
    context: WgpuContext,
    base: Mesh,
    uv_set: u32,
    input: wgpu::Buffer,
    buffer: wgpu::Buffer,
}

impl GpuTangentDerivativeOutput {
    pub fn context(&self) -> &WgpuContext {
        &self.context
    }
    pub fn base_mesh(&self) -> &Mesh {
        &self.base
    }
    pub fn uv_set(&self) -> u32 {
        self.uv_set
    }
    pub fn triangle_count(&self) -> usize {
        self.base.triangle_count()
    }
    /// Read-only input snapshot paired with these derivatives, in GpuDeformationVertex
    /// layout. Its retention consumes input bytes in addition to the face output.
    pub fn input_buffer(&self) -> &wgpu::Buffer {
        &self.input
    }
    /// Read-only by contract. STORAGE and COPY_SRC usage; one GpuTangentDerivative
    /// per source triangle. Consumers must inspect status and classification.
    pub fn buffer(&self) -> &wgpu::Buffer {
        &self.buffer
    }
}

pub(super) fn coordinates(mesh: &Mesh, set: u32) -> Result<Vec<[f32; 2]>> {
    (0..mesh.vertex_count())
        .map(|index| {
            mesh.uv_at(set, index)
                .ok_or_else(|| anyhow::anyhow!("missing UV set {set} for GPU tangent derivatives"))
        })
        .collect()
}
