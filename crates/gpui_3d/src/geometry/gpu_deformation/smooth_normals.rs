use super::{ComputeKernel, GpuDeformationLimits, GpuDeformationOutput, buffer, validate_storage};
use crate::Mesh;
use anyhow::{Result, ensure};
use gpui_wgpu::{Scene3dDeviceCapabilities, WgpuContext, wgpu};
use std::sync::Arc;

const SHADER: &str = concat!(
    include_str!("precision.wgsl"),
    "\n",
    include_str!("smooth_normals.wgsl")
);

/// Retained adjacency/index/uniform buffers and one independent normal output.
/// Excludes input vertices, CPU data, pipelines, driver overhead and readbacks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GpuSmoothNormalsMemory {
    pub adjacency_bytes: u64,
    pub index_bytes: u64,
    pub uniform_bytes: u64,
    pub output_bytes: u64,
}

impl GpuSmoothNormalsMemory {
    /// Checks counts and payload admission without a GPU device or allocation.
    pub fn plan(vertices: usize, indices: usize, limits: GpuDeformationLimits) -> Result<Self> {
        ensure!(
            vertices > 0 && vertices < u32::MAX as usize,
            "GPU smooth normal vertex count exceeds u32 indexing"
        );
        ensure!(
            indices > 0 && indices <= u32::MAX as usize && indices.is_multiple_of(3),
            "GPU smooth normals require a u32 triangle index list"
        );
        let adjacency_words = vertices as u64 + 1 + indices as u64;
        ensure!(
            adjacency_words <= u64::from(u32::MAX),
            "GPU smooth normal adjacency exceeds u32 indexing"
        );
        let memory = Self {
            adjacency_bytes: adjacency_words * 4,
            index_bytes: indices as u64 * 4,
            uniform_bytes: 16,
            output_bytes: vertices as u64 * 64,
        };
        ensure!(
            memory.adjacency_bytes + memory.index_bytes + memory.uniform_bytes
                <= limits.max_source_bytes,
            "GPU smooth normal topology exceeds source payload budget"
        );
        ensure!(
            memory.output_bytes <= limits.max_output_bytes,
            "GPU smooth normal output exceeds payload budget"
        );
        Ok(memory)
    }
}

/// Rebuilds area-weighted normals at shared source indices without changing topology.
/// Coincident vertices are not welded. Requires enabled `SHADER_F64`; source tangents
/// must be absent. Unreferenced vertices retain their input records.
pub struct GpuSmoothNormals {
    context: WgpuContext,
    base: Mesh,
    memory: GpuSmoothNormalsMemory,
    adjacency: wgpu::Buffer,
    indices: wgpu::Buffer,
    params: wgpu::Buffer,
    kernel: ComputeKernel,
}

impl GpuSmoothNormals {
    /// Checks enabled compute capabilities without allocating GPU resources.
    pub fn check_support(capabilities: &Scene3dDeviceCapabilities) -> Result<()> {
        ensure!(
            capabilities
                .enabled_features
                .contains(wgpu::Features::SHADER_F64),
            "GPU smooth normals require enabled SHADER_F64"
        );
        super::support::validate(capabilities, 4, 1, 0)
    }

    pub fn new(context: WgpuContext, base: Mesh, limits: GpuDeformationLimits) -> Result<Self> {
        ensure!(!context.device_lost(), "GPU deformation device is lost");
        ensure!(
            base.tangents().is_none(),
            "GPU smooth normals require a source without tangents"
        );
        Self::check_support(&Scene3dDeviceCapabilities::query(&context))?;
        let memory = GpuSmoothNormalsMemory::plan(base.vertex_count(), base.index_count(), limits)?;
        validate_storage(
            &context.device.limits(),
            &[
                memory.adjacency_bytes,
                memory.index_bytes,
                memory.output_bytes,
            ],
            base.vertex_count(),
        )?;
        let adjacency = adjacency(&base);
        let device = &context.device;
        let kernel = ComputeKernel::new(device, SHADER, "smooth_normals", [64, 4, 4])?;
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let adjacency = buffer(
            device,
            "gpui_3d.smooth_normals.adjacency",
            bytemuck::cast_slice(&adjacency),
            wgpu::BufferUsages::STORAGE,
        );
        let indices = buffer(
            device,
            "gpui_3d.smooth_normals.indices",
            bytemuck::cast_slice(base.indices()),
            wgpu::BufferUsages::STORAGE,
        );
        let params = buffer(
            device,
            "gpui_3d.smooth_normals.params",
            bytemuck::cast_slice(&[base.vertex_count() as u32, 0, 0, 0]),
            wgpu::BufferUsages::UNIFORM,
        );
        if let Some(error) = gpui::block_on(scope.pop()) {
            anyhow::bail!("GPU smooth normal preparation: {error}");
        }
        Ok(Self {
            context,
            base,
            memory,
            adjacency,
            indices,
            params,
            kernel,
        })
    }

    pub fn memory(&self) -> GpuSmoothNormalsMemory {
        self.memory
    }

    /// Returns an independent result for the same device and base mesh allocation.
    /// Incident face cross products accumulate in index order using f64 arithmetic.
    /// Invalid input statuses propagate; nonfinite positions, zero-area faces and
    /// cancelling normals produce failure statuses rather than skipped contributions.
    /// CPU geometry, bounds, queries and previously returned results are unchanged.
    pub fn evaluate(&self, input: &GpuDeformationOutput) -> Result<GpuDeformationOutput> {
        ensure!(
            Arc::ptr_eq(&self.context.device, &input.context.device),
            "GPU smooth normal input belongs to a different device"
        );
        ensure!(
            self.base.ptr_eq(input.base_mesh()),
            "GPU smooth normal source mesh mismatch"
        );
        self.kernel.evaluate(
            &self.context,
            self.base.clone(),
            [&input.buffer, &self.adjacency, &self.indices],
            &self.params,
        )
    }
}

// CSR rows contain index-buffer face offsets, in original triangle order.
fn adjacency(mesh: &Mesh) -> Vec<u32> {
    let vertices = mesh.vertex_count();
    let mut data = vec![0; vertices + 1 + mesh.index_count()];
    for &index in mesh.indices() {
        data[index as usize + 1] += 1;
    }
    for vertex in 0..vertices {
        data[vertex + 1] += data[vertex];
    }
    let mut cursor = data[..vertices].to_vec();
    for (corner, &index) in mesh.indices().iter().enumerate() {
        data[vertices + 1 + cursor[index as usize] as usize] = (corner / 3 * 3) as u32;
        cursor[index as usize] += 1;
    }
    data
}

#[cfg(test)]
mod tests;
