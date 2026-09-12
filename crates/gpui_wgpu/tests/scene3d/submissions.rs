use super::{bounds, layer, mesh};
use anyhow::{Context as _, Result, ensure};
use gpui::{DevicePixels, MeshDraw3d, MeshTexture3d, Primitive, Scene, Scene3dPickCapture, size};
use gpui_wgpu::{
    Scene3dMaterialProgram, Scene3dMaterialSource, Scene3dMaterialValue, Scene3dPixels,
    Scene3dVertexAttribute, Scene3dVertexStreamValue, WgpuContext, WgpuOffscreenRenderer,
    WgpuScene3dGeometry, WgpuScene3dPickFrame, wgpu,
};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

fn objects(context: &WgpuContext) -> Result<[MeshDraw3d; 2]> {
    let base = mesh(0.5, 0xffffffff, MeshTexture3d::None);
    let geometry = WgpuScene3dGeometry::new(context.clone(), base.mesh.clone(), [0; 5], None)?;
    let material = Scene3dMaterialSource::new(
        context.clone(),
        Scene3dMaterialProgram::compile_with_attributes(
            r#"
            struct Controls { value: vec4<f32> }
            @group(1) @binding(0) var<uniform> controls: Controls;
            fn material_surface(input: SurfaceInput, gradients: mat2x2<f32>) -> vec4<f32> {
                return vec4(controls.value.rgb, select(0.0, 1.0, input.attributes.gate > controls.value.w));
            }
            fn material_shading(base: vec3<f32>, input: SurfaceInput,
                gradients: SurfaceGradients, face_sign: f32) -> vec3<f32> { return base; }
        "#,
            &[Scene3dVertexAttribute::new(
                "gate",
                wgpu::VertexFormat::Float32,
            )],
        )?,
    )?;
    let mut result = Vec::new();
    for revision in 0..2 {
        let shift = if revision == 0 { -0.2 } else { 0.2 };
        let records: Vec<[[f32; 4]; 4]> = base
            .mesh
            .vertices()
            .iter()
            .map(|v| {
                [
                    [
                        v.position[0] * 0.5 + shift,
                        v.position[1],
                        0.25 + revision as f32 * 0.25,
                        0.,
                    ],
                    [0., 0., 1., 0.],
                    [0.; 4],
                    [0.; 4],
                ]
            })
            .collect();
        let buffer = context.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::cast_slice(&records),
            usage: wgpu::BufferUsages::STORAGE,
        });
        let gate: Vec<f32> = base
            .mesh
            .vertices()
            .iter()
            .map(|v| if revision == 0 { v.uv[0] } else { 1. - v.uv[0] })
            .collect();
        let streams = material.bind_vertex_streams(
            gate.len(),
            &[(
                "gate",
                Scene3dVertexStreamValue::Bytes(bytemuck::cast_slice(&gate)),
            )],
            1024,
        )?;
        let color = if revision == 0 {
            [1_f32, 0., 0., 0.5]
        } else {
            [0., 0., 1., 0.25]
        };
        let snapshot = material
            .bind(
                [(
                    0,
                    Scene3dMaterialValue::Uniform(bytemuck::cast_slice(&color).to_vec().into()),
                )],
                Default::default(),
            )?
            .with_vertex_streams(streams)?;
        let mut object = base.clone();
        object.output_id = revision + 7;
        if revision == 1 {
            let streams = material.bind_vertex_streams(
                gate.len(),
                &[(
                    "gate",
                    Scene3dVertexStreamValue::Bytes(bytemuck::cast_slice(&vec![1_f32; gate.len()])),
                )],
                1024,
            )?;
            let pass = snapshot
                .with_values(
                    [(
                        0,
                        Scene3dMaterialValue::Uniform(
                            bytemuck::cast_slice(&[0_f32, 0.25, 0., 0.5])
                                .to_vec()
                                .into(),
                        ),
                    )],
                    Default::default(),
                )?
                .with_vertex_streams(streams)?;
            object.mesh_passes = vec![gpui::MeshPass3d {
                expansion: None,
                material: gpui::MeshMaterial3d::new(Arc::new(pass)),
                state: gpui::MeshPassState3d {
                    blend: gpui::MeshPassBlend3d::Additive,
                    ..Default::default()
                },
            }]
            .into();
        }
        object.custom_material = Some(gpui::MeshMaterial3d::new(Arc::new(snapshot)));
        object.gpu_geometry = Some(gpui::MeshGpuGeometry3d::new(Arc::new(
            geometry.evaluate(&buffer)?,
        )));
        object.render_bounds = Some([[shift - 0.35, -0.7, 0.25], [shift + 0.35, 0.7, 0.5]]);
        result.push(object);
    }
    Ok(result.try_into().ok().unwrap())
}

