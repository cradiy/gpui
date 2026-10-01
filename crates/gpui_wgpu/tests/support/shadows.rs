use super::*;
use gpui::{Corners, Shadow, rgb};

pub(super) fn check(renderer: &mut WgpuOffscreenRenderer) -> anyhow::Result<()> {
    renderer.resize(size(DevicePixels(160), DevicePixels(160)));
    let region = |x, y, width, height| {
        Bounds::new(
            point(ScaledPixels(x), ScaledPixels(y)),
            size(ScaledPixels(width), ScaledPixels(height)),
        )
    };
    let encode = |alpha: f32| {
        let value = if OUTPUT_SRGB {
            if alpha <= 0.0031308 {
                alpha * 12.92
            } else {
                1.055 * alpha.powf(1. / 2.4) - 0.055
            }
        } else {
            alpha
        };
        (value * 255.).round() as i32
    };
    // Independent numerical integration of the standard normal distribution.
    let cdf = |x: f64| {
        let step = x / 4096.;
        0.5 + (0..4096)
            .map(|i| {
                let y = (i as f64 + 0.5) * step;
                (-y * y / 2.).exp() / (2. * std::f64::consts::PI).sqrt() * step
            })
            .sum::<f64>()
    };
    for sigma in [2., 4., 8.] {
        for height in [96., 4.] {
            for inset in [0, 1] {
                let mut scene = Scene::default();
                scene.insert_primitive(Shadow {
                    order: Default::default(),
                    blur_radius: ScaledPixels(sigma),
                    bounds: region(32., 32., 96., height),
                    corner_radii: Corners::default(),
                    content_mask: ContentMask {
                        bounds: region(0., 0., 160., 160.),
                    },
                    color: rgb(0xffffff).into(),
                    element_bounds: region(0., 0., 160., 160.),
                    element_corner_radii: Corners::default(),
                    inset,
                    pad: 0,
                });
                scene.finish();
                let pixels = renderer.render_rgba(&scene)?;
                let red = |y: usize| pixels[(y * 160 + 80) * 4];
                if height == 96. {
                    assert_eq!(
                        red(80),
                        if inset == 1 { 0 } else { 255 },
                        "interior, sigma={sigma}, inset={inset}"
                    );
                }
                // The wide rectangle makes horizontal Gaussian tails negligible.
                // Check both sides of the top edge and the hole's center.
                for y in [28, 32, 36, (32. + height / 2.) as usize] {
                    let position = y as f64 + 0.5;
                    let coverage = cdf((position - 32.) / sigma as f64)
                        - cdf((position - 32. - height as f64) / sigma as f64);
                    let alpha = if inset == 1 { 1. - coverage } else { coverage };
                    let expected = encode(alpha as f32);
                    assert!(
                        (red(y) as i32 - expected).abs() <= 2,
                        "sigma={sigma}, height={height}, inset={inset}, y={y}: actual={}, expected={expected}",
                        red(y)
                    );
                }
            }
        }
    }
    Ok(())
}
