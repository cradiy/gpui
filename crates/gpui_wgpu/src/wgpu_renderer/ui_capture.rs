use super::{WgpuExternalRenderTarget, WgpuRenderer};
use gpui::{Scene, SubtreeLayer, UiTexture3d};

pub(super) struct UiCapture {
    renderer: WgpuRenderer,
    pub(super) texture: wgpu::Texture,
}

impl WgpuRenderer {
    pub(super) fn commit_ui_captures(&self, encoded: bool) {
        for capture in &self.resources().ui_captures {
            capture.renderer.commit_encoded_scene(encoded);
        }
    }

    pub(super) fn encode_ui_captures(
        &mut self,
        scene: &Scene,
        encoder: &mut wgpu::CommandEncoder,
    ) -> bool {
        fn collect<'a>(scene: &'a Scene, captures: &mut Vec<(&'a SubtreeLayer, UiTexture3d)>) {
            for layer in &scene.subtree_layers {
                if let Some(texture) = layer.scene3d.as_ref().and_then(|frame| frame.ui_texture) {
                    captures.push((layer, texture));
                } else {
                    collect(&layer.scene, captures);
                    if let Some(second) = &layer.second_scene {
                        collect(second, captures);
                    }
                }
            }
        }
        let mut captures = Vec::new();
        collect(scene, &mut captures);
        self.resources_mut().ui_captures.truncate(captures.len());
        self.resources_mut().ui_capture_indices.clear();
        for (index, (layer, config)) in captures.into_iter().enumerate() {
            let size = config.pixel_size();
            let width = size.width.0 as u32;
            let height = size.height.0 as u32;
            if index == self.resources().ui_captures.len() {
                let surface = wgpu::SurfaceConfiguration {
                    width,
                    height,
                    alpha_mode: wgpu::CompositeAlphaMode::PreMultiplied,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                    ..self.surface_config.clone()
                };
                let renderer = Self::new_for_target(
                    None,
                    &self.resources().capture_context,
                    None,
                    surface,
                    None,
                    self.atlas.clone(),
                    wgpu::CompositeAlphaMode::PreMultiplied,
                    wgpu::CompositeAlphaMode::PreMultiplied,
                    true,
                    Some(self.last_error.clone()),
                )
                .expect("UI capture renderer");
                let texture = self.ui_capture_texture(width, height);
                self.resources_mut()
                    .ui_captures
                    .push(UiCapture { renderer, texture });
            } else {
                let capture = &self.resources().ui_captures[index];
                if capture.texture.width() != width || capture.texture.height() != height {
                    let texture = self.ui_capture_texture(width, height);
                    let capture = &mut self.resources_mut().ui_captures[index];
                    capture.renderer.update_drawable_size(size);
                    capture.texture = texture;
                }
            }
            let capture = &mut self.resources_mut().ui_captures[index];
            let view = capture.texture.create_view(&Default::default());
            drop(encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("clear_ui_texture"),
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
            if !capture.renderer.encode_external_scene(
                &layer.scene,
                WgpuExternalRenderTarget {
                    texture: &capture.texture,
                    view: &view,
                    command_encoder: encoder,
                },
            ) {
                return false;
            }
            self.resources_mut()
                .ui_capture_indices
                .insert(layer as *const _ as usize, index);
        }
        true
    }

    fn ui_capture_texture(&self, width: u32, height: u32) -> wgpu::Texture {
        self.resources()
            .device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("ui_texture"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: self.surface_config.format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            })
    }
}
