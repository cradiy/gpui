use super::{GpuDeformationLimits, GpuDeformationOutput, GpuDeformationVertex, validate_storage};
use crate::Mesh;
use anyhow::{Result, ensure};
use gpui_wgpu::{WgpuContext, WgpuResource, wgpu};

#[cfg(test)]
mod tests;

impl GpuDeformationOutput {
    /// Retains an application-produced buffer without copying or submitting GPU work.
    /// Requires exactly one `GpuDeformationVertex` per base vertex, in base vertex order,
    /// with STORAGE and COPY_SRC usage. The resource's creating device is checked before
    /// backend access; record contents are not read or validated by this constructor.
    /// Submit producers on this context's queue before adoption. Do not mutate, map, or
    /// destroy the buffer while this output or any queued consumer can still use it.
    pub fn from_buffer(
        context: WgpuContext,
        base: Mesh,
        buffer: WgpuResource<wgpu::Buffer>,
        limits: GpuDeformationLimits,
    ) -> Result<Self> {
        ensure!(!context.device_lost(), "GPU deformation device is lost");
        buffer.check_device(&context.device)?;
        admit(
            &context.device.limits(),
            base.vertex_count(),
            buffer.size(),
            buffer.usage(),
            wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            limits,
        )?;
        Ok(Self {
            context,
            base,
            buffer: buffer.raw().clone(),
        })
    }

    /// Copies an application buffer into a separately retained GPU result, without CPU readback.
    /// The source needs COPY_SRC usage and the same exact record layout as `from_buffer()`.
    /// Producers must already be submitted on this context's queue. After this call, later
    /// queue-ordered writes may reuse the source without changing the returned snapshot.
    /// Completion and record validity are checked by downstream consumers, not this copy.
    pub fn copy_from_buffer(
        context: WgpuContext,
        base: Mesh,
        source: &WgpuResource<wgpu::Buffer>,
        limits: GpuDeformationLimits,
    ) -> Result<Self> {
        ensure!(!context.device_lost(), "GPU deformation device is lost");
        source.check_device(&context.device)?;
        let bytes = admit(
            &context.device.limits(),
            base.vertex_count(),
            source.size(),
            source.usage(),
            wgpu::BufferUsages::COPY_SRC,
            limits,
        )?;
        let device = &context.device;
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gpui_3d.deformation.snapshot"),
            size: bytes,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::VERTEX
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("gpui_3d.deformation.snapshot"),
        });
        encoder.copy_buffer_to_buffer(source, 0, &buffer, 0, bytes);
        context.queue.submit(Some(encoder.finish()));
        if let Some(error) = gpui::block_on(scope.pop()) {
            anyhow::bail!("GPU deformation buffer snapshot: {error}");
        }
        ensure!(!context.device_lost(), "GPU deformation device is lost");
        Ok(Self {
            context,
            base,
            buffer,
        })
    }
}

fn admit(
    device: &wgpu::Limits,
    vertices: usize,
    bytes: u64,
    usage: wgpu::BufferUsages,
    required: wgpu::BufferUsages,
    limits: GpuDeformationLimits,
) -> Result<u64> {
    ensure!(
        vertices > 0 && vertices <= u32::MAX as usize,
        "GPU deformation vertex count exceeds u32"
    );
    let expected = vertices as u64 * std::mem::size_of::<GpuDeformationVertex>() as u64;
    ensure!(
        bytes == expected,
        "GPU deformation buffer needs exactly {expected} bytes, got {bytes}"
    );
    ensure!(
        usage.contains(required),
        "GPU deformation buffer requires {required:?} usage"
    );
    ensure!(
        bytes <= limits.max_source_bytes && bytes <= limits.max_output_bytes,
        "GPU deformation buffer exceeds payload budget"
    );
    validate_storage(device, &[bytes], vertices)?;
    Ok(bytes)
}
