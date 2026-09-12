use super::{Fixture, asset};
use anyhow::{Context as _, Result};
use gpui_3d::{
    AffineTransform, Camera, GpuDeformationBounds, GpuDeformationLimits,
    GpuGeometryBatchPreparation, HeadlessRenderer, Projection, Scene3dChannels, Scene3dGpuDraw,
    Scene3dOutputConfig, Scene3dVertexUpdate, SceneGraph,
};
use gpui_3d_gltf::GpuSceneDeformation;
use gpui_wgpu::wgpu;
use serde_json::json;
use std::time::{Duration, Instant};

fn read<T>(mut poll: impl FnMut() -> Result<Option<T>>) -> Result<T> {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if let Some(result) = poll()? {
            return Ok(result);
        }
        anyhow::ensure!(Instant::now() < deadline, "batch readback timed out");
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
#[ignore = "requires a compute-capable GPU"]
fn imported_batches_retain_complete_frames_after_rejection_and_cancellation() -> Result<()> {
    let mut fixture = Fixture::new();
    fixture.normals();
    let primitive = fixture.json["meshes"][0]["primitives"][0].clone();
    fixture.json["meshes"][0]["primitives"] = json!([]);
    for index in 0..3 {
        let offset = [0., (index + 1) as f32 * 0.25, index as f32 * 0.25];
        let delta = fixture.floats("VEC3", &offset.repeat(4));
        fixture.extent(delta, offset, offset);
        let mut primitive = primitive.clone();
        primitive["targets"] = json!([{"POSITION":delta}]);
        fixture.json["meshes"][0]["primitives"]
            .as_array_mut()
            .unwrap()
            .push(primitive);
    }
    let asset = asset(&fixture);
    let mut graph = SceneGraph::new();
    let instance = graph.instantiate(None, asset.subtree())?;
    let nodes: Vec<_> = asset
        .primitives()
        .iter()
        .map(|primitive| instance.node(primitive.handle).unwrap())
        .collect();
    assert_eq!(nodes.len(), 3);
    for (index, node) in nodes.iter().enumerate() {
        graph.set_transform(
            *node,
            AffineTransform::from_translation([index as f32 * 1.5 - 2., -0.5, 0.])?,
        )?;
    }
    let target = instance.node(asset.morphs()[0].node()).unwrap();
    let poses = graph.evaluate()?;
    let camera = Camera {
        eye: [0., 0., 5.],
        target: [0.; 3],
        projection: Projection::Orthographic { vertical_size: 3. },
        ..Default::default()
    };
    let mut renderer = HeadlessRenderer::new()?;
    let context = renderer.context().clone();
    let deformation = GpuSceneDeformation::new(
        context.clone(),
        &asset,
        GpuDeformationLimits::default(),
        None,
    )?;
    let bounds = GpuDeformationBounds::new(context.clone())?;
    let mut sources = Vec::new();
    let mut retained = Vec::new();
    for weight in [0., 0.5, -0.5] {
        let weights = [(target, vec![weight])];
        let cpu_meshes = asset.deform(&instance, &poses, &weights)?;
        let cpu_scene = poses.with_meshes(cpu_meshes.clone())?.scene(camera);
        let outputs = deformation.evaluate(&instance, &poses, &weights, None)?;
        assert_eq!(outputs.len(), 3);
        let scene = poses
            .with_meshes(
                outputs
                    .iter()
                    .map(|(node, output)| (*node, output.base_mesh().clone())),
            )?
            .scene(camera);
        let inputs = scene.geometry_inputs()?.collect::<Vec<_>>();
        let mut next_sources = Vec::new();
        for (index, (node, output)) in outputs.iter().enumerate() {
            let input = inputs
                .iter()
                .find(|input| input.node == Some(*node))
                .unwrap();
            next_sources.push(match sources.get(index) {
                Some(source) => output.rebind_render_source(source, input.uv_sets, None)?,
                None => output.render_source(input.uv_sets, None)?,
            });
        }
        sources = next_sources;
        let inputs: Vec<_> = outputs
            .iter()
            .zip(&sources)
            .map(|((_, output), source)| (output, source))
            .collect();
        let bytes = sources
            .iter()
            .map(|source| source.memory().vertex_bytes + source.memory().draw_bytes + 96)
            .sum::<u64>();
        assert!(GpuGeometryBatchPreparation::new(&inputs, &bounds, Some(bytes - 1)).is_err());
        let cancelled = GpuGeometryBatchPreparation::new(&inputs, &bounds, Some(bytes))?;
        drop(cancelled);
        if weight != 0. {
            let count = outputs[2].1.base_mesh().vertex_count();
            let invalid: Vec<u8> = [1_f32, 1., 1., -1.]
                .repeat(count)
                .into_iter()
                .flat_map(f32::to_le_bytes)
                .collect();
            let buffer = context.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: None,
                contents: &invalid,
                usage: wgpu::BufferUsages::COPY_SRC,
            });
            let invalid_source =
                sources[2].with_attributes(&[Scene3dVertexUpdate::ColorBuffer(&buffer)], None)?;
            let mut invalid_inputs = inputs.clone();
            invalid_inputs[2].1 = &invalid_source;
            let mut failed =
                GpuGeometryBatchPreparation::new(&invalid_inputs, &bounds, Some(bytes))?;
            let error = read(|| failed.try_read())
                .err()
                .context("invalid final primitive was published")?;
            assert_eq!(error.to_string(), "GPU preparation input 2");
            assert!(format!("{error:#}").contains("INVALID_COLOR"));
            assert!(failed.try_read().is_err());
        }
        let mut pending = GpuGeometryBatchPreparation::new(&inputs, &bounds, Some(bytes))?;
        let prepared = read(|| pending.try_read())?;
        assert_eq!(prepared.len(), outputs.len());
        assert_eq!(pending.working_bytes(), bytes);
        assert!(pending.try_read().is_err());
        let mut draws = Vec::new();
        for ((node, _), prepared) in outputs.iter().zip(prepared) {
            let mesh = &cpu_meshes
                .iter()
                .find(|(handle, _)| handle == node)
                .unwrap()
                .1;
            assert_eq!(prepared.bounds(), mesh.bounds());
            let input = scene
                .geometry_inputs()?
                .find(|input| input.node == Some(*node))
                .unwrap();
            draws.push(Scene3dGpuDraw {
                output_id: input.output_id,
                geometry: prepared.geometry().clone(),
                bounds: [prepared.bounds().min(), prepared.bounds().max()],
            });
        }
        retained.push((scene, cpu_scene, draws));
    }
    drop((asset, graph, deformation, bounds, sources));
    let mut frames = Vec::new();
    for samples in [1, 4] {
        let config = Scene3dOutputConfig {
            size: [128, 80],
            channels: Scene3dChannels::all(),
            color_samples: samples,
        };
        for (scene, cpu, draws) in retained.iter().rev().chain(retained.iter()) {
            frames.push((
                renderer.render_with_geometry(scene, config, draws)?,
                renderer.render(cpu, config)?,
            ));
        }
    }
    renderer.clear_caches();
    drop((renderer, retained));
    for (frame, reference) in frames {
        let mut request = frame.readback()?;
        let actual = read(|| request.try_read())?;
        let mut request = reference.readback()?;
        let expected = read(|| request.try_read())?;
        assert_eq!(actual.pixels.object_ids, expected.pixels.object_ids);
        assert_eq!(actual.pixels.linear_depth, expected.pixels.linear_depth);
        assert_eq!(actual.pixels.world_normals, expected.pixels.world_normals);
        assert_eq!(actual.pixels.linear_rgba, expected.pixels.linear_rgba);
        assert_eq!(actual.pixels.rgba, expected.pixels.rgba);
        assert_eq!(actual.frame_id(), frame.frame_id());
        for node in &nodes {
            let object = frame
                .objects()
                .iter()
                .find(|object| object.node == Some(*node))
                .unwrap();
            let pixels: Vec<_> = actual
                .pixels
                .object_ids
                .as_ref()
                .unwrap()
                .iter()
                .enumerate()
                .filter_map(|(index, id)| (*id == object.output_id).then_some(index))
                .collect();
            assert!(pixels.len() > 100);
            let index = pixels[pixels.len() / 2];
            let pixel = [(index % 128) as u32, (index / 128) as u32];
            let mut request = frame.pick(pixel)?;
            let pick = read(|| request.try_read())?;
            assert_eq!(pick.frame_id(), frame.frame_id());
            let hit = pick.hit.unwrap();
            assert_eq!(hit.object.node, Some(*node));
            assert_eq!(
                Some(hit.world_position),
                actual.world_position_at(pixel[0], pixel[1])?
            );
        }
    }
    Ok(())
}
