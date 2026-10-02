use super::*;

pub(super) struct GradientUpload {
    pub buffer: wgpu::Buffer,
    pub bind_group: wgpu::BindGroup,
    data: Vec<gpui::GpuGradientStop>,
}

impl GradientUpload {
    fn new(resources: &WgpuResources, length: usize) -> anyhow::Result<Self> {
        let stride = std::mem::size_of::<gpui::GpuGradientStop>() as u64;
        let required = (length.max(1) as u64)
            .checked_mul(stride)
            .ok_or_else(|| anyhow::anyhow!("gradient buffer size overflow"))?;
        let limits = resources.device.limits();
        let limit = limits
            .max_buffer_size
            .min(limits.max_storage_buffer_binding_size);
        anyhow::ensure!(
            required <= limit,
            "gradient stop buffer requires {required} bytes, device limit is {limit} bytes"
        );
        let capacity = required
            .checked_next_power_of_two()
            .unwrap_or(required)
            .min(limit);
        let buffer = resources.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gradient_stops"),
            size: capacity,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = resources
            .device
            .create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("gradient_stops"),
                layout: &resources.bind_group_layouts.gradients,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: buffer.as_entire_binding(),
                }],
            });
        Ok(Self {
            buffer,
            bind_group,
            data: Vec::new(),
        })
    }
}

impl WgpuRenderer {
    pub(super) fn prepare_gradients(&mut self, scene: &Scene) -> anyhow::Result<()> {
        let resources = self.resources_mut();
        resources.gradient_upload_bytes = 0;
        resources.gradient_indices.clear();
        if resources.gradients.is_empty() {
            let empty = GradientUpload::new(resources, 1)?;
            resources.gradients.push(empty);
        }
        let mut index = 1;
        let mut result = Ok(());
        scene.visit(&mut |scene| {
            if result.is_err() {
                return;
            }
            result = (|| -> anyhow::Result<()> {
                let data = scene.gradients.stops();
                if data.is_empty() {
                    return Ok(());
                }
                if index == resources.gradients.len() {
                    let upload = GradientUpload::new(resources, data.len())?;
                    resources.gradients.push(upload);
                } else if resources.gradients[index].buffer.size()
                    < std::mem::size_of_val(data) as u64
                {
                    resources.gradients[index] = GradientUpload::new(resources, data.len())?;
                }
                let upload = &mut resources.gradients[index];
                if let Some(range) = gpui::gradient_changed_range(&upload.data, data) {
                    resources.gradient_upload_bytes +=
                        (range.len() * std::mem::size_of::<gpui::GpuGradientStop>()) as u64;
                    resources.queue.write_buffer(
                        &upload.buffer,
                        (range.start * std::mem::size_of::<gpui::GpuGradientStop>()) as u64,
                        bytemuck::cast_slice(&data[range]),
                    );
                }
                upload.data.clear();
                upload.data.extend_from_slice(data);
                resources
                    .gradient_indices
                    .insert(scene as *const Scene as usize, index);
                index += 1;
                Ok(())
            })();
        });
        resources.gradients.truncate(index);
        result
    }

    pub(super) fn gradient_bind_group(&self, scene: &Scene) -> &wgpu::BindGroup {
        let resources = self.resources();
        let index = resources
            .gradient_indices
            .get(&(scene as *const Scene as usize))
            .copied()
            .unwrap_or(0);
        &resources.gradients[index].bind_group
    }
}
