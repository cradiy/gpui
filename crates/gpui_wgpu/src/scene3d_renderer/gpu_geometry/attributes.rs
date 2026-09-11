use super::*;

#[cfg(test)]
mod tests;

/// A complete replacement of one selected coordinate set or the linear vertex colors.
/// Every stream has exactly one value per base-mesh vertex, including unused vertices.
#[derive(Clone, Copy, Debug)]
pub enum Scene3dVertexUpdate<'a> {
    /// Finite coordinates for a set selected by `WgpuScene3dGeometry::uv_sets`.
    Uv {
        set: u32,
        coordinates: &'a [[f32; 2]],
    },
    /// Normalized linear, straight-alpha RGBA multipliers. White removes modulation.
    Color(&'a [[f32; 4]]),
    /// Copies packed Float32x2 records from a device-local COPY_SRC buffer.
    UvBuffer {
        set: u32,
        buffer: &'a crate::WgpuResource<wgpu::Buffer>,
    },
    /// Copies packed Float32x4 linear, straight-alpha RGBA records from COPY_SRC storage.
    ColorBuffer(&'a crate::WgpuResource<wgpu::Buffer>),
}

impl Scene3dVertexUpdate<'_> {
    fn buffer(&self) -> Option<&crate::WgpuResource<wgpu::Buffer>> {
        match self {
            Self::UvBuffer { buffer, .. } | Self::ColorBuffer(buffer) => Some(buffer),
            _ => None,
        }
    }

    fn bytes(&self) -> Option<&[u8]> {
        match self {
            Self::Uv { coordinates, .. } => Some(bytemuck::cast_slice(coordinates)),
            Self::Color(colors) => Some(bytemuck::cast_slice(colors)),
            _ => None,
        }
    }
}

fn validate_buffer(expected: u64, size: u64, usage: wgpu::BufferUsages) -> Result<()> {
    ensure!(
        size == expected,
        "GPU attribute buffer requires exactly {expected} bytes"
    );
    ensure!(
        usage.contains(wgpu::BufferUsages::COPY_SRC),
        "GPU attribute buffer requires COPY_SRC usage"
    );
    Ok(())
}

struct AttributePlan {
    header: [u32; 8],
    upload_bytes: u64,
}

impl AttributePlan {
    fn new(
        vertices: usize,
        sets: [u32; 5],
        updates: &[Scene3dVertexUpdate<'_>],
        limits: &wgpu::Limits,
        byte_limit: Option<u64>,
    ) -> Result<Self> {
        ensure!(
            vertices > 0 && vertices <= u32::MAX as usize / 24,
            "GPU attribute vertex addressing exceeds u32"
        );
        ensure!(updates.len() <= 6, "too many GPU attribute updates");
        let mut header = [0; 8];
        header[0] = vertices as u32;
        let mut words = 8_u64;
        for update in updates {
            let offset = u32::try_from(words)?;
            match update {
                Scene3dVertexUpdate::Uv { coordinates, .. } => {
                    ensure!(
                        coordinates.len() == vertices,
                        "GPU UV attribute count mismatch"
                    );
                    ensure!(
                        coordinates.iter().flatten().all(|v| v.is_finite()),
                        "GPU UV coordinates must be finite"
                    );
                }
                Scene3dVertexUpdate::Color(colors) => {
                    ensure!(
                        colors.len() == vertices,
                        "GPU color attribute count mismatch"
                    );
                    ensure!(
                        colors.iter().flatten().all(|v| (0. ..=1.).contains(v)),
                        "GPU vertex colors must be finite and within 0..=1"
                    );
                }
                Scene3dVertexUpdate::UvBuffer { buffer, .. } => {
                    validate_buffer(vertices as u64 * 8, buffer.size(), buffer.usage())?
                }
                Scene3dVertexUpdate::ColorBuffer(buffer) => {
                    validate_buffer(vertices as u64 * 16, buffer.size(), buffer.usage())?
                }
            }
            match update {
                Scene3dVertexUpdate::Uv { set, .. } | Scene3dVertexUpdate::UvBuffer { set, .. } => {
                    ensure!(
                        sets.contains(set),
                        "GPU geometry does not select UV set {set}"
                    );
                    for (slot, selected) in sets.iter().enumerate() {
                        if selected == set {
                            ensure!(header[slot + 1] == 0, "duplicate GPU UV set {set}");
                            header[slot + 1] = offset;
                        }
                    }
                    words += vertices as u64 * 2;
                }
                Scene3dVertexUpdate::Color(_) | Scene3dVertexUpdate::ColorBuffer(_) => {
                    ensure!(header[6] == 0, "duplicate GPU vertex color update");
                    header[6] = offset;
                    words += vertices as u64 * 4;
                }
            }
        }
        ensure!(
            words <= u64::from(u32::MAX),
            "GPU attribute upload addressing exceeds u32"
        );
        let upload_bytes = if updates.is_empty() { 0 } else { words * 4 };
        let source_bytes = if updates.is_empty() {
            0
        } else {
            vertices as u64 * std::mem::size_of::<Vertex>() as u64
        };
        for bytes in [upload_bytes, source_bytes] {
            ensure!(
                bytes <= limits.max_buffer_size && bytes <= limits.max_storage_buffer_binding_size,
                "GPU attribute buffer exceeds device limits"
            );
        }
        ensure!(
            updates.is_empty()
                || vertices.div_ceil(64) as u64
                    <= u64::from(limits.max_compute_workgroups_per_dimension),
            "GPU attribute dispatch exceeds device limits"
        );
        ensure!(
            byte_limit.is_none_or(|limit| upload_bytes + source_bytes <= limit),
            "GPU attribute update exceeds working payload budget"
        );
        Ok(Self {
            header,
            upload_bytes,
        })
    }
}

pub(super) struct AttributeKernel {
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::ComputePipeline,
}

impl AttributeKernel {
    fn new(device: &wgpu::Device) -> Result<Self> {
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let entries = [(0, 32), (1, 4)].map(|(binding, minimum)| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage {
                    read_only: binding == 0,
                },
                has_dynamic_offset: false,
                min_binding_size: wgpu::BufferSize::new(minimum),
            },
            count: None,
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("scene3d.attributes.inputs"),
            entries: &entries,
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("scene3d.attributes.layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("scene3d.attributes.shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("attributes.wgsl").into()),
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("scene3d.attributes.update"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("update_attributes"),
            compilation_options: Default::default(),
            cache: None,
        });
        if let Some(error) = gpui::block_on(scope.pop()) {
            anyhow::bail!("GPU attribute preparation: {error}");
        }
        Ok(Self { layout, pipeline })
    }
}

