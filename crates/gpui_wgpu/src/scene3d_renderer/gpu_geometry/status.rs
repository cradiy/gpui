use super::Scene3dGpuGeometry;
use crate::WgpuContext;
use anyhow::{Context as _, Result, ensure};
use std::sync::mpsc;

#[cfg(test)]
mod tests;

bitflags::bitflags! {
    /// Independent reasons a packed geometry result cannot be drawn.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct Scene3dGeometryIssues: u32 {
        const INVALID_UV = 1;
        const INVALID_COLOR = 2;
        const DEFORMATION_STATUS = 4;
        const NONFINITE_DEFORMATION = 8;
        const INVALID_TANGENT = 16;
        const TRIANGLE_TANGENT_SIGN = 32;
    }
}

/// Validation of one packed result, not visibility or coverage of a rendered frame.
/// Indices use base-mesh vertex/triangle order, including unused vertices.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Scene3dGeometryStatus {
    pub issues: Scene3dGeometryIssues,
    /// Lowest index with any vertex issue, not necessarily every issue in `issues`.
    pub first_invalid_vertex: Option<u32>,
    /// Lowest triangle with inconsistent tangent handedness.
    pub first_invalid_triangle: Option<u32>,
}

impl Scene3dGeometryStatus {
    pub fn is_drawable(self) -> bool {
        self.issues.is_empty()
    }

    fn decode(bytes: &[u8], vertices: u32, indices: u32) -> Result<Self> {
        ensure!(bytes.len() == 32, "invalid GPU geometry status size");
        let words: [u32; 8] = std::array::from_fn(|i| {
            u32::from_le_bytes(bytes[i * 4..i * 4 + 4].try_into().unwrap())
        });
        let issues = Scene3dGeometryIssues::from_bits(words[5])
            .context("unknown GPU geometry status flags")?;
        ensure!(
            words[0] == indices
                && words[1] == u32::from(issues.is_empty())
                && words[2..5] == [0; 3],
            "inconsistent GPU geometry draw status"
        );
        let vertex = (words[6] != u32::MAX).then_some(words[6]);
        let triangle = (words[7] != u32::MAX).then_some(words[7]);
        ensure!(
            vertex.is_none_or(|i| i < vertices) && triangle.is_none_or(|i| i < indices / 3),
            "GPU geometry status index out of bounds"
        );
        ensure!(
            vertex.is_some() == !(issues - Scene3dGeometryIssues::TRIANGLE_TANGENT_SIGN).is_empty()
                && triangle.is_some()
                    == issues.contains(Scene3dGeometryIssues::TRIANGLE_TANGENT_SIGN),
            "inconsistent GPU geometry issue locations"
        );
        Ok(Self {
            issues,
            first_invalid_vertex: vertex,
            first_invalid_triangle: triangle,
        })
    }
}

/// Fixed-size nonblocking readback. May outlive its geometry; dropping cancels mapping,
/// not submitted GPU work. Concurrent-request scheduling belongs to the caller.
#[must_use = "retain the request until completion or drop it to cancel mapping"]
pub struct Scene3dGeometryStatusReadback {
    pending: Option<Pending>,
}

struct Pending {
    context: WgpuContext,
    staging: wgpu::Buffer,
    receiver: mpsc::Receiver<Result<(), wgpu::BufferAsyncError>>,
    vertices: u32,
    indices: u32,
}

impl Scene3dGpuGeometry {
    /// Copies 32 bytes of validation state without reading vertex or index buffers.
    /// The optional staging limit is per request; existing outputs are not counted.
    pub fn request_status(
        &self,
        max_staging_bytes: Option<u64>,
    ) -> Result<Scene3dGeometryStatusReadback> {
        ensure!(!self.context.device_lost(), "GPU geometry device is lost");
        ensure!(
            max_staging_bytes.is_none_or(|limit| limit >= 32),
            "GPU geometry status requires 32 staging bytes"
        );
        let device = &self.context.device;
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let staging = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("scene3d.geometry.status"),
            size: 32,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("scene3d.geometry.status"),
        });
        encoder.copy_buffer_to_buffer(&self.draw, 0, &staging, 0, 32);
        self.context.queue.submit([encoder.finish()]);
        if let Some(error) = gpui::block_on(scope.pop()) {
            anyhow::bail!("GPU geometry status copy: {error}");
        }
        let (send, receiver) = mpsc::channel();
        staging
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = send.send(result);
            });
        Ok(Scene3dGeometryStatusReadback {
            pending: Some(Pending {
                context: self.context.clone(),
                staging,
                receiver,
                vertices: self.mesh.vertices().len() as u32,
                indices: self.mesh.indices().len() as u32,
            }),
        })
    }
}

impl Scene3dGeometryStatusReadback {
    /// Polls callbacks without waiting. Completed or failed requests cannot be read again.
    pub fn try_read(&mut self) -> Result<Option<Scene3dGeometryStatus>> {
        let pending = self
            .pending
            .as_ref()
            .context("GPU geometry status readback is finished")?;
        let result = pending.try_read();
        if !matches!(&result, Ok(None)) {
            self.pending.take();
        }
        result
    }
}

impl Pending {
    fn try_read(&self) -> Result<Option<Scene3dGeometryStatus>> {
        ensure!(!self.context.device_lost(), "GPU geometry device is lost");
        self.context.device.poll(wgpu::PollType::Poll)?;
        match self.receiver.try_recv() {
            Ok(result) => result.context("GPU geometry status mapping failed")?,
            Err(mpsc::TryRecvError::Empty) => return Ok(None),
            Err(mpsc::TryRecvError::Disconnected) => {
                anyhow::bail!("GPU geometry status callback was dropped")
            }
        }
        let bytes = self.staging.slice(..).get_mapped_range()?;
        Scene3dGeometryStatus::decode(&bytes, self.vertices, self.indices).map(Some)
    }
}

impl Drop for Pending {
    fn drop(&mut self) {
        self.staging.unmap();
    }
}
