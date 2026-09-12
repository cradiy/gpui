use super::*;
use gpui::{AtlasTile, EffectUniforms, ImageId, RenderImageParams, SubtreeEffectPass};
use gpui_effects::{displacement_map_shader, masked_displacement_map_shader};

fn tile(
    renderer: &WgpuOffscreenRenderer,
    id: usize,
    width: i32,
    bytes: &[u8],
) -> anyhow::Result<AtlasTile> {
    renderer
        .sprite_atlas()
        .get_or_insert_with(
            &RenderImageParams {
                image_id: ImageId(91000 + id),
                frame_index: 0,
            }
            .into(),
            &mut || {
                Ok(Some((
                    size(DevicePixels(width), DevicePixels(1)),
                    std::borrow::Cow::Borrowed(bytes),
                )))
            },
        )?
        .ok_or_else(|| anyhow::anyhow!("missing image tile"))
}

fn stage(map: AtlasTile, mask: Option<AtlasTile>, amplitude: f32) -> SubtreeEffectPass {
    SubtreeEffectPass {
        shader: if mask.is_some() {
            masked_displacement_map_shader()
        } else {
            displacement_map_shader()
        },
        images: match mask {
            Some(mask) => smallvec::smallvec![map, mask, map],
            None => smallvec::smallvec![map],
        },
        uniforms: EffectUniforms::new()
            .with_slot(0, [amplitude, 0., 0., 0.])
            .with_slot(1, [1., 1., 0., 0.]),
        time: 0.,
        bloom: None,
        feedback: None,
        distance_field: None,
        particles: None,
        particle_transition: None,
    }
}

fn scene(scale: f32, passes: Vec<SubtreeEffectPass>) -> Scene {
    let mut source = Scene::default();
    source.insert_primitive(quad(
        bounds(36. * scale, 28. * scale, 12. * scale, 12. * scale),
        0xff804080,
    ));
    let Primitive::SubtreeLayer(mut captured) = layer(
        source,
        bounds(8. * scale, 8. * scale, 80. * scale, 56. * scale),
        1.,
    ) else {
        unreachable!()
    };
    captured.intermediate_effects = passes.into();
    let mut scene = Scene::default();
    scene.insert_primitive(Primitive::SubtreeLayer(captured));
    scene.finish();
    scene
}

