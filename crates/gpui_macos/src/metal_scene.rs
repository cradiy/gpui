use anyhow::{Context as _, Result};
use foreign_types::ForeignTypeRef;
use gpui::{
    Bounds, ContentMask, DevicePixels, EffectQuad, EffectShader, Scene, Size, SubtreeLayer,
};
use gpui_wgpu::{WgpuContext, WgpuExternalRendererConfig, WgpuRenderer, wgpu};
use std::{rc::Rc, sync::Arc};

pub(crate) struct MetalSceneRenderer {
    pub context: WgpuContext,
    pub renderer: WgpuRenderer,
    targets: Vec<wgpu::Texture>,
    composite_shader: EffectShader,
}

impl MetalSceneRenderer {
    pub fn new(context: WgpuContext) -> Result<Self> {
        anyhow::ensure!(
            context.color_texture_format() == wgpu::TextureFormat::Bgra8Unorm,
            "Metal compositing requires a BGRA atlas"
        );
        let renderer = WgpuRenderer::new_external(
            &context,
            WgpuExternalRendererConfig {
                size: gpui::size(DevicePixels(1), DevicePixels(1)),
                format: wgpu::TextureFormat::Bgra8Unorm,
                alpha_mode: wgpu::CompositeAlphaMode::PreMultiplied,
                target_usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_SRC,
            },
        )?;
        Ok(Self {
            context,
            renderer,
            targets: Vec::new(),
            composite_shader: EffectShader::wgsl_image(
                "fn effect(input: EffectInput, params: EffectParams) -> vec4<f32> { return sample_effect_image(input, input.uv); }",
            ),
        })
    }

    pub fn device(&self) -> metal::Device {
        // Retain the Objective-C object independently of the HAL guard. All
        // native resources must use the same MTLDevice as the shared atlas.
        unsafe {
            let device = self
                .context
                .device
                .as_hal::<wgpu::hal::api::Metal>()
                .unwrap();
            metal::DeviceRef::from_ptr((&**device.raw_device()) as *const _ as *mut _).to_owned()
        }
    }

    pub fn command_queue(&self) -> metal::CommandQueue {
        // Both encoders submit to this queue. Tracked Metal resource hazards order
        // atlas uploads, subtree rendering, and native sampling without readback.
        unsafe {
            let queue = self
                .context
                .queue
                .as_hal::<wgpu::hal::api::Metal>()
                .unwrap();
            metal::CommandQueueRef::from_ptr(queue.as_raw() as *const _ as *mut _).to_owned()
        }
    }

    pub fn prepare(
        &mut self,
        scene: &Scene,
        size: Size<DevicePixels>,
    ) -> Result<Vec<metal::Texture>> {
        let mut layers = scene.subtree_layers.clone();
        for primitive in scene
            .particles
            .iter()
            .cloned()
            .map(gpui::Primitive::Particles)
            .chain(scene.fluids.iter().cloned().map(gpui::Primitive::Fluid))
        {
            let mut content = Scene::default();
            content.insert_primitive(primitive);
            content.finish();
            let bounds = Bounds::new(
                gpui::point(gpui::ScaledPixels(0.), gpui::ScaledPixels(0.)),
                size.map(|px| gpui::ScaledPixels(px.0 as f32)),
            );
            layers.push(SubtreeLayer {
                scene: Rc::new(content),
                scene3d: None,
                second_scene: None,
                intermediate_effects: Arc::default(),
                composite: EffectQuad {
                    order: 0,
                    bounds,
                    effect_bounds: bounds,
                    content_mask: ContentMask { bounds },
                    transformation: Default::default(),
                    corner_radii: Default::default(),
                    shader: self.composite_shader.clone(),
                    uniforms: Default::default(),
                    time: 0.,
                    opacity: 1.,
                    image_tile: None,
                    second_image_tile: None,
                    third_image_tile: None,
                    fourth_image_tile: None,
                },
            });
        }
        if self.renderer.viewport_size() != size {
            self.renderer.update_drawable_size(size);
            self.targets.clear();
        }
        self.targets.truncate(layers.len());
        while self.targets.len() < layers.len() {
            self.targets.push(
                self.context
                    .device
                    .create_texture(&wgpu::TextureDescriptor {
                        label: Some("gpui.metal.subtree"),
                        size: wgpu::Extent3d {
                            width: size.width.0 as u32,
                            height: size.height.0 as u32,
                            depth_or_array_layers: 1,
                        },
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format: wgpu::TextureFormat::Bgra8Unorm,
                        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                            | wgpu::TextureUsages::TEXTURE_BINDING
                            | wgpu::TextureUsages::COPY_SRC,
                        view_formats: &[],
                    }),
            );
        }
        self.renderer.draw_subtree_layers(&layers, &self.targets)?;
        // Native-only frames also need pending atlas writes submitted before the
        // Metal command buffer that samples them.
        self.renderer.sprite_atlas().before_frame();
        self.context.queue.submit([]);
        self.targets.iter().map(metal_texture).collect()
    }

    pub fn clear_caches(&mut self) {
        self.renderer.clear_scene3d_caches();
        self.targets.clear();
    }
}

pub(crate) fn metal_texture(texture: &wgpu::Texture) -> Result<metal::Texture> {
    // Clone retains the texture, so native command buffers can outlive WGPU's
    // cache entry. Callers only sample after submission on the shared queue.
    unsafe {
        let texture = texture
            .as_hal::<wgpu::hal::api::Metal>()
            .context("expected a Metal texture")?;
        Ok(metal::TextureRef::from_ptr(texture.raw_handle() as *const _ as *mut _).to_owned())
    }
}

fn composite_pipeline_descriptor(library: &metal::LibraryRef) -> metal::RenderPipelineDescriptor {
    let descriptor = metal::RenderPipelineDescriptor::new();
    descriptor.set_label("gpui.subtree_composite");
    descriptor.set_vertex_function(Some(&library.get_function("subtree_vertex", None).unwrap()));
    descriptor.set_fragment_function(Some(
        &library.get_function("subtree_fragment", None).unwrap(),
    ));
    let color = descriptor.color_attachments().object_at(0).unwrap();
    color.set_pixel_format(metal::MTLPixelFormat::BGRA8Unorm);
    color.set_blending_enabled(true);
    color.set_source_rgb_blend_factor(metal::MTLBlendFactor::One);
    color.set_source_alpha_blend_factor(metal::MTLBlendFactor::One);
    color.set_destination_rgb_blend_factor(metal::MTLBlendFactor::OneMinusSourceAlpha);
    color.set_destination_alpha_blend_factor(metal::MTLBlendFactor::OneMinusSourceAlpha);
    descriptor
}

pub(crate) fn composite_pipeline(
    device: &metal::DeviceRef,
    library: &metal::LibraryRef,
) -> metal::RenderPipelineState {
    device
        .new_render_pipeline_state(&composite_pipeline_descriptor(library))
        .expect("failed to create Metal subtree composite pipeline")
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod effects_tests;
