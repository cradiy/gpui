#![cfg(not(target_family = "wasm"))]

use std::sync::Arc;

use gpui::{
    Bounds, ContentMask, DevicePixels, EffectQuad, EffectShader, Primitive, Quad, ScaledPixels,
    Scene, SubtreeLayer, point, rgba, size,
};

use gpui_effects::{subtree_blur_shader, subtree_color_adjust_shader, subtree_wave_shader};
use gpui_wgpu::WgpuOffscreenRenderer;

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
        scene: Arc::new(scene),
    })
}

fn composed_scene(depth: usize) -> Scene {
    let region = bounds(8., 6., 24., 30.);
    let first = quad(bounds(10., 8., 18., 19.), 0xe8406080);
    let second = quad(bounds(18., 19., 12., 12.), 0x4080e0b0);
    let mut scene = Scene::default();
    scene.insert_primitive(quad(bounds(0., 0., 64., 48.), 0x203050ff));
    if depth == 0 {
        scene.insert_primitive(first);
        scene.insert_primitive(second);
    } else {
        let mut child = Scene::default();
        child.insert_primitive(first);
        child.insert_primitive(second);
        child.insert_primitive(quad(bounds(0., 0., 4., 4.), 0xff0000ff));
        for _ in 1..depth {
            let captured = layer(child, region, 1.);
            child = Scene::default();
            child.insert_primitive(captured);
        }
        scene.insert_primitive(layer(child, region, 1.));
    }
    if depth == 0 {
        scene.insert_primitive(quad(bounds(38., 10., 18., 20.), 0x70c84080));
    } else {
        let mut sibling = Scene::default();
        sibling.insert_primitive(quad(bounds(38., 10., 18., 20.), 0x70c840ff));
        scene.insert_primitive(layer(sibling, bounds(36., 8., 22., 24.), 128. / 255.));
    }
    scene.insert_primitive(quad(bounds(24., 24., 4., 4.), 0xe0b060ff));
    scene.finish();
    scene
}

#[test]
#[ignore = "requires a GPU adapter"]
fn subtree_gpu_compositing_preserves_pixels_and_reuses_targets() -> anyhow::Result<()> {
    let mut renderer = WgpuOffscreenRenderer::new(size(DevicePixels(64), DevicePixels(48)))?;
    for extent in [
        size(DevicePixels(64), DevicePixels(48)),
        size(DevicePixels(80), DevicePixels(60)),
    ] {
        renderer.resize(extent);
        let expected = renderer.render_rgba(&composed_scene(0))?;
        for depth in [1, 2, 1] {
            let actual = renderer.render_rgba(&composed_scene(depth))?;
            let max_error = actual
                .iter()
                .zip(&expected)
                .map(|(a, b)| a.abs_diff(*b))
                .max()
                .unwrap_or(0);
            assert!(
                max_error <= 3,
                "subtree depth {depth}: maximum channel error {max_error}"
            );
        }
    }
    renderer.resize(size(DevicePixels(64), DevicePixels(48)));
    check_builtin_neutral_states(&mut renderer)
}

fn check_builtin_neutral_states(renderer: &mut WgpuOffscreenRenderer) -> anyhow::Result<()> {
    let region = bounds(8., 6., 32., 30.);
    let content = quad(bounds(12., 10., 24., 22.), 0xe84060a0);
    let background = quad(bounds(0., 0., 64., 48.), 0x203050ff);
    let mut direct = Scene::default();
    direct.insert_primitive(background);
    direct.insert_primitive(content);
    direct.finish();
    let visible = renderer.render_rgba(&direct)?;

    for (shader, slot0, slot1) in [
        (subtree_blur_shader(), [0.; 4], [0.; 4]),
        (subtree_wave_shader(), [0., 24., 0., 0.], [1.; 4]),
        (subtree_color_adjust_shader(), [1.; 4], [0.; 4]),
    ] {
        let mut child = Scene::default();
        child.insert_primitive(content);
        let Primitive::SubtreeLayer(mut captured) = layer(child, region, 1.) else {
            unreachable!()
        };
        captured.composite.shader = shader;
        captured.composite.uniforms = gpui::EffectUniforms::new()
            .with_slot(0, slot0)
            .with_slot(1, slot1);
        captured.composite.time = 2.5;
        let mut scene = Scene::default();
        scene.insert_primitive(background);
        scene.insert_primitive(Primitive::SubtreeLayer(captured));
        scene.finish();
        let actual = renderer.render_rgba(&scene)?;
        let max_error = actual
            .iter()
            .zip(&visible)
            .map(|(a, b)| a.abs_diff(*b))
            .max()
            .unwrap_or(0);
        assert!(max_error <= 3, "maximum channel error {max_error}");
    }
    Ok(())
}
