use super::*;
use crate::{
    Camera, GpuMorph, HeadlessRenderer, Material, MorphTarget, MorphTargets, Object, Projection,
    Scene, Scene3dChannels, Scene3dGpuDraw, Scene3dMaterialProgram, Scene3dMaterialSource,
    Scene3dOutputConfig, TangentRepairKind, Vertex,
};
use gpui_wgpu::wgpu;

fn fan(mirrored: bool, mode: TangentGenerationMode) -> Mesh {
    let mut vertices = vec![Vertex {
        position: [0.; 3],
        normal: [0., 0., 1.],
        uv: [0.; 2],
    }];
    let mut uv = vec![[0.; 2]];
    for index in 0..67 {
        let angle = index as f32 / 67. * std::f32::consts::TAU;
        let (s, c) = angle.sin_cos();
        vertices.push(Vertex {
            position: [2. * c, s, 0.2 * s * c],
            normal: [0.1 * c, 0.1 * s, 1.],
            uv: [0.; 2],
        });
        uv.push([
            (c + 0.1 * (3. * angle).cos()) * if mirrored { -1. } else { 1. },
            s + 0.1 * (2. * angle).sin(),
        ]);
    }
    let mut indices = Vec::new();
    for slot in 0..67 {
        let face = slot * 31 % 67;
        indices.extend([0, face + 1, (face + 1) % 67 + 1]);
    }
    if mode != TangentGenerationMode::Strict {
        for _ in 0..65 {
            indices.extend([0, 1, 1]);
        }
    }
    if mode == TangentGenerationMode::Repair {
        let first = vertices.len() as u32;
        for position in [[5., 0., 0.], [6., 0., 0.], [5., 1., 0.]] {
            vertices.push(Vertex {
                position,
                normal: [0., 0., 1.],
                uv: [0.; 2],
            });
            uv.push([0.; 2]);
        }
        indices.extend([first, first + 1, first + 2]);
    }
    let mesh = Mesh::new(vertices, indices).with_uv_set(2, uv).unwrap();
    mesh.expand_corners(mesh.index_count())
        .unwrap()
        .mesh()
        .clone()
}

fn repair_tags(context: &WgpuContext, output: &GpuTangentsOutput) -> Result<Vec<u32>> {
    let source = output.repair_buffer();
    let staging = context.device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: source.size(),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = context.device.create_command_encoder(&Default::default());
    encoder.copy_buffer_to_buffer(source, 0, &staging, 0, source.size());
    context.queue.submit([encoder.finish()]);
    let (send, receive) = std::sync::mpsc::channel();
    staging
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |result| {
            let _ = send.send(result);
        });
    context.device.poll(wgpu::PollType::Wait {
        submission_index: None,
        timeout: Some(std::time::Duration::from_secs(30)),
    })?;
    receive.recv_timeout(std::time::Duration::from_secs(1))??;
    let bytes = staging.slice(..).get_mapped_range()?;
    Ok(bytes
        .chunks_exact(4)
        .map(|bytes| u32::from_le_bytes(bytes.try_into().unwrap()))
        .collect())
}

