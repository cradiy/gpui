use anyhow::{Result, ensure};
use gpui::Scene3dFrame;

use super::Scene3dChannels;
use crate::wgpu_renderer::scene3d::Scene3dRenderer;

/// Vertex, index, and indirect argument payload required by one direct-render request.
/// Shared geometry is counted once across instances and selected output channels.
/// Packed GPU outputs also share index payload accounting when they use the same buffer.
/// Excludes staging copies, instance/uniform buffers, textures, cache overlap,
/// and driver overhead. This is not a measurement of physical GPU usage.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Scene3dGeometryMemory {
    /// Distinct CPU mesh/material-coordinate combinations and active packed GPU outputs.
    pub meshes: u64,
    pub vertex_bytes: u64,
    pub index_bytes: u64,
    /// Indirect arguments for explicitly supplied GPU geometry.
    pub indirect_bytes: u64,
    pub total_bytes: u64,
    /// Largest individual vertex, index, or indirect argument buffer.
    pub max_buffer_bytes: u64,
    /// Frame-local object referencing the largest buffer, or `None` for no geometry.
    pub max_buffer_object_id: Option<u32>,
}

impl Scene3dGeometryMemory {
    /// Plans geometry payload without an adapter, resource uploads, or GPU allocation.
    /// Supply a valid prepared frame. Camera and shaded-output shadow culling match
    /// the renderer. Scene validation and atlas residency are checked separately.
    pub fn plan(frame: &Scene3dFrame, channels: Scene3dChannels) -> Result<Self> {
        ensure!(
            !channels.is_empty() && Scene3dChannels::all().contains(channels),
            "3D geometry planning must select known channels"
        );
        if frame
            .objects
            .iter()
            .any(|object| object.gpu_geometry.is_some())
        {
            let geometry = super::gpu_draws::frame_geometry(frame)?;
            return super::gpu_draws::memory(frame, channels, &geometry);
        }
        Scene3dRenderer::plan_geometry_memory(frame, channels.shaded())
    }

    /// Checks an individual device-buffer limit and an optional total geometry
    /// payload limit. Zero total admits only requests with no active geometry.
    pub fn validate(self, max_buffer_bytes: u64, max_total_bytes: Option<u64>) -> Result<()> {
        ensure!(
            self.max_buffer_bytes <= max_buffer_bytes,
            "3D geometry buffer for object {:?} needs {} bytes, device limit is {}",
            self.max_buffer_object_id,
            self.max_buffer_bytes,
            max_buffer_bytes
        );
        if let Some(limit) = max_total_bytes {
            ensure!(
                self.total_bytes <= limit,
                "3D geometry needs {} bytes, request limit is {limit}",
                self.total_bytes
            );
        }
        Ok(())
    }
}
