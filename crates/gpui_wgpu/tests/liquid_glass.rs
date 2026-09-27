use std::borrow::Cow;

use gpui::{
    AtlasKey, AtlasTile, BackdropBlur, Bounds, ContentMask, DevicePixels, EffectQuad,
    EffectUniforms, ImageId, RenderImageParams, ScaledPixels as P, Scene, point, size,
};
use gpui_effects::{liquid_glass_content_shader, liquid_glass_shader};
use gpui_wgpu::WgpuOffscreenRenderer;

fn bounds(x: f32, y: f32, w: f32, h: f32) -> Bounds<P> {
    Bounds::new(point(P(x), P(y)), size(P(w), P(h)))
}

fn white_mask(renderer: &WgpuOffscreenRenderer) -> AtlasTile {
    renderer
        .sprite_atlas()
        .get_or_insert_with(
            &AtlasKey::Image(RenderImageParams {
                image_id: ImageId(981),
                frame_index: 0,
            }),
            &mut || {
                Ok(Some((
                    size(DevicePixels(1), DevicePixels(1)),
                    Cow::Owned(vec![255; 4]),
                )))
            },
        )
        .unwrap()
        .unwrap()
}

fn content(tile: AtlasTile, uniforms: EffectUniforms, opacity: f32, scale: f32) -> Scene {
    let viewport = bounds(0., 0., 80. * scale, 48. * scale);
    let mut scene = Scene::default();
    scene.insert_primitive(EffectQuad {
        order: 0,
        bounds: viewport,
        effect_bounds: viewport,
        transformation: Default::default(),
        content_mask: ContentMask { bounds: viewport },
        shader: liquid_glass_content_shader(),
        uniforms,
        time: 0.,
        corner_radii: Default::default(),
        opacity,
        image_tile: Some(tile),
        second_image_tile: None,
        third_image_tile: None,
        fourth_image_tile: None,
    });
    scene.finish();
    scene
}

fn glass(center_x: f32, scale: f32, bulge: f32) -> Scene {
    let viewport = bounds(0., 0., 80. * scale, 48. * scale);
    let mut scene = Scene::default();
    scene.insert_primitive(BackdropBlur {
        order: 0,
        bounds: viewport,
        content_mask: ContentMask { bounds: viewport },
        corner_radii: Default::default(),
        blur_radius: P(0.),
        opacity: 1.,
        shader: Some(liquid_glass_shader()),
        uniforms: EffectUniforms::new()
            .with_slot(0, [1., 1., 0., 0.])
            .with_slot(1, [1.; 4])
            .with_slot(3, [-0.6, -0.8, 0., center_x * scale])
            .with_slot(4, [12. * scale; 4])
            .with_slot(6, [48. * scale, 28. * scale, 1., 24. * scale])
            .with_slot(7, [0.8, 0.2, bulge * scale, -bulge * 0.25 * scale]),
        time: 0.,
        pointer: point(0., 0.),
        pointer_active: false,
    });
    scene.finish();
    scene
}

#[test]
#[ignore = "requires a GPU adapter"]
fn content_coverage_matches_glass_during_fractional_motion_and_deformation() -> anyhow::Result<()> {
    let mut renderer = WgpuOffscreenRenderer::new(size(DevicePixels(80), DevicePixels(48)))?;
    let tile = white_mask(&renderer);
    for scale in [1., 1.5, 2.] {
        renderer.resize(size(
            DevicePixels((80. * scale) as i32),
            DevicePixels((48. * scale) as i32),
        ));
        for bulge in [0., 3.] {
            let mut edge_samples = Vec::new();
            for step in 0..8 {
                let center_x = 40. + step as f32 / 8.;
                let uniforms = EffectUniforms::new()
                    .with_slot(0, [center_x * scale, 24. * scale, 48. * scale, 28. * scale])
                    .with_slot(1, [12. * scale; 4])
                    .with_slot(2, [0.8, 0.2, bulge * scale, -bulge * 0.25 * scale])
                    .with_slot(3, [1.; 4])
                    .with_slot(4, [0., 0., 0., 1.]);
                let actual = renderer.render_rgba(&content(tile, uniforms, 1., scale))?;
                let expected = renderer.render_rgba(&glass(center_x, scale, bulge))?;
                // Backdrop composition can quantize coverage in an intermediate
                // UNORM target. Compare linear coverage, not amplified dark sRGB values.
                let linear = |value: u8| {
                    let value = value as f32 / 255.;
                    if value <= 0.04045 {
                        value / 12.92
                    } else {
                        ((value + 0.055) / 1.055).powf(2.4)
                    }
                };
                let worst = actual
                    .chunks_exact(4)
                    .zip(expected.chunks_exact(4))
                    .enumerate()
                    .max_by(|(_, (a, b)), (_, (c, d))| {
                        (linear(a[0]) - linear(b[0]))
                            .abs()
                            .total_cmp(&(linear(c[0]) - linear(d[0])).abs())
                    })
                    .unwrap();
                assert!(
                    actual
                        .chunks_exact(4)
                        .zip(expected.chunks_exact(4))
                        .all(|(a, b)| a[0].abs_diff(b[0]) <= 1
                            || (linear(a[0]) - linear(b[0])).abs() <= 2. / 255.),
                    "content boundary must follow glass at scale {scale}, bulge {bulge}, step {step}: worst {worst:?}"
                );
                if scale == 1. && bulge == 0. {
                    edge_samples.push(expected[(24 * 80 + 16) * 4]);
                }
            }
            if !edge_samples.is_empty() {
                assert!(
                    edge_samples.windows(2).all(|pair| pair[0] > pair[1]),
                    "fractional motion must not hold then jump at integer pixels: {edge_samples:?}"
                );
            }
        }
    }
    Ok(())
}

