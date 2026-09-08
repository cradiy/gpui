use super::*;
use gpui::{EffectUniforms, SubtreeDistanceFieldPass, SubtreeEffectPass};
use gpui_effects::{contour_glow_shader, subtree_identity_shader};

fn ring(scale: f32) -> Scene {
    let mut scene = Scene::default();
    for [x, y, w, h] in [
        [32., 32., 64., 16.],
        [32., 80., 64., 16.],
        [32., 48., 16., 32.],
        [80., 48., 16., 32.],
    ] {
        scene.insert_primitive(quad(
            bounds(x * scale, y * scale, w * scale, h * scale),
            0xffffffff,
        ));
    }
    scene
}

fn capture(
    source: Scene,
    region: Bounds<ScaledPixels>,
    shader: EffectShader,
    uniforms: EffectUniforms,
) -> Primitive {
    let Primitive::SubtreeLayer(mut captured) = layer(source, region, 1.) else {
        unreachable!()
    };
    captured.intermediate_effects = vec![SubtreeEffectPass {
        shader: subtree_identity_shader(),
        uniforms,
        time: 0.,
        bloom: None,
        feedback: None,
        distance_field: Some(SubtreeDistanceFieldPass {
            threshold: 0.5,
            composite: shader,
        }),
    }]
    .into();
    Primitive::SubtreeLayer(captured)
}

fn root(primitive: Primitive, scale: f32) -> Scene {
    let mut scene = Scene::default();
    scene.insert_primitive(quad(bounds(0., 0., 160. * scale, 128. * scale), 0x000000ff));
    scene.insert_primitive(primitive);
    scene.finish();
    scene
}

pub(super) fn check(renderer: &mut WgpuOffscreenRenderer) -> anyhow::Result<()> {
    let visualize = EffectShader::wgsl_two_images(
        "fn effect(input: EffectInput, params: EffectParams) -> vec4<f32> { let field = sample_effect_second_image(input, input.uv); return vec4<f32>(clamp(0.5 + field.r / (64.0 * params.slots[3].x), 0.0, 1.0), field.g, 0.0, 1.0); }",
    );
    let analytic = EffectShader::wgsl_image(
        "fn box_distance(p: vec2<f32>, half_size: f32) -> f32 { let q = abs(p - vec2<f32>(64.0)) - half_size; return length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0); } fn effect(input: EffectInput, params: EffectParams) -> vec4<f32> { let p = input.position / params.slots[3].x; let distance = max(box_distance(p, 32.0), -box_distance(p, 16.0)); return vec4<f32>(clamp(0.5 + distance / 64.0, 0.0, 1.0), 1.0, 0.0, 1.0); }",
    );
    for scale in [1., 1.5, 2., 1.] {
        let width = (160. * scale) as usize;
        renderer.resize(size(
            DevicePixels(width as i32),
            DevicePixels((128. * scale) as i32),
        ));
        for origin in [8., -8.] {
            let region = bounds(origin * scale, origin * scale, 144. * scale, 136. * scale);
            let uniforms = EffectUniforms::new().with_slot(3, [scale, 0., 0., 0.]);
            let actual = renderer.render_rgba(&root(
                capture(ring(scale), region, visualize.clone(), uniforms),
                scale,
            ))?;
            let Primitive::SubtreeLayer(mut reference) = layer(ring(scale), region, 1.) else {
                unreachable!()
            };
            reference.composite.shader = analytic.clone();
            reference.composite.uniforms = uniforms;
            let expected =
                renderer.render_rgba(&root(Primitive::SubtreeLayer(reference), scale))?;
            for y in (12. * scale) as usize..(116. * scale) as usize {
                for x in (12. * scale) as usize..(124. * scale) as usize {
                    let pixel = (y * width + x) * 4;
                    assert!(
                        actual[pixel].abs_diff(expected[pixel]) <= 5,
                        "distance at ({x}, {y}), scale {scale}: {} vs {}",
                        actual[pixel],
                        expected[pixel]
                    );
                    assert_eq!(actual[pixel + 1], expected[pixel + 1]);
                }
            }
        }
        let region = bounds(8. * scale, 8. * scale, 112. * scale, 112. * scale);
        let uniforms = EffectUniforms::new()
            .with_slot(0, [0.25, 0.8, 1., 1.])
            .with_slot(1, [10. * scale, 1.5 * scale, 0., 0.])
            .with_slot(2, [1.3, 0., 0., 0.]);
        let scene = root(
            capture(ring(scale), region, contour_glow_shader(), uniforms),
            scale,
        );
        let pixels = renderer.render_rgba(&scene)?;
        let pixel = |x: f32, y: f32| (((y * scale) as usize) * width + (x * scale) as usize) * 4;
        assert_eq!(&pixels[pixel(40., 64.)..pixel(40., 64.) + 3], &[255; 3]);
        assert_eq!(&pixels[pixel(64., 64.)..pixel(64., 64.) + 3], &[0; 3]);
        assert_eq!(&pixels[pixel(16., 64.)..pixel(16., 64.) + 3], &[0; 3]);
        assert!(pixels[pixel(30., 64.) + 2] > 20);
        assert!(pixels[pixel(49., 64.) + 2] > 20);
        let mut siblings = root(
            capture(ring(scale), region, contour_glow_shader(), uniforms),
            scale,
        );
        siblings.insert_primitive(capture(
            Scene::default(),
            bounds(24. * scale, 24. * scale, 32. * scale, 48. * scale),
            contour_glow_shader(),
            uniforms,
        ));
        siblings.finish();
        assert_eq!(pixels, renderer.render_rgba(&siblings)?);
        let empty = root(
            capture(Scene::default(), region, contour_glow_shader(), uniforms),
            scale,
        );
        assert!(
            renderer
                .render_rgba(&empty)?
                .chunks_exact(4)
                .all(|p| p[..3] == [0; 3])
        );
        let mut clipped = root(
            capture(ring(scale), region, contour_glow_shader(), uniforms),
            scale,
        );
        clipped.subtree_layers[0].composite.content_mask.bounds =
            bounds(0., 0., 28. * scale, 128. * scale);
        assert_eq!(renderer.render_rgba(&clipped)?[pixel(30., 64.) + 2], 0);
    }
    Ok(())
}