fn compose(objects: &[MeshDraw3d; 2], captures: &[Scene3dPickCapture; 2], swapped: bool) -> Scene {
    let mut scene = Scene::default();
    for index in 0..2 {
        let mut layer = layer(
            bounds(if index == 0 { -8. } else { 80. }, 8., 64., 64.),
            Scene::default(),
            vec![objects[index ^ usize::from(swapped)].clone()],
            1.,
        );
        let frame = Arc::make_mut(layer.scene3d.as_mut().unwrap());
        frame.pick_capture = Some(captures[index].clone());
        frame.viewport_quality =
            gpui::Scene3dViewportQuality::new(if index == 0 { 1. } else { 0.5 }, 4);
        scene.insert_primitive(Primitive::SubtreeLayer(layer));
    }
    scene.finish();
    scene
}

fn read(frame: &WgpuScene3dPickFrame) -> Result<Scene3dPixels> {
    let mut request = frame.gpu().readback()?;
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if let Some(pixels) = request.try_read()? {
            return Ok(pixels);
        }
        ensure!(Instant::now() < deadline, "capture readback timed out");
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn verify(frame: &WgpuScene3dPickFrame, revision: usize) -> Result<Scene3dPixels> {
    let pixels = read(frame)?;
    let rect = frame.projection_rect();
    let ids = pixels.object_ids.as_ref().unwrap();
    let depths = pixels.linear_depth.as_ref().unwrap();
    let mut hits = 0;
    for y in 0..pixels.size[1] {
        for x in 0..pixels.size[0] {
            let wx = 2. * (x as f32 + 0.5 - rect[0]) / rect[2] - 1.;
            let wy = 1. - 2. * (y as f32 + 0.5 - rect[1]) / rect[3];
            let (low, high) = if revision == 0 {
                (-0.2, 0.15)
            } else {
                (-0.15, 0.375)
            };
            let covered = wx > low && wx < high && wy.abs() < 0.7;
            let i = (y * pixels.size[0] + x) as usize;
            assert_eq!(
                ids[i],
                if covered { revision as u32 + 7 } else { 0 },
                "pixel {x}, {y}"
            );
            let expected_depth = if covered {
                2.75 - revision as f32 * 0.25
            } else {
                0.
            };
            assert!((depths[i] - expected_depth).abs() < 1e-5);
            hits += usize::from(covered);
        }
    }
    assert!(hits > 10);
    Ok(pixels)
}

#[test]
#[ignore = "requires a compute-capable GPU"]
fn material_deformation_captures_survive_replay_resize_and_device_replacement() -> Result<()> {
    let mut renderer = WgpuOffscreenRenderer::new(size(DevicePixels(160), DevicePixels(80)))?;
    let context = renderer
        .sprite_atlas()
        .renderer_context()
        .unwrap()
        .downcast::<WgpuContext>()
        .unwrap();
    let objects = objects(&context)?;
    let captures = std::array::from_fn(|_| Scene3dPickCapture::new(1024 * 1024));
    let original = compose(&objects, &captures, false);
    let changed = compose(&objects, &captures, true);
    let mut retained: Vec<(Arc<WgpuScene3dPickFrame>, usize)> = Vec::new();
    for (scene, swapped, extent) in [
        (&original, false, 160),
        (&original, false, 160),
        (&changed, true, 176),
        (&changed, true, 176),
        (&original, false, 160),
    ] {
        renderer.resize(size(DevicePixels(extent), DevicePixels(80)));
        let pixels = renderer.render_rgba(scene)?;
        for index in 0..2 {
            let revision = index ^ usize::from(swapped);
            let frame = captures[index]
                .read::<WgpuScene3dPickFrame>()
                .context("capture missing")?
                .unwrap();
            assert!(frame.matches_frame(scene.subtree_layers[index].scene3d.as_ref().unwrap()));
            assert!(
                retained
                    .iter()
                    .all(|(old, _)| old.gpu().frame_id() != frame.gpu().frame_id())
            );
            let x = if index == 0 { 24 } else { 112 };
            let color = &pixels[(40 * extent as usize + x) * 4..][..4];
            let expected = if revision == 0 {
                [255, 0, 0, 255]
            } else {
                [0, 137, 255, 255]
            };
            for (actual, expected) in color.iter().zip(expected) {
                assert!(actual.abs_diff(expected) <= 1);
            }
            if revision == 1 && index == 0 {
                let pass_only = &pixels[(40 * extent as usize + 38) * 4..][..4];
                for (actual, expected) in pass_only.iter().zip([0, 137, 0, 255]) {
                    assert!(
                        actual.abs_diff(expected) <= 1,
                        "pass-only color: {pass_only:?}"
                    );
                }
            }
            verify(&frame, revision)?;
            retained.push((frame, revision));
        }
    }
    let mut replacement = WgpuOffscreenRenderer::new(size(DevicePixels(160), DevicePixels(80)))?;
    assert!(replacement.render_rgba(&original).is_err());
    for capture in &captures {
        let error = capture
            .read::<WgpuScene3dPickFrame>()
            .context("failure missing")?
            .err()
            .context("foreign resources accepted")?;
        assert!(error.contains("different or lost device"), "{error}");
    }
    let fresh = replacement
        .sprite_atlas()
        .renderer_context()
        .unwrap()
        .downcast::<WgpuContext>()
        .unwrap();
    let mut fresh_objects = self::objects(&fresh)?;
    let saved = fresh_objects[0].gpu_geometry.clone();
    let saved_mesh = fresh_objects[0].mesh.clone();
    fresh_objects[0].gpu_geometry = objects[0].gpu_geometry.clone();
    fresh_objects[0].mesh = objects[0].mesh.clone();
    let mixed = compose(&fresh_objects, &captures, false);
    assert!(replacement.render_rgba(&mixed).is_err());
    for capture in &captures {
        let error = capture
            .read::<WgpuScene3dPickFrame>()
            .unwrap()
            .err()
            .context("foreign geometry accepted")?;
        assert!(
            error.contains("different") && error.contains("device"),
            "{error}"
        );
    }
    fresh_objects[0].gpu_geometry = saved;
    fresh_objects[0].mesh = saved_mesh;
    let renewed = compose(&fresh_objects, &captures, false);
    replacement.render_rgba(&renewed)?;
    for (index, capture) in captures.iter().enumerate() {
        let frame = capture.read::<WgpuScene3dPickFrame>().unwrap().unwrap();
        assert!(frame.matches_frame(renewed.subtree_layers[index].scene3d.as_ref().unwrap()));
        verify(&frame, index)?;
    }
    drop((renderer, objects, original, changed));
    for (frame, revision) in &retained {
        verify(frame, *revision)?;
    }
    let expected = read(&retained[0].0)?;
    let mut interrupted = retained[0].0.gpu().readback()?;
    context.device.destroy();
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        match interrupted.try_read() {
            Ok(Some(pixels)) => {
                assert_eq!(pixels.object_ids, expected.object_ids);
                assert_eq!(pixels.linear_depth, expected.linear_depth);
                break;
            }
            Err(_) => break,
            Ok(None) => {
                ensure!(
                    Instant::now() < deadline,
                    "destroyed-device readback did not terminate"
                );
                std::thread::sleep(Duration::from_millis(2));
            }
        }
    }
    assert!(interrupted.try_read().is_err());
    assert!(read(&retained[0].0).is_err());
    replacement.render_rgba(&renewed)?;
    let frame = captures[0].read::<WgpuScene3dPickFrame>().unwrap().unwrap();
    verify(&frame, 0)?;
    Ok(())
}
