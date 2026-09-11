use super::*;
use crate::{GpuMorph, MorphTarget, MorphTargets, Vertex};

mod numeric;

fn mesh() -> Mesh {
    Mesh::new(
        [[0., 0., 0.], [2., 0., 0.], [0., 3., 0.], [2., 3., 0.]]
            .into_iter()
            .map(|position| Vertex {
                position,
                normal: [0., 0., 1.],
                uv: [0.; 2],
            })
            .collect(),
        vec![2, 1, 3, 0, 1, 2],
    )
    .with_uv_set(2, vec![[0., 0.], [-1., 0.], [0., 1.], [-1., 1.]])
    .unwrap()
    .with_uv_set(3, vec![[0., 0.], [1., 0.], [0., 1.], [1., 1.]])
    .unwrap()
}

#[test]
fn selected_coordinates_and_payload_admission_preserve_indexed_topology() {
    let mesh = mesh();
    let uv = coordinates(&mesh, 2).unwrap();
    assert_ne!(uv, coordinates(&mesh, 0).unwrap());
    assert_eq!(uv[mesh.indices()[0] as usize], [0., 1.]);
    assert!(coordinates(&mesh, 4).is_err());
    let limits = GpuDeformationLimits {
        max_source_bytes: 72,
        max_output_bytes: 128,
    };
    let plan =
        GpuTangentDerivativesMemory::plan(mesh.vertex_count(), mesh.index_count(), limits).unwrap();
    assert_eq!(plan.uv_bytes as usize, std::mem::size_of_val(uv.as_slice()));
    assert_eq!(
        plan.index_bytes as usize,
        std::mem::size_of_val(mesh.indices())
    );
    assert_eq!(
        plan.output_bytes as usize,
        mesh.triangle_count() * std::mem::size_of::<GpuTangentDerivative>()
    );
    assert!(
        GpuTangentDerivativesMemory::plan(
            4,
            6,
            GpuDeformationLimits {
                max_source_bytes: 71,
                ..limits
            }
        )
        .is_err()
    );
    assert!(
        GpuTangentDerivativesMemory::plan(
            4,
            6,
            GpuDeformationLimits {
                max_output_bytes: 127,
                ..limits
            }
        )
        .is_err()
    );
    for (vertices, indices) in [(0, 3), (3, 0), (3, 4), (usize::MAX, 3), (3, usize::MAX)] {
        assert!(GpuTangentDerivativesMemory::plan(vertices, indices, limits).is_err());
    }
}

#[test]
fn shader_layout_matches_face_and_vertex_records() {
    use wgpu::naga::{
        TypeInner,
        front::wgsl,
        valid::{Capabilities, ValidationFlags, Validator},
    };
    let module = wgsl::parse_str(SHADER).unwrap();
    assert!(
        Validator::new(ValidationFlags::all(), Capabilities::empty())
            .validate(&module)
            .is_err()
    );
    let info = Validator::new(ValidationFlags::all(), Capabilities::FLOAT64)
        .validate(&module)
        .unwrap();
    #[cfg(target_os = "linux")]
    wgpu::naga::back::spv::write_vec(&module, &info, &Default::default(), None).unwrap();
    #[cfg(not(target_os = "linux"))]
    let _ = info;
    let globals: Vec<_> = module
        .global_variables
        .iter()
        .map(|(_, v)| {
            (
                v.binding.as_ref().unwrap().binding,
                &module.types[v.ty].inner,
            )
        })
        .collect();
    assert_eq!(
        globals.iter().map(|entry| entry.0).collect::<Vec<_>>(),
        [0, 1, 2, 3, 4]
    );
    for (binding, stride) in [(0, 64), (1, 8), (2, 4), (3, 64)] {
        assert!(
            matches!(globals[binding].1, TypeInner::Array { stride: actual, .. } if *actual == stride)
        );
    }
    let TypeInner::Array { base, .. } = globals[3].1 else {
        panic!("expected face array")
    };
    let TypeInner::Struct { members, span } = &module.types[*base].inner else {
        panic!("expected face record")
    };
    assert_eq!(*span as usize, std::mem::size_of::<GpuTangentDerivative>());
    assert_eq!(
        members
            .iter()
            .map(|m| m.offset as usize)
            .collect::<Vec<_>>(),
        [
            std::mem::offset_of!(GpuTangentDerivative, tangent),
            std::mem::offset_of!(GpuTangentDerivative, bitangent),
            std::mem::offset_of!(GpuTangentDerivative, classification),
            std::mem::offset_of!(GpuTangentDerivative, status),
        ]
    );
    assert!(matches!(globals[4].1, TypeInner::Struct { span: 16, .. }));
    assert_eq!(module.entry_points[0].workgroup_size, [64, 1, 1]);
}

