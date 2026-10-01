#![cfg(not(target_family = "wasm"))]

use gpui::{
    Bounds, ContentMask, Corners, DevicePixels, ScaledPixels, Scene, Shadow, Underline, point, rgb,
    rgba, size,
};
use gpui_wgpu::WgpuOffscreenRenderer;

const OUTPUT_SRGB: bool = true;
#[path = "support/shadows.rs"]
mod shadows;

#[test]
#[ignore = "requires a GPU adapter"]
fn normalized_shadows_preserve_empty_holes_and_edge_falloff() -> anyhow::Result<()> {
    let mut renderer = WgpuOffscreenRenderer::new(size(DevicePixels(160), DevicePixels(160)))?;
    shadows::check(&mut renderer)
}

fn bounds(x: f32, y: f32, w: f32, h: f32) -> Bounds<ScaledPixels> {
    Bounds::new(
        point(ScaledPixels(x), ScaledPixels(y)),
        size(ScaledPixels(w), ScaledPixels(h)),
    )
}

#[test]
#[ignore = "requires a GPU adapter"]
fn shared_shadows_preserve_inset_blur_and_clipping() -> anyhow::Result<()> {
    let mut scene = Scene::default();
    for index in 0..4 {
        let x = (index % 2) as f32 * 128.;
        let y = (index / 2) as f32 * 96.;
        scene.insert_primitive(Shadow {
            order: Default::default(),
            blur_radius: ScaledPixels(if index % 2 == 0 { 0. } else { 4. }),
            bounds: bounds(x + 24., y + 24., 64., 48.),
            corner_radii: Corners::all(ScaledPixels(10.)),
            content_mask: ContentMask {
                bounds: bounds(x + 12., y + 8., 88., 80.),
            },
            color: rgb(0xffffff).into(),
            element_bounds: bounds(x + 8., y + 8., 96., 80.),
            element_corner_radii: Corners::all(ScaledPixels(16.)),
            inset: u32::from(index >= 2),
            pad: 0,
        });
    }
    scene.finish();
    let mut renderer = WgpuOffscreenRenderer::new(size(DevicePixels(256), DevicePixels(192)))?;
    let pixels = renderer.render_rgba(&scene)?;
    let red = |x: usize, y: usize| pixels[(y * 256 + x) * 4];
    assert_eq!(red(56, 48), 255, "sharp outer shadow center");
    assert!(red(184, 48) > 250, "blurred outer shadow center");
    assert_eq!(red(20, 48), 0, "sharp shadow has no tail");
    assert!(red(148, 48) > 0, "blurred shadow has a tail");
    for x in [0, 128] {
        assert_eq!(red(x + 56, 144), 0, "inset hole stays empty");
        assert!(red(x + 16, 144) > 200, "inset edge is filled");
        assert_eq!(red(x + 10, 144), 0, "content mask clips inset shadow");
        assert_eq!(red(x + 12, 104), 0, "rounded element clips inset corner");
    }
    Ok(())
}

#[test]
#[ignore = "requires a GPU adapter"]
fn shared_underlines_apply_opacity_once_for_straight_and_wavy_lines() -> anyhow::Result<()> {
    let mut scene = Scene::default();
    for index in 0..4 {
        let y = index as f32 * 24.;
        scene.insert_primitive(Underline {
            order: Default::default(),
            pad: 0,
            bounds: bounds(8., y + 4., 112., 16.),
            content_mask: ContentMask {
                bounds: bounds(16., y, 96., 24.),
            },
            color: if index % 2 == 0 {
                rgb(0xffffff).into()
            } else {
                rgba(0xffffff80).into()
            },
            thickness: ScaledPixels(3.),
            wavy: (index >= 2).into(),
        });
    }
    scene.finish();
    let mut renderer = WgpuOffscreenRenderer::new(size(DevicePixels(128), DevicePixels(96)))?;
    let pixels = renderer.render_rgba(&scene)?;
    let red = |x: usize, y: usize| pixels[(y * 128 + x) * 4];
    // The render target is sRGB; a 0.5 linear intensity encodes to approximately 188.
    assert_eq!(red(48, 12), 255);
    assert!(
        (187..=189).contains(&red(48, 36)),
        "straight opacity: {}",
        red(48, 36)
    );
    let mut fully_covered = 0;
    let mut empty = 0;
    for y in 52..68 {
        for x in 16..112 {
            if red(x, y) == 255 {
                assert!((187..=189).contains(&red(x, y + 24)), "wavy opacity");
                fully_covered += 1;
            } else if red(x, y) == 0 {
                assert_eq!(red(x, y + 24), 0);
                empty += 1;
            }
        }
    }
    assert!(
        fully_covered > 50 && empty > 50,
        "wave has both filled and empty pixels"
    );
    for y in 0..96 {
        assert_eq!(red(12, y), 0, "left clip");
        assert_eq!(red(116, y), 0, "right clip");
    }
    Ok(())
}
