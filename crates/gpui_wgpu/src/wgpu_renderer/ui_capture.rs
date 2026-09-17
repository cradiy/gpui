use super::scene_snapshot::{OutputValidity, SceneSnapshot};
use super::{SceneEncoding, WgpuExternalRenderTarget, WgpuRenderer};
use gpui::{Scene, SubtreeLayer, UiTexture3d};

pub(super) struct UiCapture {
    renderer: WgpuRenderer,
    pub(super) texture: wgpu::Texture,
    snapshot: Option<SceneSnapshot>,
    validity: OutputValidity,
    external_writes: bool,
}

impl UiCapture {
    pub(super) fn invalidate_encoding(&mut self) {
        self.validity = OutputValidity::default();
        for capture in &mut self.renderer.resources_mut().ui_captures {
            capture.invalidate_encoding();
        }
    }

    pub(super) fn invalidate_scene3d_outputs(&mut self) {
        self.renderer.invalidate_scene3d_outputs();
    }

    pub(super) fn clear_scene3d_caches(&mut self) {
        self.snapshot = None;
        self.validity = OutputValidity::default();
        self.renderer.clear_scene3d_caches();
    }

    pub(super) fn commit_scene3d_outputs(&self, submitted: bool) {
        self.validity.commit(submitted);
        self.renderer.commit_scene3d_outputs(submitted);
    }

