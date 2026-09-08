use super::*;
use gpui::{
    EffectHistoryId, ParticleFrame, ParticleMask, ParticlePhysics, ParticleSpawn,
    SubtreeEffectPass, SubtreeParticlePass, px, rgb,
};
use std::time::Duration;

fn scene(
    frame: &ParticleFrame,
    scale: f32,
    mask: ParticleMask,
    source: bool,
    opacity: f32,
) -> Scene {
    let mut child = Scene::default();
    if source {
        for (rect, color) in [
            ([20., 24., 32., 12.], 0xff0000ff),
            ([20., 60., 32., 12.], 0xff0000ff),
            ([20., 36., 10., 24.], 0xff0000ff),
            ([42., 36., 10., 24.], 0xff0000ff),
            ([76., 24., 28., 48.], 0x0000ffff),
        ] {
            let [x, y, w, h] = rect.map(|v| v * scale);
            child.insert_primitive(quad(bounds(x, y, w, h), color));
        }
    }
    let Primitive::SubtreeLayer(mut captured) = layer(
        child,
        bounds(8. * scale, 8. * scale, 112. * scale, 112. * scale),
        opacity,
    ) else {
        unreachable!()
    };
    captured.intermediate_effects = vec![SubtreeEffectPass {
        shader: gpui_effects::subtree_identity_shader(),
        uniforms: Default::default(),
        time: 0.,
        bloom: None,
        feedback: None,
        distance_field: None,
        images: Default::default(),
        particle_transition: None,
        particles: Some(SubtreeParticlePass {
            frame: Arc::new(frame.clone()),
            mask,
            scale_factor: scale,
        }),
    }]
    .into();
    let mut scene = Scene::default();
    scene.insert_primitive(Primitive::SubtreeLayer(captured));
    scene.finish();
    scene
}

pub(super) fn check(renderer: &mut WgpuOffscreenRenderer) -> anyhow::Result<()> {
    for scale in [1., 1.5, 2.] {
        let width = (128. * scale) as usize;
        renderer.resize(size(DevicePixels(width as i32), DevicePixels(width as i32)));
        for (edge, inherit) in [(0., true), (2., true), (0., false)] {
            let mask = ParticleMask {
                edge_width: px(edge),
                inherit_color: inherit,
                ..Default::default()
            };
            let spawn = ParticleSpawn {
                count: 2048,
                velocity: point(px(0.), px(0.)),
                speed: px(0.)..px(0.),
                radius: px(0.5)..px(0.5),
                lifetime: Duration::from_secs(1)..Duration::from_secs(1),
                color: rgb(0x00ff00),
                ..Default::default()
            };
            let mut frame = ParticleFrame {
                id: EffectHistoryId::new(),
                generation: 0,
                frame: 1,
                time: Duration::ZERO,
                capacity: 2048,
                physics: ParticlePhysics {
                    acceleration: point(px(0.), px(0.)),
                    drag: 0.,
                    ..Default::default()
                },
                spawns: vec![spawn.clone()].into(),
                needs_animation: true,
            };
            let first = scene(&frame, scale, mask, true, 1.);
            let mut identity = scene(&frame, scale, mask, true, 1.);
            identity.subtree_layers[0].intermediate_effects = Arc::default();
            let expected = renderer.render_rgba(&identity)?;
            assert_eq!(
                expected,
                renderer.render_rgba(&first)?,
                "emission preserves source pixels"
            );
            frame.frame += 1;
            frame.time = Duration::from_millis(100);
            frame.spawns = Arc::default();
            let visible = scene(&frame, scale, mask, false, 1.);
            let pixels = renderer.render_rgba(&visible)?;
            let sum = |pixels: &[u8], area: [f32; 4], channel: usize| -> u64 {
                let [left, top, right, bottom] = area.map(|v| (v * scale) as usize);
                (top..bottom)
                    .flat_map(|y| {
                        (left..right).map(move |x| u64::from(pixels[(y * width + x) * 4 + channel]))
                    })
                    .sum()
            };
            let left = [18., 22., 54., 74.];
            let right = [74., 22., 106., 74.];
            assert!(sum(&pixels, left, usize::from(!inherit)) > 1000);
            assert!(sum(&pixels, right, if inherit { 2 } else { 1 }) > 1000);
            for channel in 0..3 {
                assert_eq!(
                    sum(&pixels, [56., 20., 70., 80.], channel),
                    0,
                    "transparent gap emits nothing"
                );
                assert_eq!(
                    sum(&pixels, [33., 40., 39., 56.], channel),
                    0,
                    "shape holes emit nothing"
                );
            }
            if inherit {
                assert_eq!(sum(&pixels, left, 2), 0);
                assert_eq!(sum(&pixels, right, 0), 0);
                let center = sum(&pixels, [86., 40., 94., 56.], 2);
                if edge > 0. {
                    assert_eq!(center, 0);
                } else {
                    assert!(center > 100);
                }
            } else {
                assert_eq!(sum(&pixels, left, 0) + sum(&pixels, right, 2), 0);
            }
            assert_eq!(
                pixels,
                renderer.render_rgba(&visible)?,
                "replay preserves particle positions"
            );
            let faded = renderer.render_rgba(&scene(&frame, scale, mask, false, 0.5))?;
            let channel = usize::from(!inherit);
            assert!(sum(&faded, left, channel) < sum(&pixels, left, channel));
            let mut clipped = scene(&frame, scale, mask, false, 1.);
            clipped.subtree_layers[0].composite.content_mask.bounds =
                bounds(0., 0., 64. * scale, 128. * scale);
            let clipped = renderer.render_rgba(&clipped)?;
            for channel in 0..3 {
                assert_eq!(sum(&clipped, right, channel), 0);
            }
            frame.frame += 1;
            frame.spawns = vec![spawn.clone()].into();
            assert_eq!(
                pixels,
                renderer.render_rgba(&scene(&frame, scale, mask, false, 1.))?,
                "empty emissions preserve live particles"
            );
            frame.spawns = Arc::default();
            frame.generation += 1;
            frame.frame += 1;
            let cleared = renderer.render_rgba(&scene(&frame, scale, mask, false, 1.))?;
            assert!(cleared.chunks_exact(4).all(|p| p[..3] == [0; 3]));
            frame.frame += 1;
            frame.spawns = vec![spawn].into();
            renderer.render_rgba(&scene(&frame, scale, mask, false, 1.))?;
            frame.frame += 1;
            frame.time += Duration::from_millis(100);
            frame.spawns = Arc::default();
            let empty = renderer.render_rgba(&scene(&frame, scale, mask, false, 1.))?;
            assert_eq!(cleared, empty, "empty masks cannot reuse old candidates");
        }
    }
    Ok(())
}
