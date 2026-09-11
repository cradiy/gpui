use super::{asset, generated_fixture};
use gpui::{Bounds, point, px, size};
use gpui_3d::{
    AffineTransform, Camera, GpuDeformationBounds, GpuDeformationLimits, HeadlessRenderer,
    Projection, Scene3dChannels, Scene3dGpuDraw, Scene3dOutputConfig, SceneGraph,
};
use gpui_3d_gltf::GpuSceneDeformation;
use serde_json::json;
use std::time::{Duration, Instant};

fn read<T>(mut poll: impl FnMut() -> anyhow::Result<Option<T>>) -> anyhow::Result<T> {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if let Some(result) = poll()? {
            return Ok(result);
        }
        anyhow::ensure!(Instant::now() < deadline, "readback timed out");
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
#[ignore = "requires a compute-capable GPU with SHADER_F64"]
fn imported_generated_directions_render_and_pick_retained_deformation_frames() -> anyhow::Result<()>
{
    let mut fixture = generated_fixture(false);
    fixture.json["meshes"].as_array_mut().unwrap().truncate(1);
    fixture.json["meshes"][0]["primitives"]
        .as_array_mut()
        .unwrap()
        .truncate(1);
    fixture.json["nodes"] = json!([
        {"children":[1,2]},
        {},
        {"mesh":0,"skin":0,"translation":[6,0,0],"weights":[0.35]}
    ]);
    let asset = asset(&fixture);
    let mut graph = SceneGraph::new();
    let instance = graph.instantiate(None, asset.subtree())?;
    let primitive = instance.node(asset.primitives()[0].handle).unwrap();
    let target = instance.node(asset.morphs()[0].node()).unwrap();
    let joint = instance.node(asset.skins()[0].joints()[0]).unwrap();
    let mut renderer = HeadlessRenderer::new()?;
    let context = renderer.context().clone();
    let deformation = GpuSceneDeformation::new(
        context.clone(),
        &asset,
        GpuDeformationLimits::default(),
        None,
    )?;
    let bounds = GpuDeformationBounds::new(context)?;
    let config = Scene3dOutputConfig {
        size: [96, 80],
        channels: Scene3dChannels::all(),
        color_samples: 4,
    };
    let viewport = Bounds::new(point(px(0.), px(0.)), size(px(96.), px(80.)));
    let mut retained = Vec::new();
    let mut packing = None;
    for (index, weight) in [None, Some(-0.4), Some(0.), Some(0.8)]
        .into_iter()
        .enumerate()
    {
        graph.set_transform(
            joint,
            AffineTransform::from_translation([
                index as f32 * 0.12,
                -0.15,
                0.2 + index as f32 * 0.1,
            ])?,
        )?;
        let poses = graph.evaluate()?;
        let weights = weight
            .map(|weight| (target, vec![weight]))
            .into_iter()
            .collect::<Vec<_>>();
        let camera = Camera {
            eye: [0.5 + index as f32 * 0.08, 0.5, 6.],
            target: [0.5, 0.5, 0.],
            projection: Projection::Orthographic { vertical_size: 2.5 },
            ..Default::default()
        };
        let cpu_meshes = asset.deform(&instance, &poses, &weights)?;
        let mesh = &cpu_meshes[0].1;
        let triangle = &mesh.indices()[3..6];
        let center = std::array::from_fn(|axis| {
            triangle
                .iter()
                .map(|&i| mesh.vertices()[i as usize].position[axis])
                .sum::<f32>()
                / 3.
        });
        let center = poses.node(primitive).unwrap().world.transform_point(center);
        let projected = camera.world_to_screen(viewport, center)?.unwrap();
        let pixel = [
            f32::from(projected.position.x).floor() as u32,
            f32::from(projected.position.y).floor() as u32,
        ];
        let cpu_scene = poses.with_meshes(cpu_meshes)?.scene(camera);
        let ray = camera.screen_to_ray(
            viewport,
            point(px(pixel[0] as f32 + 0.5), px(pixel[1] as f32 + 0.5)),
        )?;
        let expected_hit = cpu_scene.raycast(ray).unwrap();
        assert_eq!(expected_hit.node, Some(primitive));
        let cpu_frame = renderer.render(&cpu_scene, config)?;

        let outputs = deformation.evaluate(&instance, &poses, &weights, None)?;
        let scene = poses
            .with_meshes(
                outputs
                    .iter()
                    .map(|(node, output)| (*node, output.base_mesh().clone())),
            )?
            .scene(camera);
        let inputs = scene.geometry_inputs()?.collect::<Vec<_>>();
        let mut draws = Vec::new();
        for (node, output) in outputs {
            let input = inputs
                .iter()
                .find(|input| input.node == Some(node))
                .unwrap();
            let source = match &packing {
                Some(source) => output.rebind_render_source(source, input.uv_sets, None)?,
                None => output.render_source(input.uv_sets, None)?,
            };
            let mut pending = output.prepare_render_geometry(&source, &bounds, None)?;
            packing = Some(source);
            let prepared = read(|| pending.try_read())?;
            draws.push(Scene3dGpuDraw {
                output_id: input.output_id,
                geometry: prepared.geometry().clone(),
                bounds: [prepared.bounds().min(), prepared.bounds().max()],
            });
        }
        let frame = renderer.render_with_geometry(&scene, config, &draws)?;
        retained.push((frame, cpu_frame, pixel, camera, expected_hit.position));
    }
    drop(deformation);
    drop(packing);
    drop(bounds);
    drop(asset);
    drop(graph);
    drop(renderer);

    let mut previous_id = None;
    for (frame, cpu_frame, pixel, camera, expected_position) in retained {
        assert_ne!(previous_id.as_ref(), Some(frame.frame_id()));
        previous_id = Some(frame.frame_id().clone());
        let mut request = frame.readback()?;
        let actual = read(|| request.try_read())?;
        let mut request = cpu_frame.readback()?;
        let expected = read(|| request.try_read())?;
        assert_eq!(actual.frame_id(), frame.frame_id());
        let actual_ids = actual.pixels.object_ids.as_ref().unwrap();
        let expected_ids = expected.pixels.object_ids.as_ref().unwrap();
        let mut surface_pixels = 0;
        for y in 1..79 {
            for x in 1..95 {
                let i = y * 96 + x;
                let id = expected_ids[i];
                if !(y - 1..=y + 1)
                    .all(|row| (x - 1..=x + 1).all(|col| expected_ids[row * 96 + col] == id))
                {
                    continue;
                }
                assert_eq!(actual_ids[i], id, "coverage at ({x}, {y})");
                if id == 0 {
                    continue;
                }
                surface_pixels += 1;
                let a = actual.pixels.linear_depth.as_ref().unwrap()[i];
                let b = expected.pixels.linear_depth.as_ref().unwrap()[i];
                assert!((a - b).abs() < 1e-4, "depth at ({x}, {y}): {a} != {b}");
                for (a, b) in actual.pixels.world_normals.as_ref().unwrap()[i]
                    .into_iter()
                    .zip(expected.pixels.world_normals.as_ref().unwrap()[i])
                {
                    assert!((a - b).abs() < 2e-3, "normal at ({x}, {y}): {a} != {b}");
                }
                for (a, b) in actual.pixels.linear_rgba.as_ref().unwrap()[i]
                    .into_iter()
                    .zip(expected.pixels.linear_rgba.as_ref().unwrap()[i])
                {
                    assert!((a - b).abs() < 2e-3, "color at ({x}, {y}): {a} != {b}");
                }
            }
        }
        assert!(surface_pixels > 100);
        let mut request = frame.pick(pixel)?;
        let pick = read(|| request.try_read())?;
        assert_eq!(pick.frame_id(), frame.frame_id());
        assert_eq!(pick.camera().eye, camera.eye);
        let hit = pick.hit.unwrap();
        assert_eq!(hit.object.node, Some(primitive));
        super::super::near(hit.world_position, expected_position);
        let mut request = frame.pick([0, 0])?;
        assert!(read(|| request.try_read())?.hit.is_none());
    }
    Ok(())
}
