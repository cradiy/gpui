#![cfg(not(target_family = "wasm"))]

use gpui::{
    Background, BorderStyle, Bounds, ColorSpace, ContentMask, Corners, DevicePixels, Edges,
    GradientKind, Quad, ScaledPixels, Scene, border_color_stop, border_gradient, checkerboard,
    linear_color_stop, multi_linear_gradient, pattern_slash, point, rgb, rgba, size,
};
use gpui_wgpu::WgpuOffscreenRenderer;

fn bounds(x: f32, y: f32, w: f32, h: f32) -> Bounds<ScaledPixels> {
    Bounds::new(
        point(ScaledPixels(x), ScaledPixels(y)),
        size(ScaledPixels(w), ScaledPixels(h)),
    )
}

fn scene() -> Scene {
    let mut scene = Scene::default();
    let gradient = multi_linear_gradient(
        90.,
        [
            linear_color_stop(rgb(0xff0000), 0.),
            linear_color_stop(rgb(0xff0000), 0.5),
            linear_color_stop(rgb(0x0000ff), 0.5),
            linear_color_stop(rgb(0x0000ff), 1.),
        ],
    )
    .color_space(ColorSpace::Srgb);
    let smooth = multi_linear_gradient(
        35.,
        [
            linear_color_stop(rgba(0xff6020cc), 0.),
            linear_color_stop(rgba(0x20ccffff), 0.4),
            linear_color_stop(rgba(0x9020ff80), 1.),
        ],
    );
    let backgrounds: [Background; 12] = [
        rgb(0xff0000).into(),
        gradient,
        smooth,
        smooth.color_space(ColorSpace::Srgb),
        smooth.gradient_kind(GradientKind::Radial),
        smooth
            .gradient_kind(GradientKind::Angular)
            .angular_seam_width(0.1),
        smooth.gradient_kind(GradientKind::Diamond),
        smooth.gradient_midpoint(0, 0.25),
        checkerboard(rgb(0x00ff00), 8.),
        pattern_slash(rgb(0xff8000), 4., 8.),
        rgba(0xffffff80).into(),
        rgb(0x204080).into(),
    ];
    for (index, background) in backgrounds.into_iter().enumerate() {
        let x = (index % 4) as f32 * 64.;
        let y = (index / 4) as f32 * 64.;
        let mut quad = Quad {
            bounds: bounds(x + 4., y + 4., 56., 56.),
            content_mask: ContentMask {
                bounds: bounds(x + 4., y + 4., 48., 56.),
            },
            background,
            ..Default::default()
        };
        if index >= 2 {
            quad.corner_radii = Corners::all(ScaledPixels(12.));
            quad.border_widths = Edges {
                top: ScaledPixels(2.),
                right: ScaledPixels(6.),
                bottom: ScaledPixels(4.),
                left: ScaledPixels(8.),
            };
            quad.border_colors = Edges::all(rgb(0xffffff).into());
            if index % 2 == 0 {
                quad.border_style = BorderStyle::Dashed;
            }
            if index % 3 == 0 {
                quad.border_gradient = border_gradient([
                    border_color_stop(rgb(0xff0000), 0.),
                    border_color_stop(rgb(0x00ff00), 0.3),
                    border_color_stop(rgb(0x0000ff), 0.7),
                ]);
            }
        }
        scene.insert_primitive(quad);
    }
    scene.finish();
    scene
}

#[test]
#[ignore = "requires a GPU adapter"]
fn shared_quad_pixels_preserve_fills_clipping_and_rounded_borders() -> anyhow::Result<()> {
    let mut renderer = WgpuOffscreenRenderer::new(size(DevicePixels(256), DevicePixels(192)))?;
    let pixels = renderer.render_rgba(&scene())?;
    let pixel = |x: usize, y: usize| &pixels[(y * 256 + x) * 4..(y * 256 + x) * 4 + 4];
    assert_eq!(pixel(16, 16), &[255, 0, 0, 255]);
    assert!(pixel(76, 32)[0] > 240 && pixel(76, 32)[2] < 20);
    assert!(pixel(108, 32)[2] > 240 && pixel(108, 32)[0] < 20);
    for index in 0..12 {
        let x = index % 4 * 64;
        let y = index / 4 * 64;
        assert_eq!(&pixel(x + 56, y + 32)[..3], &[0, 0, 0], "clip {index}");
        assert!(
            [24, 32, 40]
                .into_iter()
                .any(|dx| pixel(x + dx, y + 24)[..3].iter().any(|c| *c > 20)),
            "fill {index}"
        );
        if index >= 2 {
            assert_eq!(&pixel(x + 4, y + 4)[..3], &[0, 0, 0], "corner {index}");
        }
    }
    Ok(())
}
