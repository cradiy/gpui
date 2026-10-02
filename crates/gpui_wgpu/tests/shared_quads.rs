#![cfg(not(target_family = "wasm"))]

use gpui::{
    Background, BorderStyle, Bounds, ColorSpace, ContentMask, Corners, DevicePixels, Edges,
    GradientKind, Quad, ScaledPixels, Scene, border_color_stop, border_gradient, checkerboard,
    linear_color_stop, multi_linear_gradient, pattern_slash, point, rgb, rgba, size,
};
use gpui_wgpu::WgpuOffscreenRenderer;

fn border_sample_scene(scene: &mut Scene, gradient: gpui::BorderGradient, position: f32) {
    scene.clear();
    scene.insert_primitive(Quad {
        bounds: bounds(0., 0., 128., 128.),
        content_mask: ContentMask {
            bounds: bounds(0., 0., 128., 128.),
        },
        border_gradient: gradient.phase(position - 64.5 / 512.),
        border_widths: Edges::all(ScaledPixels(8.)),
        ..Default::default()
    });
    scene.finish();
}

fn assert_border_sample(pixels: &[u8], expected: [u8; 3]) {
    let pixel = &pixels[(2 * 128 + 64) * 4..][..4];
    for (actual, expected) in pixel[..3].iter().zip(expected) {
        assert!(
            actual.abs_diff(expected) <= 3,
            "sample {pixel:?}, expected {expected}"
        );
    }
}

#[test]
#[ignore = "requires a GPU adapter"]
fn border_hard_edges_choose_the_last_coincident_stop() -> anyhow::Result<()> {
    let mut renderer = WgpuOffscreenRenderer::new(size(DevicePixels(128), DevicePixels(128)))?;
    let mut scene = Scene::default();
    let red = |position| border_color_stop(rgb(0xff0000), position);
    let blue = |position| border_color_stop(rgb(0x0000ff), position);
    let epsilon = 1. / 65536.;
    for (stops, edge) in [
        (vec![red(0.), red(0.5), blue(0.5), blue(1.)], 0.5),
        (
            vec![red(0.5), border_color_stop(rgb(0x00ff00), 0.5), blue(0.5)],
            0.5,
        ),
        (vec![red(0.5), blue(0.5)], 0.5),
        (vec![red(0.), blue(0.)], 0.),
        (vec![red(1.), blue(1.)], 0.),
        (vec![red(0.), blue(0.), blue(0.5), red(0.5), red(1.)], 0.),
    ] {
        let gradient = border_gradient(stops).color_space(ColorSpace::Srgb);
        for (position, expected) in [
            (edge - epsilon, [255, 0, 0]),
            (edge, [0, 0, 255]),
            (edge + epsilon, [0, 0, 255]),
        ] {
            border_sample_scene(&mut scene, gradient.clone(), position);
            assert_border_sample(&renderer.render_rgba(&scene)?, expected);
        }
    }
    // A short but nonzero segment must not be widened by an epsilon clamp.
    border_sample_scene(
        &mut scene,
        border_gradient([red(0.5), blue(0.5 + epsilon)]).color_space(ColorSpace::Srgb),
        0.5 + epsilon * 0.5,
    );
    assert_border_sample(&renderer.render_rgba(&scene)?, [128, 0, 128]);
    Ok(())
}

#[test]
#[ignore = "requires a GPU adapter"]
fn border_midpoints_control_regular_and_wraparound_segments() -> anyhow::Result<()> {
    let mut renderer = WgpuOffscreenRenderer::new(size(DevicePixels(128), DevicePixels(128)))?;
    let mut scene = Scene::default();
    for color_space in [ColorSpace::Srgb, ColorSpace::Oklab] {
        for count in [2, 3] {
            let mut stops = vec![
                border_color_stop(rgb(0), 0.),
                border_color_stop(rgb(0xffffff), 0.5),
            ];
            if count == 3 {
                stops.push(stops[1]);
            }
            let base = border_gradient(stops).color_space(color_space);
            let hinted = base
                .clone()
                .gradient_midpoint(0, 0.25)
                .gradient_midpoint(count - 1, 0.75);
            let expected = if color_space == ColorSpace::Srgb {
                128
            } else {
                99
            };
            for position in [0.125, 0.875] {
                border_sample_scene(&mut scene, hinted.clone(), position);
                let pixels = renderer.render_rgba(&scene)?;
                assert_border_sample(&pixels, [expected; 3]);
                border_sample_scene(&mut scene, base.clone(), position);
                assert_ne!(pixels, renderer.render_rgba(&scene)?);
            }
            if count == 3 {
                border_sample_scene(&mut scene, hinted.clone().gradient_midpoint(1, 0.1), 0.5);
                assert_border_sample(&renderer.render_rgba(&scene)?, [255; 3]);
            }
        }
    }
    let original = border_gradient([
        border_color_stop(rgb(0), 0.),
        border_color_stop(rgb(0xffffff), 0.5),
    ])
    .color_space(ColorSpace::Srgb)
    .gradient_midpoint(0, 0.25);
    let mut edited = original.clone();
    for frame in 0..8 {
        let midpoint = if frame % 2 == 0 { 0.25 } else { 0.75 };
        edited = edited.gradient_midpoint(0, midpoint);
        border_sample_scene(&mut scene, edited.clone(), 0.125);
        assert_border_sample(
            &renderer.render_rgba(&scene)?,
            [if midpoint == 0.25 { 128 } else { 9 }; 3],
        );
        if frame > 0 {
            assert_eq!(
                renderer.memory_stats().gradient_upload_bytes,
                std::mem::size_of::<gpui::GpuGradientStop>() as u64
            );
        }
    }
    border_sample_scene(&mut scene, original, 0.125);
    assert_border_sample(&renderer.render_rgba(&scene)?, [128; 3]);
    Ok(())
}