#[test]
#[ignore = "requires a GPU adapter"]
fn content_recolors_only_covered_pixels_and_preserves_transparency() -> anyhow::Result<()> {
    let mut renderer = WgpuOffscreenRenderer::new(size(DevicePixels(80), DevicePixels(48)))?;
    let tile = white_mask(&renderer);
    let uniforms = EffectUniforms::new()
        .with_slot(0, [60., 24., 40., 48.])
        .with_slot(3, [0., 0., 1., 1.])
        .with_slot(4, [1., 0., 0., 0.5]);
    let pixels = renderer.render_rgba(&content(tile, uniforms, 0.5, 1.))?;
    let left = &pixels[(24 * 80 + 20) * 4..][..3];
    let right = &pixels[(24 * 80 + 60) * 4..][..3];
    assert_eq!([left[0], left[1], left[2]], [right[2], right[1], right[0]]);
    assert_eq!(left[1..], [0, 0]);
    // Linear alpha 0.5 * element opacity 0.5, encoded into an sRGB render target.
    assert!(
        (135..=139).contains(&left[0]),
        "alpha applied exactly once: {left:?}"
    );
    let transparent = uniforms.with_slot(4, [1., 0., 0., 0.]);
    let pixels = renderer.render_rgba(&content(tile, transparent, 1., 1.))?;
    assert!(pixels.chunks_exact(4).all(|pixel| pixel[..3] == [0, 0, 0]));
    Ok(())
}

#[test]
#[ignore = "requires a GPU adapter"]
fn glass_refracts_recolored_content_beyond_its_original_bounds() -> anyhow::Result<()> {
    let mut renderer = WgpuOffscreenRenderer::new(size(DevicePixels(80), DevicePixels(48)))?;
    let tile = white_mask(&renderer);
    for scale in [1., 1.5, 2.] {
        let width = (80. * scale) as usize;
        renderer.resize(size(
            DevicePixels(width as i32),
            DevicePixels((48. * scale) as i32),
        ));
        for fraction in [0., 0.25, 0.5, 0.75] {
            let center = 40. + fraction;
            let source = EffectUniforms::new()
                .with_slot(0, [center * scale, 24. * scale, 48. * scale, 28. * scale])
                .with_slot(1, [12. * scale; 4])
                .with_slot(3, [0., 0., 1., 1.])
                .with_slot(4, [1., 0., 0., 1.]);
            let render = |renderer: &mut WgpuOffscreenRenderer,
                          refraction: f32,
                          thickness: f32|
             -> anyhow::Result<Vec<u8>> {
                let mut scene = Scene::default();
                for (x, stroke_width) in [(8., 3.), (50. + fraction, 3.), (59. + fraction, 1.)] {
                    let mut stroke = content(tile, source, 1., scale).effects.remove(0);
                    stroke.bounds =
                        bounds(x * scale, 4. * scale, stroke_width * scale, 40. * scale);
                    stroke.effect_bounds = stroke.bounds;
                    scene.insert_primitive(stroke);
                }
                let mut lens = glass(center, scale, 0.).backdrop_blurs.remove(0);
                lens.uniforms = lens
                    .uniforms
                    .with_slot(0, [1., 1., refraction * scale, thickness * scale])
                    .with_slot(1, [0.; 4])
                    .with_slot(2, [0., 0., 0., 1.]);
                scene.insert_primitive(lens);
                scene.finish();
                renderer.render_rgba(&scene)
            };
            let flat = render(&mut renderer, 0., 10.)?;
            let refracted = render(&mut renderer, 12., 10.)?;
            let pixel = |pixels: &[u8], x: f32| {
                let index = ((24. * scale) as usize * width + (x * scale) as usize) * 4;
                [pixels[index], pixels[index + 1], pixels[index + 2]]
            };
            assert_eq!(pixel(&flat, 9.), pixel(&refracted, 9.));
            assert_eq!(
                pixel(&refracted, 9.),
                [255, 0, 0],
                "uncovered content keeps its source color"
            );
            assert_eq!(pixel(&flat, 61. + fraction), [0; 3]);
            let bent = pixel(&refracted, 61. + fraction);
            assert!(
                bent[2] > 180 && bent[0] < 10 && bent[1] < 10,
                "the blue stroke must bend beyond its source bounds at scale {scale}, offset {fraction}: {bent:?}"
            );
            assert_eq!(
                pixel(&flat, 51. + fraction),
                pixel(&refracted, 51. + fraction),
                "the flat interior preserves the stroke"
            );
            let compact = render(&mut renderer, 3., 6.)?;
            for x in 28..58 {
                assert_eq!(
                    pixel(&compact, x as f32 + fraction),
                    pixel(&flat, x as f32 + fraction),
                    "a compact lens preserves interior geometry at scale {scale}, offset {fraction}, x {x}"
                );
            }
            assert_eq!(pixel(&flat, 62. + fraction), [0; 3]);
            // A one-pixel source stroke can straddle two atlas samples. Look
            // for displaced coverage across the edge band rather than requiring
            // an opaque result at one subpixel phase.
            let edge = (61..64)
                .map(|x| pixel(&compact, x as f32 + fraction))
                .max_by_key(|pixel| pixel[2])
                .unwrap();
            assert!(
                edge[2] > 64 && edge[0] < 10 && edge[1] < 10,
                "the narrow band must still bend a crossing stroke at scale {scale}, offset {fraction}: {edge:?}"
            );
        }
    }
    Ok(())
}
