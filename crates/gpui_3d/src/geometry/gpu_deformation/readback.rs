use super::{GpuDeformationOutput, decode};
use crate::Mesh;
use anyhow::{Context as _, Result, ensure};
use gpui_wgpu::{WgpuContext, wgpu};
use std::{sync::mpsc, time::Duration};

/// An owned readback of one immutable deformation output.
/// Dropping it releases staging resources and cancels mapping, not submitted GPU work.
/// Requests may outlive their source output. Scheduling and concurrent-request limits
/// belong to the caller.
#[must_use = "keep the request to read its result; dropping it cancels mapping"]
pub struct GpuDeformationReadback {
    pending: Option<Pending>,
    staging_bytes: u64,
}

struct Pending {
    context: WgpuContext,
    base: Mesh,
    staging: wgpu::Buffer,
    submission: wgpu::SubmissionIndex,
    receiver: mpsc::Receiver<Result<(), wgpu::BufferAsyncError>>,
}

impl GpuDeformationOutput {
    /// Copies attributes into owned staging storage and requests asynchronous mapping.
    /// An optional per-request byte limit is checked before allocation. Concurrent requests
    /// each own a staging buffer; this is not a total residency quota.
    pub fn request_readback(
        &self,
        max_staging_bytes: Option<u64>,
    ) -> Result<GpuDeformationReadback> {
        ensure!(
            !self.context.device_lost(),
            "GPU deformation device is lost"
        );
        let size = self.buffer.size();
        ensure!(
            max_staging_bytes.is_none_or(|limit| size <= limit),
            "GPU deformation readback needs {size} staging bytes, exceeding its budget"
        );
        let device = &self.context.device;
        let staging = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gpui_3d.deformation.readback"),
            size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("gpui_3d.deformation.readback"),
        });
        encoder.copy_buffer_to_buffer(&self.buffer, 0, &staging, 0, size);
        let submission = self.context.queue.submit(Some(encoder.finish()));
        let (send, receiver) = mpsc::channel();
        staging
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = send.send(result);
            });
        Ok(GpuDeformationReadback {
            pending: Some(Pending {
                context: self.context.clone(),
                base: self.base.clone(),
                staging,
                submission,
                receiver,
            }),
            staging_bytes: size,
        })
    }
}

impl GpuDeformationReadback {
    /// Staging payload admitted for this request. Excludes CPU mesh copies and driver overhead.
    pub fn staging_bytes(&self) -> u64 {
        self.staging_bytes
    }

    /// Pumps device callbacks without waiting for GPU completion. Returns `None` while pending.
    /// A ready result is validated and materialized into a CPU mesh during this call; that CPU
    /// work scales with mesh size. Completed or failed requests cannot be read again.
    /// Existing meshes, scenes, bounds, and spatial query snapshots remain unchanged.
    pub fn try_read(&mut self) -> Result<Option<Mesh>> {
        let pending = self
            .pending
            .as_ref()
            .context("GPU deformation readback is finished")?;
        let result = pending.try_read();
        if !matches!(&result, Ok(None)) {
            self.pending.take();
        }
        result
    }

    pub(super) fn wait(mut self) -> Result<Mesh> {
        let pending = self
            .pending
            .take()
            .context("GPU deformation readback is finished")?;
        pending.context.device.poll(wgpu::PollType::Wait {
            submission_index: Some(pending.submission.clone()),
            timeout: Some(Duration::from_secs(30)),
        })?;
        pending.receiver.recv_timeout(Duration::from_secs(1))??;
        pending.decode()
    }
}

impl Pending {
    fn try_read(&self) -> Result<Option<Mesh>> {
        ensure!(
            !self.context.device_lost(),
            "GPU deformation device is lost"
        );
        self.context.device.poll(wgpu::PollType::Poll)?;
        match self.receiver.try_recv() {
            Ok(result) => result.context("failed to map GPU deformation output")?,
            Err(mpsc::TryRecvError::Empty) => return Ok(None),
            Err(mpsc::TryRecvError::Disconnected) => {
                anyhow::bail!("GPU deformation readback callback was dropped")
            }
        }
        self.decode().map(Some)
    }

    fn decode(&self) -> Result<Mesh> {
        let bytes = self.staging.slice(..).get_mapped_range()?;
        decode(&self.base, &bytes)
    }
}

impl Drop for Pending {
    fn drop(&mut self) {
        self.staging.unmap();
    }
}
