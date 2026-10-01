#![cfg(not(target_family = "wasm"))]

use gpui::{
    Bounds, ColorSpace, ContentMask, DevicePixels, Path, Scene, linear_color_stop,
    multi_linear_gradient, point, px, rgb, rgba, size,
};
use gpui::{EffectQuad, EffectShader, ScaledPixels, SubtreeLayer};
use gpui_wgpu::WgpuOffscreenRenderer;
use std::{rc::Rc, sync::Arc};

#[test]
#[ignore = "requires a GPU adapter"]
fn path_composition_preserves_overlap_in_offset_capture() -> anyhow::Result<()> {
    for scale in [1., 1.5, 2.] {
        let region = Bounds::new(point(px(20.), px(12.)), size(px(80.), px(72.)));
        let mut child = Scene::default();
        for (x, y) in [(32., 24.), (48., 36.)] {
            let mut path = Path::new(point(px(x), px(y)));
            path.line_to(point(px(x + 32.), px(y)));
            path.line_to(point(px(x + 32.), px(y + 32.)));
            path.line_to(point(px(x), px(y + 32.)));
            path.color = rgba(0xffffff80).into();
            path.content_mask = ContentMask { bounds: region };
            child.insert_primitive(path.scale(scale));
        }
        child.finish();
        let width = (128. * scale) as usize;
        let mut renderer = WgpuOffscreenRenderer::new(size(
            DevicePixels(width as i32),
            DevicePixels((96. * scale) as i32),
        ))?;
        let direct = renderer.render_rgba(&child)?;
        let pixel = |data: &[u8], x: f32, y: f32| {
            data[((y * scale) as usize * width + (x * scale) as usize) * 4]
        };
        assert!(
            (187..=189).contains(&pixel(&direct, 40., 32.)),
            "single path alpha"
        );
        assert!(
            (224..=226).contains(&pixel(&direct, 56., 44.)),
            "overlap must be composited once"
        );
        let bounds: Bounds<ScaledPixels> = region.scale(scale);
        let mut captured = Scene::default();
        child.raster_scale = Some(1.);
        captured.insert_primitive(gpui::Primitive::SubtreeLayer(SubtreeLayer {
            scene3d: None,
            second_scene: None,
            intermediate_effects: Arc::default(),
            composite: EffectQuad {
                order: 0,
                bounds,
                effect_bounds: bounds,
                transformation: Default::default(),
                content_mask: ContentMask { bounds },
                shader: EffectShader::wgsl_image("fn effect(input: EffectInput, params: EffectParams) -> vec4<f32> { return sample_effect_image(input, input.uv); }"),
                uniforms: Default::default(),
                time: 0.,
                corner_radii: Default::default(),
                opacity: 1.,
                image_tile: None,
                second_image_tile: None,
                third_image_tile: None,
                fourth_image_tile: None,
            },
            scene: Rc::new(child),
        }));
        captured.finish();
        let output = renderer.render_rgba(&captured)?;
        for (a, b) in direct.iter().zip(&output) {
            assert!(
                a.abs_diff(*b) <= 1,
                "offset capture pixel mismatch at scale {scale}: {a} vs {b}"
            );
        }
    }
    Ok(())
}

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
