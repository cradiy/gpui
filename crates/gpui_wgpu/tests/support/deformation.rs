use super::*;
use gpui_effects::deformation_shader;

fn scene(scale: f32, offset: f32, opacity: f32) -> Scene {
    let region = bounds(0., 0., 128. * scale, 96. * scale);
    let mut source = Scene::default();
    source.insert_primitive(quad(
        bounds(30. * scale, 46. * scale, 4. * scale, 4. * scale),
        0xff804080,
    ));
    source.insert_primitive(quad(
        bounds(112. * scale, 8. * scale, 8. * scale, 8. * scale),
        0x60a0ffff,
    ));
    let Primitive::SubtreeLayer(mut captured) = layer(source, region, opacity) else {
        unreachable!()
    };
    captured.composite.shader = deformation_shader();
    captured.composite.uniforms.set_slot(0, [0.25, 0.5, 0., 0.]);
    captured
        .composite
        .uniforms
        .set_slot(1, [offset * scale, 0., 60. * scale, 0.]);
    let mut scene = Scene::default();
    scene.insert_primitive(quad(region, 0x000000ff));
    scene.insert_primitive(Primitive::SubtreeLayer(captured));
    scene.finish();
    scene
}

pub(super) fn check(renderer: &mut WgpuOffscreenRenderer) -> anyhow::Result<()> {
    for scale in [1., 2.] {
        let s = scale as usize;
        renderer.resize(size(
            DevicePixels(128 * s as i32),
            DevicePixels(96 * s as i32),
        ));
        let source = renderer.render_rgba(&scene(scale, 0., 1.))?;
        let mut identity = scene(scale, 0., 1.);
        identity.subtree_layers[0].composite.shader = gpui_effects::subtree_identity_shader();
        assert_eq!(source, renderer.render_rgba(&identity)?);
        let moved = renderer.render_rgba(&scene(scale, 18., 1.))?;
        let pixel = |x: usize, y: usize| (y * s * 128 * s + x * s) * 4;
        let original = pixel(32, 48);
        let destination = pixel(50, 48);
        assert!(moved[original] < 3);
        assert!(moved[destination].abs_diff(source[original]) <= 3);
        let limited = renderer.render_rgba(&scene(scale, 21., 1.))?;
        let excessive = renderer.render_rgba(&scene(scale, 200., 1.))?;
        assert_eq!(limited, excessive);
        for y in 0..96 * s {
            for x in 0..128 * s {
                if (x as f32 / scale - 32.).hypot(y as f32 / scale - 48.) > 61. {
                    let i = (y * 128 * s + x) * 4;
                    assert_eq!(&source[i..i + 4], &moved[i..i + 4]);
                }
            }
        }
        let faded = renderer.render_rgba(&scene(scale, 18., 0.5))?;
        let faded_source = renderer.render_rgba(&scene(scale, 0., 0.5))?;
        assert!(faded[destination] < moved[destination]);
        assert!(faded[destination].abs_diff(faded_source[original]) <= 3);
        let mut clipped = scene(scale, 18., 1.);
        clipped.subtree_layers[0].composite.content_mask.bounds =
            bounds(0., 0., 45. * scale, 96. * scale);
        assert_eq!(renderer.render_rgba(&clipped)?[destination], 0);
    }
    Ok(())
}
