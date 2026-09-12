use super::*;

fn linear_channel(value: u8) -> f64 {
    let srgb = f64::from(value) / 255.;
    if !OUTPUT_SRGB {
        return srgb;
    }
    if srgb <= 0.04045 {
        srgb / 12.92
    } else {
        ((srgb + 0.055) / 1.055).powf(2.4)
    }
}

fn scene(scale: f32, displacement: [f32; 2], opacity: f32) -> Scene {
    let region = bounds(0., 0., 64. * scale, 48. * scale);
    let mut source = Scene::default();
    source.insert_primitive(quad(
        bounds(30. * scale, 20. * scale, 4. * scale, 8. * scale),
        0xff804080,
    ));
    let Primitive::SubtreeLayer(mut captured) = layer(source, region, opacity) else {
        unreachable!()
    };
    captured.composite.shader = gpui_effects::motion_blur_shader();
    captured.composite.uniforms.set_slot(
        0,
        [displacement[0] * scale, displacement[1] * scale, 0., 0.],
    );
    captured.composite.uniforms.set_slot(1, [65., 0., 0., 0.]);
    let mut scene = Scene::default();
    scene.insert_primitive(quad(region, 0x000000ff));
    scene.insert_primitive(Primitive::SubtreeLayer(captured));
    scene.finish();
    scene
}

pub(super) fn check(renderer: &mut WgpuOffscreenRenderer) -> anyhow::Result<()> {
    for scale in [1., 1.5, 2.] {
        let width = (64. * scale) as usize;
        renderer.resize(size(
            DevicePixels(width as i32),
            DevicePixels((48. * scale) as i32),
        ));
        let source = renderer.render_rgba(&scene(scale, [0., 0.], 1.))?;
        let mut identity = scene(scale, [0., 0.], 1.);
        identity.subtree_layers[0].composite.shader = gpui_effects::subtree_identity_shader();
        assert_eq!(source, renderer.render_rgba(&identity)?);
        for displacement in [[20., 0.], [0., 20.], [12., 16.]] {
            let blurred = renderer.render_rgba(&scene(scale, displacement, 1.))?;
            let reversed = renderer.render_rgba(&scene(scale, displacement.map(|v| -v), 1.))?;
            assert!(
                blurred
                    .iter()
                    .zip(&reversed)
                    .all(|(a, b)| a.abs_diff(*b) <= 1)
            );
            for channel in 0..3 {
                let sum = |pixels: &[u8]| {
                    pixels
                        .chunks_exact(4)
                        .map(|p| linear_channel(p[channel]))
                        .sum::<f64>()
                };
                let ratio = sum(&blurred) / sum(&source);
                assert!(
                    (ratio - 1.).abs() < 0.05,
                    "blur energy changed: {ratio} at scale {scale}"
                );
            }
            let pixel = |x: usize, y: usize| {
                (((y as f32 * scale) as usize) * width + (x as f32 * scale) as usize) * 4
            };
            let center = pixel(32, 24);
            assert!(blurred[center] < source[center]);
            let outside = if displacement[0] == 0. {
                pixel(32, if OUTPUT_SRGB { 17 } else { 19 })
            } else if displacement[1] != 0. {
                // Probe inside the diagonal trail, away from its rasterized edge.
                pixel(29, 20)
            } else {
                pixel(if OUTPUT_SRGB { 27 } else { 29 }, 24)
            };
            assert!(
                blurred[outside] > source[outside],
                "no blur spread at scale {scale}, displacement {displacement:?}: {} <= {}",
                blurred[outside],
                source[outside]
            );
            if displacement[1] == 0. {
                assert_eq!(blurred[pixel(32, 18)], 0);
            }
            let faded = renderer.render_rgba(&scene(scale, displacement, 0.5))?;
            assert!(
                faded
                    .iter()
                    .step_by(4)
                    .zip(blurred.iter().step_by(4))
                    .all(|(a, b)| (linear_channel(*a) - linear_channel(*b) * 0.5).abs() <= 0.005)
            );
            let mut clipped = scene(scale, displacement, 1.);
            clipped.subtree_layers[0].composite.content_mask.bounds =
                bounds(0., 0., 32. * scale, 48. * scale);
            assert_eq!(renderer.render_rgba(&clipped)?[pixel(34, 24)], 0);
        }
    }
    Ok(())
}
