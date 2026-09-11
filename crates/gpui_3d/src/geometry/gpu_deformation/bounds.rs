use super::{GpuDeformationOutput, validate_storage};
use crate::Aabb;
use anyhow::{Context as _, Result, ensure};
use gpui_wgpu::{WgpuContext, wgpu};
use std::sync::{Arc, mpsc};
use wgpu::util::DeviceExt as _;

#[cfg(test)]
mod tests;

const RESULT_BYTES: u64 = 32;
const INITIAL: [u32; 8] = [
    0xff800000, 0xff800000, 0xff800000, 0, 0x007fffff, 0x007fffff, 0x007fffff, 0,
];

/// Reusable mesh-local bounds reduction for deformation outputs on one device.
/// Includes all vertices, even those not referenced by indices. Does not update CPU meshes.
pub struct GpuDeformationBounds {
    pub(super) context: WgpuContext,
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::ComputePipeline,
}

impl GpuDeformationBounds {
    /// Checks enabled compute limits without allocating resources or submitting work.
    /// Input size, request admission, device health, and output validity are checked separately.
    pub fn check_support(capabilities: &gpui_wgpu::Scene3dDeviceCapabilities) -> Result<()> {
        super::support::validate(capabilities, 2, 0, 64 * (16 + 16 + 4))
    }

    pub fn new(context: WgpuContext) -> Result<Self> {
        ensure!(!context.device_lost(), "GPU deformation device is lost");
        Self::check_support(&gpui_wgpu::Scene3dDeviceCapabilities::query(&context))?;
        let device = &context.device;
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let entries =
            [(0, 64), (1, RESULT_BYTES)].map(|(binding, minimum)| wgpu::BindGroupLayoutEntry {
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
            label: Some("gpui_3d.bounds.inputs"),
            entries: &entries,
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("gpui_3d.bounds.layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("gpui_3d.bounds.shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("bounds.wgsl").into()),
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("gpui_3d.bounds.reduce"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("reduce_bounds"),
            compilation_options: Default::default(),
            cache: None,
        });
        if let Some(error) = gpui::block_on(scope.pop()) {
            anyhow::bail!("GPU bounds pipeline: {error}");
        }
        Ok(Self {
            context,
            layout,
            pipeline,
        })
    }

    /// Submits a reduction and maps its 32-byte result asynchronously.
    /// The optional budget covers 64 bytes per request: result plus staging storage.
    /// It excludes the existing input, pipeline, workgroup memory, and driver overhead.
    /// Concurrent request counts and total residency are caller-owned.
    pub fn request(
        &self,
        output: &GpuDeformationOutput,
        max_working_bytes: Option<u64>,
    ) -> Result<GpuDeformationBoundsReadback> {
        ensure!(
            !self.context.device_lost(),
            "GPU deformation device is lost"
        );
        ensure!(
            Arc::ptr_eq(&self.context.device, &output.context.device),
            "GPU bounds input belongs to a different device"
        );
        ensure!(
            max_working_bytes.is_none_or(|limit| RESULT_BYTES * 2 <= limit),
            "GPU bounds request exceeds its working byte budget"
        );
        let device = &self.context.device;
        validate_storage(
            &device.limits(),
            &[output.buffer.size(), RESULT_BYTES],
            output.base.vertex_count(),
        )?;
        let result = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("gpui_3d.bounds.result"),
            contents: bytemuck::cast_slice(&INITIAL),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        });
        let staging = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gpui_3d.bounds.readback"),
            size: RESULT_BYTES,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let entries = [&output.buffer, &result]
            .into_iter()
            .enumerate()
            .map(|(binding, buffer)| wgpu::BindGroupEntry {
                binding: binding as u32,
                resource: buffer.as_entire_binding(),
            })
            .collect::<Vec<_>>();
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("gpui_3d.bounds.bind"),
            layout: &self.layout,
            entries: &entries,
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("gpui_3d.bounds"),
        });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("gpui_3d.bounds.reduce"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &bind, &[]);
            pass.dispatch_workgroups((output.base.vertex_count() as u32).div_ceil(64), 1, 1);
        }
        encoder.copy_buffer_to_buffer(&result, 0, &staging, 0, RESULT_BYTES);
        self.context.queue.submit(Some(encoder.finish()));
        let (send, receiver) = mpsc::channel();
        staging
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = send.send(result);
            });
        Ok(GpuDeformationBoundsReadback {
            pending: Some(Pending {
                context: self.context.clone(),
                staging,
                receiver,
            }),
        })
    }
}

/// Owned bounds readback for one immutable deformation output.
/// May outlive both the output and reducer. Dropping cancels mapping, not submitted GPU work.
#[must_use = "keep the request to read its result; dropping it cancels mapping"]
pub struct GpuDeformationBoundsReadback {
    pending: Option<Pending>,
}

impl GpuDeformationBoundsReadback {
    /// Pumps callbacks without waiting for GPU completion. Returns `None` while pending.
    /// Invalid vertex status or nonfinite positions fail the whole bounds result.
    /// Success or failure releases staging storage; subsequent reads return an error.
    /// Results belong to the requested output, not to newer poses. CPU picking is unchanged.
    pub fn try_read(&mut self) -> Result<Option<Aabb>> {
        let pending = self
            .pending
            .as_ref()
            .context("GPU bounds readback is finished")?;
        let result = pending.try_read();
        if !matches!(&result, Ok(None)) {
            self.pending.take();
        }
        result
    }
}

struct Pending {
    context: WgpuContext,
    staging: wgpu::Buffer,
    receiver: mpsc::Receiver<Result<(), wgpu::BufferAsyncError>>,
}

impl Pending {
    fn try_read(&self) -> Result<Option<Aabb>> {
        ensure!(
            !self.context.device_lost(),
            "GPU deformation device is lost"
        );
        self.context.device.poll(wgpu::PollType::Poll)?;
        match self.receiver.try_recv() {
            Ok(result) => result.context("failed to map GPU bounds")?,
            Err(mpsc::TryRecvError::Empty) => return Ok(None),
            Err(mpsc::TryRecvError::Disconnected) => {
                anyhow::bail!("GPU bounds callback was dropped")
            }
        }
        let bytes = self.staging.slice(..).get_mapped_range()?;
        decode(&bytes).map(Some)
    }
}

impl Drop for Pending {
    fn drop(&mut self) {
        self.staging.unmap();
    }
}

fn decode(bytes: &[u8]) -> Result<Aabb> {
    ensure!(
        bytes.len() == RESULT_BYTES as usize,
        "GPU bounds output size mismatch"
    );
    let values: [u32; 8] = bytemuck::pod_read_unaligned(bytes);
    ensure!(
        values[3] == 0 && values[7] == 0,
        "GPU bounds contain invalid vertex status"
    );
    let float = |value: u32| {
        f32::from_bits(if value & 0x80000000 != 0 {
            value ^ 0x80000000
        } else {
            !value
        })
    };
    Aabb::new(
        std::array::from_fn(|i| float(values[i])),
        std::array::from_fn(|i| float(values[i + 4])),
    )
    .context("GPU bounds are nonfinite or reversed")
}
