use super::*;
use gpui_wgpu::{
    Scene3dChannels, Scene3dGpuOutput, Scene3dOutputConfig, Scene3dPixels, WgpuContext,
    WgpuScene3dPickFrame, WgpuScene3dRenderer, wgpu,
};

fn read(output: &Scene3dGpuOutput, context: &WgpuContext) -> anyhow::Result<Scene3dPixels> {
    let mut pending = output.readback()?;
    context.device.poll(wgpu::PollType::Wait {
        submission_index: None,
        timeout: Some(std::time::Duration::from_secs(15)),
    })?;
    Ok(pending.try_read()?.unwrap())
}

#[test]
#[ignore = "requires a GPU adapter"]
fn grouped_occlusion_matches_direct_output_and_retains_capture_identity() -> anyhow::Result<()> {
    for extent in [64, 128] {
        let mut renderer =
            WgpuOffscreenRenderer::new(size(DevicePixels(extent), DevicePixels(extent)))?;
        let context = renderer
            .sprite_atlas()
            .renderer_context()
            .unwrap()
            .downcast::<WgpuContext>()
            .unwrap();
        let capture = gpui::Scene3dPickCapture::new(16 * 1024 * 1024);
        let front = mesh(0.2, 0xff8040ff, MeshTexture3d::None);
        let mut other = mesh(0.4, 0x4080ffff, MeshTexture3d::None);
        other.output_id = 2;
        other.model[0][0] = 0.5;
        other.model[3][0] = 0.35;
        let mut auxiliary = mesh(0.7, 0xffffffff, MeshTexture3d::None);
        auxiliary.output_id = 3;
        auxiliary.alpha_mode = gpui::AlphaMode3d::Opaque;
        let mut surface = layer(
            bounds(0., 0., extent as f32, extent as f32),
            Scene::default(),
            vec![front, other],
            1.,
        );
        let frame = Arc::make_mut(surface.scene3d.as_mut().unwrap());
        frame.pick_capture = Some(capture.clone());
        frame.occlusion_groups = vec![gpui::OcclusionGroup3d {
            id: 42,
            members: vec![1].into(),
            occluders: vec![auxiliary].into(),
            points: Default::default(),
            lines: Default::default(),
            pixel_scale: 1.,
        }]
        .into();
        let source = surface.scene3d.as_ref().unwrap().clone();
        renderer.render_rgba(&scene(surface))?;
        let captured = capture
            .read_for_frame::<WgpuScene3dPickFrame>(&source)
            .unwrap()
            .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        let group = &captured.gpu().occlusion_groups()[0];
        assert_eq!(group.parent_frame_id(), captured.gpu().frame_id());
        assert_eq!(group.projection_rect(), captured.projection_rect());
        let mut direct = WgpuScene3dRenderer::new((*context).clone())?;
        let config = Scene3dOutputConfig {
            size: [extent as u32; 2],
            channels: Scene3dChannels::OBJECT_ID | Scene3dChannels::LINEAR_DEPTH,
            color_samples: 1,
        };
        let expected = direct.render(&source, config)?;
        let actual_pixels = read(group.gpu(), &context)?;
        let expected_pixels = read(expected.occlusion_groups()[0].gpu(), &context)?;
        assert_eq!(actual_pixels.object_ids, expected_pixels.object_ids);
        assert_eq!(actual_pixels.linear_depth, expected_pixels.linear_depth);
        assert!(actual_pixels.object_ids.as_ref().unwrap().contains(&2));
        assert!(actual_pixels.object_ids.as_ref().unwrap().contains(&3));
        assert!(!actual_pixels.object_ids.as_ref().unwrap().contains(&1));
        assert!(
            read(captured.gpu(), &context)?
                .object_ids
                .unwrap()
                .contains(&1)
        );
    }
    Ok(())
}

