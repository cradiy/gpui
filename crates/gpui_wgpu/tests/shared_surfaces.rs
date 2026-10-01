#![cfg(not(target_family = "wasm"))]

use gpui::{
    Bounds, ColorRange, ContentMask, Corners, DevicePixels, PaintSurface, ScaledPixels, Scene,
    SurfaceColorInfo, SurfaceFormat, SurfaceFrame, SurfaceHandle, SurfacePlane, YuvMatrix, point,
    size,
};
use gpui_wgpu::WgpuOffscreenRenderer;

fn bounds(x: f32, y: f32, width: f32, height: f32) -> Bounds<ScaledPixels> {
    Bounds::new(
        point(ScaledPixels(x), ScaledPixels(y)),
        size(ScaledPixels(width), ScaledPixels(height)),
    )
}

fn scene(frame: SurfaceFrame, opacity: f32) -> Scene {
    let mut scene = Scene::default();
    scene.insert_primitive(PaintSurface {
        order: 0,
        bounds: bounds(8., 8., 64., 48.),
        clip_bounds: bounds(8., 8., 64., 48.),
        content_mask: ContentMask {
            bounds: bounds(0., 0., 60., 64.),
        },
        corner_radii: Corners::all(ScaledPixels(12.)),
        opacity,
        source: frame.into(),
    });
    scene.finish();
    scene
}

fn pixel(data: &[u8], x: usize, y: usize) -> &[u8] {
    &data[(y * 80 + x) * 4..(y * 80 + x + 1) * 4]
}

#[test]
#[ignore = "requires a GPU adapter"]
fn shared_surface_rgba_crop_alpha_and_frame_updates() -> anyhow::Result<()> {
    let mut renderer = WgpuOffscreenRenderer::new(size(DevicePixels(80), DevicePixels(64)))?;
    let handle = SurfaceHandle::new();
    for (sequence, format, green) in [
        (0, SurfaceFormat::Rgba8, false),
        (1, SurfaceFormat::Rgba8, true),
        (2, SurfaceFormat::Bgra8, false),
    ] {
        // Crop a red/green center out of an opaque blue border, with row padding.
        let mut bytes = vec![0; 40 * 8];
        for y in 0..8 {
            for x in 0..8 {
                let mut rgba = if (2..6).contains(&x) && (2..6).contains(&y) {
                    if green {
                        [0, 255, 0, 128]
                    } else {
                        [255, 0, 0, 128]
                    }
                } else {
                    [0, 0, 255, 255]
                };
                if format == SurfaceFormat::Bgra8 {
                    rgba.swap(0, 2);
                }
                bytes[y * 40 + x * 4..y * 40 + x * 4 + 4].copy_from_slice(&rgba);
            }
        }
        let frame = SurfaceFrame::new(
            handle.clone(),
            sequence,
            size(DevicePixels(8), DevicePixels(8)),
            Bounds::new(
                point(DevicePixels(2), DevicePixels(2)),
                size(DevicePixels(4), DevicePixels(4)),
            ),
            size(DevicePixels(4), DevicePixels(4)),
            format,
            [SurfacePlane::new(bytes, 40)],
            SurfaceColorInfo::default(),
        )?;
        let scene = scene(frame, 0.5);
        let result = renderer.render_rgba(&scene)?;
        let center = pixel(&result, 36, 32);
        for (channel, value) in center[..3].iter().enumerate() {
            if channel == usize::from(green) {
                assert!(
                    (136..=138).contains(value),
                    "sequence {sequence}: {center:?}"
                );
            } else {
                assert_eq!(*value, 0, "crop/channel order: {center:?}");
            }
        }
        for (x, y) in [(8, 8), (62, 32), (4, 32)] {
            assert_eq!(
                &pixel(&result, x, y)[..3],
                [0, 0, 0],
                "rounded/content clip {x},{y}"
            );
        }
        assert_eq!(
            renderer.render_rgba(&scene)?,
            result,
            "cached frame must render identically"
        );
    }
    Ok(())
}

#[test]
#[ignore = "requires a GPU adapter"]
fn shared_surface_nv12_color_matrices_and_ranges() -> anyhow::Result<()> {
    let mut renderer = WgpuOffscreenRenderer::new(size(DevicePixels(80), DevicePixels(64)))?;
    let handle = SurfaceHandle::new();
    let mut sequence = 0;
    for (matrix, kr, kb) in [
        (YuvMatrix::Bt601, 0.299_f32, 0.114_f32),
        (YuvMatrix::Bt709, 0.2126, 0.0722),
    ] {
        for range in [ColorRange::Full, ColorRange::Limited] {
            let (black, white) = if range == ColorRange::Full {
                (0, 255)
            } else {
                (16, 235)
            };
            for (y, u, v) in [(black, 128, 128), (white, 128, 128), (128, 160, 192)] {
                let frame = SurfaceFrame::nv12(
                    handle.clone(),
                    sequence,
                    size(DevicePixels(4), DevicePixels(4)),
                    vec![y; 16],
                    4,
                    [u, v].repeat(4),
                    4,
                    SurfaceColorInfo { matrix, range },
                )?;
                sequence += 1;
                let result = renderer.render_rgba(&scene(frame, 1.))?;
                let (luma, cb, cr) = if range == ColorRange::Full {
                    (
                        y as f32 / 255.,
                        (u as f32 - 128.) / 255.,
                        (v as f32 - 128.) / 255.,
                    )
                } else {
                    (
                        (y as f32 - 16.) / 219.,
                        (u as f32 - 128.) / 224.,
                        (v as f32 - 128.) / 224.,
                    )
                };
                let expected = [
                    luma + 2. * (1. - kr) * cr,
                    luma - 2. * kb * (1. - kb) / (1. - kr - kb) * cb
                        - 2. * kr * (1. - kr) / (1. - kr - kb) * cr,
                    luma + 2. * (1. - kb) * cb,
                ];
                let actual = pixel(&result, 36, 32);
                for (got, linear) in actual[..3].iter().zip(expected) {
                    let linear = linear.clamp(0., 1.);
                    let encoded = if linear <= 0.0031308 {
                        linear * 12.92
                    } else {
                        1.055 * linear.powf(1. / 2.4) - 0.055
                    };
                    assert!(
                        (*got as f32 - encoded * 255.).abs() <= 2.,
                        "{matrix:?} {range:?}, YUV={y},{u},{v}: {actual:?}, expected {expected:?}"
                    );
                }
                assert_eq!(&pixel(&result, 62, 32)[..3], [0, 0, 0]);
            }
        }
    }
    Ok(())
}