fn read(output: &GpuTangentDerivativeOutput) -> Result<Vec<GpuTangentDerivative>> {
    let context = output.context();
    let staging = context.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("tangent derivative verification"),
        size: output.buffer().size(),
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = context.device.create_command_encoder(&Default::default());
    encoder.copy_buffer_to_buffer(output.buffer(), 0, &staging, 0, output.buffer().size());
    let submission = context.queue.submit(Some(encoder.finish()));
    let (sender, receiver) = std::sync::mpsc::channel();
    staging
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
    context.device.poll(wgpu::PollType::Wait {
        submission_index: Some(submission),
        timeout: Some(std::time::Duration::from_secs(30)),
    })?;
    receiver.recv_timeout(std::time::Duration::from_secs(1))??;
    let mapped = staging.slice(..).get_mapped_range()?;
    let records = mapped
        .chunks_exact(64)
        .map(bytemuck::pod_read_unaligned)
        .collect();
    drop(mapped);
    staging.unmap();
    Ok(records)
}

#[test]
#[ignore = "requires a compute-capable GPU"]
fn gpu_derivatives_follow_morph_uv_orientation_and_report_degeneracy_and_failures() -> Result<()> {
    let context = WgpuContext::new_headless()?;
    let limits = GpuDeformationLimits::default();
    let base = mesh();
    let targets = MorphTargets::new(
        base.clone(),
        [MorphTarget {
            positions: Some(
                base.vertices()
                    .iter()
                    .map(|v| [0., 0., v.position[0] * 0.5])
                    .collect(),
            ),
            ..Default::default()
        }],
    )?;
    let morph = GpuMorph::new(context.clone(), targets.clone(), limits)?;
    let derivatives = GpuTangentDerivatives::new(context.clone(), base.clone(), 2, limits)?;
    let mut outputs = Vec::new();
    for weight in [0., 1., -0.5] {
        let input = morph.evaluate(&[weight])?;
        let output = derivatives.evaluate(&input)?;
        assert_eq!(output.uv_set(), 2);
        assert!(Arc::ptr_eq(&output.base_mesh().0, &base.0));
        outputs.push((weight, output));
    }
    let foreign = GpuDeformationOutput::upload(context.clone(), mesh(), limits)?;
    assert!(derivatives.evaluate(&foreign).is_err());
    let input = morph.evaluate(&[0.])?;
    let positive =
        GpuTangentDerivatives::new(context.clone(), base.clone(), 3, limits)?.evaluate(&input)?;
    assert_eq!(positive.input_buffer(), input.buffer());
    for record in read(&positive)? {
        assert_eq!(record.classification, [0, 0, 1, 0]);
        assert_eq!(record.status, [0; 4]);
        assert_eq!(record.tangent, [1., 0., 0., 2.]);
        assert_eq!(record.bitangent, [0., 1., 0., 3.]);
    }
    let zero_uv =
        GpuTangentDerivatives::new(context.clone(), base.clone(), 0, limits)?.evaluate(&input)?;
    for record in read(&zero_uv)? {
        assert_eq!(record.classification, [0, 1, 0, 1]);
        assert_eq!(record.status, [0; 4]);
        assert_eq!(record.tangent, [0.; 4]);
        assert_eq!(record.bitangent, [0.; 4]);
    }
    for expected_status in [[0; 4], [0, 0, 7, 0], [1, 0, 0, 0]] {
        let mut records = crate::geometry::gpu_deformation::pack_mesh(&base);
        if expected_status == [0; 4] {
            for record in &mut records {
                record.position = [0.; 4];
            }
        } else if expected_status[0] == 1 {
            records[1].position[0] = f32::NAN;
        } else {
            records[1].status = expected_status;
        }
        let input = GpuDeformationOutput {
            context: context.clone(),
            base: base.clone(),
            buffer: buffer(
                &context.device,
                "tangent derivative input",
                bytemuck::cast_slice(&records),
                wgpu::BufferUsages::STORAGE,
            ),
        };
        for record in read(&derivatives.evaluate(&input)?)? {
            assert_eq!(record.status, expected_status);
            if expected_status == [0; 4] {
                assert_eq!(record.classification, [1, 0, 0, 1]);
            }
        }
    }
    drop(derivatives);
    drop(morph);
    for (weight, output) in outputs {
        let cpu = targets.evaluate(&[weight])?;
        let tangents = cpu.generate_tangents_for_uv_set(2, crate::TangentGenerationMode::Strict)?;
        let length = (4. + weight * weight).sqrt();
        let expected = [-2. / length, 0., -weight / length, length];
        for (triangle, record) in read(&output)?.iter().enumerate() {
            assert_eq!(record.classification, [0, 0, 0, 0]);
            assert_eq!(record.status, [0; 4]);
            for (a, b) in record.tangent.into_iter().zip(expected) {
                assert!((a - b).abs() < 1e-5, "{a} != {b}");
            }
            for (a, b) in record.bitangent.into_iter().zip([0., 1., 0., 3.]) {
                assert!((a - b).abs() < 1e-5);
            }
            // MikkTSpace projects the face derivative against each corner's normal.
            let corner = tangents.mesh().indices()[triangle * 3] as usize;
            let tangent = tangents.mesh().tangents().unwrap()[corner];
            assert!((tangent[0] + 1.).abs() < 1e-5);
            assert_eq!(tangent[3], -1.);
        }
    }
    Ok(())
}
