use super::*;
use gpui::{EffectHistoryId, FluidDraw, FluidFrame, FluidOptions, FluidSplat, px, rgb};
use std::time::Duration;

fn scene(frame: &FluidFrame, opacity: f32) -> Scene {
    let mut scene = Scene::default();
    scene.insert_primitive(quad(bounds(0., 0., 64., 48.), 0x000000ff));
    scene.insert_primitive(FluidDraw {
        order: 0,
        bounds: bounds(4., 4., 56., 40.),
        content_mask: ContentMask {
            bounds: bounds(4., 4., 56., 40.),
        },
        scale_factor: 1.,
        opacity,
        frame: Arc::new(frame.clone()),
    });
    scene.finish();
    scene
}
fn center(pixels: &[u8]) -> f64 {
    let (sum, weighted) =
        pixels
            .chunks_exact(4)
            .enumerate()
            .fold((0., 0.), |(sum, weighted), (i, p)| {
                let value = f64::from(p[0]);
                (sum + value, weighted + value * (i % 64) as f64)
            });
    assert!(sum > 0.);
    weighted / sum
}
pub(super) fn check(renderer: &mut WgpuOffscreenRenderer) -> anyhow::Result<()> {
    renderer.resize(size(DevicePixels(64), DevicePixels(48)));
    let mut frame = FluidFrame {
        id: EffectHistoryId::new(),
        generation: 0,
        frame: 0,
        time: Duration::ZERO,
        options: FluidOptions {
            resolution: 64,
            dye_decay: 0.,
            vorticity: 0.,
            velocity_decay: 0.,
            ..Default::default()
        },
        splats: Arc::default(),
        needs_animation: false,
    };
    let empty = renderer.render_rgba(&scene(&frame, 1.))?;
    assert!(empty.chunks_exact(4).all(|p| p[..3] == [0, 0, 0]));
    frame.frame += 1;
    frame.splats = vec![FluidSplat {
        from: point(px(15.), px(20.)),
        to: point(px(24.), px(20.)),
        radius: px(4.),
        velocity: point(px(90.), px(0.)),
        amount: 2.,
        color: rgb(0xff0000),
    }]
    .into();
    let injected = renderer.render_rgba(&scene(&frame, 1.))?;
    for x in 19..28 {
        assert!(
            injected[(24 * 64 + x) * 4] > 100,
            "line injection must cover the entire segment"
        );
    }
    assert!(injected.chunks_exact(4).all(|p| p[1] == 0 && p[2] == 0));
    assert_eq!(
        injected,
        renderer.render_rgba(&scene(&frame, 1.))?,
        "replay must not reinject dye"
    );
    let faded = renderer.render_rgba(&scene(&frame, 0.5))?;
    assert!(faded[(24 * 64 + 22) * 4] < injected[(24 * 64 + 22) * 4]);
    assert_eq!(
        injected,
        renderer.render_rgba(&scene(&frame, 1.))?,
        "opacity must not change simulation state"
    );
    frame.splats = Arc::default();
    let mut moved = injected.clone();
    for _ in 0..8 {
        frame.frame += 1;
        frame.time += Duration::from_millis(16);
        moved = renderer.render_rgba(&scene(&frame, 1.))?;
    }
    assert!(
        center(&moved) > center(&injected) + 0.5,
        "velocity must transport dye"
    );
    let mut clipped = scene(&frame, 1.);
    clipped.fluids[0].content_mask.bounds = bounds(0., 0., 23., 48.);
    let pixels = renderer.render_rgba(&clipped)?;
    assert!(
        pixels
            .chunks_exact(4)
            .enumerate()
            .all(|(i, p)| i % 64 < 23 || p[..3] == [0, 0, 0])
    );
    let mut child = Scene::default();
    child.insert_primitive(scene(&frame, 1.).fluids[0].clone());
    let Primitive::SubtreeLayer(mut captured) = layer(child, bounds(0., 0., 64., 48.), 1.) else {
        unreachable!()
    };
    captured.intermediate_effects = vec![bloom_pass(2)].into();
    let mut composed = Scene::default();
    composed.insert_primitive(quad(bounds(0., 0., 64., 48.), 0x000000ff));
    composed.insert_primitive(Primitive::SubtreeLayer(captured));
    composed.finish();
    let glow = renderer.render_rgba(&composed)?;
    let red_sum = |pixels: &[u8]| pixels.chunks_exact(4).map(|p| u64::from(p[0])).sum::<u64>();
    assert!(
        red_sum(&glow) > red_sum(&moved),
        "fluid content must feed the bloom chain"
    );
    frame.generation += 1;
    assert_eq!(
        empty,
        renderer.render_rgba(&scene(&frame, 1.))?,
        "clear must remove dye without advancing the clock"
    );
    frame.frame += 1;
    frame.splats = vec![FluidSplat {
        from: point(px(20.), px(20.)),
        to: point(px(20.), px(20.)),
        ..Default::default()
    }]
    .into();
    renderer.render_rgba(&scene(&frame, 1.))?;
    frame.splats = Arc::default();
    frame.frame += 1;
    frame.time += Duration::from_secs(20);
    frame.options.dye_decay = 1.;
    assert_eq!(
        empty,
        renderer.render_rgba(&scene(&frame, 1.))?,
        "density must decay over elapsed time"
    );
    Ok(())
}
