use super::*;
use gpui::{EffectHistoryId, ParticleDraw, ParticleFrame, ParticlePhysics, ParticleSpawn, px, rgb};
use std::time::Duration;

fn scene(frame: &ParticleFrame, opacity: f32) -> Scene {
    let mut scene = Scene::default();
    scene.insert_primitive(quad(bounds(0., 0., 64., 48.), 0x000000ff));
    scene.insert_primitive(ParticleDraw {
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

pub(super) fn check(renderer: &mut WgpuOffscreenRenderer) -> anyhow::Result<()> {
    renderer.resize(size(DevicePixels(64), DevicePixels(48)));
    let mut frame = ParticleFrame {
        id: EffectHistoryId::new(),
        generation: 0,
        frame: 1,
        time: Duration::ZERO,
        capacity: 4,
        physics: ParticlePhysics {
            acceleration: point(px(0.), px(0.)),
            drag: 0.,
            ..Default::default()
        },
        spawns: vec![ParticleSpawn {
            from: point(px(16.), px(20.)),
            to: point(px(16.), px(20.)),
            count: 1,
            velocity: point(px(40.), px(0.)),
            speed: px(0.)..px(0.),
            radius: px(2.)..px(2.),
            lifetime: Duration::from_secs(1)..Duration::from_secs(1),
            color: rgb(0xff0000),
            stretch: 0.,
        }]
        .into(),
        needs_animation: true,
    };
    renderer.render_rgba(&scene(&frame, 1.))?;
    frame.frame += 1;
    frame.time = Duration::from_millis(100);
    frame.spawns = Arc::default();
    let drawn = renderer.render_rgba(&scene(&frame, 1.))?;
    assert!(
        drawn[(24 * 64 + 24) * 4] > 100,
        "particle must move according to velocity"
    );
    assert!(
        drawn[(24 * 64 + 20) * 4] < 10,
        "previous position must not remain painted"
    );
    assert_eq!(
        drawn,
        renderer.render_rgba(&scene(&frame, 1.))?,
        "frame replay must not simulate twice"
    );
    let dimmed = renderer.render_rgba(&scene(&frame, 0.5))?;
    assert!(dimmed[(24 * 64 + 24) * 4] < drawn[(24 * 64 + 24) * 4]);
    assert_eq!(
        drawn,
        renderer.render_rgba(&scene(&frame, 1.))?,
        "paint opacity must not alter retained state"
    );
    let mut clipped = scene(&frame, 1.);
    clipped.particles[0].content_mask.bounds = bounds(0., 0., 24., 48.);
    let clipped_pixels = renderer.render_rgba(&clipped)?;
    assert!(
        clipped_pixels
            .chunks_exact(4)
            .enumerate()
            .all(|(i, pixel)| i % 64 < 24 || pixel[..3] == [0, 0, 0])
    );
    let mut cleared_frame = frame.clone();
    cleared_frame.generation += 1;
    let cleared = renderer.render_rgba(&scene(&cleared_frame, 1.))?;
    assert!(
        cleared.chunks_exact(4).all(|pixel| pixel[..3] == [0, 0, 0]),
        "clear must remove live particles"
    );
    frame.generation = cleared_frame.generation + 1;
    frame.frame += 1;
    frame.capacity = 2;
    frame.spawns = [12., 28., 44.]
        .map(|x| ParticleSpawn {
            from: point(px(x), px(20.)),
            to: point(px(x), px(20.)),
            count: 1,
            velocity: point(px(0.), px(0.)),
            speed: px(0.)..px(0.),
            radius: px(2.)..px(2.),
            color: rgb(0xffffff),
            lifetime: Duration::from_secs(1)..Duration::from_secs(1),
            stretch: 0.,
        })
        .into();
    renderer.render_rgba(&scene(&frame, 1.))?;
    frame.frame += 1;
    frame.time = Duration::from_millis(200);
    frame.spawns = Arc::default();
    let bounded = renderer.render_rgba(&scene(&frame, 1.))?;
    assert_eq!(
        bounded[(24 * 64 + 16) * 4],
        0,
        "oldest emission must be replaced at capacity"
    );
    assert!(bounded[(24 * 64 + 32) * 4] > 100 && bounded[(24 * 64 + 48) * 4] > 100);
    let mut child = Scene::default();
    child.insert_primitive(scene(&frame, 1.).particles[0].clone());
    let Primitive::SubtreeLayer(mut captured) = layer(child, bounds(0., 0., 64., 48.), 1.) else {
        unreachable!()
    };
    captured.intermediate_effects = vec![bloom_pass(2)].into();
    let mut composed = Scene::default();
    composed.insert_primitive(quad(bounds(0., 0., 64., 48.), 0x000000ff));
    composed.insert_primitive(Primitive::SubtreeLayer(captured));
    composed.finish();
    let glow = renderer.render_rgba(&composed)?;
    assert!(
        glow[(24 * 64 + 36) * 4] > bounded[(24 * 64 + 36) * 4],
        "particle content must feed the bloom chain"
    );
    frame.frame += 1;
    frame.time = Duration::from_secs(2);
    let expired = renderer.render_rgba(&scene(&frame, 1.))?;
    assert!(expired.chunks_exact(4).all(|pixel| pixel[..3] == [0, 0, 0]));
    frame.generation += 1;
    frame.frame += 1;
    let cleared = renderer.render_rgba(&scene(&frame, 1.))?;
    assert_eq!(expired, cleared);
    for strength in [-750., 750.] {
        frame.generation += 1;
        frame.frame += 1;
        frame.physics.attractor = point(px(48.), px(20.));
        frame.physics.radius = px(100.);
        frame.physics.strength = px(strength);
        frame.spawns = vec![ParticleSpawn {
            from: point(px(28.), px(20.)),
            to: point(px(28.), px(20.)),
            count: 1,
            velocity: point(px(0.), px(0.)),
            speed: px(0.)..px(0.),
            radius: px(2.)..px(2.),
            color: rgb(0xffffff),
            stretch: 0.,
            ..Default::default()
        }]
        .into();
        renderer.render_rgba(&scene(&frame, 1.))?;
        frame.frame += 1;
        frame.time += Duration::from_millis(100);
        frame.spawns = Arc::default();
        let pixels = renderer.render_rgba(&scene(&frame, 1.))?;
        let (sum, weighted) =
            pixels
                .chunks_exact(4)
                .enumerate()
                .fold((0., 0.), |(sum, weighted), (i, pixel)| {
                    let value = f64::from(pixel[0]);
                    (sum + value, weighted + value * ((i % 64) as f64 + 0.5))
                });
        assert!(sum > 0.);
        let center = weighted / sum;
        assert!(
            (center - 32.) * f64::from(strength.signum()) > 1.,
            "force direction must move the particle toward or away from the attractor"
        );
    }
    renderer.render_rgba(&Scene::default())?;
    let removed = renderer.render_rgba(&scene(&frame, 1.))?;
    assert!(
        removed.chunks_exact(4).all(|pixel| pixel[..3] == [0, 0, 0]),
        "removing a system must release its retained state"
    );
    Ok(())
}
