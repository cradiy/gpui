use super::*;
use gpui_effects::{TransitionKind, transition_shader};

fn input(region: Bounds<ScaledPixels>, color: u32) -> Scene {
    let mut scene = Scene::default();
    scene.insert_primitive(quad(region, color));
    scene.finish();
    scene
}

fn pair(
    first: Scene,
    second: Scene,
    region: Bounds<ScaledPixels>,
    kind: TransitionKind,
    progress: f32,
    opacity: f32,
) -> Primitive {
    let Primitive::SubtreeLayer(mut captured) = layer(first, region, opacity) else {
        unreachable!()
    };
    captured.second_scene = Some(Rc::new(second));
    captured.composite.shader = transition_shader(kind);
    captured
        .composite
        .uniforms
        .set_slot(0, [progress, 6., 0.15, 0.]);
    captured.composite.uniforms.set_slot(1, [1., 0., 0., 0.]);
    Primitive::SubtreeLayer(captured)
}

pub(super) fn check(renderer: &mut WgpuOffscreenRenderer) -> anyhow::Result<()> {
    for scale in [1., 1.5, 2.] {
        let width = (64. * scale) as usize;
        renderer.resize(size(
            DevicePixels(width as i32),
            DevicePixels((48. * scale) as i32),
        ));
        let region = bounds(8. * scale, 6. * scale, 48. * scale, 36. * scale);
        let viewport = bounds(0., 0., 64. * scale, 48. * scale);
        let background = quad(viewport, 0x203040ff);
        let scene = |kind, progress, opacity| {
            let mut scene = Scene::default();
            scene.insert_primitive(background);
            scene.insert_primitive(pair(
                input(region, 0xff000080),
                input(region, 0x0000ff40),
                region,
                kind,
                progress,
                opacity,
            ));
            scene.finish();
            scene
        };
        for kind in [
            TransitionKind::CrossFade,
            TransitionKind::BlurFade,
            TransitionKind::WipeRight,
        ] {
            for (progress, color) in [(0., 0xff000080), (1., 0x0000ff40)] {
                let mut expected = Scene::default();
                expected.insert_primitive(background);
                expected.insert_primitive(layer(input(region, color), region, 1.));
                expected.finish();
                assert_eq!(
                    renderer.render_rgba(&scene(kind, progress, 1.))?,
                    renderer.render_rgba(&expected)?
                );
            }
        }
        for opacity in [0.5, 1.] {
            let mut expected = Scene::default();
            expected.insert_primitive(background);
            let mut mixed = quad(region, 0x00000000);
            mixed.background = gpui::Rgba {
                r: 2. / 3.,
                g: 0.,
                b: 1. / 3.,
                a: 96. / 255. * opacity,
            }
            .into();
            expected.insert_primitive(mixed);
            expected.finish();
            let expected = renderer.render_rgba(&expected)?;
            let actual = renderer.render_rgba(&scene(TransitionKind::CrossFade, 0.5, opacity))?;
            assert!(
                actual
                    .iter()
                    .zip(&expected)
                    .all(|(a, b)| a.abs_diff(*b) <= 2),
                "crossfade alpha or group opacity mismatch at scale {scale}"
            );
        }

        let mut nested = Scene::default();
        nested.insert_primitive(pair(
            input(region, 0x0000ffff),
            input(region, 0x00ff00ff),
            region,
            TransitionKind::CrossFade,
            1.,
            1.,
        ));
        nested.finish();
        let mut outer = Scene::default();
        outer.insert_primitive(background);
        outer.insert_primitive(pair(
            input(region, 0xff0000ff),
            nested,
            region,
            TransitionKind::CrossFade,
            1.,
            1.,
        ));
        outer.finish();
        assert_eq!(outer.subtree_target_count(), 4);
        let mut expected = Scene::default();
        expected.insert_primitive(background);
        expected.insert_primitive(quad(region, 0x00ff00ff));
        expected.finish();
        assert_eq!(
            renderer.render_rgba(&outer)?,
            renderer.render_rgba(&expected)?
        );

        let mut clipped = scene(TransitionKind::CrossFade, 0.5, 1.);
        clipped.subtree_layers[0].composite.content_mask.bounds =
            bounds(0., 0., 24. * scale, 48. * scale);
        let pixels = renderer.render_rgba(&clipped)?;
        let pixel = |x: f32, y: f32| ((y * scale) as usize * width + (x * scale) as usize) * 4;
        let background_only = renderer.render_rgba(&input(viewport, 0x203040ff))?;
        let right = pixel(40., 24.);
        assert_eq!(
            &pixels[right..right + 4],
            &background_only[right..right + 4]
        );

        let thin = bounds(28. * scale, 12. * scale, 4. * scale, 24. * scale);
        let sparse = |kind| {
            let mut scene = Scene::default();
            scene.insert_primitive(pair(
                input(thin, 0xffffffff),
                Scene::default(),
                region,
                kind,
                0.5,
                1.,
            ));
            scene.finish();
            scene
        };
        let sharp = renderer.render_rgba(&sparse(TransitionKind::CrossFade))?;
        let soft = renderer.render_rgba(&sparse(TransitionKind::BlurFade))?;
        let shoulder = ((24. * scale) as usize * width + (28. * scale) as usize - 1) * 4;
        assert_eq!(sharp[shoulder], 0);
        assert!(
            soft[shoulder] > 3,
            "blur did not spread the glyph-like edge"
        );

        let wipe = renderer.render_rgba(&scene(TransitionKind::WipeRight, 0.5, 1.))?;
        let from = renderer.render_rgba(&scene(TransitionKind::CrossFade, 0., 1.))?;
        let to = renderer.render_rgba(&scene(TransitionKind::CrossFade, 1., 1.))?;
        let left = pixel(16., 24.);
        let right = pixel(48., 24.);
        assert_eq!(&wipe[left..left + 4], &to[left..left + 4]);
        assert_eq!(&wipe[right..right + 4], &from[right..right + 4]);
    }
    Ok(())
}
