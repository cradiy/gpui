use super::*;
use gpui::{px, rgb};
use gpui_effects::{SdfOptions, SdfScene, SdfShape, SdfTransform};

fn scene(field: &SdfScene, scale: f32, opacity: f32) -> Scene {
    let region = bounds(0., 0., 64. * scale, 48. * scale);
    let mut scene = Scene::default();
    scene.insert_primitive(quad(region, 0x000000ff));
    scene.insert_primitive(EffectQuad {
        order: 0,
        bounds: region,
        effect_bounds: region,
        transformation: Default::default(),
        content_mask: ContentMask { bounds: region },
        shader: field.shader(),
        uniforms: field.uniforms(scale),
        time: 0.,
        corner_radii: Default::default(),
        opacity,
        image_tile: None,
        second_image_tile: None,
        third_image_tile: None,
        fourth_image_tile: None,
    });
    scene.finish();
    scene
}
fn pixel(pixels: &[u8], x: usize, y: usize) -> &[u8] {
    &pixels[(y * 64 + x) * 4..(y * 64 + x) * 4 + 3]
}

pub(super) fn check(renderer: &mut WgpuOffscreenRenderer) -> anyhow::Result<()> {
    renderer.resize(size(DevicePixels(64), DevicePixels(48)));
    let a = SdfShape::circle(point(px(24.), px(24.)), px(12.), rgb(0xff0000));
    let b = SdfShape::circle(point(px(40.), px(24.)), px(12.), rgb(0x0000ff));
    let union =
        renderer.render_rgba(&scene(&SdfScene::new(a.clone().union(b.clone()))?, 1., 1.))?;
    let intersect = renderer.render_rgba(&scene(
        &SdfScene::new(a.clone().intersect(b.clone()))?,
        1.,
        1.,
    ))?;
    let cut = renderer.render_rgba(&scene(&SdfScene::new(a.subtract(b))?, 1., 1.))?;
    assert!(pixel(&union, 16, 24)[0] > 200 && pixel(&union, 48, 24)[2] > 200);
    assert_eq!(pixel(&intersect, 16, 24), [0, 0, 0]);
    assert!(pixel(&intersect, 32, 24).iter().any(|v| *v > 200));
    assert_eq!(pixel(&cut, 32, 24), [0, 0, 0]);
    assert!(pixel(&cut, 24, 24)[0] > 200 && pixel(&cut, 24, 24)[2] == 0);

    let a = SdfShape::circle(point(px(20.), px(24.)), px(8.), rgb(0xff0000));
    let b = SdfShape::circle(point(px(40.), px(24.)), px(8.), rgb(0x0000ff));
    let mut soft = SdfScene::new(a.smooth_union(b))?;
    soft.set_options(SdfOptions {
        smoothing: px(16.),
        ..Default::default()
    });
    let blended = renderer.render_rgba(&scene(&soft, 1., 1.))?;
    assert!(
        pixel(&blended, 30, 24)[0] > 40 && pixel(&blended, 30, 24)[2] > 40,
        "smooth union must bridge the gap and blend colors"
    );
    soft.set_options(SdfOptions {
        smoothing: px(0.),
        ..Default::default()
    });
    let separated = renderer.render_rgba(&scene(&soft, 1., 1.))?;
    assert_eq!(pixel(&separated, 30, 24), [0, 0, 0]);

    let mut capsule = SdfScene::new(SdfShape::capsule(
        point(px(32.), px(24.)),
        px(20.),
        px(5.),
        rgb(0x00ff00),
    ))?;
    let horizontal = renderer.render_rgba(&scene(&capsule, 1., 1.))?;
    assert!(pixel(&horizontal, 44, 24)[1] > 200);
    assert_eq!(pixel(&horizontal, 32, 36), [0, 0, 0]);
    capsule.set_transform(
        0,
        SdfTransform {
            center: point(px(32.), px(24.)),
            rotation: std::f32::consts::FRAC_PI_2,
            ..Default::default()
        },
    );
    let vertical = renderer.render_rgba(&scene(&capsule, 1., 1.))?;
    assert_eq!(pixel(&vertical, 44, 24), [0, 0, 0]);
    assert!(pixel(&vertical, 32, 36)[1] > 200);

    let rounded = SdfScene::new(SdfShape::rounded_rect(
        point(px(32.), px(24.)),
        size(px(32.), px(20.)),
        px(6.),
        rgb(0xffffff),
    ))?;
    let rounded_pixels = renderer.render_rgba(&scene(&rounded, 1., 1.))?;
    assert_eq!(pixel(&rounded_pixels, 16, 14), [0, 0, 0]);
    assert!(pixel(&rounded_pixels, 20, 18)[0] > 200);
    let mut field = SdfScene::new(SdfShape::circle(
        point(px(32.), px(24.)),
        px(10.),
        rgb(0x00ff00),
    ))?;
    field.set_options(SdfOptions {
        fill_opacity: 0.,
        stroke_width: px(2.),
        ..Default::default()
    });
    let outlined = renderer.render_rgba(&scene(&field, 1., 1.))?;
    assert_eq!(pixel(&outlined, 32, 24), [0, 0, 0]);
    assert!(pixel(&outlined, 42, 24)[1] > 100);
    field.set_options(SdfOptions {
        outer_glow: px(12.),
        outer_glow_opacity: 0.8,
        ..Default::default()
    });
    let glow = renderer.render_rgba(&scene(&field, 1., 1.))?;
    assert!(pixel(&glow, 46, 24)[1] > pixel(&glow, 56, 24)[1] && pixel(&glow, 46, 24)[1] > 5);
    let mut clipped = scene(&field, 1., 1.);
    clipped.effects[0].content_mask.bounds = bounds(0., 0., 32., 48.);
    let clipped_pixels = renderer.render_rgba(&clipped)?;
    assert!(
        clipped_pixels
            .chunks_exact(4)
            .enumerate()
            .all(|(i, p)| i % 64 < 32 || p[..3] == [0, 0, 0])
    );
    let dim = renderer.render_rgba(&scene(&field, 1., 0.5))?;
    assert!(pixel(&dim, 32, 24)[1] > 0 && pixel(&dim, 32, 24)[1] < pixel(&glow, 32, 24)[1]);

    field.set_options(SdfOptions::default());
    renderer.resize(size(DevicePixels(128), DevicePixels(96)));
    let scaled = renderer.render_rgba(&scene(&field, 2., 1.))?;
    assert!(scaled[(48 * 128 + 64) * 4 + 1] > 200);
    assert_eq!(scaled[(48 * 128 + 88) * 4 + 1], 0);
    Ok(())
}