#[test]
#[ignore = "requires a GPU adapter"]
fn long_border_edits_upload_one_stop_and_preserve_phase_and_snapshots() -> anyhow::Result<()> {
    let mut renderer = WgpuOffscreenRenderer::new(size(DevicePixels(128), DevicePixels(128)))?;
    let original = border_gradient(
        (0..32)
            .map(|i| border_color_stop(rgb(0xff0000), i as f32 / 32.))
            .collect::<Vec<_>>(),
    )
    .color_space(ColorSpace::Srgb);
    let fill = multi_linear_gradient(
        90.,
        [
            linear_color_stop(rgb(0x00ff00), 0.),
            linear_color_stop(rgb(0x00ff00), 0.5),
            linear_color_stop(rgb(0x00ff00), 1.),
        ],
    );
    let mut border = original.clone();
    let paint = |scene: &mut Scene, border| {
        scene.clear();
        scene.insert_primitive(Quad {
            bounds: bounds(0., 0., 128., 128.),
            content_mask: ContentMask {
                bounds: bounds(0., 0., 128., 128.),
            },
            background: fill.clone(),
            border_gradient: border,
            border_widths: Edges::all(ScaledPixels(8.)),
            ..Default::default()
        });
        scene.finish();
    };
    let mut scene = Scene::default();
    for frame in 0..8 {
        let blue = frame % 2 == 0;
        border.set_gradient_stop(
            4,
            border_color_stop(rgb(if blue { 0x0000ff } else { 0xff0000 }), 0.125),
        );
        paint(&mut scene, border.clone());
        let pixels = renderer.render_rgba(&scene)?;
        let pixel = &pixels[(2 * 128 + 64) * 4..][..4];
        assert!(pixel[if blue { 2 } else { 0 }] > 230, "{pixel:?}");
        assert_gradient_plateau(&pixels[(64 * 128 + 64) * 4..][..4], [0., 1., 0.]);
        if frame > 0 {
            assert_eq!(
                renderer.memory_stats().gradient_upload_bytes,
                std::mem::size_of::<gpui::GpuGradientStop>() as u64
            );
        }
    }
    border.set_gradient_stop(4, border_color_stop(rgb(0x0000ff), 0.125));
    paint(&mut scene, border.clone());
    renderer.render_rgba(&scene)?;
    paint(&mut scene, border.phase(0.5));
    let pixels = renderer.render_rgba(&scene)?;
    assert_eq!(renderer.memory_stats().gradient_upload_bytes, 0);
    assert!(pixels[(125 * 128 + 63) * 4 + 2] > 230);
    paint(&mut scene, original);
    let pixels = renderer.render_rgba(&scene)?;
    assert_gradient_plateau(&pixels[(2 * 128 + 64) * 4..][..4], [1., 0., 0.]);
    Ok(())
}

