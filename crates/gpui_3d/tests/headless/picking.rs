use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use gpui_3d::{
    Camera, GpuDeformationLimits, GpuMorph, HeadlessRenderer, Material, Mesh, MorphTarget,
    MorphTargets, Node, Projection, Ray, Scene3dChannels, Scene3dGpuDraw, Scene3dOutputConfig,
    Scene3dReadbackConfig, Scene3dReadbackRegion, SceneGraph,
};

fn read<T>(mut poll: impl FnMut() -> anyhow::Result<Option<T>>) -> anyhow::Result<T> {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if let Some(result) = poll()? {
            return Ok(result);
        }
        anyhow::ensure!(Instant::now() < deadline, "readback timed out");
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
#[ignore = "requires a compute-capable GPU"]
fn regional_readback_and_picking_follow_deformed_geometry_and_retained_frames() -> anyhow::Result<()>
{
    let mut renderer = HeadlessRenderer::new()?;
    let base = Mesh::plane();
    let morph = GpuMorph::new(
        renderer.context().clone(),
        MorphTargets::new(
            base.clone(),
            [MorphTarget {
                positions: Some(vec![[1., 0., 0.]; base.vertex_count()].into()),
                ..Default::default()
            }],
        )?,
        GpuDeformationLimits::default(),
    )?;
    let output = morph.evaluate(&[1.])?;
    let render_source = output.render_source([0; 5], None)?;
    let geometry = Arc::new(output.render_geometry(&render_source)?);
    let mut graph = SceneGraph::new();
    let node = graph.insert(
        None,
        Node::new()
            .id("deformed")
            .mesh(base, Material::color(gpui::white())),
    )?;
    let camera = Camera {
        eye: [0., 0., 5.],
        target: [0., 0., 0.],
        projection: Projection::Orthographic { vertical_size: 4. },
        ..Default::default()
    };
    let scene = graph.evaluate()?.scene(camera);
    assert!(
        scene
            .raycast(Ray::new([1., 0., 5.], [0., 0., -1.])?)
            .is_none()
    );
    let config = Scene3dOutputConfig {
        size: [64, 64],
        channels: Scene3dChannels::all(),
        color_samples: 4,
    };
    let frame = renderer.render_with_geometry(
        &scene,
        config,
        &[Scene3dGpuDraw {
            output_id: 1,
            geometry,
            bounds: [[0.5, -0.5, 0.], [1.5, 0.5, 0.]],
        }],
    )?;
    drop(output);
    drop(morph);
    let mut full = frame.readback()?;
    let full = read(|| full.try_read())?;
    let region = Scene3dReadbackRegion {
        origin: [39, 23],
        size: [18, 19],
    };
    let mut cropped = frame
        .gpu()
        .readback_region(region, Scene3dReadbackConfig::new(config.channels))?;
    assert_eq!(cropped.region(), region);
    assert!(frame.pick([48, 32]).is_err());
    let crop = read(|| cropped.try_read())?;
    assert!(cropped.try_read().is_err());
    assert_eq!(crop.size, region.size);
    assert_eq!(crop.depth_background, full.pixels.depth_background);
    for y in 0..region.size[1] {
        for x in 0..region.size[0] {
            let local = (y * region.size[0] + x) as usize;
            let original =
                ((region.origin[1] + y) * config.size[0] + region.origin[0] + x) as usize;
            assert_eq!(
                crop.object_ids.as_ref().unwrap()[local],
                full.pixels.object_ids.as_ref().unwrap()[original]
            );
            assert_eq!(
                crop.linear_depth.as_ref().unwrap()[local],
                full.pixels.linear_depth.as_ref().unwrap()[original]
            );
            assert_eq!(
                crop.world_normals.as_ref().unwrap()[local],
                full.pixels.world_normals.as_ref().unwrap()[original]
            );
            assert_eq!(
                crop.linear_rgba.as_ref().unwrap()[local],
                full.pixels.linear_rgba.as_ref().unwrap()[original]
            );
            assert_eq!(
                crop.rgba.as_ref().unwrap()[local * 4..local * 4 + 4],
                full.pixels.rgba.as_ref().unwrap()[original * 4..original * 4 + 4]
            );
        }
    }
    assert!(frame.pick([64, 0]).is_err());
    let mut background = frame.pick([0, 0])?;
    assert!(read(|| background.try_read())?.hit.is_none());
    assert!(background.try_read().is_err());
    let mut pick = frame.pick([48, 32])?;
    assert_eq!(pick.memory().staging_bytes, 512);
    assert_eq!(pick.memory().cpu_bytes, 8);

    graph.remove_subtree(node)?;
    graph.insert(
        None,
        Node::new()
            .id("later")
            .mesh(Mesh::cube(), Material::color(gpui::white())),
    )?;
    let later = renderer.render(
        &graph.evaluate()?.scene(Camera::orbit(0.5, 0.3, 8.)),
        Scene3dOutputConfig::new([17, 11]),
    )?;
    assert_eq!(later.object(1).unwrap().id, Some("later".into()));
    assert!(later.pick([8, 5]).is_err());
    drop(frame);
    drop(renderer);
    drop(graph);
    let result = read(|| pick.try_read())?;
    assert!(pick.try_read().is_err());
    assert_eq!(result.pixel, [48, 32]);
    assert_eq!(result.size, [64, 64]);
    assert_eq!(result.camera().eye, camera.eye);
    let hit = result.hit.unwrap();
    assert_eq!(hit.object.node, Some(node));
    assert_eq!(hit.object.id, Some("deformed".into()));
    let expected = full.world_position_at(48, 32)?.unwrap();
    for (actual, expected) in hit.world_position.into_iter().zip(expected) {
        assert!((actual - expected).abs() < 1e-5);
    }
    assert!((hit.world_position[0] - 1.03125).abs() < 1e-5);
    assert!((hit.world_position[1] + 0.03125).abs() < 1e-5);
    assert!(hit.world_position[2].abs() < 1e-5);
    assert!((hit.linear_depth - 5.).abs() < 1e-5);
    Ok(())
}
