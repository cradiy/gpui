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
