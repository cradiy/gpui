use super::*;
use gpui::{EffectUniforms, Point, px};
use gpui_effects::{DeformationOptions, LensOptions, deformation_shader, lens_shader};

pub(super) fn check(renderer: &mut WgpuOffscreenRenderer) -> anyhow::Result<()> {
    let logical_bounds = Bounds::new(point(px(8.), px(8.)), size(px(112.), px(80.)));
    let deformation = DeformationOptions {
        center: point(0.35, 0.45),
        radius: px(55.),
        offset: point(px(18.), px(-4.)),
    };
    for scale in [0.75, 1., 1.5, 2.] {
        let width = (128. * scale) as usize;
        renderer.resize(size(
            DevicePixels(width as i32),
            DevicePixels((96. * scale) as i32),
        ));
        let region = logical_bounds.map(|value| ScaledPixels(f32::from(value) * scale));
        for magnification in [0.5, 1.8, 3.] {
            let lens = LensOptions {
                center: point(0.3, 0.4),
                radius: px(72.),
                magnification,
                edge_fade: px(1.),
                ..Default::default()
            };
            let stages = [
                (
                    lens_shader(),
                    EffectUniforms::new()
                        .with_slot(
                            0,
                            [lens.center.x, lens.center.y, magnification, lens.softness],
                        )
                        .with_slot(1, [72. * scale, scale, 0., 0.]),
                ),
                (
                    deformation_shader(),
                    EffectUniforms::new()
                        .with_slot(0, [deformation.center.x, deformation.center.y, 0., 0.])
                        .with_slot(1, [18. * scale, -4. * scale, 55. * scale, 0.]),
                ),
            ];
            let scene = |order: &[usize]| {
                let Primitive::SubtreeLayer(template) = layer(Scene::default(), region, 1.) else {
                    unreachable!()
                };
                let mut gradient = template.composite;
                gradient.shader = EffectShader::wgsl(
                    "fn effect(input: EffectInput, params: EffectParams) -> vec4<f32> { return vec4<f32>(vec2<f32>(0.25) + input.uv * 0.5, 0.0, 1.0); }",
                );
                let mut source = Scene::default();
                source.insert_primitive(gradient);
                for &index in order {
                    let Primitive::SubtreeLayer(mut captured) = layer(source, region, 1.) else {
                        unreachable!()
                    };
                    captured.composite.shader = stages[index].0.clone();
                    captured.composite.uniforms = stages[index].1;
                    source = Scene::default();
                    source.insert_primitive(Primitive::SubtreeLayer(captured));
                }
                source.finish();
                source
            };
            let baseline = renderer.render_rgba(&scene(&[]))?;
            for order in [&[0][..], &[1][..], &[0, 1][..], &[1, 0][..]] {
                let actual = renderer.render_rgba(&scene(order))?;
                for y in (16..80).step_by(3) {
                    for x in (16..112).step_by(3) {
                        let dx = (x as f32 * scale) as usize;
                        let dy = (y as f32 * scale) as usize;
                        let destination =
                            point(px((dx as f32 + 0.5) / scale), px((dy as f32 + 0.5) / scale));
                        let mut interior = true;
                        let source = order.iter().rev().fold(destination, |p, &index| {
                            let mapped = if index == 0 {
                                lens.source_position(p, logical_bounds, scale)
                            } else {
                                deformation.source_position(p, logical_bounds)
                            };
                            interior &= logical_bounds.dilate(px(-2. / scale)).contains(&mapped);
                            mapped
                        });
                        // Compare geometry where filter footprints stay inside the capture.
                        if !interior {
                            continue;
                        }
                        for channel in 0..2 {
                            let expected = sample(
                                &baseline,
                                width,
                                source.map(|p| f32::from(p) * scale - 0.5),
                                channel,
                            );
                            let value = actual[(dy * width + dx) * 4 + channel];
                            assert!(
                                (f32::from(value) - expected).abs() <= 3.,
                                "scale {scale}, zoom {magnification}, stages {order:?}, pixel ({dx}, {dy}), channel {channel}: {value} != {expected}"
                            );
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

fn sample(pixels: &[u8], width: usize, p: Point<f32>, channel: usize) -> f32 {
    let x = p.x.floor() as usize;
    let y = p.y.floor() as usize;
    let at = |x, y| f32::from(pixels[(y * width + x) * 4 + channel]);
    let a = at(x, y) * (1. - p.x.fract()) + at(x + 1, y) * p.x.fract();
    let b = at(x, y + 1) * (1. - p.x.fract()) + at(x + 1, y + 1) * p.x.fract();
    a * (1. - p.y.fract()) + b * p.y.fract()
}
