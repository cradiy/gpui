use anyhow::Result;
use gpui_3d::{
    AlphaMode, Camera, GpuDeformationLimits, GpuMorph, HeadlessRenderer, Material, Mesh,
    MorphTarget, MorphTargets, Object, Projection, Scene, Scene3dChannels, Scene3dGpuDraw,
    Scene3dMaterialProgram, Scene3dMaterialSource, Scene3dMaterialValue, Scene3dOutputConfig,
};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

fn read<T>(mut poll: impl FnMut() -> Result<Option<T>>) -> Result<T> {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if let Some(result) = poll()? {
            return Ok(result);
        }
        anyhow::ensure!(Instant::now() < deadline, "transparency readback timed out");
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn alpha(x: f32) -> f32 {
    if x < -0.125 {
        -0.5
    } else if x < 0.125 {
        1. / 1024.
    } else if x < 0.375 {
        0.25
    } else if x < 0.625 {
        0.75
    } else {
        1.5
    }
}

fn encode(value: f32) -> f32 {
    if value <= 0.0031308 {
        value * 12.92
    } else {
        1.055 * value.powf(1. / 2.4) - 0.055
    }
}

#[test]
#[ignore = "requires a compute-capable GPU"]
fn custom_transparency_keeps_nearest_surface_distinct_from_color_contribution() -> Result<()> {
    let mut renderer = HeadlessRenderer::new()?;
    let context = renderer.context().clone();
    let plane = Mesh::plane();
    let base = plane.with_vertices(
        plane
            .vertices()
            .iter()
            .copied()
            .map(|mut v| {
                v.normal = [0., 0.6, 0.8];
                v
            })
            .collect(),
        None,
    )?;
    let targets = MorphTargets::new(
        base.clone(),
        [MorphTarget {
            positions: Some(vec![[0.25, 0., 0.]; base.vertex_count()].into()),
            ..Default::default()
        }],
    )?;
    let cpu = targets.evaluate(&[1.])?;
    let morph = GpuMorph::new(context.clone(), targets, GpuDeformationLimits::default())?;
    let output = morph.evaluate(&[1.])?;
    let packing = output.render_source([0; 5], None)?;
    let geometry = Arc::new(output.render_geometry(&packing)?);
    let source = Scene3dMaterialSource::new(
        context,
        Scene3dMaterialProgram::compile(
            r#"
        struct Controls { offset: vec4<f32> }
        @group(1) @binding(0) var<uniform> controls: Controls;
        fn material_surface(input: SurfaceInput, gradients: mat2x2<f32>) -> vec4<f32> {
            let x = input.world.x + controls.offset.x;
            var alpha = 1.5;
            if x < -0.125 { alpha = -0.5; }
            else if x < 0.125 { alpha = 0.0009765625; }
            else if x < 0.375 { alpha = 0.25; }
            else if x < 0.625 { alpha = 0.75; }
            return vec4<f32>(1.0, 0.0, 0.0, alpha);
        }
        fn material_shading(base: vec3<f32>, input: SurfaceInput,
            gradients: SurfaceGradients, face_sign: f32) -> vec3<f32> { return base; }
    "#,
        )?,
    )?;
    let value = |offset: f32| {
        [(
            0,
            Scene3dMaterialValue::Uniform(
                [offset, 0., 0., 0.]
                    .into_iter()
                    .flat_map(f32::to_le_bytes)
                    .collect::<Vec<_>>()
                    .into(),
            ),
        )]
    };
    let original = source.bind(value(0.), Default::default())?;
    let shifted = original.with_values(value(0.25), Default::default())?;
    let camera = Camera {
        eye: [0., 0., 5.],
        target: [0.; 3],
        projection: Projection::Orthographic { vertical_size: 2. },
        ..Default::default()
    };
    let rear = Object::new(
        plane.clone(),
        Material::color(gpui::Rgba {
            r: 0.,
            g: 0.,
            b: 1.,
            a: 0.,
        })
        .unlit(true)
        .alpha_mode(AlphaMode::Opaque),
    )
    .scale([2., 2., 1.])
    .id("rear");
    let middle = Object::new(
        plane,
        Material::color(gpui::Rgba {
            r: 0.,
            g: 1.,
            b: 0.,
            a: 0.5,
        })
        .unlit(true)
        .alpha_mode(AlphaMode::Blend),
    )
    .scale([2., 2., 1.])
    .position([0., 0., 1.])
    .id("middle");
    let mut frames = Vec::new();
    for samples in [1, 4] {
        for gpu in [false, true] {
            for mode in [AlphaMode::Opaque, AlphaMode::Mask, AlphaMode::Blend] {
                for background in [false, true] {
                    for (offset, snapshot) in [(0., &original), (0.25, &shifted)] {
                        let front = Object::new(
                            if gpu { base.clone() } else { cpu.clone() },
                            Material::color(gpui::white())
                                .program(snapshot.clone())
                                .alpha_mode(mode),
                        )
                        .position([0., 0., 2.])
                        .id("front");
                        let mut objects = vec![middle.clone(), front];
                        if background {
                            objects.push(rear.clone());
                        }
                        if offset != 0. {
                            objects.reverse();
                        }
                        let scene = objects
                            .into_iter()
                            .fold(Scene::new().camera(camera), |scene, object| {
                                scene.object(object)
                            });
                        let draws = if gpu {
                            let input = scene
                                .geometry_inputs()?
                                .find(|object| object.id == Some(&"front".into()))
                                .unwrap();
                            vec![Scene3dGpuDraw {
                                output_id: input.output_id,
                                geometry: geometry.clone(),
                                bounds: [[-0.25, -0.5, 0.], [0.75, 0.5, 0.]],
                            }]
                        } else {
                            vec![]
                        };
                        let frame = renderer.render_with_geometry(
                            &scene,
                            Scene3dOutputConfig {
                                size: [64, 64],
                                channels: Scene3dChannels::all(),
                                color_samples: samples,
                            },
                            &draws,
                        )?;
                        frames.push((frame, mode, background, offset));
                    }
                }
            }
        }
    }
    renderer.clear_caches();
    drop((
        renderer, source, original, shifted, geometry, output, packing, morph, base, cpu,
    ));
    for (frame, mode, background, offset) in frames.into_iter().rev() {
        let mut request = frame.readback()?;
        let result = read(|| request.try_read())?;
        assert_eq!(result.frame_id(), frame.frame_id());
        for x in [18_u32, 25, 31, 39, 47, 55, 60] {
            let pixel = [x, 32];
            let world_x = (x as f32 + 0.5) / 32. - 1.;
            let raw_alpha = alpha(world_x + offset).clamp(0., 1.);
            let inside = (-0.25..0.75).contains(&world_x);
            let front_alpha = if !inside {
                0.
            } else {
                match mode {
                    AlphaMode::Opaque => 1.,
                    AlphaMode::Mask => {
                        if raw_alpha >= 0.5 {
                            1.
                        } else {
                            0.
                        }
                    }
                    AlphaMode::Blend => raw_alpha,
                }
            };
            let expected = [
                front_alpha,
                0.5 * (1. - front_alpha),
                if background {
                    0.5 * (1. - front_alpha)
                } else {
                    0.
                },
                if background {
                    1.
                } else {
                    front_alpha + 0.5 * (1. - front_alpha)
                },
            ];
            let index = (32 * 64 + x) as usize;
            for (actual, expected) in result.pixels.linear_rgba.as_ref().unwrap()[index]
                .into_iter()
                .zip(expected)
            {
                assert!(
                    (actual - expected).abs() < 0.001,
                    "mode {mode:?}, background {background}, offset {offset}, x {x}: {actual} != {expected}"
                );
            }
            let display = result.pixels.rgba.as_ref().unwrap();
            for axis in 0..4 {
                let value = if axis == 3 {
                    expected[3]
                } else {
                    encode(expected[axis] / expected[3]) * expected[3]
                };
                assert!((f32::from(display[index * 4 + axis]) - value * 255.).abs() <= 2.);
            }
            let name = if front_alpha > 0. { "front" } else { "middle" };
            assert_eq!(result.object_at(x, 32).unwrap().id, Some(name.into()));
            let depth = if front_alpha > 0. { 3. } else { 4. };
            assert!((result.pixels.linear_depth.as_ref().unwrap()[index] - depth).abs() < 0.00001);
            let normal = if front_alpha > 0. {
                [0., 0.6, 0.8, 1.]
            } else {
                [0., 0., 1., 1.]
            };
            for (actual, expected) in result.pixels.world_normals.as_ref().unwrap()[index]
                .into_iter()
                .zip(normal)
            {
                assert!((actual - expected).abs() < 0.001);
            }
            let mut request = frame.pick(pixel)?;
            let pick = read(|| request.try_read())?;
            assert_eq!(pick.frame_id(), frame.frame_id());
            let hit = pick.hit.unwrap();
            assert_eq!(hit.object.id, Some(name.into()));
            assert_eq!(hit.linear_depth, depth);
            assert_eq!(Some(hit.world_position), result.world_position_at(x, 32)?);
        }
    }
    Ok(())
}