#[test]
#[ignore = "requires a compute-capable GPU with SHADER_F64"]
fn generated_frames_match_cpu_weighting_inheritance_and_repair_snapshots() -> Result<()> {
    let context = WgpuContext::new_headless()?;
    let limits = GpuDeformationLimits::default();
    let mut retained = Vec::new();
    for mirrored in [false, true] {
        for mode in [
            TangentGenerationMode::Strict,
            TangentGenerationMode::Inherit,
            TangentGenerationMode::Repair,
        ] {
            let base = fan(mirrored, mode);
            let targets = MorphTargets::new(
                base.clone(),
                [MorphTarget {
                    positions: Some(
                        base.vertices()
                            .iter()
                            .map(|v| {
                                let [x, y, _] = v.position;
                                [-0.1 * y, 0.2 * x, 0.3 * x * y]
                            })
                            .collect::<Vec<_>>()
                            .into(),
                    ),
                    normals: Some(vec![[0.1, 0.2, 0.]; base.vertex_count()].into()),
                    ..Default::default()
                }],
            )?;
            let morph = GpuMorph::new(context.clone(), targets.clone(), limits)?;
            let generator = GpuTangentGeneration::new(context.clone(), base, 2, mode, limits)?;
            for weight in [-0.5, 0., 1.] {
                let input = targets.evaluate(&[weight])?;
                let expected = input.generate_tangents_for_uv_set(2, mode)?;
                let output = generator.evaluate(&morph.evaluate(&[weight])?)?;
                let source = output.deformation().render_source([2; 5], None)?;
                let packed = output.deformation().render_geometry(&source)?;
                let status = packed.request_status(None)?;
                retained.push((mirrored, mode, weight, input, expected, output, status));
            }
        }
    }
    let mut renderer = HeadlessRenderer::with_context(context.clone())?;
    let source = Scene3dMaterialSource::new(
        context.clone(),
        Scene3dMaterialProgram::compile(
            r#"
        fn material_surface(input: SurfaceInput, gradients: mat2x2<f32>) -> vec4<f32> {
            return vec4(1.0);
        }
        fn material_shading(base: vec3<f32>, input: SurfaceInput,
            gradients: SurfaceGradients, face_sign: f32) -> vec3<f32> {
            return unit_vector(input.tangent.xyz) * input.tangent.w * 0.5 + vec3(0.5);
        }
    "#,
        )?,
    )?;
    let material = Material::color(gpui::white()).program(source.bind([], Default::default())?);
    let camera = Camera {
        eye: [1.5, 0., 8.],
        target: [1.5, 0., 0.],
        projection: Projection::Orthographic { vertical_size: 5. },
        ..Default::default()
    };
    let mut rendered = Vec::new();
    for (mirrored, mode, weight, input, expected, output, mut status) in retained.into_iter().rev()
    {
        let actual = output
            .deformation()
            .readback()
            .with_context(|| format!("mirror {mirrored}, mode {mode:?}, weight {weight}"))?;
        let mut tags = vec![0; input.index_count()];
        for repair in expected.repairs() {
            tags[repair.triangle * 3 + repair.corner] = match repair.kind {
                TangentRepairKind::TriangleDerivative => 1,
                TangentRepairKind::OrthonormalBasis => 2,
            };
        }
        assert_eq!(
            repair_tags(&context, &output)?,
            tags,
            "mirror {mirrored}, mode {mode:?}, weight {weight}"
        );
        assert!(
            status
                .try_read()?
                .expect("packing status pending")
                .is_drawable()
        );
        for (corner, (&actual_index, &expected_index)) in actual
            .indices()
            .iter()
            .zip(expected.mesh().indices())
            .enumerate()
        {
            let a = actual.tangents().unwrap()[actual_index as usize];
            let b = expected.mesh().tangents().unwrap()[expected_index as usize];
            assert_eq!(a[3], b[3], "corner {corner}");
            for (a, b) in a[..3].iter().zip(b) {
                assert!(
                    (a - b).abs() < 2e-5,
                    "mirror {mirrored}, mode {mode:?}, weight {weight}, corner {corner}: {a} != {b}"
                );
            }
        }
        let deformation = output.deformation();
        let packing = deformation.render_source([0; 5], None)?;
        let scene = Scene::new().camera(camera).object(Object::new(
            deformation.base_mesh().clone(),
            material.clone(),
        ));
        let reference = Scene::new()
            .camera(camera)
            .object(Object::new(expected.mesh().clone(), material.clone()));
        let geometry = Scene3dGpuDraw {
            output_id: 1,
            geometry: std::sync::Arc::new(deformation.render_geometry(&packing)?),
            bounds: [input.bounds().min(), input.bounds().max()],
        };
        for samples in [1, 4] {
            let config = Scene3dOutputConfig {
                size: [96, 64],
                channels: Scene3dChannels::all(),
                color_samples: samples,
            };
            rendered.push((
                renderer.render_with_geometry(&scene, config, std::slice::from_ref(&geometry))?,
                renderer.render(&reference, config)?,
            ));
        }
    }
    drop((renderer, source, material));
    let read = |frame: &crate::RenderedFrame| -> Result<_> {
        let mut request = frame.readback()?;
        context.device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_secs(30)),
        })?;
        Ok(request.try_read()?.expect("tangent render pending").pixels)
    };
    for (actual, expected) in rendered {
        let a = read(&actual)?;
        let b = read(&expected)?;
        assert_eq!(a.object_ids, b.object_ids);
        assert!(
            a.object_ids
                .as_ref()
                .unwrap()
                .iter()
                .filter(|&&id| id != 0)
                .count()
                > 100
        );
        for (a, b) in a
            .linear_rgba
            .unwrap()
            .iter()
            .flatten()
            .zip(b.linear_rgba.unwrap().iter().flatten())
        {
            assert!((a - b).abs() <= 0.001, "tangent color: {a} != {b}");
        }
        for (a, b) in a
            .world_normals
            .unwrap()
            .iter()
            .flatten()
            .zip(b.world_normals.unwrap().iter().flatten())
        {
            assert!((a - b).abs() <= 0.001, "normal: {a} != {b}");
        }
        for (a, b) in a.linear_depth.unwrap().iter().zip(b.linear_depth.unwrap()) {
            assert!((a - b).abs() <= 1e-5, "depth: {a} != {b}");
        }
        for (a, b) in a.rgba.unwrap().iter().zip(b.rgba.unwrap()) {
            assert!(a.abs_diff(b) <= 1);
        }
    }
    Ok(())
}
