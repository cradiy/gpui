use anyhow::{Result, ensure};
use gpui::Scene3dFrame;

use super::Scene3dChannels;
use crate::wgpu_renderer::scene3d::Scene3dRenderer;

/// Mesh work after frustum culling and adjacent material batching, summed across
/// selected output passes. Counts describe submissions, not rasterized fragments
/// or GPU timings. Texture preparation and fullscreen passes are excluded.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Scene3dDrawStatistics {
    pub camera_draws: u64,
    pub shadow_draws: u64,
    pub camera_instances: u64,
    pub shadow_instances: u64,
    pub camera_triangles: u64,
    pub shadow_triangles: u64,
    /// Upload batches, including batches shared by camera and shadow passes.
    pub batches: u64,
    /// Instance payload bytes copied per submission, excluding buffer capacity.
    pub instance_upload_bytes: u64,
    /// Material/frame parameter bytes copied per submission.
    pub uniform_upload_bytes: u64,
}

impl Scene3dDrawStatistics {
    /// Runs the renderer's CPU draw planner without an adapter or resource uploads.
    /// Supply a prepared, valid frame and a positive instance limit, obtainable
    /// from `WgpuScene3dRenderer::max_instances_per_batch` for a specific device.
    /// COLOR and LINEAR_COLOR share one shaded pass; each geometry channel adds
    /// its own pass. Shadow work is included only when a shaded output is selected.
    /// This does not validate atlas residency, device support, or scene parameters.
    pub fn plan(
        frame: &Scene3dFrame,
        channels: Scene3dChannels,
        max_instances_per_batch: usize,
    ) -> Result<Self> {
        ensure!(
            !channels.is_empty() && Scene3dChannels::all().contains(channels),
            "3D draw planning must select known channels"
        );
        ensure!(
            max_instances_per_batch > 0 && max_instances_per_batch as u64 <= u64::from(u32::MAX),
            "3D instance limit must be between 1 and u32::MAX"
        );
        let mut statistics = Self::default();
        if channels.shaded() {
            statistics += Scene3dRenderer::plan_statistics(frame, true, max_instances_per_batch);
        }
        let geometry = channels
            & (Scene3dChannels::OBJECT_ID
                | Scene3dChannels::LINEAR_DEPTH
                | Scene3dChannels::WORLD_NORMAL);
        if !geometry.is_empty() {
            let planned = Scene3dRenderer::plan_statistics(frame, false, max_instances_per_batch);
            for _ in 0..geometry.bits().count_ones() {
                statistics += planned;
            }
        }
        Ok(statistics)
    }
}

impl std::ops::AddAssign for Scene3dDrawStatistics {
    fn add_assign(&mut self, rhs: Self) {
        self.camera_draws += rhs.camera_draws;
        self.shadow_draws += rhs.shadow_draws;
        self.camera_instances += rhs.camera_instances;
        self.shadow_instances += rhs.shadow_instances;
        self.camera_triangles += rhs.camera_triangles;
        self.shadow_triangles += rhs.shadow_triangles;
        self.batches += rhs.batches;
        self.instance_upload_bytes += rhs.instance_upload_bytes;
        self.uniform_upload_bytes += rhs.uniform_upload_bytes;
    }
}
