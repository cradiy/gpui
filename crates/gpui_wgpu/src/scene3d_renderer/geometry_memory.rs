use anyhow::{Result, ensure};
use gpui::Scene3dFrame;

use super::Scene3dChannels;
use crate::wgpu_renderer::scene3d::Scene3dRenderer;

/// Vertex and index payload required by one direct-render request.
/// Shared geometry is counted once across instances and selected output channels.
/// Excludes staging copies, instance/uniform buffers, textures, cache overlap,
/// and driver overhead. This is not a measurement of physical GPU usage.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Scene3dGeometryMemory {
    /// Distinct mesh-allocation/material-coordinate combinations.
    pub meshes: u64,
    pub vertex_bytes: u64,
    pub index_bytes: u64,
    pub total_bytes: u64,
    /// Largest individual vertex or index buffer.
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