#[test]
#[ignore = "requires a GPU adapter"]
fn external_border_matches_inline_at_seams_corners_and_alpha() -> anyhow::Result<()> {
    let mut renderer = WgpuOffscreenRenderer::new(size(DevicePixels(96), DevicePixels(96)))?;
    for color_space in [ColorSpace::Srgb, ColorSpace::Oklab] {
        for phase in [0., 0.15, 0.5, 0.95] {
            let render = |renderer: &mut WgpuOffscreenRenderer, stops: &[gpui::BorderColorStop]| {
                let mut scene = Scene::default();
                scene.insert_primitive(Quad {
                    bounds: bounds(4., 4., 88., 88.),
                    content_mask: ContentMask {
                        bounds: bounds(0., 0., 96., 96.),
                    },
                    border_gradient: border_gradient(stops).color_space(color_space).phase(phase),
                    border_widths: Edges::all(ScaledPixels(8.)),
                    corner_radii: Corners::all(ScaledPixels(16.)),
                    ..Default::default()
                });
                scene.finish();
                renderer.render_rgba(&scene)
            };
            // Alpha varies linearly in both color spaces, including across the
            // cyclic seam. The extra stop must preserve the same interpolation.
            let inline = [
                border_color_stop(rgba(0xcc336640), 0.2),
                border_color_stop(rgba(0xcc3366c0), 0.7),
            ];
            let external = [
                inline[0],
                border_color_stop(rgba(0xcc336680), 0.45),
                inline[1],
            ];
            let a = render(&mut renderer, &inline)?;
            let b = render(&mut renderer, &external)?;
            for (a, b) in a.iter().zip(&b) {
                assert!(a.abs_diff(*b) <= 1);
            }
            let visible = a.chunks_exact(4).filter(|p| p[0] > 30).count();
            assert!(visible > 500, "border must actually be visible");
            let opaque = inline.map(|mut stop| {
                stop.color.a = 1.;
                stop
            });
            assert_ne!(
                a,
                render(&mut renderer, &opaque)?,
                "alpha must affect compositing on the opaque target"
            );
        }
    }
    Ok(())
}

#[test]
#[ignore = "requires a GPU adapter"]
fn long_gradient_edits_refresh_pixels_and_preserve_snapshots() -> anyhow::Result<()> {
    let mut renderer = WgpuOffscreenRenderer::new(size(DevicePixels(380), DevicePixels(64)))?;
    let mut background = multi_linear_gradient(
        90.,
        std::array::from_fn::<_, 20, _>(|i| linear_color_stop(rgb(0xff0000), i as f32 / 19.)),
    )
    .color_space(ColorSpace::Srgb);
    let paint = |scene: &mut Scene, background: Background| {
        scene.clear();
        scene.insert_primitive(Quad {
            bounds: bounds(0., 0., 380., 64.),
            content_mask: ContentMask {
                bounds: bounds(0., 0., 380., 64.),
            },
            background,
            ..Default::default()
        });
        scene.finish();
    };
    let mut original = Scene::default();
    paint(&mut original, background.clone());
    let mut edited = Scene::default();
    for frame in 0..12 {
        let blue = frame % 2 == 0;
        background.set_gradient_stop(
            10,
            linear_color_stop(rgb(if blue { 0x0000ff } else { 0xff0000 }), 10. / 19.),
        );
        paint(&mut edited, background.clone());
        let pixels = renderer.render_rgba(&edited)?;
        if frame > 0 {
            assert_eq!(
                renderer.memory_stats().gradient_upload_bytes,
                std::mem::size_of::<gpui::GpuGradientStop>() as u64
            );
        }
        let pixel = &pixels[(32 * 380 + 200) * 4..][..4];
        assert!(
            pixel[if blue { 2 } else { 0 }] > 235,
            "frame {frame}: {pixel:?}"
        );
        assert_eq!(pixel[3], 255);
        assert_gradient_plateau(&pixels[(32 * 380 + 30) * 4..][..4], [1., 0., 0.]);
    }
    let pixels = renderer.render_rgba(&original)?;
    assert_gradient_plateau(&pixels[(32 * 380 + 200) * 4..][..4], [1., 0., 0.]);
    renderer.render_rgba(&original)?;
    assert_eq!(renderer.memory_stats().gradient_upload_bytes, 0);
    Ok(())
}

