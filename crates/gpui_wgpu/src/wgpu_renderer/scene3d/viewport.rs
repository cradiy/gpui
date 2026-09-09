use super::{
    RenderRegion,
    output_cache::{Output, OutputKey},
};
use super::{Scene3dRenderer, WgpuAtlas};
use gpui::{Scene, Scene3dViewportCapabilities, SubtreeLayer};
use std::collections::HashMap;

const OUTPUT_CACHE_BYTES: u64 = 64 * 1024 * 1024;

pub(super) fn visit_scenes(scene: &Scene, mut visit: impl FnMut(&Scene)) {
    let mut pending = vec![scene];
    while let Some(scene) = pending.pop() {
        visit(scene);
        for layer in scene.subtree_layers.iter().rev() {
            if let Some(second) = &layer.second_scene {
                pending.push(second);
            }
            if layer
                .scene3d
                .as_ref()
                .and_then(|frame| frame.ui_texture)
                .is_none()
            {
                pending.push(&layer.scene);
            }
        }
    }
}

pub(in crate::wgpu_renderer) struct ViewportRenderer {
    renderers: [Option<Scene3dRenderer>; 2],
    capabilities: Scene3dViewportCapabilities,
    format: wgpu::TextureFormat,
    outputs: Vec<Option<Output>>,
    output_indices: HashMap<usize, usize>,
    surface_size: [u32; 2],
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
            outputs: Vec::new(),
            output_indices: HashMap::new(),
            surface_size: [0; 2],
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(in crate::wgpu_renderer) fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        scene: &Scene,
        width: u32,
        height: u32,
        atlas: &WgpuAtlas,
        retain_outputs: bool,
    ) {
        let mut needed = [false; 2];
        if self.surface_size != [width, height] {
            self.outputs.clear();
            self.surface_size = [width, height];
        }
        let mut previous = std::mem::take(&mut self.outputs).into_iter();
        self.output_indices.clear();
        let mut budget = OUTPUT_CACHE_BYTES;
        visit_scenes(scene, |scene| {
            for layer in &scene.subtree_layers {
                if let Some(frame) = &layer.scene3d {
                    let samples = self.capabilities.color_samples_for(frame.viewport_quality);
                    needed[usize::from(samples == 4)] = true;
                    let old = previous.next().flatten();
                    let bounds = layer.composite.bounds;
                    let output = retain_outputs
                        .then(|| {
                            RenderRegion::viewport(
                                [
                                    bounds.origin.x.0,
                                    bounds.origin.y.0,
                                    bounds.size.width.0,
                                    bounds.size.height.0,
                                ],
                                [width, height],
                                frame.viewport_quality.resolution_scale(),
                                self.capabilities.max_texture_dimension,
                            )
                        })
                        .flatten()
                        .and_then(|region| {
                            let bytes = u64::from(region.output_size[0])
                                * u64::from(region.output_size[1])
                                * u64::from(self.format.block_copy_size(None).unwrap_or(16));
                            if bytes > budget {
                                return None;
                            }
                            let key =
                                OutputKey::new(layer, region, |tile| atlas.tile_generation(tile))?;
                            let matching = old.filter(|output| output.key.matches(&key));
                            if matching.is_some() {
                                budget -= bytes;
                            }
                            Some(match matching {
                                Some(output) if output.validity.reusable() => output,
                                Some(_) => Output::new(device, self.format, key, region, true),
                                None => Output::new(device, self.format, key, region, false),
                            })
                        });
                    self.output_indices
                        .insert(layer as *const _ as usize, self.outputs.len());
                    self.outputs.push(output);
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

    pub(in crate::wgpu_renderer) fn commit_outputs(&self, submitted: bool) {
        for output in self.outputs.iter().flatten() {
            output.validity.commit(submitted);
        }
    }

    pub(in crate::wgpu_renderer) fn invalidate_outputs(&mut self) {
        self.outputs.clear();
        self.output_indices.clear();
    }

    #[allow(clippy::too_many_arguments)]
    pub(in crate::wgpu_renderer) fn encode(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        atlas: &WgpuAtlas,
        layer: &SubtreeLayer,
        source: &wgpu::TextureView,
        destination: &wgpu::Texture,
        encoder: &mut wgpu::CommandEncoder,
    ) {
        let output = self
            .output_indices
            .get(&(layer as *const _ as usize))
            .and_then(|index| self.outputs[*index].as_ref())
            .filter(|output| output.texture.is_some());
        let view = destination.create_view(&Default::default());
        if let Some(output) = output.filter(|output| output.validity.reusable()) {
            drop(encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("scene3d_restore_output"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            }));
            output.copy(destination, encoder, true);
            return;
        }
        let samples = self
            .capabilities
            .color_samples_for(layer.scene3d.as_ref().unwrap().viewport_quality);
        self.renderers[usize::from(samples == 4)]
            .as_ref()
            .unwrap()
            .encode(device, queue, atlas, layer, source, &view, encoder);
        if let Some(output) = output {
            output.copy(destination, encoder, false);
            output.validity.encoded();
        }
    }
}
