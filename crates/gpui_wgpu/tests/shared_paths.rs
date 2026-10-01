#![cfg(not(target_family = "wasm"))]

use gpui::{
    Bounds, ColorSpace, ContentMask, DevicePixels, Path, Scene, linear_color_stop,
    multi_linear_gradient, point, px, rgb, rgba, size,
};
use gpui_wgpu::WgpuOffscreenRenderer;

#[test]
#[ignore = "requires a GPU adapter"]
fn shared_paths_preserve_curves_gradients_opacity_and_clipping() -> anyhow::Result<()> {
    for scale in [1., 1.5, 2.] {
        let mut scene = Scene::default();
        let mask = ContentMask {
            bounds: Bounds::new(point(px(0.), px(0.)), size(px(256.), px(192.))),
        };

        let mut curve = Path::new(point(px(16.), px(72.)));
        curve.curve_to(point(px(112.), px(72.)), point(px(64.), px(8.)));
        curve.color = rgb(0xffffff).into();
        curve.content_mask = mask;
        scene.insert_primitive(curve.scale(scale));

        let mut triangle = Path::new(point(px(144.), px(16.)));
        triangle.line_to(point(px(240.), px(16.)));
        triangle.line_to(point(px(192.), px(80.)));
        triangle.color = rgba(0xffffff80).into();
        triangle.content_mask = mask;
        scene.insert_primitive(triangle.scale(scale));

        let mut rectangle = Path::new(point(px(16.), px(112.)));
        rectangle.line_to(point(px(112.), px(112.)));
        rectangle.line_to(point(px(112.), px(176.)));
        rectangle.line_to(point(px(16.), px(176.)));
        rectangle.color = multi_linear_gradient(
            90.,
            [
                linear_color_stop(rgb(0xff0000), 0.),
                linear_color_stop(rgb(0x0000ff), 1.),
            ],
        )
        .color_space(ColorSpace::Srgb);
        rectangle.content_mask = ContentMask {
            bounds: Bounds::new(point(px(0.), px(96.)), size(px(96.), px(96.))),
        };
        scene.insert_primitive(rectangle.scale(scale));
        scene.finish();

        let width = (256. * scale) as usize;
        let mut renderer = WgpuOffscreenRenderer::new(size(
            DevicePixels(width as i32),
            DevicePixels((192. * scale) as i32),
        ))?;
        let pixels = renderer.render_rgba(&scene)?;
        let pixel = |x: f32, y: f32| {
            let offset = ((y * scale) as usize * width + (x * scale) as usize) * 4;
            &pixels[offset..offset + 4]
        };
        assert_eq!(
            &pixel(64., 24.)[..3],
            &[0; 3],
            "outside quadratic at scale {scale}"
        );
        assert_eq!(&pixel(64., 56.)[..3], &[255; 3], "inside quadratic");
        assert!(
            (187..=189).contains(&pixel(192., 40.)[0]),
            "triangle alpha applies once"
        );
        assert_eq!(&pixel(144., 72.)[..3], &[0; 3], "outside triangle");
        assert!(
            pixel(28., 144.)[0] > pixel(28., 144.)[2],
            "gradient starts red"
        );
        assert!(
            pixel(84., 144.)[2] > pixel(84., 144.)[0],
            "gradient ends blue"
        );
        assert_eq!(&pixel(100., 144.)[..3], &[0; 3], "path content mask");
        let curved_edge = (36..45)
            .flat_map(|y| (48..80).map(move |x| (x, y)))
            .any(|(x, y)| {
                let red = pixel(x as f32, y as f32)[0];
                red > 0 && red < 255
            });
        assert!(curved_edge, "curve edge is antialiased at scale {scale}");
    }
    Ok(())
}