#[test]
#[ignore = "requires a GPU adapter; prints CPU preparation and render/readback timings"]
fn gradient_edit_render_timings() -> anyhow::Result<()> {
    fn measure<const N: usize>() -> anyhow::Result<()> {
        let mut renderer = WgpuOffscreenRenderer::new(size(DevicePixels(900), DevicePixels(600)))?;
        let mut background = multi_linear_gradient(
            90.,
            std::array::from_fn::<_, N, _>(|i| {
                linear_color_stop(
                    gpui::Hsla {
                        h: i as f32 / N as f32,
                        s: 0.8,
                        l: 0.55,
                        a: 1.,
                    },
                    i as f32 / (N - 1) as f32,
                )
            }),
        )
        .color_space(ColorSpace::Oklab);
        let mut scene = Scene::default();
        let mut prepare = Vec::new();
        let mut render = Vec::new();
        for frame in 0..45 {
            let start = std::time::Instant::now();
            let mut stop = background.gradient_stops()[1];
            stop.color.h = (frame as f32 * 0.01) % 1.;
            background.set_gradient_stop(1, stop);
            scene.clear();
            for index in 0..100 {
                scene.insert_primitive(Quad {
                    bounds: bounds(
                        (index % 10) as f32 * 90.,
                        (index / 10) as f32 * 60.,
                        90.,
                        60.,
                    ),
                    content_mask: ContentMask {
                        bounds: bounds(0., 0., 900., 600.),
                    },
                    background: background.clone(),
                    ..Default::default()
                });
            }
            scene.finish();
            let cpu = start.elapsed().as_secs_f64() * 1000.;
            let start = std::time::Instant::now();
            renderer.render_rgba(&scene)?;
            let gpu = start.elapsed().as_secs_f64() * 1000.;
            if frame >= 5 {
                prepare.push(cpu);
                render.push(gpu);
            }
        }
        prepare.sort_by(f64::total_cmp);
        render.sort_by(f64::total_cmp);
        eprintln!(
            "{N} stops / 100 quads: CPU p50 {:.3} / p95 {:.3} ms; render+readback p50 {:.3} / p95 {:.3} ms; stop upload {} bytes",
            prepare[20],
            prepare[38],
            render[20],
            render[38],
            renderer.memory_stats().gradient_upload_bytes
        );
        Ok(())
    }
    measure::<2>()?;
    measure::<4>()?;
    measure::<20>()?;
    measure::<256>()
}

#[test]
#[ignore = "requires a GPU adapter"]
fn external_gradient_matches_inline_geometry_midpoints_and_opacity() -> anyhow::Result<()> {
    let mut renderer = WgpuOffscreenRenderer::new(size(DevicePixels(96), DevicePixels(96)))?;
    let first = linear_color_stop(rgba(0xf04020d0), 0.);
    let last = linear_color_stop(rgba(0x2070ff80), 1.);
    for space in [ColorSpace::Srgb, ColorSpace::Oklab] {
        for kind in [
            GradientKind::Linear,
            GradientKind::Radial,
            GradientKind::Angular,
            GradientKind::Diamond,
        ] {
            let render = |renderer: &mut WgpuOffscreenRenderer,
                          background: Background|
             -> anyhow::Result<Vec<u8>> {
                let mut scene = Scene::default();
                scene.insert_primitive(Quad {
                    bounds: bounds(0., 0., 96., 96.),
                    content_mask: ContentMask {
                        bounds: bounds(0., 0., 96., 96.),
                    },
                    background: background
                        .color_space(space)
                        .gradient_kind(kind)
                        .opacity(0.6),
                    ..Default::default()
                });
                scene.finish();
                renderer.render_rgba(&scene)
            };
            let inline = render(
                &mut renderer,
                multi_linear_gradient(35., [first, last]).gradient_midpoint(0, 0.25),
            )?;
            let external = render(
                &mut renderer,
                multi_linear_gradient(35., [first, first, last]).gradient_midpoint(1, 0.25),
            )?;
            for (index, (a, b)) in inline.iter().zip(&external).enumerate() {
                assert!(
                    a.abs_diff(*b) <= 2,
                    "{space:?} / {kind:?}, byte {index}: {a} != {b}"
                );
            }
        }
    }
    Ok(())
}

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
        smooth.clone().clone(),
        smooth.clone().color_space(ColorSpace::Srgb),
        smooth.clone().gradient_kind(GradientKind::Radial),
        smooth
            .clone()
            .gradient_kind(GradientKind::Angular)
            .angular_seam_width(0.1),
        smooth.clone().gradient_kind(GradientKind::Diamond),
        smooth.clone().gradient_midpoint(0, 0.25),
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
    for (x, expected) in [
        (76, [1., 0., 0.]),
        (95, [1., 0., 0.]),
        (96, [0., 0., 1.]),
        (108, [0., 0., 1.]),
    ] {
        assert_gradient_plateau(pixel(x, 32), expected);
    }
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

fn assert_gradient_plateau(pixel: &[u8], linear_rgb: [f32; 3]) {
    // The shader adds linear RGB and alpha noise before premultiplied blending.
    // Encode those bounds for the sRGB attachment and allow one readback LSB.
    let encode = |linear: f32| {
        let linear = linear.clamp(0., 1.);
        let srgb = if linear <= 0.0031308 {
            linear * 12.92
        } else {
            1.055 * linear.powf(1. / 2.4) - 0.055
        };
        srgb * 255.
    };
    for (channel, expected) in linear_rgb.into_iter().enumerate() {
        let low = encode((expected - 2. / 255.).max(0.) * (1. - 3. / 255.)) - 1.;
        let high = encode((expected + 2. / 255.) * (1. + 3. / 255.)) + 1.;
        assert!(
            (low..=high).contains(&(pixel[channel] as f32)),
            "gradient plateau {linear_rgb:?}, channel {channel}: {pixel:?}, expected {low}..={high}"
        );
    }
}
