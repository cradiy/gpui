use super::*;
use gpui::{Primitive, Scene3dPickCapture};
use std::rc::Rc;

#[test]
fn failed_scene_replaces_nested_pick_publications_without_revoking_retained_results() {
    let captures = std::array::from_fn::<_, 4, _>(|_| Scene3dPickCapture::new(4096));
    let retained = Arc::new(vec![3_u32, 7]);
    captures[1].publish(Ok(retained.clone()));
    captures[3].publish(Ok(retained.clone()));
    let layer = |index: usize| {
        let mut layer = output_layer(MeshTexture3d::None);
        Arc::make_mut(layer.scene3d.as_mut().unwrap()).pick_capture = Some(captures[index].clone());
        layer
    };
    let mut root = layer(0);
    Arc::make_mut(root.scene3d.as_mut().unwrap()).ui_texture = Some(gpui::UiTexture3d::new(
        gpui::size(gpui::px(32.), gpui::px(24.)),
        1.,
    ));
    Rc::get_mut(&mut root.scene)
        .unwrap()
        .insert_primitive(Primitive::SubtreeLayer(layer(1)));
    let mut second = Scene::default();
    second.insert_primitive(Primitive::SubtreeLayer(layer(2)));
    root.second_scene = Some(Rc::new(second));
    let mut scene = Scene::default();
    scene.insert_primitive(Primitive::SubtreeLayer(root));

    let error = gpui::SharedString::from("material resource unavailable");
    fail_pick_captures(&scene, error.clone());
    for capture in &captures[..3] {
        assert_eq!(capture.read::<Vec<u32>>().unwrap().unwrap_err(), error);
    }
    assert!(Arc::ptr_eq(
        &captures[3].read::<Vec<u32>>().unwrap().unwrap(),
        &retained,
    ));
    assert_eq!(&*retained, &[3, 7]);

    captures[1].publish(Ok(Arc::new(vec![11_u32])));
    assert_eq!(*captures[1].read::<Vec<u32>>().unwrap().unwrap(), vec![11]);
    assert_eq!(captures[0].read::<Vec<u32>>().unwrap().unwrap_err(), error);
}

#[test]
#[ignore = "requires a GPU adapter"]
fn failed_viewport_preparation_publishes_pick_errors_and_allows_resubmission() -> anyhow::Result<()>
{
    let context = crate::WgpuContext::new_headless()?;
    let format = wgpu::TextureFormat::Rgba8UnormSrgb;
    let mut renderer = WgpuRenderer::new_external(
        &context,
        crate::WgpuExternalRendererConfig {
            size: gpui::size(gpui::DevicePixels(64), gpui::DevicePixels(64)),
            format,
            alpha_mode: wgpu::CompositeAlphaMode::Opaque,
            target_usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        },
    )?;
    let texture = context.device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width: 64,
            height: 64,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let view = texture.create_view(&Default::default());
    let capture = Scene3dPickCapture::new(1024 * 1024);
    let nested_capture = Scene3dPickCapture::new(1024 * 1024);
    let mut nested_layer = output_layer(MeshTexture3d::None);
    Arc::make_mut(nested_layer.scene3d.as_mut().unwrap()).pick_capture =
        Some(nested_capture.clone());
    let mut nested = Scene::default();
    nested.insert_primitive(Primitive::SubtreeLayer(nested_layer));
    nested.finish();
    let nested = Rc::new(nested);
    let scene = |invalid_geometry: bool, invalid_material: bool| {
        let mut layer = output_layer(MeshTexture3d::None);
        layer.scene = nested.clone();
        let frame = Arc::make_mut(layer.scene3d.as_mut().unwrap());
        frame.pick_capture = Some(capture.clone());
        frame.ui_texture = Some(gpui::UiTexture3d::new(
            gpui::size(gpui::px(64.), gpui::px(64.)),
            1.,
        ));
        let object = &mut Arc::make_mut(&mut frame.objects)[0];
        if invalid_geometry {
            object.gpu_geometry = Some(gpui::MeshGpuGeometry3d::new(Arc::new(())));
        }
        if invalid_material {
            object.custom_material = Some(gpui::MeshMaterial3d::new(Arc::new(())));
        }
        let mut scene = Scene::default();
        scene.insert_primitive(Primitive::SubtreeLayer(layer));
        scene.finish();
        scene
    };
    let valid = scene(false, false);
    assert!(renderer.draw_external(&valid, &texture, &view, wgpu::Color::TRANSPARENT));
    let retained = capture
        .read::<crate::WgpuScene3dPickFrame>()
        .unwrap()
        .unwrap();
    let nested_retained = nested_capture
        .read::<crate::WgpuScene3dPickFrame>()
        .unwrap()
        .unwrap();
    for (geometry, material, expected) in [
        (true, false, "unsupported GPU geometry backend"),
        (false, true, "unsupported 3D material backend"),
    ] {
        let invalid = scene(geometry, material);
        assert!(!renderer.draw_external(&invalid, &texture, &view, wgpu::Color::TRANSPARENT));
        for capture in [&capture, &nested_capture] {
            let error = capture
                .read::<crate::WgpuScene3dPickFrame>()
                .unwrap()
                .err()
                .expect("failed preparation must publish an error");
            assert!(error.contains(expected), "{error}");
        }
        assert!(retained.matches_frame(valid.subtree_layers[0].scene3d.as_ref().unwrap()));

        assert!(renderer.draw_external(&valid, &texture, &view, wgpu::Color::TRANSPARENT));
        let current = capture
            .read::<crate::WgpuScene3dPickFrame>()
            .unwrap()
            .unwrap();
        assert!(current.matches_frame(valid.subtree_layers[0].scene3d.as_ref().unwrap()));
        assert!(!Arc::ptr_eq(&retained, &current));
        let nested_current = nested_capture
            .read::<crate::WgpuScene3dPickFrame>()
            .unwrap()
            .unwrap();
        assert!(nested_current.matches_frame(nested.subtree_layers[0].scene3d.as_ref().unwrap()));
        assert!(!Arc::ptr_eq(&nested_retained, &nested_current));
    }
    Ok(())
}
