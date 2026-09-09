use super::{Scene3dRenderer, WgpuAtlas};
use gpui::{Scene, Scene3dViewportCapabilities, SubtreeLayer};

pub(in crate::wgpu_renderer) struct ViewportRenderer {
    renderers: [Option<Scene3dRenderer>; 2],
    capabilities: Scene3dViewportCapabilities,
    format: wgpu::TextureFormat,
}

impl ViewportRenderer {
    pub(in crate::wgpu_renderer) fn new(
        format: wgpu::TextureFormat,
        capabilities: Scene3dViewportCapabilities,
    ) -> Self {
        Self {
            renderers: [None, None],
            capabilities,
            format,
        }
    }

    pub(in crate::wgpu_renderer) fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        scene: &Scene,
        width: u32,
        height: u32,
    ) {
        let mut needed = [false; 2];
        scene.visit(&mut |scene| {
            for layer in &scene.subtree_layers {
                if let Some(frame) = &layer.scene3d {
                    let samples = self.capabilities.color_samples_for(frame.viewport_quality);
                    needed[usize::from(samples == 4)] = true;
                }
            }
        });
        for (index, samples) in [1, 4].into_iter().enumerate() {
            if !needed[index] {
                self.renderers[index] = None;
                continue;
            }
            let renderer = self.renderers[index]
                .get_or_insert_with(|| Scene3dRenderer::new(device, queue, self.format, samples));
            renderer.prepare(device, queue, scene, width, height, self.capabilities);
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(in crate::wgpu_renderer) fn encode(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        atlas: &WgpuAtlas,
        layer: &SubtreeLayer,
        source: &wgpu::TextureView,
        destination: &wgpu::TextureView,
        encoder: &mut wgpu::CommandEncoder,
    ) {
        let samples = self
            .capabilities
            .color_samples_for(layer.scene3d.as_ref().unwrap().viewport_quality);
        self.renderers[usize::from(samples == 4)]
            .as_ref()
            .unwrap()
            .encode(device, queue, atlas, layer, source, destination, encoder);
    }
}
