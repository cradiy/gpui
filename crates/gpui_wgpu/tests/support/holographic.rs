use super::*;
use gpui_effects::{HolographicOptions, holographic_image_shader};

fn scene(options: HolographicOptions, scale: f32, opacity: f32, color: u32) -> Scene {
    let region = bounds(0., 0., 64. * scale, 48. * scale);
    let mut source = Scene::default();
    source.insert_primitive(quad(
        bounds(8. * scale, 8. * scale, 48. * scale, 32. * scale),
        color,
    ));
    let Primitive::SubtreeLayer(mut captured) = layer(source, region, opacity) else {
        unreachable!()
    };
    captured.composite.shader = holographic_image_shader();
    captured.composite.uniforms = options.uniforms(gpui::rgb(0xffffff));
    let mut scene = Scene::default();
    scene.insert_primitive(quad(region, 0x000000ff));
    scene.insert_primitive(Primitive::SubtreeLayer(captured));
    scene.finish();
    scene
}

pub(super) fn check(renderer: &mut WgpuOffscreenRenderer) -> anyhow::Result<()> {
    renderer.resize(size(DevicePixels(64), DevicePixels(48)));
    let mut options = HolographicOptions::default();
    let normal = renderer.render_rgba(&scene(options, 1., 1., 0x68758780))?;
    options.surface.tilt = point(0.45, -0.35);
    let tilted = renderer.render_rgba(&scene(options, 1., 1., 0x68758780))?;
    assert!(
        normal
            .chunks_exact(4)
            .zip(tilted.chunks_exact(4))
            .any(|(a, b)| { a[..3].iter().zip(&b[..3]).any(|(a, b)| a.abs_diff(*b) > 20) })
    );
    options.strength = 0.;
    let unshaded = renderer.render_rgba(&scene(options, 1., 1., 0x68758780))?;
    for (i, ((a, b), source)) in normal
        .chunks_exact(4)
        .zip(tilted.chunks_exact(4))
        .zip(unshaded.chunks_exact(4))
        .enumerate()
    {
        if !(8..56).contains(&(i % 64)) || !(8..40).contains(&(i / 64)) {
            assert_eq!(a, source, "material must not light transparent pixels");
            assert_eq!(b, source);
        }
    }
    let mut identity = scene(options, 1., 1., 0x68758780);
    identity.subtree_layers[0].composite.shader = gpui_effects::subtree_identity_shader();
    assert_eq!(unshaded, renderer.render_rgba(&identity)?);
    let options = HolographicOptions::default();
    let dim = renderer.render_rgba(&scene(options, 1., 0.5, 0x68758780))?;
    let faded_opaque = renderer.render_rgba(&scene(options, 1., 128. / 255., 0x687587ff))?;
    let pixel = (24 * 64 + 32) * 4;
    for channel in 0..3 {
        assert!(dim[pixel + channel] > 0 && dim[pixel + channel] < normal[pixel + channel]);
        assert!(
            normal[pixel + channel].abs_diff(faded_opaque[pixel + channel]) <= 2,
            "source and layer opacity must agree: {:?} / {:?}",
            &normal[pixel..pixel + 4],
            &faded_opaque[pixel..pixel + 4]
        );
    }
    renderer.resize(size(DevicePixels(128), DevicePixels(96)));
    let mut options = options;
    options.surface.texture = 0.;
    let large = renderer.render_rgba(&scene(options, 2., 1., 0x68758780))?;
    renderer.resize(size(DevicePixels(64), DevicePixels(48)));
    let small = renderer.render_rgba(&scene(options, 1., 1., 0x68758780))?;
    for channel in 0..4 {
        assert!(small[pixel + channel].abs_diff(large[(48 * 128 + 64) * 4 + channel]) < 12);
    }
    Ok(())
}
