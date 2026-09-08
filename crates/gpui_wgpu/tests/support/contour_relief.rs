use super::*;
use gpui::{EffectUniforms, SubtreeDistanceFieldPass, SubtreeEffectPass};
use gpui_effects::{contour_relief_shader, subtree_identity_shader};

fn scene(scale: f32, depth: f32, light_x: f32, opacity: f32, empty: bool) -> Scene {
    let region = bounds(8. * scale, 8. * scale, 112. * scale, 112. * scale);
    let mut source = Scene::default();
    if !empty {
        for [x, y, w, h] in [
            [32., 32., 64., 16.],
            [32., 80., 64., 16.],
            [32., 48., 16., 32.],
            [80., 48., 16., 32.],
        ] {
            source.insert_primitive(quad(
                bounds((x + 0.25) * scale, (y + 0.25) * scale, w * scale, h * scale),
                0x707070ff,
            ));
        }
    }
    let Primitive::SubtreeLayer(mut captured) = layer(source, region, opacity) else {
        unreachable!()
    };
    captured.intermediate_effects = vec![SubtreeEffectPass {
        shader: subtree_identity_shader(),
        uniforms: EffectUniforms::new()
            .with_slot(0, [5. * scale, depth * scale, 0.75 * scale, 0.])
            .with_slot(1, [light_x, 0., 0.6, 1.2])
            .with_slot(2, [0.4, 0.65, 1., 0.3])
            .with_slot(3, [1., 1., 1., 0.]),
        time: 0.,
        bloom: None,
        feedback: None,
        distance_field: Some(SubtreeDistanceFieldPass {
            threshold: 0.5,
            composite: contour_relief_shader(),
        }),
    }]
    .into();
    let mut scene = Scene::default();
    scene.insert_primitive(Primitive::SubtreeLayer(captured));
    scene.finish();
    scene
}

pub(super) fn check(renderer: &mut WgpuOffscreenRenderer) -> anyhow::Result<()> {
    let mut baseline: Option<f32> = None;
    for scale in [1., 1.5, 2.] {
        let width = (128. * scale) as usize;
        renderer.resize(size(DevicePixels(width as i32), DevicePixels(width as i32)));
        let pixel = |x: f32, y: f32| (((y * scale) as usize) * width + (x * scale) as usize) * 4;
        let raised = renderer.render_rgba(&scene(scale, 3., -0.8, 1., false))?;
        let recessed = renderer.render_rgba(&scene(scale, -3., -0.8, 1., false))?;
        let from_right = renderer.render_rgba(&scene(scale, 3., 0.8, 1., false))?;
        let flat = renderer.render_rgba(&scene(scale, 0., -0.8, 1., false))?;
        let mut identity = scene(scale, 0., -0.8, 1., false);
        identity.subtree_layers[0].intermediate_effects = Arc::default();
        assert_eq!(flat, renderer.render_rgba(&identity)?);
        let left = pixel(34., 64.);
        let right = pixel(93., 64.);
        assert!(i32::from(raised[left]) - i32::from(raised[right]) > 30);
        assert!(i32::from(recessed[right]) - i32::from(recessed[left]) > 30);
        assert!(i32::from(from_right[right]) - i32::from(from_right[left]) > 30);
        assert!(i32::from(raised[pixel(81., 64.)]) - i32::from(raised[pixel(46., 64.)]) > 30);
        for ((lit, inset), source) in raised
            .chunks_exact(4)
            .zip(recessed.chunks_exact(4))
            .zip(flat.chunks_exact(4))
        {
            if source[..3] == [0; 3] {
                assert_eq!(lit, source);
                assert_eq!(inset, source);
            }
        }
        let alpha_shader = EffectShader::wgsl_image(
            "fn effect(input: EffectInput, params: EffectParams) -> vec4<f32> { let alpha = sample_effect_image(input, input.uv).a; return vec4<f32>(vec3<f32>(alpha), 1.0); }",
        );
        identity.subtree_layers[0].composite.shader = alpha_shader.clone();
        let expected_alpha = renderer.render_rgba(&identity)?;
        for depth in [3., -3.] {
            let mut alpha = scene(scale, depth, -0.8, 1., false);
            alpha.subtree_layers[0].composite.shader = alpha_shader.clone();
            assert_eq!(expected_alpha, renderer.render_rgba(&alpha)?);
        }
        let faded = renderer.render_rgba(&scene(scale, 3., -0.8, 0.5, false))?;
        assert!(faded[left] > 0 && faded[left] < raised[left]);
        let mut clipped = scene(scale, 3., -0.8, 1., false);
        clipped.subtree_layers[0].composite.content_mask.bounds =
            bounds(0., 0., 64. * scale, 128. * scale);
        assert_eq!(renderer.render_rgba(&clipped)?[right], 0);
        let empty = renderer.render_rgba(&scene(scale, 3., -0.8, 1., true))?;
        assert!(empty.chunks_exact(4).all(|pixel| pixel[..3] == [0; 3]));
        let mut total = 0u32;
        let mut count = 0u32;
        for y in (60. * scale) as usize..(68. * scale) as usize {
            for x in (32. * scale) as usize..(40. * scale) as usize {
                total += u32::from(raised[(y * width + x) * 4]);
                count += 1;
            }
        }
        let average = total as f32 / count as f32;
        if let Some(value) = baseline {
            assert!(
                (average - value).abs() < 8.,
                "bevel brightness at scale {scale}: {average} vs {value}"
            );
        } else {
            baseline = Some(average);
        }
    }
    Ok(())
}
