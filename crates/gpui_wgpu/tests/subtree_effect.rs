#![cfg(not(target_family = "wasm"))]

use std::{rc::Rc, sync::Arc};

use gpui::{
    Bounds, ContentMask, DevicePixels, EffectQuad, EffectShader, Primitive, Quad, ScaledPixels,
    Scene, SubtreeLayer, point, rgba, size,
};

use gpui_effects::{subtree_blur_shader, subtree_color_adjust_shader, subtree_wave_shader};
use gpui_wgpu::WgpuOffscreenRenderer;

#[path = "support/contour_glow.rs"]
mod contour_glow;
#[path = "support/contour_relief.rs"]
mod contour_relief;
#[path = "support/contour_shadow.rs"]
mod contour_shadow;
#[path = "support/deformation.rs"]
mod deformation;
#[path = "support/displacement_map.rs"]
mod displacement_map;
#[path = "support/feedback.rs"]
mod feedback;
#[path = "support/fluid.rs"]
mod fluid;
#[path = "support/holographic.rs"]
mod holographic;
#[path = "support/interaction_mapping.rs"]
mod interaction_mapping;
#[path = "support/particle_mask.rs"]
mod particle_mask;
#[path = "support/particle_transition.rs"]
mod particle_transition;
#[path = "support/particles.rs"]
mod particles;
#[path = "support/path_morph.rs"]
mod path_morph;
#[path = "support/path_motion.rs"]
mod path_motion;
#[path = "support/sdf.rs"]
mod sdf;

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
    check_builtin_neutral_states(&mut renderer)?;
    check_bloom_highlights(&mut renderer)?;
    check_effect_chains(&mut renderer)?;
    check_bloom_spread_and_highlight_contrast(&mut renderer)?;
    feedback::check(&mut renderer)?;
    particles::check(&mut renderer)?;
    particle_mask::check(&mut renderer)?;
    particle_transition::check(&mut renderer)?;
    fluid::check(&mut renderer)?;
    sdf::check(&mut renderer)?;
    path_motion::check(&mut renderer)?;
    holographic::check(&mut renderer)?;
    path_morph::check(&mut renderer)?;
    deformation::check(&mut renderer)?;
    interaction_mapping::check(&mut renderer)?;
    displacement_map::check(&mut renderer)?;
    contour_glow::check(&mut renderer)?;
    contour_relief::check(&mut renderer)?;
    contour_shadow::check(&mut renderer)
}

