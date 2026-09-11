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
                Scene3dVertexUpdate::Uv { set, coordinates } => {
                    ensure!(
                        sets.contains(set),
                        "GPU geometry does not select UV set {set}"
                    );
                    ensure!(
                        coordinates.len() == vertices,
                        "GPU UV attribute count mismatch"
                    );
                    ensure!(
                        coordinates.iter().flatten().all(|v| v.is_finite()),
                        "GPU UV coordinates must be finite"
                    );
                    for (slot, selected) in sets.iter().enumerate() {
                        if selected == set {
                            ensure!(header[slot + 1] == 0, "duplicate GPU UV set {set}");
                            header[slot + 1] = offset;
                        }
                    }
                    words += vertices as u64 * 2;
                }
                Scene3dVertexUpdate::Color(colors) => {
                    ensure!(header[6] == 0, "duplicate GPU vertex color update");
                    ensure!(
                        colors.len() == vertices,
                        "GPU color attribute count mismatch"
                    );
                    ensure!(
                        colors.iter().flatten().all(|v| (0. ..=1.).contains(v)),
                        "GPU vertex colors must be finite and within 0..=1"
                    );
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

    fn pack(&self, updates: &[Scene3dVertexUpdate<'_>]) -> Vec<u32> {
        if updates.is_empty() {
            return Vec::new();
        }
        let mut words = Vec::with_capacity(self.upload_bytes as usize / 4);
        words.extend(self.header);
        for update in updates {
            let data: &[f32] = match update {
                Scene3dVertexUpdate::Uv { coordinates, .. } => bytemuck::cast_slice(coordinates),
                Scene3dVertexUpdate::Color(colors) => bytemuck::cast_slice(colors),
            };
            words.extend(data.iter().map(|v| v.to_bits()));
        }
        words
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
    /// Only supplied streams are uploaded. The GPU copies the interleaved source before
    /// updating it. `max_working_bytes` admits the new source plus the temporary upload,
    /// excluding existing sources, results, shared indices, pipelines, and driver overhead.
    ///
    /// CPU mesh metadata is unchanged. Use GPU coverage/picking for the resulting draw.
    /// UV updates do not regenerate tangents: when changing tangent-space coordinates,
    /// supply matching tangents in the deformation buffer passed to `evaluate`.
    pub fn with_attributes(
        &self,
        updates: &[Scene3dVertexUpdate<'_>],
        max_working_bytes: Option<u64>,
    ) -> Result<Self> {
        ensure!(!self.context.device_lost(), "GPU geometry device is lost");
        let device = &self.context.device;
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
        let words = plan.pack(updates);
        let mut cached = self.attribute_kernel.lock();
        if cached.is_none() {
            *cached = Some(AttributeKernel::new(device)?);
        }
        let kernel = cached.as_ref().expect("initialized attribute kernel");
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let upload = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("scene3d.attributes.upload"),
            contents: bytemuck::cast_slice(&words),
            usage: wgpu::BufferUsages::STORAGE,
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
