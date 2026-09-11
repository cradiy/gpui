use std::time::{Duration, Instant};

use anyhow::Result;
use gpui_3d::{
    AffineTransform, AlphaMode, Camera, GpuDeformationBounds, GpuDeformationLimits, GpuMorph,
    HeadlessRenderer, Material, Mesh, MeshPass, MeshPassBlend, MeshPassExpansion, MeshPassSpace,
    MeshPassState, MorphTarget, MorphTargets, Node, ObjectUpdate, Projection, Scene3dChannels,
    Scene3dMaterialProgram, Scene3dMaterialSource, Scene3dMaterialValue, Scene3dOutputConfig,
    Scene3dVertexAttribute, Scene3dVertexStreamValue, SceneGraph,
};
use gpui_wgpu::wgpu;

fn program() -> Result<Scene3dMaterialProgram> {
    Scene3dMaterialProgram::compile_with_attributes(
        r#"
        struct Parameters { value: vec4<f32> }
        @group(1) @binding(0) var<uniform> parameters: Parameters;
        fn material_surface(input: SurfaceInput, gradients: mat2x2<f32>) -> vec4<f32> {
            let alpha = select(0.0, 1.0, input.attributes.gate >= parameters.value.w);
            return vec4<f32>(parameters.value.xyz, alpha);
        }
        fn material_shading(base: vec3<f32>, input: SurfaceInput,
            gradients: SurfaceGradients, face_sign: f32) -> vec3<f32> { return base; }
        "#,
        &[Scene3dVertexAttribute::new(
            "gate",
            wgpu::VertexFormat::Float32,
        )],
    )
}

#[test]
fn submission_coverage_program_validates() {
    program().unwrap();
}

