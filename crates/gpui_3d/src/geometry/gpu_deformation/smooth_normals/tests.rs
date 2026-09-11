use super::*;
use crate::{GpuMorph, MorphTarget, MorphTargets, NormalMode, Vertex};

fn mesh() -> Mesh {
    Mesh::new(
        [
            [0., 0., 0.],
            [2., 0., 0.],
            [0., 1., 0.],
            [0., 0., 3.],
            [0., 0., 0.],
            [0., 1., 0.],
            [0., 0., 3.],
            [9., 9., 9.],
        ]
        .into_iter()
        .map(|position| Vertex {
            position,
            normal: [0., 1., 0.],
            uv: [0.; 2],
        })
        .collect(),
        vec![0, 1, 2, 0, 2, 3, 4, 5, 6],
    )
}

#[test]
fn adjacency_preserves_face_order_seams_and_unused_rows() {
    let mesh = mesh();
    let data = adjacency(&mesh);
    for vertex in 0..mesh.vertex_count() {
        let start = mesh.vertex_count() + 1;
        let actual = &data[start + data[vertex] as usize..start + data[vertex + 1] as usize];
        let expected: Vec<_> = mesh
            .indices()
            .iter()
            .enumerate()
            .filter(|(_, index)| **index as usize == vertex)
            .map(|(offset, _)| (offset / 3 * 3) as u32)
            .collect();
        assert_eq!(actual, expected);
    }
    assert_eq!(data[1] - data[0], 2);
    assert_eq!(data[5] - data[4], 1);
    assert_eq!(data[8], data[7]);
}

#[test]
fn memory_plan_matches_uploaded_topology_and_bounds_both_payloads() {
    let mesh = mesh();
    let memory =
        GpuSmoothNormalsMemory::plan(mesh.vertex_count(), mesh.index_count(), Default::default())
            .unwrap();
    assert_eq!(
        memory.adjacency_bytes,
        std::mem::size_of_val(adjacency(&mesh).as_slice()) as u64
    );
    assert_eq!(
        memory.index_bytes,
        std::mem::size_of_val(mesh.indices()) as u64
    );
    let limits = GpuDeformationLimits {
        max_source_bytes: memory.adjacency_bytes + memory.index_bytes + memory.uniform_bytes,
        max_output_bytes: memory.output_bytes,
    };
    assert_eq!(GpuSmoothNormalsMemory::plan(8, 9, limits).unwrap(), memory);
    assert!(
        GpuSmoothNormalsMemory::plan(
            8,
            9,
            GpuDeformationLimits {
                max_source_bytes: limits.max_source_bytes - 1,
                ..limits
            }
        )
        .is_err()
    );
    assert!(
        GpuSmoothNormalsMemory::plan(
            8,
            9,
            GpuDeformationLimits {
                max_output_bytes: limits.max_output_bytes - 1,
                ..limits
            }
        )
        .is_err()
    );
    for (vertices, indices) in [
        (0, 3),
        (3, 0),
        (3, 4),
        (usize::MAX, 3),
        (3, usize::MAX),
        (u32::MAX as usize - 1, 3),
    ] {
        assert!(GpuSmoothNormalsMemory::plan(vertices, indices, Default::default()).is_err());
    }
}

#[test]
fn shader_validates_wide_arithmetic_and_canonical_record_layout() {
    use wgpu::naga::{
        TypeInner,
        front::wgsl,
        valid::{Capabilities, ValidationFlags, Validator},
    };
    let module = wgsl::parse_str(SHADER).unwrap();
    Validator::new(ValidationFlags::all(), Capabilities::FLOAT64)
        .validate(&module)
        .unwrap();
    assert!(
        Validator::new(ValidationFlags::all(), Capabilities::empty())
            .validate(&module)
            .is_err()
    );
    for (_, global) in module.global_variables.iter() {
        let binding = global.binding.as_ref().unwrap().binding;
        match binding {
            0 | 3 => assert!(matches!(
                module.types[global.ty].inner,
                TypeInner::Array { stride: 64, .. }
            )),
            1 | 2 => assert!(matches!(
                module.types[global.ty].inner,
                TypeInner::Array { stride: 4, .. }
            )),
            4 => assert!(matches!(
                module.types[global.ty].inner,
                TypeInner::Struct { span: 16, .. }
            )),
            _ => panic!("unexpected binding"),
        }
    }
    assert_eq!(module.entry_points[0].workgroup_size, [64, 1, 1]);
}

