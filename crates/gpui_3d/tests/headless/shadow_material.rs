use anyhow::Result;
use gpui_3d::{
    AlphaMode, Camera, DirectionalShadow, GpuDeformationLimits, GpuMorph, HeadlessRenderer, Light,
    Material, Mesh, MorphTarget, MorphTargets, Object, Projection, PunctualLight, ReadFrame,
    RenderedFrame, Scene, Scene3dChannels, Scene3dGpuDraw, Scene3dMaterialProgram,
    Scene3dMaterialSource, Scene3dMaterialValue, Scene3dOutputConfig, Scene3dVertexAttribute,
    Scene3dVertexStreamValue,
};
use gpui_wgpu::wgpu;
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

fn read(frame: &RenderedFrame) -> Result<ReadFrame> {
    let mut pending = frame.readback()?;
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if let Some(result) = pending.try_read()? {
            return Ok(result);
        }
        anyhow::ensure!(Instant::now() < deadline, "shadow readback timed out");
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn scene(receiver: Material, caster: Object) -> Scene {
    Scene::new()
        .camera(Camera {
            projection: Projection::Orthographic { vertical_size: 2. },
            ..Default::default()
        })
        .light(Light {
            ambient: 0.12,
            ..Default::default()
        })
        .lights([PunctualLight::directional([1., 0., 1.]).intensity(0.8)])
        .directional_shadow(Some(DirectionalShadow {
            resolution: 512,
            softness: 0.,
            ..DirectionalShadow::new([0.; 3], [3.; 3])
        }))
        .object(Object::new(Mesh::plane(), receiver).scale([4., 4., 1.]))
        .object(caster)
}

#[test]
#[ignore = "requires a compute-capable GPU"]
fn custom_shadow_coverage_matches_cropped_geometry_after_attribute_and_pose_updates() -> Result<()>
{
    let mut renderer = HeadlessRenderer::new()?;
    let context = renderer.context().clone();
    let source = Scene3dMaterialSource::new(
        context.clone(),
        Scene3dMaterialProgram::compile_with_attributes(
            r#"
        struct Crop { threshold: vec4<f32> }
        @group(1) @binding(0) var<uniform> crop: Crop;
        fn material_surface(input: SurfaceInput, gradients: mat2x2<f32>) -> vec4<f32> {
            let covered = input.attributes.gate >= crop.threshold.x && input.world.y >= 0.0;
            return vec4<f32>(1.0, 1.0, 1.0, select(0.0, 1.0, covered));
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
    let receiver_source = Scene3dMaterialSource::new(
        context.clone(),
        Scene3dMaterialProgram::compile(
            r#"
        fn material_surface(input: SurfaceInput, gradients: mat2x2<f32>) -> vec4<f32> {
            return builtin_surface(input, gradients);
        }
        fn material_shading(base: vec3<f32>, input: SurfaceInput,
            gradients: SurfaceGradients, face_sign: f32) -> vec3<f32> {
            let normal = unit_vector(input.normal) * face_sign;
            var light = material_ambient(normal);
            for (var i = 0u; i < material_light_count(); i++) {
                let sample = material_light(i, input.world, normal, gradients.shadow_depth);
                light += sample.energy * max(dot(normal, sample.direction), 0.0);
            }
            return base * light;
        }
        "#,
        )?,
    )?;
    let receiver = Material::color(gpui::rgb(0x908070));
    let custom_receiver = receiver
        .clone()
        .program(receiver_source.bind([], Default::default())?);
    let base = Mesh::plane();
    let morph = GpuMorph::new(
        context,
        MorphTargets::new(
            base.clone(),
            [MorphTarget {
                positions: Some(vec![[0.5, 0., 0.]; base.vertex_count()].into()),
                ..Default::default()
            }],
        )?,
        GpuDeformationLimits::default(),
    )?;
    let uniform = |threshold: f32| {
        [(
            0,
            Scene3dMaterialValue::Uniform(bytemuck::cast_slice(&[threshold, 0., 0., 0.]).into()),
        )]
    };
    let initial = source.bind(uniform(0.5), Default::default())?;
    let mut submissions = Vec::new();
    for (gpu, right, threshold, mode) in [
        (false, true, 0.5, AlphaMode::Mask),
        (true, false, 0.5, AlphaMode::Mask),
        (true, true, 0.25, AlphaMode::Mask),
        (false, true, 0.75, AlphaMode::Mask),
        (true, false, 0.75, AlphaMode::Opaque),
        (true, true, 0.5, AlphaMode::Blend),
    ] {
        let gates: Vec<f32> = base
            .vertices()
            .iter()
            .map(|v| {
                if right {
                    v.position[0] + 0.5
                } else {
                    0.5 - v.position[0]
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
        let material = Material::color(gpui::white()).alpha_mode(mode).program(
            initial
                .with_values(uniform(threshold), Default::default())?
                .with_vertex_streams(streams)?,
        );
        let caster = Object::new(base.clone(), material)
            .position([1.5, 0., 1.5])
            .scale([0.5, 0.5, 1.]);
        let shift = if gpu { 0.25 } else { 0. };
        let (left, width, bottom, height) = if mode == AlphaMode::Opaque {
            (-0.25 + shift, 0.5, -0.25, 0.5)
        } else if right {
            (
                (threshold - 0.5) * 0.5 + shift,
                (1. - threshold) * 0.5,
                0.,
                0.25,
            )
        } else {
            (-0.25 + shift, (1. - threshold) * 0.5, 0., 0.25)
        };
        let reference = Object::new(base.clone(), Material::color(gpui::white()).unlit(true))
            .position([1.5 + left + width * 0.5, bottom + height * 0.5, 1.5])
            .scale([width, height, 1.])
            .cast_shadows(mode != AlphaMode::Blend);
        let reference_scene = scene(receiver.clone(), reference);
        let custom_scene = scene(custom_receiver.clone(), caster);
        let draws = if gpu {
            let output = morph.evaluate(&[1.])?;
            let packing = output.render_source([0; 5], None)?;
            vec![Scene3dGpuDraw {
                output_id: 2,
                geometry: Arc::new(output.render_geometry(&packing)?),
                bounds: [[0., -0.5, 0.], [1., 0.5, 0.]],
            }]
        } else {
            vec![]
        };
        submissions.push((
            reference_scene,
            custom_scene,
            draws,
            [left, width, bottom, height],
            mode == AlphaMode::Blend,
        ));
    }
    let mut retained = Vec::new();
    for (reference_scene, custom_scene, draws, bounds, unshadowed) in submissions {
        for samples in [1, 4] {
            let config = Scene3dOutputConfig {
                size: [65, 65],
                channels: Scene3dChannels::all(),
                color_samples: samples,
            };
            let expected = renderer.render(&reference_scene, config)?;
            let actual = renderer.render_with_geometry(&custom_scene, config, &draws)?;
            retained.push((expected, actual, bounds, unshadowed));
        }
    }
    renderer.clear_caches();
    drop(renderer);
    for (expected, actual, [left, width, bottom, height], unshadowed) in retained {
        let expected = read(&expected)?;
        let actual = read(&actual)?;
        assert_eq!(expected.pixels.object_ids, actual.pixels.object_ids);
        assert_eq!(expected.pixels.linear_depth, actual.pixels.linear_depth);
        assert_eq!(expected.pixels.world_normals, actual.pixels.world_normals);
        let mut shadow_pixels = 0;
        let mut lit_pixels = 0;
        for y in 0..65 {
            for x in 0..65 {
                let world = actual.world_position_at(x, y)?.expect("receiver missing");
                if [
                    world[0] - left,
                    world[0] - left - width,
                    world[1] - bottom,
                    world[1] - bottom - height,
                ]
                .into_iter()
                .any(|distance| distance.abs() < 0.04)
                {
                    continue;
                }
                let index = (y * 65 + x) as usize;
                let reference = expected.pixels.linear_rgba.as_ref().unwrap()[index];
                let color = actual.pixels.linear_rgba.as_ref().unwrap()[index];
                for (actual, expected) in color.iter().zip(reference) {
                    assert!(
                        (actual - expected).abs() < 0.002,
                        "world {world:?}: {color:?} != {reference:?}"
                    );
                }
                let shadowed = !unshadowed
                    && world[0] > left
                    && world[0] < left + width
                    && world[1] > bottom
                    && world[1] < bottom + height;
                if shadowed {
                    assert!(color[0] < 0.06, "missing shadow at {world:?}: {color:?}");
                    shadow_pixels += 1;
                } else {
                    assert!(color[0] > 0.15, "unexpected shadow at {world:?}: {color:?}");
                    lit_pixels += 1;
                }
            }
        }
        assert!(lit_pixels > 100);
        if !unshadowed {
            assert!(shadow_pixels >= 4);
        }
    }
    Ok(())
}