impl WgpuScene3dGeometry {
    /// Returns an independent GPU source with selected UV/color streams replaced.
    /// Duplicate streams and unselected UV sets are rejected. Unchanged streams, topology,
    /// packing pipelines, and previous source versions are preserved. Empty updates clone
    /// this source without allocating buffers or submitting work.
    ///
    /// Only CPU streams are uploaded; external streams are copied on the GPU. The GPU
    /// copies the interleaved source before updating it. `max_working_bytes` admits
    /// the new source plus the temporary copy buffer, excluding existing sources,
    /// external inputs, results, shared indices, pipelines, and driver overhead.
    ///
    /// CPU mesh metadata is unchanged. Use GPU coverage/picking for the resulting draw.
    /// UV updates do not regenerate tangents: when changing tangent-space coordinates,
    /// supply matching tangents in the deformation buffer passed to `evaluate`.
    ///
    /// External buffers must be created through this source's context. Submit producers
    /// on its queue before calling; later queue writes may reuse the buffers without
    /// changing this snapshot. Do not map or destroy inputs until the copy completes.
    /// GPU values are not read back: packing suppresses draws with nonfinite UVs or
    /// color lanes outside [0, 1]. Buffer inputs count toward the temporary copy payload.
    pub fn with_attributes(
        &self,
        updates: &[Scene3dVertexUpdate<'_>],
        max_working_bytes: Option<u64>,
    ) -> Result<Self> {
        ensure!(!self.context.device_lost(), "GPU geometry device is lost");
        let device = &self.context.device;
        for update in updates {
            if let Some(buffer) = update.buffer() {
                buffer.check_device(device)?;
            }
        }
        let plan = AttributePlan::new(
            self.mesh.vertices().len(),
            self.uv_sets,
            updates,
            &device.limits(),
            max_working_bytes,
        )?;
        if updates.is_empty() {
            return Ok(self.clone());
        }
        let mut cached = self.attribute_kernel.lock();
        if cached.is_none() {
            *cached = Some(AttributeKernel::new(device)?);
        }
        let kernel = cached.as_ref().expect("initialized attribute kernel");
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let upload = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("scene3d.attributes.upload"),
            size: plan.upload_bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let source = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("scene3d.attributes.source"),
            size: self.memory.source_vertex_bytes,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("scene3d.attributes.bind"),
            layout: &kernel.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: upload.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: source.as_entire_binding(),
                },
            ],
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("scene3d.attributes"),
        });
        self.context
            .queue
            .write_buffer(&upload, 0, bytemuck::cast_slice(&plan.header));
        let mut offset = 32;
        for update in updates {
            if let Some(bytes) = update.bytes() {
                self.context.queue.write_buffer(&upload, offset, bytes);
                offset += bytes.len() as u64;
            } else if let Some(buffer) = update.buffer() {
                encoder.copy_buffer_to_buffer(buffer, 0, &upload, offset, buffer.size());
                offset += buffer.size();
            }
        }
        encoder.copy_buffer_to_buffer(&self.source, 0, &source, 0, self.memory.source_vertex_bytes);
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("scene3d.attributes"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&kernel.pipeline);
            pass.set_bind_group(0, &bind, &[]);
            pass.dispatch_workgroups(self.mesh.vertices().len().div_ceil(64) as u32, 1, 1);
        }
        self.context.queue.submit([encoder.finish()]);
        if let Some(error) = gpui::block_on(scope.pop()) {
            anyhow::bail!("GPU attribute submission: {error}");
        }
        Ok(Self {
            source,
            ..self.clone()
        })
    }
}
