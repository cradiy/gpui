use super::*;
use gpui::{ParticleTransitionOptions, SubtreeEffectPass, SubtreeParticleTransitionPass, px};

fn scene(progress: f32, scale: f32, empty: bool, seed: u32) -> Scene {
    let mut source = Scene::default();
    if !empty {
        for [x, y, w, h] in [
            [24., 36., 30., 8.],
            [24., 60., 30., 8.],
            [24., 44., 8., 16.],
            [46., 44., 8., 16.],
        ] {
            source.insert_primitive(quad(
                bounds(x * scale, y * scale, w * scale, h * scale),
                0xff4040ff,
            ));
        }
        source.insert_primitive(quad(
            bounds(65. * scale, 40. * scale, 20. * scale, 24. * scale),
            0x4080ffff,
        ));
    }
    let Primitive::SubtreeLayer(mut layer) = layer(
        source,
        bounds(8. * scale, 8. * scale, 144. * scale, 112. * scale),
        1.,
    ) else {
        unreachable!()
    };
    layer.intermediate_effects = vec![SubtreeEffectPass {
        shader: gpui_effects::subtree_identity_shader(),
        uniforms: Default::default(),
        time: 0.,
        bloom: None,
        feedback: None,
        distance_field: None,
        particles: None,
        particle_transition: Some(SubtreeParticleTransitionPass {
            progress,
            scale_factor: scale,
            options: ParticleTransitionOptions {
                cell_size: px(2.),
                scatter: point(px(35.), px(0.)),
                spread: px(12.),
                radius: px(0.8),
                streak: px(4.),
                seed,
            },
        }),
    }]
    .into();
    let mut scene = Scene::default();
    scene.insert_primitive(Primitive::SubtreeLayer(layer));
    scene.finish();
    scene
}

pub(super) fn check(renderer: &mut WgpuOffscreenRenderer) -> anyhow::Result<()> {
    for scale in [1., 1.5, 2.] {
        let width = (160. * scale) as usize;
        renderer.resize(size(
            DevicePixels(width as i32),
            DevicePixels((128. * scale) as i32),
        ));
        let mut identity = scene(0., scale, false, 7);
        identity.subtree_layers[0].intermediate_effects = Arc::default();
        let original = renderer.render_rgba(&identity)?;
        assert_eq!(original, renderer.render_rgba(&scene(0., scale, false, 7))?);
        let early = renderer.render_rgba(&scene(0.00001, scale, false, 7))?;
        assert!(
            original
                .iter()
                .zip(&early)
                .all(|(a, b)| a.abs_diff(*b) <= 1),
            "fragment handoff preserves source pixels"
        );
        let middle = renderer.render_rgba(&scene(0.5, scale, false, 7))?;
        assert_ne!(original, middle);
        assert!(
            middle
                .chunks_exact(4)
                .any(|p| i32::from(p[0]) - i32::from(p[2]) > 20)
        );
        assert!(
            middle
                .chunks_exact(4)
                .any(|p| i32::from(p[2]) - i32::from(p[0]) > 20)
        );
        let centroid = |pixels: &[u8]| {
            let (sum, weighted) =
                pixels
                    .chunks_exact(4)
                    .enumerate()
                    .fold((0., 0.), |(sum, weighted), (i, p)| {
                        let light = f64::from(p[0]) + f64::from(p[1]) + f64::from(p[2]);
                        (sum + light, weighted + light * ((i % width) as f64 + 0.5))
                    });
            weighted / sum
        };
        assert!(centroid(&middle) > centroid(&original) + f64::from(scale) * 3.);
        let end = renderer.render_rgba(&scene(1., scale, false, 7))?;
        assert!(end.chunks_exact(4).all(|p| p[..3] == [0; 3]));
        assert_eq!(
            middle,
            renderer.render_rgba(&scene(0.5, scale, false, 7))?,
            "reverse progress retraces identical positions"
        );
        assert_eq!(
            original,
            renderer.render_rgba(&scene(0., scale, false, 7))?,
            "gathering restores the exact source"
        );
        assert_ne!(middle, renderer.render_rgba(&scene(0.5, scale, false, 19))?);
        let empty = renderer.render_rgba(&scene(0.5, scale, true, 7))?;
        assert_eq!(end, empty);
        let mut faded = scene(0.5, scale, false, 7);
        faded.subtree_layers[0].composite.opacity = 0.5;
        let faded = renderer.render_rgba(&faded)?;
        assert!(
            faded.iter().map(|&v| u64::from(v)).sum::<u64>()
                < middle.iter().map(|&v| u64::from(v)).sum()
        );
        let mut clipped = scene(0.5, scale, false, 7);
        clipped.subtree_layers[0].composite.content_mask.bounds =
            bounds(0., 0., 70. * scale, 128. * scale);
        let clipped = renderer.render_rgba(&clipped)?;
        assert!(
            clipped
                .chunks_exact(4)
                .enumerate()
                .all(|(i, p)| i % width < (70. * scale) as usize || p[..3] == [0; 3])
        );
    }
    Ok(())
}
