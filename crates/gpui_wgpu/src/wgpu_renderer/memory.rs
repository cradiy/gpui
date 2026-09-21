use super::WgpuRenderer;
use crate::WgpuAtlasMemoryStats;

/// Retained payload for 2D rendering, including nested UI capture renderers.
/// The shared atlas is counted once. This is not total GPU or process memory:
/// excludes swapchains, driver overhead, pipelines, in-flight work, video,
/// 3D attachments/geometry, particle/fluid buffers and distance-field storage.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WgpuMemoryStats {
    pub atlas: WgpuAtlasMemoryStats,
    pub instance_buffer_bytes: u64,
    pub path_texture_bytes: u64,
    pub backdrop_texture_bytes: u64,
    pub subtree_texture_bytes: u64,
    pub bloom_texture_bytes: u64,
    pub feedback_texture_bytes: u64,
    pub ui_capture_texture_bytes: u64,
}

impl WgpuRenderer {
    /// Reports currently retained 2D payload without submitting or waiting for GPU work.
    pub fn memory_stats(&self) -> WgpuMemoryStats {
        let mut stats = WgpuMemoryStats {
            atlas: self.atlas.memory_stats(),
            ..Default::default()
        };
        self.add_memory_stats(&mut stats);
        stats
    }

    fn add_memory_stats(&self, stats: &mut WgpuMemoryStats) {
        let Some(resources) = &self.resources else {
            return;
        };
        stats.instance_buffer_bytes += resources.instance_buffer.size();
        for texture in [
            &resources.path_intermediate_texture,
            &resources.path_msaa_texture,
        ]
        .into_iter()
        .flatten()
        {
            stats.path_texture_bytes += texture_bytes(texture);
        }
        for texture in [
            &resources.backdrop_source_texture,
            &resources.backdrop_horizontal_texture,
            &resources.backdrop_result_texture,
        ]
        .into_iter()
        .flatten()
        {
            stats.backdrop_texture_bytes += texture_bytes(texture);
        }
        stats.subtree_texture_bytes += resources
            .subtree_textures
            .iter()
            .map(texture_bytes)
            .sum::<u64>();
        stats.bloom_texture_bytes += resources
            .bloom_textures
            .values()
            .flatten()
            .map(texture_bytes)
            .sum::<u64>();
        stats.feedback_texture_bytes += resources
            .feedback_textures
            .values()
            .flat_map(|feedback| &feedback.textures)
            .map(texture_bytes)
            .sum::<u64>();
        for capture in &resources.ui_captures {
            stats.ui_capture_texture_bytes += texture_bytes(&capture.texture);
            capture.renderer.add_memory_stats(stats);
        }
    }
}

fn texture_bytes(texture: &wgpu::Texture) -> u64 {
    u64::from(texture.width())
        * u64::from(texture.height())
        * u64::from(texture.format().block_copy_size(None).unwrap_or(0))
        * u64::from(texture.sample_count())
}