    pub(super) fn retain_external_scene3d_uploads(&self) {
        self.renderer.retain_external_scene3d_uploads();
    }
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
        retain_outputs: bool,
    ) -> anyhow::Result<bool> {
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
                let mut renderer = Self::new_for_target(
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
                renderer.scene3d_output_budget = self.scene3d_output_budget.clone();
                let texture = self.ui_capture_texture(width, height);
                self.resources_mut().ui_captures.push(UiCapture {
                    renderer,
                    texture,
                    snapshot: None,
                    validity: OutputValidity::default(),
                    external_writes: false,
                });
            } else {
                let capture = &self.resources().ui_captures[index];
                // Caller-owned commands can write the old target after the next frame.
                if capture.texture.width() != width
                    || capture.texture.height() != height
                    || capture.external_writes
                {
                    let texture = self.ui_capture_texture(width, height);
                    let capture = &mut self.resources_mut().ui_captures[index];
                    capture.renderer.update_drawable_size(size);
                    capture.texture = texture;
                    capture.snapshot = None;
                    capture.validity = OutputValidity::default();
                    capture.external_writes = false;
                }
            }
            let snapshot = retain_outputs
                .then(|| SceneSnapshot::new(&layer.scene, |tile| self.atlas.tile_generation(tile)))
                .flatten();
            let capture = &mut self.resources_mut().ui_captures[index];
            let reusable = capture.validity.reusable()
                && snapshot
                    .as_ref()
                    .zip(capture.snapshot.as_ref())
                    .is_some_and(|(new, old)| new.matches(old));
            capture.snapshot = snapshot;
            if reusable {
                self.resources_mut()
                    .ui_capture_indices
                    .insert(layer as *const _ as usize, index);
                continue;
            }
            capture.validity = OutputValidity::default();
            capture.external_writes = !retain_outputs;
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
            if capture.renderer.encode_external_scene(
                &layer.scene,
                WgpuExternalRenderTarget {
                    texture: &capture.texture,
                    view: &view,
                    command_encoder: encoder,
                },
                retain_outputs,
            )? != SceneEncoding::Complete
            {
                return Ok(false);
            }
            capture.validity.encoded();
            self.resources_mut()
                .ui_capture_indices
                .insert(layer as *const _ as usize, index);
        }
        Ok(true)
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

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{
        Bounds, ContentMask, DevicePixels, EffectQuad, EffectShader, Quad, ScaledPixels, point,
        rgba, size,
    };
    use std::{rc::Rc, sync::Arc};

    #[test]
    #[ignore = "requires a GPU adapter"]
    fn scene3d_capture_reuses_submitted_repaints_and_isolates_unsubmitted_writes()
    -> anyhow::Result<()> {
        let context = crate::WgpuContext::new_headless()?;
        let mut renderer = WgpuRenderer::new_external(
            &context,
            crate::WgpuExternalRendererConfig {
                size: size(DevicePixels(64), DevicePixels(64)),
                format: wgpu::TextureFormat::Rgba8UnormSrgb,
                alpha_mode: wgpu::CompositeAlphaMode::Opaque,
                target_usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::COPY_SRC,
            },
        )?;
        let make_scene = |color, extent| {
            let bounds = Bounds::new(
                point(ScaledPixels(0.), ScaledPixels(0.)),
                size(ScaledPixels(extent), ScaledPixels(extent)),
            );
            let mut source = Scene::default();
            source.insert_primitive(Quad {
                bounds,
                content_mask: ContentMask { bounds },
                background: rgba(color).into(),
                ..Default::default()
            });
            source.finish();
            let identity = [
                [1., 0., 0., 0.],
                [0., 1., 0., 0.],
                [0., 0., 1., 0.],
                [0., 0., 0., 1.],
            ];
            let frame = gpui::Scene3dFrame {
                pick_capture: None,
                occlusion_groups: Default::default(),
                depth_background: Default::default(),
                viewport_quality: Default::default(),
                ui_texture: Some(UiTexture3d::new(
                    size(gpui::px(extent), gpui::px(extent)),
                    1.,
                )),
                view_projection: identity,
                world_to_view: identity,
                camera_position: [0., 0., 3.],
                orthographic_view_direction: None,
                light_direction: [0., 0., 1.],
                light: [1.; 4],
                lights: None,
                directional_shadow: None,
                ambient: 0.3,
                diffuse_environment: None,
                background: None,
                specular_environment: None,
                color_output: Default::default(),
                objects: Arc::default(),
            };
            let mut scene = Scene::default();
            scene.insert_primitive(gpui::Primitive::SubtreeLayer(SubtreeLayer {
                scene3d: Some(Arc::new(frame)), scene: Rc::new(source), second_scene: None,
                intermediate_effects: Arc::default(),
                composite: EffectQuad {
                    order: 0, bounds, effect_bounds: bounds, transformation: Default::default(),
                    content_mask: ContentMask { bounds },
                    shader: EffectShader::wgsl_image("fn effect(input: EffectInput, params: EffectParams) -> vec4<f32> { return sample_effect_image(input, input.uv); }"),
                    uniforms: Default::default(), time: 0., corner_radii: Default::default(), opacity: 1.,
                    image_tile: None, second_image_tile: None, third_image_tile: None, fourth_image_tile: None,
                },
            }));
            scene.finish();
            scene
        };
        let commands = || context.device.create_command_encoder(&Default::default());
        let mut first = commands();
        assert!(renderer.encode_ui_captures(&make_scene(0xff0000ff, 32.), &mut first, true)?);
        assert!(!renderer.resources().ui_captures[0].validity.reusable());
        context.queue.submit([first.finish()]);
        renderer.commit_scene3d_outputs(true);
        assert!(renderer.resources().ui_captures[0].validity.reusable());
        let first_texture = renderer.resources().ui_captures[0].texture.clone();

        let mut repaint = commands();
        assert!(renderer.encode_ui_captures(&make_scene(0xff0000ff, 32.), &mut repaint, true)?);
        renderer.commit_scene3d_outputs(false);
        assert!(renderer.resources().ui_captures[0].validity.reusable());
        assert_eq!(first_texture, renderer.resources().ui_captures[0].texture);
        drop(repaint);

        let mut changed = commands();
        assert!(renderer.encode_ui_captures(&make_scene(0x0000ffff, 32.), &mut changed, true)?);
        renderer.commit_scene3d_outputs(false);
        assert!(!renderer.resources().ui_captures[0].validity.reusable());
        drop(changed);

        let mut retry = commands();
        assert!(renderer.encode_ui_captures(&make_scene(0x0000ffff, 32.), &mut retry, true)?);
        context.queue.submit([retry.finish()]);
        renderer.commit_scene3d_outputs(true);
        assert!(renderer.resources().ui_captures[0].validity.reusable());

        let mut external = commands();
        assert!(renderer.encode_ui_captures(&make_scene(0xff0000ff, 32.), &mut external, false)?);
        let external_texture = renderer.resources().ui_captures[0].texture.clone();
        let mut owned = commands();
        assert!(renderer.encode_ui_captures(&make_scene(0x0000ffff, 32.), &mut owned, true)?);
        assert_ne!(
            external_texture,
            renderer.resources().ui_captures[0].texture
        );
        context.queue.submit([owned.finish()]);
        renderer.commit_scene3d_outputs(true);
        context.queue.submit([external.finish()]);

        let texture = renderer.resources().ui_captures[0].texture.clone();
        let mut resized = commands();
        assert!(renderer.encode_ui_captures(&make_scene(0x0000ffff, 48.), &mut resized, true)?);
        assert_ne!(texture, renderer.resources().ui_captures[0].texture);
        assert_eq!(renderer.resources().ui_captures[0].texture.width(), 48);
        renderer.commit_scene3d_outputs(false);
        assert!(!renderer.resources().ui_captures[0].validity.reusable());
        drop(resized);

        let parent_capacity = renderer.instance_buffer_capacity;
        let child = &mut renderer.resources_mut().ui_captures[0].renderer;
        let child_limit = child.max_buffer_size;
        child.instance_buffer_capacity = 64;
        child.max_buffer_size = 64;
        let target = renderer.ui_capture_texture(64, 64);
        let target_view = target.create_view(&Default::default());
        let mut failed = commands();
        assert!(
            renderer
                .encode_external_scene(
                    &make_scene(0x0000ffff, 48.),
                    WgpuExternalRenderTarget {
                        texture: &target,
                        view: &target_view,
                        command_encoder: &mut failed,
                    },
                    true,
                )
                .is_err()
        );
        assert_eq!(renderer.instance_buffer_capacity, parent_capacity);
        assert_eq!(
            renderer.resources().ui_captures[0]
                .renderer
                .instance_buffer_capacity,
            64
        );
        drop(failed);

        renderer.resources_mut().ui_captures[0]
            .renderer
            .max_buffer_size = child_limit;
        let mut retry = commands();
        assert_eq!(
            renderer.encode_external_scene(
                &make_scene(0x0000ffff, 48.),
                WgpuExternalRenderTarget {
                    texture: &target,
                    view: &target_view,
                    command_encoder: &mut retry,
                },
                true,
            )?,
            SceneEncoding::CaptureCapacity,
        );
        assert_eq!(renderer.instance_buffer_capacity, parent_capacity);
        assert_eq!(
            renderer.resources().ui_captures[0]
                .renderer
                .instance_buffer_capacity,
            128
        );
        Ok(())
    }
}
