use std::{rc::Rc, sync::Arc};

// Native Metal uses an unorm attachment; the WGPU offscreen harness uses sRGB.
const OUTPUT_SRGB: bool = false;

use gpui::{
    Bounds, ContentMask, DevicePixels, EffectQuad, EffectShader, Primitive, Quad, ScaledPixels,
    Scene, SubtreeLayer, point, rgba, size,
};

#[path = "../../../gpui_wgpu/tests/support/contour_glow.rs"]
mod contour_glow;
#[path = "../../../gpui_wgpu/tests/support/contour_relief.rs"]
mod contour_relief;
#[path = "../../../gpui_wgpu/tests/support/contour_shadow.rs"]
mod contour_shadow;
#[path = "../../../gpui_wgpu/tests/support/deformation.rs"]
mod deformation;
#[path = "../../../gpui_wgpu/tests/support/depth_parallax.rs"]
mod depth_parallax;
#[path = "../../../gpui_wgpu/tests/support/displacement_map.rs"]
mod displacement_map;
#[path = "../../../gpui_wgpu/tests/support/feedback.rs"]
mod feedback;
#[path = "../../../gpui_wgpu/tests/support/fluid.rs"]
mod fluid;
#[path = "../../../gpui_wgpu/tests/support/holographic.rs"]
mod holographic;
#[path = "../../../gpui_wgpu/tests/support/interaction_mapping.rs"]
mod interaction_mapping;
#[path = "../../../gpui_wgpu/tests/support/motion_blur.rs"]
mod motion_blur;
#[path = "../../../gpui_wgpu/tests/support/particle_mask.rs"]
mod particle_mask;
#[path = "../../../gpui_wgpu/tests/support/particle_transition.rs"]
mod particle_transition;
#[path = "../../../gpui_wgpu/tests/support/particles.rs"]
mod particles;
#[path = "../../../gpui_wgpu/tests/support/path_morph.rs"]
mod path_morph;
#[path = "../../../gpui_wgpu/tests/support/path_motion.rs"]
mod path_motion;
#[path = "../../../gpui_wgpu/tests/support/sdf.rs"]
mod sdf;
#[path = "../../../gpui_wgpu/tests/support/subtree_transition.rs"]
mod subtree_transition;

use crate::metal_renderer::{InstanceBufferPool, MetalRenderer};
use parking_lot::Mutex;

// Adapter for the rendering fixtures shared with the WGPU backend.
struct WgpuOffscreenRenderer {
    renderer: MetalRenderer,
    size: gpui::Size<DevicePixels>,
}
impl WgpuOffscreenRenderer {
    fn new(size: gpui::Size<DevicePixels>) -> anyhow::Result<Self> {
        Ok(Self {
            renderer: MetalRenderer::new_headless(Arc::new(Mutex::new(
                InstanceBufferPool::default(),
            ))),
            size,
        })
    }
    fn resize(&mut self, size: gpui::Size<DevicePixels>) {
        self.size = size;
    }
    fn render_rgba(&mut self, scene: &Scene) -> anyhow::Result<Vec<u8>> {
        Ok(self
            .renderer
            .render_scene_to_image(scene, self.size)?
            .into_raw())
    }
    fn sprite_atlas(&self) -> Arc<dyn gpui::PlatformAtlas> {
        self.renderer.sprite_atlas().clone()
    }
}

macro_rules! native_effect_test {
    ($($name:ident),* $(,)?) => {$(
        #[test]
        fn $name() -> anyhow::Result<()> {
            objc::rc::autoreleasepool(|| {
                let mut renderer = WgpuOffscreenRenderer::new(size(DevicePixels(64), DevicePixels(48)))?;
                $name::check(&mut renderer)
            })
        }
    )*};
}
native_effect_test!(
    contour_glow,
    contour_relief,
    contour_shadow,
    deformation,
    depth_parallax,
    displacement_map,
    feedback,
    fluid,
    holographic,
    interaction_mapping,
    motion_blur,
    particle_mask,
    particle_transition,
    particles,
    path_morph,
    path_motion,
    sdf,
    subtree_transition
);
fn bounds(x: f32, y: f32, width: f32, height: f32) -> Bounds<ScaledPixels> {
    Bounds::new(
        point(ScaledPixels(x), ScaledPixels(y)),
        size(ScaledPixels(width), ScaledPixels(height)),
    )
}

fn quad(bounds: Bounds<ScaledPixels>, color: u32) -> Quad {
    Quad {
        bounds,
        content_mask: ContentMask { bounds },
        background: rgba(color).into(),
        ..Default::default()
    }
}

fn layer(mut scene: Scene, bounds: Bounds<ScaledPixels>, opacity: f32) -> Primitive {
    scene.finish();
    Primitive::SubtreeLayer(SubtreeLayer {
        scene3d: None,
        second_scene: None,
        intermediate_effects: Arc::default(),
        composite: EffectQuad {
            order: 0,
            bounds,
            effect_bounds: bounds,
            transformation: Default::default(),
            content_mask: ContentMask { bounds },
            shader: EffectShader::wgsl_image(
                "fn effect(input: EffectInput, params: EffectParams) -> vec4<f32> { return sample_effect_image(input, input.uv); }",
            ),
            uniforms: Default::default(),
            time: 0.,
            corner_radii: Default::default(),
            opacity,
            image_tile: None,
            second_image_tile: None,
            third_image_tile: None,
            fourth_image_tile: None,
        },
        scene: Rc::new(scene),
    })
}

fn bloom_pass(downsample: u32) -> gpui::SubtreeEffectPass {
    gpui::SubtreeEffectPass {
        shader: gpui_effects::subtree_identity_shader(),
        uniforms: gpui::EffectUniforms::new()
            .with_slot(0, [0.4, 0.1, 1., 0.])
            .with_slot(1, [12., 0., 0., 0.]),
        time: 0.,
        bloom: Some(gpui::SubtreeBloomPass {
            extract: gpui_effects::bloom_extract_shader(),
            blur: gpui_effects::bloom_blur_shader(),
            composite: gpui_effects::bloom_composite_shader(),
            downsample,
        }),
        feedback: None,
        distance_field: None,
        particles: None,
        images: Default::default(),
        particle_transition: None,
    }
}