pub(super) fn check(renderer: &mut WgpuOffscreenRenderer) -> anyhow::Result<()> {
    let neutral = tile(renderer, 0, 1, &[0, 128, 128, 255])?;
    let right = tile(renderer, 1, 1, &[0, 128, 255, 255])?;
    let clear = tile(renderer, 2, 1, &[0, 128, 255, 0])?;
    let white = tile(renderer, 3, 1, &[255, 255, 255, 255])?;
    let black = tile(renderer, 4, 1, &[0, 0, 0, 255])?;
    let halves = tile(renderer, 5, 2, &[0, 128, 0, 255, 0, 128, 255, 255])?;
    let translucent = tile(renderer, 6, 1, &[0, 64, 128, 128])?;
    let mask_bytes = (0..16)
        .flat_map(|x| if x < 6 { [0, 0, 0, 255] } else { [255; 4] })
        .collect::<Vec<_>>();
    let local_mask = tile(renderer, 7, 16, &mask_bytes)?;
    for scale in [1., 1.5, 2.] {
        let width = (96. * scale) as usize;
        renderer.resize(size(
            DevicePixels(width as i32),
            DevicePixels((72. * scale) as i32),
        ));
        let original = renderer.render_rgba(&scene(scale, vec![]))?;
        for pass in [
            stage(neutral, None, 8. * scale),
            stage(right, None, 0.),
            stage(clear, None, 8. * scale),
            stage(right, Some(black), 8. * scale),
        ] {
            assert_eq!(original, renderer.render_rgba(&scene(scale, vec![pass]))?);
        }
        let shifted = renderer.render_rgba(&scene(scale, vec![stage(right, None, 8. * scale)]))?;
        let pixel = |data: &[u8], x: f32, y: f32| {
            let offset = (((y * scale) as usize) * width + (x * scale) as usize) * 4;
            data[offset..offset + 4].to_vec()
        };
        assert_eq!(pixel(&shifted, 30., 32.), pixel(&original, 38., 32.));
        assert_eq!(pixel(&shifted, 44., 32.)[..3], [0, 0, 0]);
        assert_eq!(
            shifted,
            renderer.render_rgba(&scene(scale, vec![stage(right, Some(white), 8. * scale)]))?
        );
        let twice = renderer.render_rgba(&scene(
            scale,
            vec![
                stage(right, None, 4. * scale),
                stage(right, None, 4. * scale),
            ],
        ))?;
        assert_eq!(shifted, twice);
        let local = renderer.render_rgba(&scene(
            scale,
            vec![stage(right, Some(local_mask), 8. * scale)],
        ))?;
        assert_eq!(pixel(&local, 30., 32.), pixel(&original, 30., 32.));
        assert_ne!(pixel(&local, 30., 32.), pixel(&shifted, 30., 32.));
        assert_eq!(pixel(&local, 44., 32.), pixel(&shifted, 44., 32.));
        assert_ne!(pixel(&local, 44., 32.), pixel(&original, 44., 32.));

        let mut edge = scene(scale, vec![stage(right, None, 8. * scale)]);
        let mut filled = Scene::default();
        filled.insert_primitive(quad(
            bounds(8. * scale, 8. * scale, 80. * scale, 56. * scale),
            0xffffffff,
        ));
        filled.finish();
        edge.subtree_layers[0].scene = Rc::new(filled);
        let transparent_edge = renderer.render_rgba(&edge)?;
        assert_eq!(pixel(&transparent_edge, 84., 32.)[..3], [0; 3]);
        Arc::make_mut(&mut edge.subtree_layers[0].intermediate_effects)[0]
            .uniforms
            .set_slot(2, [0., 0., 0., 1.]);
        let clamped_edge = renderer.render_rgba(&edge)?;
        assert_eq!(pixel(&clamped_edge, 84., 32.)[..3], [255; 3]);
        assert_eq!(pixel(&clamped_edge, 90., 32.)[..3], [0; 3]);
        let mut clipped = scene(scale, vec![stage(right, None, 8. * scale)]);
        clipped.subtree_layers[0].composite.content_mask.bounds =
            bounds(32. * scale, 0., 64. * scale, 72. * scale);
        let clipped = renderer.render_rgba(&clipped)?;
        assert_eq!(pixel(&clipped, 30., 32.)[..3], [0, 0, 0]);
        assert_eq!(pixel(&clipped, 34., 32.), pixel(&shifted, 34., 32.));

        for offset in [-100., 100.] {
            let mut pass = stage(neutral, None, 8. * scale);
            pass.uniforms.set_slot(1, [1., 1., offset, offset]);
            assert_eq!(
                original,
                renderer.render_rgba(&scene(scale, vec![pass]))?,
                "atlas edges must not leak adjacent images"
            );
        }
        let mut pass = stage(halves, None, 8. * scale);
        pass.uniforms.set_slot(2, [0.25, 0., 1., 0.]);
        let still = renderer.render_rgba(&scene(scale, vec![pass.clone()]))?;
        pass.time = 2.;
        assert_ne!(still, renderer.render_rgba(&scene(scale, vec![pass]))?);

        let mut probe = stage(halves, None, 0.);
        probe.shader = EffectShader::wgsl_two_images(
            "fn effect(input: EffectInput, params: EffectParams) -> vec4<f32> { return sample_effect_second_image_repeat(input, vec2<f32>(0.0, 0.5)); }",
        );
        let seam = renderer.render_rgba(&scene(scale, vec![probe]))?;
        assert!(
            pixel(&seam, 40., 32.)[0].abs_diff(if OUTPUT_SRGB { 188 } else { 128 }) <= 1,
            "repeat filtering must wrap within the image tile"
        );

        let mut probe = stage(translucent, None, 0.);
        probe.shader = EffectShader::wgsl_two_images(
            "fn effect(input: EffectInput, params: EffectParams) -> vec4<f32> { return sample_effect_second_image(input, input.uv); }",
        );
        let raw = renderer.render_rgba(&scene(scale, vec![probe]))?;
        let center = pixel(&raw, 40., 32.);
        assert!(
            center[0].abs_diff(if OUTPUT_SRGB { 137 } else { 64 }) <= 1
                && center[1].abs_diff(if OUTPUT_SRGB { 99 } else { 32 }) <= 1,
            "external images keep straight alpha: {center:?}"
        );
    }
    Ok(())
}