#[test]
#[ignore = "requires a GPU adapter"]
fn edit_overlay_composes_without_capture_and_preserves_captured_element_ids() -> anyhow::Result<()>
{
    let mut renderer = WgpuOffscreenRenderer::new(size(DevicePixels(96), DevicePixels(96)))?;
    let context = renderer
        .sprite_atlas()
        .renderer_context()
        .unwrap()
        .downcast::<WgpuContext>()
        .unwrap();
    let mut surface = layer(
        bounds(0., 0., 96., 96.),
        Scene::default(),
        vec![mesh(0.4, 0x4080ffff, MeshTexture3d::None)],
        1.,
    );
    let frame = Arc::make_mut(surface.scene3d.as_mut().unwrap());
    frame.occlusion_groups = vec![gpui::OcclusionGroup3d {
        id: 7,
        members: vec![1].into(),
        occluders: Default::default(),
        lines: vec![gpui::EditLine3d {
            id: 8,
            start: [-0.8, 0., 0.6],
            end: [0.8, 0., 0.6],
            style: gpui::EditStyle3d::new(4., gpui::rgb(0xff0000)),
        }]
        .into(),
        points: vec![gpui::EditPoint3d {
            id: 9,
            position: [0., 0., 0.6],
            style: gpui::EditStyle3d::new(10., gpui::rgb(0xffffff)),
        }]
        .into(),
        pixel_scale: 1.,
    }]
    .into();
    let implicit = renderer.render_rgba(&scene(surface.clone()))?;
    assert_eq!(
        &implicit[(48 * 96 + 48) * 4..(48 * 96 + 48) * 4 + 3],
        &[255, 255, 255]
    );
    let capture = gpui::Scene3dPickCapture::new(4 * 1024 * 1024);
    Arc::make_mut(surface.scene3d.as_mut().unwrap()).pick_capture = Some(capture.clone());
    let captured_color = renderer.render_rgba(&scene(surface.clone()))?;
    assert_eq!(implicit, captured_color);
    let captured = capture
        .read::<WgpuScene3dPickFrame>()
        .unwrap()
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
    let group = captured.gpu().occlusion_groups()[0].clone();
    let data = read(group.elements().unwrap(), &context)?;
    let ids = data.object_ids.as_ref().unwrap();
    assert_eq!(ids[48 * 96 + 24], 8);
    assert_eq!(ids[48 * 96 + 48], 9);
    assert_eq!(
        read(captured.gpu(), &context)?.object_ids.unwrap()[48 * 96 + 48],
        1
    );
    let mut direct = WgpuScene3dRenderer::new((*context).clone())?;
    let config = Scene3dOutputConfig {
        size: [96; 2],
        channels: Scene3dChannels::COLOR,
        color_samples: 4,
    };
    let output = direct.render(surface.scene3d.as_ref().unwrap(), config)?;
    assert_eq!(
        read(output.occlusion_groups()[0].elements().unwrap(), &context)?.object_ids,
        data.object_ids
    );
    let direct_color = read(&output, &context)?.rgba.unwrap();
    for (index, (direct, viewport)) in direct_color
        .chunks_exact(4)
        .zip(captured_color.chunks_exact(4))
        .enumerate()
    {
        assert!(
            direct[..3]
                .iter()
                .zip(&viewport[..3])
                .all(|(a, b)| a.abs_diff(*b) <= 1),
            "color mismatch at {index}: {direct:?} / {viewport:?}"
        );
    }
    for density in [0.5, 1., 2.] {
        Arc::make_mut(surface.scene3d.as_mut().unwrap()).viewport_quality =
            gpui::Scene3dViewportQuality::new(density, 4);
        renderer.render_rgba(&scene(surface.clone()))?;
        let latest = capture
            .read::<WgpuScene3dPickFrame>()
            .unwrap()
            .map_err(|e| anyhow::anyhow!(e.to_string()))?;
        let g = &latest.gpu().occlusion_groups()[0];
        assert_eq!(g.parent_frame_id(), latest.gpu().frame_id());
        let elements = g.elements().unwrap();
        let width = elements.config().size[0] as usize;
        let ids = read(elements, &context)?.object_ids.unwrap();
        assert_eq!(
            (0..width)
                .filter(|y| ids[y * width + width / 4] == 8)
                .count(),
            (4. * density) as usize
        );
    }
    surface.composite.bounds = bounds(-24., -12., 96., 96.);
    surface.composite.effect_bounds = surface.composite.bounds;
    surface.composite.content_mask.bounds = surface.composite.bounds;
    renderer.render_rgba(&scene(surface.clone()))?;
    let clipped = capture
        .read::<WgpuScene3dPickFrame>()
        .unwrap()
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
    let pixel = clipped.pixel_at([0.5, 0.5]).unwrap();
    let element = clipped.gpu().occlusion_groups()[0].elements().unwrap();
    assert_eq!(
        read(element, &context)?.object_ids.unwrap()
            [(pixel[1] * element.config().size[0] + pixel[0]) as usize],
        9
    );
    drop(renderer);
    assert_eq!(
        read(group.elements().unwrap(), &context)?.object_ids,
        data.object_ids
    );
    Ok(())
}
