use super::{ComputeKernel, GpuDeformationLimits, GpuDeformationOutput, buffer, validate_storage};
use crate::Mesh;
use anyhow::{Result, ensure};
use gpui_wgpu::{WgpuContext, wgpu};
use std::sync::Arc;

const SHADER: &str = concat!(
    include_str!("precision.wgsl"),
    "\n",
    include_str!("flat_normals.wgsl")
);

#[cfg(test)]
mod tests;

/// Payload sizes for a reusable corner topology and one normal-rebuilt output.
/// Excludes the input deformation buffer, CPU topology, and driver overhead.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GpuFlatNormalsMemory {
    pub face_offsets_bytes: u64,
    pub index_bytes: u64,
    pub uniform_bytes: u64,
    pub output_bytes: u64,
}

impl GpuFlatNormalsMemory {
    pub fn plan(vertices: usize, limits: GpuDeformationLimits) -> Result<Self> {
        ensure!(
            vertices > 0 && vertices <= u32::MAX as usize && vertices.is_multiple_of(3),
            "GPU flat normals require a positive u32 triangle-corner count"
        );
        let memory = Self {
            face_offsets_bytes: vertices as u64 * 4,
            index_bytes: vertices as u64 * 4,
            uniform_bytes: 16,
            output_bytes: vertices as u64 * 64,
        };
        ensure!(
            memory.face_offsets_bytes + memory.index_bytes + memory.uniform_bytes
                <= limits.max_source_bytes,
            "GPU flat normal topology exceeds source payload budget"
        );
        ensure!(
            memory.output_bytes <= limits.max_output_bytes,
            "GPU flat normal output exceeds payload budget"
        );
        Ok(memory)
    }
}

/// Rebuilds face normals from deformed triangle positions without changing topology.
/// Each source vertex must occur exactly once in the index buffer. Split shared vertices
/// and remap Morph/Skin inputs before creating the source. Tangents must be absent;
/// generate them from the rebuilt normals before rendering a normal-mapped material.
/// Requires enabled `SHADER_F64` for position differences and normal reconstruction.
pub struct GpuFlatNormals {
    context: WgpuContext,
    base: Mesh,
    memory: GpuFlatNormalsMemory,
    offsets: wgpu::Buffer,
    indices: wgpu::Buffer,
    params: wgpu::Buffer,
    kernel: ComputeKernel,
}

impl GpuFlatNormals {
    /// Checks enabled compute capabilities without allocating resources or submitting work.
    pub fn check_support(capabilities: &gpui_wgpu::Scene3dDeviceCapabilities) -> Result<()> {
        ensure!(
            capabilities
                .enabled_features
                .contains(wgpu::Features::SHADER_F64),
            "GPU flat normals require enabled SHADER_F64"
        );
        super::support::validate(capabilities, 4, 1, 0)
    }

    pub fn new(context: WgpuContext, base: Mesh, limits: GpuDeformationLimits) -> Result<Self> {
        ensure!(!context.device_lost(), "GPU deformation device is lost");
        Self::check_support(&gpui_wgpu::Scene3dDeviceCapabilities::query(&context))?;
        let memory = GpuFlatNormalsMemory::plan(base.vertex_count(), limits)?;
        validate_storage(
            &context.device.limits(),
            &[
                memory.face_offsets_bytes,
                memory.index_bytes,
                memory.output_bytes,
            ],
            base.vertex_count(),
        )?;
        let offsets = face_offsets(&base)?;
        let device = &context.device;
        let kernel = ComputeKernel::new(device, SHADER, "flat_normals", [64, 4, 4])?;
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let offsets = buffer(
            device,
            "gpui_3d.normals.faces",
            bytemuck::cast_slice(&offsets),
            wgpu::BufferUsages::STORAGE,
        );
        let indices = buffer(
            device,
            "gpui_3d.normals.indices",
            bytemuck::cast_slice(base.indices()),
            wgpu::BufferUsages::STORAGE,
        );
        let params = buffer(
            device,
            "gpui_3d.normals.params",
            bytemuck::cast_slice(&[base.vertex_count() as u32, 0, 0, 0]),
            wgpu::BufferUsages::UNIFORM,
        );
        if let Some(error) = gpui::block_on(scope.pop()) {
            anyhow::bail!("GPU flat normal preparation: {error}");
        }
        Ok(Self {
            context,
            base,
            memory,
            offsets,
            indices,
            params,
            kernel,
        })
    }

    pub fn memory(&self) -> GpuFlatNormalsMemory {
        self.memory
    }

    /// Returns an independent output on the same device and source mesh allocation.
    /// Existing vertex failures are propagated. Zero-area triangles and nonfinite
    /// positions are reported through vertex status, readback, and draw suppression.
    /// CPU bounds, queries, and previously returned outputs remain unchanged.
    pub fn evaluate(&self, input: &GpuDeformationOutput) -> Result<GpuDeformationOutput> {
        ensure!(
            Arc::ptr_eq(&self.context.device, &input.context.device),
            "GPU flat normal input belongs to a different device"
        );
        ensure!(
            Arc::ptr_eq(&self.base.0, &input.base.0),
            "GPU flat normal source mesh mismatch"
        );
        self.kernel.evaluate(
            &self.context,
            self.base.clone(),
            [&input.buffer, &self.offsets, &self.indices],
            &self.params,
        )
    }
}

fn face_offsets(mesh: &Mesh) -> Result<Vec<u32>> {
    ensure!(
        mesh.tangents().is_none(),
        "GPU flat normals require a source without tangents"
    );
    ensure!(
        mesh.index_count() == mesh.vertex_count(),
        "GPU flat normals require one index per source vertex"
    );
    let mut offsets = vec![u32::MAX; mesh.vertex_count()];
    for (offset, &index) in mesh.indices().iter().enumerate() {
        let entry = &mut offsets[index as usize];
        ensure!(
            *entry == u32::MAX,
            "GPU flat normals require unshared triangle corners; vertex {index} is repeated"
        );
        *entry = (offset / 3 * 3) as u32;
    }
    Ok(offsets)
}
