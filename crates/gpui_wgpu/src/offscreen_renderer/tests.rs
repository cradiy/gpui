use super::*;
use gpui::{
    Bounds, ContentMask, FontId, GlyphId, MonochromeSprite, RenderGlyphParams, ScaledPixels,
    SubpixelSprite, TransformationMatrix, point, px, rgba, size,
};
use std::borrow::Cow;

fn bounds(x: f32, y: f32, width: f32, height: f32) -> Bounds<ScaledPixels> {
    Bounds::new(
        point(ScaledPixels(x), ScaledPixels(y)),
        size(ScaledPixels(width), ScaledPixels(height)),
    )
}

fn glyph_tile(
    renderer: &WgpuOffscreenRenderer,
    subpixel: bool,
    pixels: &[u8],
) -> anyhow::Result<gpui::AtlasTile> {
    Ok(renderer
        .sprite_atlas()
        .get_or_insert_with(
            &RenderGlyphParams {
                font_id: FontId(0),
                glyph_id: GlyphId(1),
                font_size: px(12.),
                subpixel_variant: point(0, 0),
                scale_factor: 1.,
                is_emoji: false,
                subpixel_rendering: subpixel,
                dilation: 0,
                blur_radius: 0,
            }
            .into(),
            &mut || {
                Ok(Some((
                    size(DevicePixels(3), DevicePixels(1)),
                    Cow::Borrowed(pixels),
                )))
            },
        )?
        .unwrap())
}

#[test]
#[ignore = "requires a GPU adapter"]
fn shared_monochrome_coverage_opacity_and_transformed_clip() -> anyhow::Result<()> {
    let mut renderer = WgpuOffscreenRenderer::new(size(DevicePixels(80), DevicePixels(48)))?;
    let tile = glyph_tile(&renderer, false, &[0, 128, 255])?;
    let mut scene = Scene::default();
    scene.insert_primitive(MonochromeSprite {
        order: 0,
        pad: 0,
        bounds: bounds(0., 0., 30., 12.),
        content_mask: ContentMask {
            bounds: bounds(8., 8., 54., 24.),
        },
        background: rgba(0xffffff80).into(),
        background_bounds: bounds(0., 0., 30., 12.),
        tile,
        transformation: TransformationMatrix {
            rotation_scale: [[2., 0.], [0., 2.]],
            translation: [8.5, 8.],
        },
    });
    scene.finish();
    let result = renderer.render_rgba(&scene)?;
    let pixel = |x: usize, y: usize| &result[(y * 80 + x) * 4..(y * 80 + x + 1) * 4];
    for (x, y) in [(4, 20), (18, 20), (64, 20), (38, 4), (38, 36)] {
        assert_eq!(&pixel(x, y)[..3], [0, 0, 0], "mask/clip at {x},{y}");
    }
    let full = pixel(58, 20);
    assert!(
        (187..=189).contains(&full[0]),
        "half-opacity white: {full:?}"
    );
    let partial = pixel(38, 20);
    let gamma = std::env::var("ZED_FONTS_GAMMA")
        .ok()
        .and_then(|value| value.parse::<f32>().ok())
        .unwrap_or(1.8)
        .clamp(1., 2.2);
    let ratios = gpui::get_gamma_correction_ratios(gamma);
    // Pixel center 38.5 samples the middle texel's center.
    // White has brightness 1, so enhanced contrast has no effect.
    let coverage = 128. / 255.;
    let corrected = coverage
        + coverage * (1. - coverage) * ((ratios[0] + ratios[1]) * coverage + ratios[2] + ratios[3]);
    let expected = (255. * (1.055 * (corrected * (128. / 255.)).powf(1. / 2.4) - 0.055)).round();
    assert!(
        (partial[0] as f32 - expected).abs() <= 2.,
        "gamma-corrected coverage: {partial:?}, expected {expected}"
    );
    assert!(
        partial[0] > 0 && partial[0] < full[0],
        "partial coverage: {partial:?}"
    );
    assert_eq!(partial[0], partial[1]);
    assert_eq!(partial[1], partial[2]);
    Ok(())
}