fn read<T>(mut poll: impl FnMut() -> Result<Option<T>>) -> Result<T> {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if let Some(value) = poll()? {
            return Ok(value);
        }
        anyhow::ensure!(Instant::now() < deadline, "readback timed out");
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
#[ignore = "requires a compute-capable GPU"]
fn submitted_deformation_materials_and_passes_retain_coverage_and_picking() -> Result<()> {
    let mut renderer = HeadlessRenderer::new()?;
    let context = renderer.context().clone();
    let base = Mesh::plane();
    let morph = GpuMorph::new(
        context.clone(),
        MorphTargets::new(
            base.clone(),
            [MorphTarget {
                positions: Some(vec![[0.5, 0., 0.]; base.vertex_count()].into()),
                ..Default::default()
            }],
        )?,
        GpuDeformationLimits::default(),
    )?;
    let bounds = GpuDeformationBounds::new(context.clone())?;
    let source = Scene3dMaterialSource::new(context, program()?)?;
    let values = |value: [f32; 4]| {
        [(
            0,
            Scene3dMaterialValue::Uniform(bytemuck::cast_slice(&value).to_vec().into()),
        )]
    };
    let initial = source.bind(values([1., 0., 0., 0.5]), Default::default())?;
    let mut graph = SceneGraph::new();
    let node = graph.insert(
        None,
        Node::new()
            .id("surface")
            .mesh(base.clone(), Material::color(gpui::white())),
    )?;
    let camera = Camera {
        eye: [0., 0., 5.],
        target: [0.; 3],
        projection: Projection::Orthographic { vertical_size: 4. },
        ..Default::default()
    };
    let scene = graph.evaluate()?.scene(camera);
    let mut scenes = Vec::new();
    let mut packing = None;
    for revision in 0..2 {
        let output = morph.evaluate(&[revision as f32])?;
        if packing.is_none() {
            packing = Some(output.render_source([0; 5], None)?);
        }
        let mut preparation =
            output.prepare_render_geometry(packing.as_ref().unwrap(), &bounds, None)?;
        let prepared = read(|| preparation.try_read())?;
        let gates: Vec<f32> = base
            .vertices()
            .iter()
            .map(|vertex| {
                if revision == 0 {
                    vertex.position[0] + 0.5
                } else {
                    0.5 - vertex.position[0]
                }
            })
            .collect();
        let streams = source.bind_vertex_streams(
            base.vertex_count(),
            &[(
                "gate",
                Scene3dVertexStreamValue::Bytes(bytemuck::cast_slice(&gates)),
            )],
            1024,
        )?;
        let snapshot = if revision == 0 {
            initial.clone()
        } else {
            initial.with_values(values([0., 1., 0., 0.25]), Default::default())?
        }
        .with_vertex_streams(streams)?;
        let mut material = Material::color(gpui::white())
            .alpha_mode(AlphaMode::Mask)
            .program(snapshot);
        if revision == 1 {
            let widths = vec![1_f32; base.vertex_count()];
            let pass_streams = source.bind_vertex_streams(
                base.vertex_count(),
                &[(
                    "gate",
                    Scene3dVertexStreamValue::Bytes(bytemuck::cast_slice(&widths)),
                )],
                1024,
            )?;
            let pass_material = initial
                .with_values(values([0., 0., 0.25, 0.]), Default::default())?
                .with_vertex_streams(pass_streams)?;
            material = material.mesh_passes([MeshPass::new(pass_material)
                .state(MeshPassState {
                    alpha_mode: AlphaMode::Mask,
                    blend: MeshPassBlend::Additive,
                    ..Default::default()
                })
                .expansion(MeshPassExpansion::new(MeshPassSpace::World, 0.05).weight("gate", 1.))]);
        }
        let world = if revision == 0 {
            [-0.75, 0., 0.]
        } else {
            [0.25, 0., 1.]
        };
        scenes.push(
            scene.with_object_updates([(
                1,
                ObjectUpdate::new()
                    .gpu_geometry(prepared.geometry().clone(), prepared.bounds())
                    .world(AffineTransform::from_translation(world)?)
                    .material(material),
            )])?,
        );
    }
    let config = Scene3dOutputConfig {
        size: [64, 64],
        channels: Scene3dChannels::all(),
        color_samples: 1,
    };
    let first = renderer.render(&scenes[0], config)?;
    let second = renderer.render(&scenes[1], config)?;
    let repeated = renderer.render(&scenes[0], config)?;
    assert_ne!(first.frame_id(), second.frame_id());
    assert_ne!(first.frame_id(), repeated.frame_id());
    renderer.clear_caches();
    drop((
        renderer, scenes, scene, graph, morph, bounds, source, initial, packing,
    ));

    let mut retained_pixels = None;
    for (frame, revision) in [(&first, 0), (&second, 1), (&repeated, 0)] {
        let mut pending = frame.readback()?;
        let result = read(|| pending.try_read())?;
        assert_eq!(result.frame_id(), frame.frame_id());
        let pixels = &result.pixels;
        for y in 0..64 {
            for x in 0..64 {
                let world_x = (x as f32 + 0.5) / 16. - 2.;
                let world_y = 2. - (y as f32 + 0.5) / 16.;
                let inside = world_y.abs() < 0.5
                    && if revision == 0 {
                        (-1.25..-0.25).contains(&world_x)
                    } else {
                        (0.25..1.25).contains(&world_x)
                    };
                let covered = inside
                    && if revision == 0 {
                        world_x >= -0.75
                    } else {
                        world_x <= 1.
                    };
                let index = (y * 64 + x) as usize;
                assert_eq!(
                    pixels.object_ids.as_ref().unwrap()[index],
                    u32::from(covered),
                    "revision {revision} at {x},{y}"
                );
                let depth = pixels.linear_depth.as_ref().unwrap()[index];
                let expected_depth = if covered {
                    5. - revision as f32
                } else {
                    pixels.depth_background.value()
                };
                assert!((depth - expected_depth).abs() < 1e-5);
                assert_eq!(
                    pixels.world_normals.as_ref().unwrap()[index],
                    if covered { [0., 0., 1., 1.] } else { [0.; 4] }
                );
                let mut expected = [0.; 4];
                if covered {
                    expected[revision] = 1.;
                    expected[3] = 1.;
                }
                if inside && revision == 1 {
                    expected[2] = 0.25;
                    expected[3] = 1.;
                }
                for (actual, expected) in pixels.linear_rgba.as_ref().unwrap()[index]
                    .into_iter()
                    .zip(expected)
                {
                    assert!(
                        (actual - expected).abs() < 1e-4,
                        "revision {revision} at {x},{y}: {actual} != {expected}"
                    );
                }
            }
        }
        if revision == 0 {
            if let Some(previous) = &retained_pixels {
                assert_eq!(pixels.linear_rgba.as_ref().unwrap(), previous);
            } else {
                retained_pixels = pixels.linear_rgba.clone();
            }
        }
        let pixel = if revision == 0 { [23, 32] } else { [43, 32] };
        let mut picking = frame.pick(pixel)?;
        let picked = read(|| picking.try_read())?;
        assert_eq!(picked.frame_id(), frame.frame_id());
        let hit = picked.hit.unwrap();
        assert_eq!(hit.object.node, Some(node));
        assert_eq!(hit.object.id, Some("surface".into()));
        assert!((hit.world_position[2] - revision as f32).abs() < 1e-5);
        let missing = if revision == 0 { [43, 32] } else { [49, 32] };
        let mut picking = frame.pick(missing)?;
        assert!(read(|| picking.try_read())?.hit.is_none());
    }
    Ok(())
}
