use super::WgpuRenderer;
use gpui::{CacheDiagnostics, CaptureTextureDiagnostics, RendererDiagnostics};

impl WgpuRenderer {
    /// Reports the latest encoding attempt and retained capture allocations, without GPU waits.
    /// Texture estimates exclude atlas, path/MSAA, backdrop, media and 3D allocations.
    pub fn diagnostics(&self) -> Option<RendererDiagnostics> {
        self.resources.as_ref()?;
        if !self.diagnostics_valid.get() || self.device_lost() {
            return None;
        }
        let mut result = RendererDiagnostics {
            capture_cache: self.capture_diagnostics.get(),
            ..Default::default()
        };
        self.collect_capture_textures(&mut result.capture_textures);
        Some(result)
    }

    fn collect_capture_textures(&self, output: &mut Vec<CaptureTextureDiagnostics>) {
        let Some(resources) = &self.resources else {
            return;
        };
        let mut add = |texture: &wgpu::Texture, raster_scale| {
            // Capture targets use uncompressed renderable color formats, one mip and sample.
            let bytes = u64::from(texture.format().block_copy_size(None).unwrap_or(0));
            output.push(CaptureTextureDiagnostics {
                width: texture.width(),
                height: texture.height(),
                estimated_bytes: u64::from(texture.width()) * u64::from(texture.height()) * bytes,
                raster_scale,
            });
        };
        for texture in &resources.subtree_textures {
            add(texture, None);
        }
        for capture in &resources.ui_captures {
            add(&capture.texture, Some(capture.raster_scale));
        }
        for capture in &resources.ui_captures {
            capture.renderer.collect_capture_textures(output);
        }
    }

    pub(super) fn record_capture_diagnostics(&self, hit: bool) {
        let mut stats = self.capture_diagnostics.get();
        if hit {
            stats.hits += 1;
        } else {
            stats.misses += 1;
        }
        self.capture_diagnostics.set(stats);
    }

    pub(super) fn add_capture_diagnostics(&self, child: CacheDiagnostics) {
        let mut stats = self.capture_diagnostics.get();
        stats.hits += child.hits;
        stats.misses += child.misses;
        self.capture_diagnostics.set(stats);
    }
}