#[test]
#[ignore = "requires a GPU adapter"]
fn monochrome_external_gradient_updates_without_replacing_glyph() -> anyhow::Result<()> {
    let mut renderer = WgpuOffscreenRenderer::new(size(DevicePixels(80), DevicePixels(48)))?;
    let tile = glyph_tile(&renderer, false, &[255, 255, 255])?;
    let bounds = bounds(0., 0., 80., 48.);
    let mut background = gpui::multi_linear_gradient(
        90.,
        [
            gpui::linear_color_stop(gpui::rgb(0xff0000), 0.),
            gpui::linear_color_stop(gpui::rgb(0xff0000), 0.5),
            gpui::linear_color_stop(gpui::rgb(0xff0000), 1.),
        ],
    );
    let mut scene = Scene::default();
    for blue in [false, true] {
        if blue {
            background.set_gradient_stop(1, gpui::linear_color_stop(gpui::rgb(0x0000ff), 0.5));
        }
        scene.clear();
        scene.insert_primitive(MonochromeSprite {
            order: 0,
            pad: 0,
            bounds,
            content_mask: ContentMask { bounds },
            background: background.clone(),
            background_bounds: bounds,
            tile,
            transformation: TransformationMatrix::unit(),
        });
        scene.finish();
        let pixels = renderer.render_rgba(&scene)?;
        let pixel = &pixels[(24 * 80 + 40) * 4..][..4];
        assert!(pixel[if blue { 2 } else { 0 }] > 235, "{pixel:?}");
        assert!(pixel[if blue { 0 } else { 2 }] < 35, "{pixel:?}");
    }
    Ok(())
}

#[test]
#[ignore = "requires a GPU adapter with dual-source blending"]
fn shared_subpixel_rgb_bgr_and_clip() -> anyhow::Result<()> {
    let mut renderer = WgpuOffscreenRenderer::new(size(DevicePixels(80), DevicePixels(48)))?;
    anyhow::ensure!(
        renderer.context.supports_dual_source_blending(),
        "test requires dual-source blending"
    );
    // BGRA coverage: red, green, blue. Coverage is independent of the atlas alpha.
    let tile = glyph_tile(
        &renderer,
        true,
        &[0, 0, 255, 255, 0, 255, 0, 255, 255, 0, 0, 255],
    )?;
    let mut scene = Scene::default();
    scene.insert_primitive(SubpixelSprite {
        order: 0,
        pad: 0,
        bounds: bounds(0., 0., 30., 12.),
        content_mask: ContentMask {
            bounds: bounds(8., 8., 54., 24.),
        },
        background: rgba(0xffffff80).into(),
        background_bounds: bounds(0., 0., 30., 12.),
        tile,
        transformation: TransformationMatrix {
            rotation_scale: [[2., 0.], [0., 2.]],
            translation: [8.5, 8.],
        },
    });
    scene.finish();
    for is_bgr in [false, true] {
        renderer.renderer.set_subpixel_layout(is_bgr);
        let result = renderer.render_rgba(&scene)?;
        let pixel = |x: usize, y: usize| &result[(y * 80 + x) * 4..(y * 80 + x + 1) * 4];
        for (x, channel) in [
            (18, if is_bgr { 2 } else { 0 }),
            (58, if is_bgr { 0 } else { 2 }),
        ] {
            let p = pixel(x, 20);
            for (index, value) in p[..3].iter().enumerate() {
                if index == channel {
                    assert!((187..=189).contains(value), "BGR={is_bgr}, {x}: {p:?}");
                } else {
                    assert_eq!(*value, 0, "BGR={is_bgr}, {x}: {p:?}");
                }
            }
        }
        assert_eq!(&pixel(64, 20)[..3], [0, 0, 0]);
        assert_eq!(&pixel(4, 20)[..3], [0, 0, 0]);
    }
    Ok(())
}