#[test]
#[ignore = "requires a compute-capable GPU with SHADER_F64"]
fn gpu_smooth_normals_match_deformed_area_weights_and_keep_source_correspondence() -> Result<()> {
    let context = WgpuContext::new_headless()?;
    let base = mesh()
        .with_uv_set(2, vec![[0.2, 0.7]; 8])?
        .with_vertex_colors(vec![[0.1, 0.3, 0.5, 0.7]; 8])?;
    let source = MorphTargets::new(
        base.clone(),
        [MorphTarget {
            positions: Some(
                vec![
                    [0., 0., 0.],
                    [0., 0., 0.5],
                    [0., 0.5, 0.],
                    [0.5, 0., 0.],
                    [0.; 3],
                    [0.; 3],
                    [0.; 3],
                    [0.; 3],
                ]
                .into(),
            ),
            ..Default::default()
        }],
    )?;
    let morph = GpuMorph::new(context.clone(), source.clone(), Default::default())?;
    let normals = GpuSmoothNormals::new(context.clone(), base.clone(), Default::default())?;
    let mut retained = Vec::new();
    for weight in [0., 1., -0.5] {
        retained.push((weight, normals.evaluate(&morph.evaluate(&[weight])?)?));
    }
    let foreign_mesh = GpuDeformationOutput::upload(context.clone(), mesh(), Default::default())?;
    assert!(normals.evaluate(&foreign_mesh).is_err());
    assert!(
        GpuSmoothNormals::new(
            context.clone(),
            base.with_tangents(vec![[1., 0., 0., 1.]; 8])?,
            Default::default()
        )
        .is_err()
    );
    drop(normals);
    drop(morph);
    for (weight, output) in retained {
        assert!(output.base_mesh().ptr_eq(&base));
        let actual = output.readback()?;
        let expected = source
            .evaluate(&[weight])?
            .generate_normals(NormalMode::Smooth)?;
        assert_eq!(actual.indices(), base.indices());
        assert_eq!(actual.vertex_colors(), base.vertex_colors());
        assert_eq!(actual.vertices()[7].position, base.vertices()[7].position);
        assert_eq!(actual.vertices()[7].normal, base.vertices()[7].normal);
        assert_eq!(actual.vertices()[7].uv, base.vertices()[7].uv);
        for (corner, &vertex) in expected.source_vertices().iter().enumerate() {
            let a = actual.vertices()[vertex as usize];
            let b = expected.mesh().vertices()[corner];
            for (a, b) in a
                .position
                .into_iter()
                .chain(a.normal)
                .zip(b.position.into_iter().chain(b.normal))
            {
                assert!((a - b).abs() < 1e-5, "{a} != {b}");
            }
            assert_eq!(
                actual.uv_at(2, vertex as usize),
                base.uv_at(2, vertex as usize)
            );
        }
        let render = output.render_source([0; 5], None)?;
        output.render_geometry(&render)?;
    }
    for scale in [1e-30_f32, 1e30] {
        let original = mesh();
        let scaled = Mesh::new(
            original
                .vertices()
                .iter()
                .map(|vertex| Vertex {
                    position: vertex.position.map(|component| component * scale),
                    ..*vertex
                })
                .collect(),
            original.indices().to_vec(),
        );
        let expected = scaled.generate_normals(NormalMode::Smooth)?;
        let input =
            GpuDeformationOutput::upload(context.clone(), scaled.clone(), Default::default())?;
        let normals = GpuSmoothNormals::new(context.clone(), scaled, Default::default())?;
        let actual = normals.evaluate(&input)?.readback()?;
        for (corner, &vertex) in expected.source_vertices().iter().enumerate() {
            for (a, b) in actual.vertices()[vertex as usize]
                .normal
                .into_iter()
                .zip(expected.mesh().vertices()[corner].normal)
            {
                assert!((a - b).abs() < 1e-5, "scale {scale}: {a} != {b}");
            }
        }
    }
    Ok(())
}

#[test]
#[ignore = "requires a compute-capable GPU with SHADER_F64"]
fn gpu_smooth_normals_reject_degenerate_cancelled_and_failed_incident_faces() -> Result<()> {
    let context = WgpuContext::new_headless()?;
    for (indices, invalid, expected) in [
        (vec![0, 1, 2, 0, 2, 1], None, [2, 0, 0, 0]),
        (vec![0, 0, 2], None, [4, 0, 0, 0]),
        (vec![0, 1, 2], Some(false), [0, 7, 0, 0]),
        (vec![0, 1, 2], Some(true), [1, 0, 0, 0]),
    ] {
        let base = Mesh::new(mesh().vertices().to_vec(), indices);
        let mut records = super::super::pack_mesh(&base);
        if let Some(nonfinite) = invalid {
            if nonfinite {
                records[1].position[0] = f32::NAN;
            } else {
                records[1].status = expected;
            }
        }
        let input = GpuDeformationOutput {
            context: context.clone(),
            base: base.clone(),
            buffer: context.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: None,
                contents: bytemuck::cast_slice(&records),
                usage: wgpu::BufferUsages::STORAGE,
            }),
        };
        let normals = GpuSmoothNormals::new(context.clone(), base, Default::default())?;
        let error = normals
            .evaluate(&input)?
            .readback()
            .unwrap_err()
            .to_string();
        assert!(
            error.contains(&format!("vertex 0 failed with status {expected:?}")),
            "{error}"
        );
    }
    Ok(())
}
