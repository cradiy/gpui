use std::time::Duration;

use super::*;
use gpui::{EffectHistoryId, SubtreeEffectPass, SubtreeFeedbackPass};

fn frame() -> SubtreeFeedbackPass {
    SubtreeFeedbackPass {
        id: EffectHistoryId::new(),
        shader: gpui_effects::feedback_shader(),
        generation: 0,
        frame: 1,
        time: Duration::ZERO,
        fade_duration: Duration::from_secs(1),
        capture: true,
        needs_animation: false,
        scale_factor: 1.,
        downsample: 1,
    }
}

fn scene(
    feedback: &SubtreeFeedbackPass,
    content: &[Quad],
    region: Bounds<ScaledPixels>,
    bloom: bool,
) -> Scene {
    let mut child = Scene::default();
    for quad in content {
        child.insert_primitive(*quad);
    }
    let Primitive::SubtreeLayer(mut captured) = layer(child, region, 1.) else {
        unreachable!()
    };
    let mut passes = vec![SubtreeEffectPass {
        shader: gpui_effects::subtree_identity_shader(),
        uniforms: Default::default(),
        time: 0.,
        bloom: None,
        feedback: Some(feedback.clone()),
        distance_field: None,
        particles: None,
        images: Default::default(),
        particle_transition: None,
    }];
    if bloom {
        passes.push(bloom_pass(2));
    }
    captured.intermediate_effects = passes.into();
    let mut root = direct(&[]);
    root.insert_primitive(Primitive::SubtreeLayer(captured));
    root.finish();
    root
}

fn direct(content: &[Quad]) -> Scene {
    let mut scene = Scene::default();
    scene.insert_primitive(quad(bounds(0., 0., 80., 60.), 0x000000ff));
    for quad in content {
        scene.insert_primitive(*quad);
    }
    scene.finish();
    scene
}

fn compare(actual: &[u8], expected: &[u8]) {
    assert_eq!(actual.len(), expected.len());
    let error = actual
        .iter()
        .zip(expected)
        .map(|(a, b)| a.abs_diff(*b))
        .max()
        .unwrap_or(0);
    assert!(error <= 3, "feedback channel error {error}");
}

pub(super) fn check(renderer: &mut WgpuOffscreenRenderer) -> anyhow::Result<()> {
    renderer.resize(size(DevicePixels(64), DevicePixels(48)));
    let region = bounds(2., 2., 60., 44.);
    let red_bounds = bounds(10., 12., 12., 12.);
    let blue_bounds = bounds(40., 12., 12., 12.);
    let red = quad(red_bounds, 0xff2000ff);
    let blue = quad(blue_bounds, 0x20a0ffff);
    let mut feedback = frame();
    let first_scene = scene(&feedback, &[red], region, false);
    let first = renderer.render_rgba(&first_scene)?;
    let replay = renderer.render_rgba(&first_scene)?;
    compare(&replay, &first);
    // A reference draw removes histories absent from that frame.
    let reference = renderer.render_rgba(&direct(&[red]))?;
    compare(&first, &reference);
    renderer.render_rgba(&first_scene)?;

    feedback.frame += 1;
    feedback.time = Duration::from_millis(100);
    let second_scene = scene(&feedback, &[blue], region, false);
    let second = renderer.render_rgba(&second_scene)?;
    let mut frozen = feedback.clone();
    frozen.capture = false;
    compare(
        &renderer.render_rgba(&scene(&frozen, &[red], region, false))?,
        &second,
    );

    feedback.frame += 1;
    feedback.time = Duration::from_millis(200);
    feedback.capture = false;
    let third = renderer.render_rgba(&scene(&feedback, &[red], region, false))?;
    let expected = renderer.render_rgba(&direct(&[
        quad(red_bounds, 0xff200040),
        quad(blue_bounds, 0x20a0ff80),
    ]))?;
    compare(&third, &expected);

    feedback.capture = true;
    feedback.frame += 1;
    renderer.render_rgba(&scene(&feedback, &[red], region, false))?;
    feedback.capture = false;
    feedback.generation += 1;
    feedback.frame += 1;
    let cleared = renderer.render_rgba(&scene(&feedback, &[], region, false))?;
    let empty = renderer.render_rgba(&direct(&[]))?;
    compare(&cleared, &empty);

    // Removing a surface releases its history, even when its identity is reused later.
    feedback.capture = true;
    feedback.frame += 1;
    renderer.render_rgba(&scene(&feedback, &[red], region, false))?;
    renderer.render_rgba(&direct(&[]))?;
    feedback.capture = false;
    feedback.frame += 1;
    compare(
        &renderer.render_rgba(&scene(&feedback, &[], region, false))?,
        &empty,
    );

    // Capture geometry and device scale changes reset the retained coordinates.
    for (new_region, scale, downsample) in [
        (bounds(3., 2., 59., 44.), 1., 1),
        (region, 2., 1),
        (region, 1., 2),
    ] {
        feedback.capture = true;
        feedback.frame += 1;
        feedback.scale_factor = 1.;
        feedback.downsample = 1;
        renderer.render_rgba(&scene(&feedback, &[red], region, false))?;
        feedback.capture = false;
        feedback.frame += 1;
        feedback.scale_factor = scale;
        feedback.downsample = downsample;
        compare(
            &renderer.render_rgba(&scene(&feedback, &[], new_region, false))?,
            &empty,
        );
    }
    feedback.capture = true;
    feedback.frame += 1;
    renderer.render_rgba(&scene(&feedback, &[red], region, false))?;
    feedback.capture = false;
    feedback.frame += 1;
    renderer.resize(size(DevicePixels(65), DevicePixels(49)));
    let resized = renderer.render_rgba(&scene(&feedback, &[], region, false))?;
    compare(&resized, &renderer.render_rgba(&direct(&[]))?);
    renderer.resize(size(DevicePixels(64), DevicePixels(48)));

    let mut left = frame();
    let mut right = frame();
    let pair_scene = |left: &SubtreeFeedbackPass, right: &SubtreeFeedbackPass| {
        let a = scene(left, &[red], region, false);
        let b = scene(right, &[blue], region, false);
        let mut root = direct(&[]);
        root.insert_primitive(Primitive::SubtreeLayer(a.subtree_layers[0].clone()));
        root.insert_primitive(Primitive::SubtreeLayer(b.subtree_layers[0].clone()));
        root.finish();
        root
    };
    renderer.render_rgba(&pair_scene(&left, &right))?;
    left.capture = false;
    left.generation += 1;
    left.frame += 1;
    right.capture = false;
    right.frame += 1;
    let independent = renderer.render_rgba(&pair_scene(&left, &right))?;
    compare(&independent, &renderer.render_rgba(&direct(&[blue]))?);

    let mut combined = frame();
    let lit = renderer.render_rgba(&scene(&combined, &[red], region, true))?;
    combined.capture = false;
    combined.frame += 1;
    let held = renderer.render_rgba(&scene(&combined, &[], region, true))?;
    compare(&held, &lit);
    assert!(
        lit[(18 * 64 + 8) * 4] > 0,
        "bloom must extend beyond the retained shape"
    );
    let mut opacity_history = frame();
    let mut dimmed = scene(&opacity_history, &[red], region, false);
    dimmed.subtree_layers[0].composite.opacity = 0.25;
    renderer.render_rgba(&dimmed)?;
    opacity_history.frame += 1;
    opacity_history.capture = false;
    let restored = renderer.render_rgba(&scene(&opacity_history, &[], region, false))?;
    compare(&restored, &reference);
    Ok(())
}
