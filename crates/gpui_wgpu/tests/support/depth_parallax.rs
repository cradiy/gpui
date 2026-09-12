use super::*;
use gpui::{AtlasTile, ImageId, RenderImageParams};
use gpui_effects::{DepthParallaxOptions, depth_parallax_shader};

fn tile(
    renderer: &WgpuOffscreenRenderer,
    id: usize,
    width: i32,
    height: i32,
    bytes: &[u8],
) -> anyhow::Result<AtlasTile> {
    renderer
        .sprite_atlas()
        .get_or_insert_with(
            &RenderImageParams {
                image_id: ImageId(97000 + id),
                frame_index: 0,
            }
            .into(),
            &mut || {
                Ok(Some((
                    size(DevicePixels(width), DevicePixels(height)),
                    std::borrow::Cow::Borrowed(bytes),
                )))
            },
        )?
        .ok_or_else(|| anyhow::anyhow!("missing depth test tile"))
}

fn scene(
    color: AtlasTile,
    depth: AtlasTile,
    region: Bounds<ScaledPixels>,
    options: DepthParallaxOptions,
) -> Scene {
    let mut scene = Scene::default();
    scene.insert_primitive(EffectQuad {
        order: 0,
        bounds: region,
        effect_bounds: region,
        transformation: Default::default(),
        content_mask: ContentMask { bounds: region },
        shader: depth_parallax_shader(),
        uniforms: options.uniforms(),
        time: 0.,
        corner_radii: Default::default(),
        opacity: 1.,
        image_tile: Some(color),
        second_image_tile: Some(depth),
        third_image_tile: None,
        fourth_image_tile: None,
    });
    scene.finish();
    scene
}

#[test]
#[ignore = "requires a GPU adapter"]
fn depth_parallax_preserves_framing_and_resolves_depth() -> anyhow::Result<()> {
    let mut renderer = WgpuOffscreenRenderer::new(size(DevicePixels(64), DevicePixels(32)))?;
    check(&mut renderer)
}

pub(super) fn check(renderer: &mut WgpuOffscreenRenderer) -> anyhow::Result<()> {
    let gradient = (0..32)
        .flat_map(|y| (0..64).flat_map(move |x| [0, (y * 8) as u8, (x * 4) as u8, 255]))
        .collect::<Vec<_>>();
    let color = tile(renderer, 0, 64, 32, &gradient)?;
    let near = tile(renderer, 1, 1, 1, &[255; 4])?;
    let far = tile(renderer, 2, 1, 1, &[0, 0, 0, 255])?;
    let middle = tile(renderer, 3, 1, 1, &[128, 128, 128, 255])?;
    let transparent = tile(renderer, 4, 1, 1, &[255, 255, 255, 0])?;
    for (width, height) in [(64, 32), (32, 64), (128, 64)] {
        renderer.resize(size(DevicePixels(width), DevicePixels(height)));
        let region = bounds(0., 0., width as f32, height as f32);
        let still = DepthParallaxOptions {
            strength: 0.15,
            focus: 128. / 255.,
            ..Default::default()
        };
        let moving = DepthParallaxOptions {
            offset: point(1., 0.5),
            ..still
        };
        let reference = renderer.render_rgba(&scene(color, middle, region, still))?;
        for depth in [middle, transparent] {
            let actual = renderer.render_rgba(&scene(color, depth, region, moving))?;
            assert!(
                actual
                    .iter()
                    .zip(&reference)
                    .all(|(a, b)| a.abs_diff(*b) <= 1)
            );
        }
        let forward = renderer.render_rgba(&scene(color, near, region, moving))?;
        let backward = renderer.render_rgba(&scene(color, far, region, moving))?;
        let center = ((height / 2 * width + width / 2) * 4) as usize;
        assert!(forward[center] < reference[center] && reference[center] < backward[center]);
        assert!(
            forward[center + 1] < reference[center + 1]
                && reference[center + 1] < backward[center + 1]
        );
        let inverted = renderer.render_rgba(&scene(
            color,
            near,
            region,
            DepthParallaxOptions {
                invert_depth: true,
                ..moving
            },
        ))?;
        assert!(
            inverted
                .iter()
                .zip(&backward)
                .all(|(a, b)| a.abs_diff(*b) <= 1)
        );
        let zero = DepthParallaxOptions {
            strength: 0.,
            ..moving
        };
        assert_eq!(
            renderer.render_rgba(&scene(color, near, region, zero))?,
            renderer.render_rgba(&scene(color, far, region, zero))?
        );
    }
    let stripe_color = (0..32)
        .flat_map(|_| {
            (0..64).flat_map(|x| {
                if (26..38).contains(&x) {
                    [0, 0, 255, 255]
                } else {
                    [255, 0, 0, 255]
                }
            })
        })
        .collect::<Vec<_>>();
    let stripe_depth = (0..32)
        .flat_map(|_| {
            (0..64).flat_map(|x| {
                if (26..38).contains(&x) {
                    [255; 4]
                } else {
                    [0, 0, 0, 255]
                }
            })
        })
        .collect::<Vec<_>>();
    let color = tile(renderer, 5, 64, 32, &stripe_color)?;
    let depth = tile(renderer, 6, 64, 32, &stripe_depth)?;
    renderer.resize(size(DevicePixels(64), DevicePixels(32)));
    let region = bounds(0., 0., 64., 32.);
    let options = DepthParallaxOptions {
        strength: 0.15,
        ..Default::default()
    };
    let still = renderer.render_rgba(&scene(color, depth, region, options))?;
    let moving = renderer.render_rgba(&scene(
        color,
        depth,
        region,
        DepthParallaxOptions {
            offset: point(1., 0.),
            ..options
        },
    ))?;
    let revealed_edge = (16 * 64 + 41) * 4;
    assert!(still[revealed_edge + 2] > 240);
    assert!(moving[revealed_edge] > 240 && moving[revealed_edge + 2] < 15);
    Ok(())
}
