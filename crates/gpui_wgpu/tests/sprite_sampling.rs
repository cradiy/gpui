#![cfg(not(target_family = "wasm"))]

use std::borrow::Cow;

use gpui::{
    Bounds, ContentMask, DevicePixels, ImageId, PolychromeSprite, Quad, RenderImageParams,
    ScaledPixels, Scene, point, rgba, size,
};
use gpui_wgpu::WgpuOffscreenRenderer;

fn bounds(x: f32, y: f32, width: f32, height: f32) -> Bounds<ScaledPixels> {
    Bounds::new(
        point(ScaledPixels(x), ScaledPixels(y)),
        size(ScaledPixels(width), ScaledPixels(height)),
    )
}

#[test]
#[ignore = "requires a GPU adapter"]
fn enlarged_image_edges_do_not_sample_neighboring_atlas_pixels() -> anyhow::Result<()> {
    let mut renderer = WgpuOffscreenRenderer::new(size(DevicePixels(80), DevicePixels(80)))?;
    // A white image surrounded by opaque colored texels in the same atlas.
    let mut pixels = [0, 0, 255, 255].repeat(32 * 32);
    for y in 12..20 {
        for x in 12..20 {
            pixels[(y * 32 + x) * 4..(y * 32 + x + 1) * 4].fill(255);
        }
    }
    let mut tile = renderer
        .sprite_atlas()
        .get_or_insert_with(
            &RenderImageParams {
                image_id: ImageId(1),
                frame_index: 0,
            }
            .into(),
            &mut || {
                Ok(Some((
                    size(DevicePixels(32), DevicePixels(32)),
                    Cow::Borrowed(&pixels),
                )))
            },
        )?
        .unwrap();
    tile.bounds.origin.x += DevicePixels(12);
    tile.bounds.origin.y += DevicePixels(12);
    tile.bounds.size = size(DevicePixels(8), DevicePixels(8));

    for (extent, offset, radius) in [(8., 8., 0.), (48., 8., 0.), (63.5, 8.25, 12.)] {
        let region = bounds(offset, offset, extent, extent);
        let viewport = bounds(0., 0., 80., 80.);
        let mut scene = Scene::default();
        scene.insert_primitive(Quad {
            bounds: viewport,
            content_mask: ContentMask { bounds: viewport },
            background: rgba(0xffffffff).into(),
            ..Default::default()
        });
        scene.insert_primitive(PolychromeSprite {
            order: 0,
            pad: 0,
            grayscale: false.into(),
            opacity: 1.,
            bounds: region,
            clip_bounds: region,
            content_mask: ContentMask { bounds: viewport },
            corner_radii: gpui::Corners::all(ScaledPixels(radius)),
            tile,
            transformation: Default::default(),
        });
        scene.finish();
        let result = renderer.render_rgba(&scene)?;
        for (index, pixel) in result.chunks_exact(4).enumerate() {
            assert!(
                pixel.iter().all(|channel| *channel >= 254),
                "extent {extent}, offset {offset}, radius {radius}, pixel {},{}: {pixel:?}",
                index % 80,
                index / 80,
            );
        }
    }
    Ok(())
}

#[test]
#[ignore = "requires a GPU adapter"]
fn color_sprites_preserve_filtering_alpha_and_transformed_clipping() -> anyhow::Result<()> {
    let mut renderer = WgpuOffscreenRenderer::new(size(DevicePixels(96), DevicePixels(80)))?;
    let viewport = bounds(0., 0., 96., 80.);
    // BGRA: opaque black/white for filtering, then translucent green for blending.
    for (id, pixels, grayscale, opacity) in [
        (2, vec![0, 0, 0, 255, 255, 255, 255, 255], false, 1.),
        (3, vec![0, 255, 0, 128, 0, 255, 0, 128], true, 0.5),
    ] {
        let tile = renderer
            .sprite_atlas()
            .get_or_insert_with(
                &RenderImageParams {
                    image_id: ImageId(id),
                    frame_index: 0,
                }
                .into(),
                &mut || {
                    Ok(Some((
                        size(DevicePixels(2), DevicePixels(1)),
                        Cow::Borrowed(&pixels),
                    )))
                },
            )?
            .unwrap();
        let mut scene = Scene::default();
        scene.insert_primitive(Quad {
            bounds: viewport,
            content_mask: ContentMask { bounds: viewport },
            background: rgba(0x000000ff).into(),
            ..Default::default()
        });
        scene.insert_primitive(PolychromeSprite {
            order: 0,
            pad: 0,
            grayscale: grayscale.into(),
            opacity,
            bounds: bounds(0., 0., 32., 24.),
            clip_bounds: bounds(0., 0., 32., 24.),
            content_mask: ContentMask {
                bounds: bounds(24., 0., 48., 80.),
            },
            corner_radii: gpui::Corners::all(ScaledPixels(4.)),
            tile,
            transformation: gpui::TransformationMatrix {
                rotation_scale: [[2., 0.], [0., 2.]],
                translation: [16., 16.],
            },
        });
        scene.finish();
        let result = renderer.render_rgba(&scene)?;
        let pixel = |x: usize, y: usize| &result[(y * 96 + x) * 4..(y * 96 + x + 1) * 4];
        for (x, y) in [(8, 32), (20, 32), (76, 32), (48, 8), (48, 70)] {
            assert_eq!(
                pixel(x, y),
                [0, 0, 0, 255],
                "outside transformed clip {x},{y}"
            );
        }
        if grayscale {
            // Luminance 0.7152, texel alpha 128/255 and opacity 0.5, over black.
            let expected = ((1.055 * (0.7152_f32 * (128. / 255.) * 0.5).powf(1. / 2.4) - 0.055)
                * 255.)
                .round() as i16;
            for channel in &pixel(48, 32)[..3] {
                assert!(
                    (*channel as i16 - expected).abs() <= 2,
                    "blended grayscale: {:?}",
                    pixel(48, 32)
                );
            }
        } else {
            assert_eq!(pixel(24, 32), [0, 0, 0, 255]);
            assert_eq!(pixel(71, 32), [255, 255, 255, 255]);
            let center = pixel(48, 32);
            assert!(
                (186..=192).contains(&center[0]),
                "linear filtering: {center:?}"
            );
            assert_eq!(center[0], center[1]);
            assert_eq!(center[1], center[2]);
        }
    }
    Ok(())
}