fn check_bloom_spread_and_highlight_contrast(
    renderer: &mut WgpuOffscreenRenderer,
) -> anyhow::Result<()> {
    renderer.resize(size(DevicePixels(160), DevicePixels(100)));
    let region = bounds(0., 0., 160., 100.);
    let render_scene = |content: &[Quad], options: gpui_effects::BloomOptions| {
        let mut source = Scene::default();
        for quad in content {
            source.insert_primitive(*quad);
        }
        let Primitive::SubtreeLayer(mut captured) = layer(source, region, 1.) else {
            unreachable!()
        };
        let mut effect = bloom_pass(options.downsample);
        effect.uniforms.set_slot(
            0,
            [options.threshold, options.soft_knee, options.intensity, 0.],
        );
        effect
            .uniforms
            .set_slot(1, [f32::from(options.radius), 0., 0., 0.]);
        captured.intermediate_effects = vec![effect].into();
        let mut scene = Scene::default();
        scene.insert_primitive(quad(region, 0x101010ff));
        scene.insert_primitive(Primitive::SubtreeLayer(captured));
        scene.finish();
        scene
    };
    let pixel = |x: usize, y: usize| (y * 160 + x) * 4;
    let thin = [quad(bounds(80., 30., 2., 40.), 0x9aeeffff)];
    let glow = renderer.render_rgba(&render_scene(&thin, Default::default()))?;
    let baseline = renderer.render_rgba(&render_scene(
        &thin,
        gpui_effects::BloomOptions {
            intensity: 0.,
            ..Default::default()
        },
    ))?;
    assert!(
        glow[pixel(64, 50) + 1] > baseline[pixel(64, 50) + 1] + 4,
        "thin highlight lost its near glow"
    );
    assert!(
        glow[pixel(50, 50) + 1] > baseline[pixel(50, 50) + 1],
        "thin highlight lost its outer glow"
    );
    let inner = u16::from(glow[pixel(76, 50) + 1].saturating_sub(baseline[pixel(76, 50) + 1]));
    let shoulder = u16::from(glow[pixel(64, 50) + 1].saturating_sub(baseline[pixel(64, 50) + 1]));
    assert!(
        inner > 2 * shoulder,
        "highlight falloff is too flat: {inner}, {shoulder}"
    );

    let tones = [
        quad(bounds(30., 30., 30., 30.), 0xe0e0e0ff),
        quad(bounds(90., 30., 30., 30.), 0xf0f0f0ff),
    ];
    let options = gpui_effects::BloomOptions {
        radius: gpui::px(12.),
        ..Default::default()
    };
    let glow = renderer.render_rgba(&render_scene(&tones, options))?;
    let lower = glow[pixel(45, 45)];
    let upper = glow[pixel(105, 45)];
    assert!(
        upper < 255 && upper > lower + 3,
        "highlight contrast clipped: {lower}, {upper}"
    );
    Ok(())
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

fn check_bloom_highlights(renderer: &mut WgpuOffscreenRenderer) -> anyhow::Result<()> {
    for extent in [(64, 48), (65, 49), (64, 48)] {
        renderer.resize(size(DevicePixels(extent.0), DevicePixels(extent.1)));
        for downsample in [1, 2, 4] {
            let region = bounds(3., 3., 54., 40.);
            let source = || {
                let mut scene = Scene::default();
                scene.insert_primitive(quad(bounds(16., 15., 8., 10.), 0xff4000ff));
                scene.insert_primitive(quad(bounds(42., 15., 8., 10.), 0x182030ff));
                scene
            };
            let scene = |effect: Option<gpui::SubtreeEffectPass>| {
                let Primitive::SubtreeLayer(mut captured) = layer(source(), region, 1.) else {
                    unreachable!()
                };
                if let Some(effect) = effect {
                    captured.intermediate_effects = vec![effect].into();
                }
                let mut scene = Scene::default();
                scene.insert_primitive(quad(
                    bounds(0., 0., extent.0 as f32, extent.1 as f32),
                    0x101010ff,
                ));
                scene.insert_primitive(Primitive::SubtreeLayer(captured));
                scene.finish();
                scene
            };
            let expected = renderer.render_rgba(&scene(None))?;
            let glow = renderer.render_rgba(&scene(Some(bloom_pass(downsample))))?;
            let pixel = |x: usize, y: usize| (y * extent.0 as usize + x) * 4;
            let halo = pixel(13, 20);
            assert!(
                glow[halo] > expected[halo] + 5,
                "missing colored halo at 1/{downsample}, {extent:?}"
            );
            assert!(
                glow[halo] > glow[halo + 2] + 5,
                "halo must retain the highlight hue"
            );
            for (x, y) in [(20, 20), (46, 20), (1, 20), (13, 4)] {
                let offset = pixel(x, y);
                if x == 20 {
                    assert!(
                        glow[offset] >= 253 && glow[offset + 2] <= 2,
                        "source detail changed"
                    );
                } else {
                    for channel in 0..4 {
                        assert!(
                            glow[offset + channel].abs_diff(expected[offset + channel]) <= 2,
                            "unlit pixels changed at {x}, {y}, divisor {downsample}"
                        );
                    }
                }
            }
            for slot in [[1., 0., 1., 0.], [0.4, 0.1, 0., 0.]] {
                let mut effect = bloom_pass(downsample);
                effect.uniforms.set_slot(0, slot);
                let actual = renderer.render_rgba(&scene(Some(effect)))?;
                let error = actual
                    .iter()
                    .zip(&expected)
                    .map(|(a, b)| a.abs_diff(*b))
                    .max()
                    .unwrap_or(0);
                assert!(error <= 2, "neutral bloom changed pixels by {error}");
            }
        }
    }
    Ok(())
}

fn check_effect_chains(renderer: &mut WgpuOffscreenRenderer) -> anyhow::Result<()> {
    let region = bounds(4., 4., 52., 40.);
    let blur = gpui::SubtreeEffectPass {
        shader: subtree_blur_shader(),
        uniforms: gpui::EffectUniforms::new().with_slot(0, [2., 0., 0., 0.]),
        time: 0.,
        bloom: None,
        feedback: None,
        distance_field: None,
        particles: None,
        images: Default::default(),
        particle_transition: None,
    };
    let color = gpui::SubtreeEffectPass {
        shader: subtree_color_adjust_shader(),
        uniforms: gpui::EffectUniforms::new().with_slot(0, [0.7, 1.8, 1.1, 0.]),
        time: 0.,
        bloom: None,
        feedback: None,
        distance_field: None,
        particles: None,
        images: Default::default(),
        particle_transition: None,
    };
    let available = [blur, color, bloom_pass(4)];
    for count in [1, 2, 3, 8] {
        for reverse in [false, true] {
            let stages = (0..count)
                .map(|i| available[if reverse { 2 - i % 3 } else { i % 3 }].clone())
                .collect::<Vec<_>>();
            let source = || {
                let mut scene = Scene::default();
                scene.insert_primitive(quad(bounds(10., 10., 20., 22.), 0xf02080a0));
                scene.insert_primitive(quad(bounds(22., 18., 22., 18.), 0x30e040b0));
                scene
            };
            let Primitive::SubtreeLayer(mut captured) = layer(source(), region, 0.6) else {
                unreachable!()
            };
            let last = stages.last().unwrap();
            captured.composite.shader = last.shader.clone();
            captured.composite.uniforms = last.uniforms;
            let intermediate_count = if last.bloom.is_some() {
                count
            } else {
                count - 1
            };
            // Match the nested reference's resolve passes and UNORM quantization.
            let mut intermediate = Vec::new();
            for (index, stage) in stages[..intermediate_count].iter().enumerate() {
                intermediate.push(stage.clone());
                if stage.bloom.is_some() && index + 1 < count {
                    intermediate.push(gpui::SubtreeEffectPass {
                        shader: gpui_effects::subtree_identity_shader(),
                        uniforms: Default::default(),
                        time: 0.,
                        bloom: None,
                        feedback: None,
                        distance_field: None,
                        particles: None,
                        images: Default::default(),
                        particle_transition: None,
                    });
                }
            }
            captured.intermediate_effects = intermediate.into();
            let mut chained = Scene::default();
            chained.insert_primitive(Primitive::SubtreeLayer(captured));
            chained.finish();
            assert_eq!(chained.subtree_depth(), 1);
            assert_eq!(
                chained.subtree_target_count(),
                if intermediate_count == 0 { 1 } else { 2 }
            );
            let actual = renderer.render_rgba(&chained)?;

            let mut nested = source();
            for (index, stage) in stages.iter().enumerate() {
                let Primitive::SubtreeLayer(mut captured) =
                    layer(nested, region, if index + 1 == count { 0.6 } else { 1. })
                else {
                    unreachable!()
                };
                captured.composite.shader = stage.shader.clone();
                captured.composite.uniforms = stage.uniforms;
                if stage.bloom.is_some() {
                    captured.intermediate_effects = vec![stage.clone()].into();
                }
                nested = Scene::default();
                nested.insert_primitive(Primitive::SubtreeLayer(captured));
            }
            nested.finish();
            let expected = renderer.render_rgba(&nested)?;
            let error = actual
                .iter()
                .zip(&expected)
                .map(|(a, b)| a.abs_diff(*b))
                .max()
                .unwrap_or(0);
            assert!(
                error <= 3,
                "{count} stages, reverse {reverse}: channel error {error}"
            );
        }
    }
    Ok(())
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
