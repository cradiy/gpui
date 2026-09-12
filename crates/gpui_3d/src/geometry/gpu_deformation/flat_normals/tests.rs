use super::*;
use crate::{
    AffineTransform, GpuMorph, GpuSkin, MorphTarget, MorphTargets, NormalMode, Skin, SkinInfluence,
    Vertex,
};

fn corners() -> Mesh {
    Mesh::new(
        [
            [0., 0., 0.],
            [1., 0., 0.],
            [0., 1., 0.],
            [0., 0., 0.],
            [0., 1., 0.],
            [0., 0., 1.],
        ]
        .into_iter()
        .map(|position| Vertex {
            position,
            normal: [0., 0., 1.],
            uv: [0.; 2],
        })
        .collect(),
        vec![5, 3, 4, 2, 0, 1],
    )
}

#[test]
fn topology_maps_permuted_corners_and_rejects_shared_or_unused_vertices_and_tangents() {
    let mesh = corners();
    let offsets = face_offsets(&mesh).unwrap();
    assert_eq!(offsets, [3, 3, 3, 0, 0, 0]);
    assert!(face_offsets(&Mesh::plane()).is_err());
    let repeated = Mesh::new(mesh.vertices().to_vec(), vec![0, 1, 2, 0, 4, 5]);
    assert!(
        face_offsets(&repeated)
            .unwrap_err()
            .to_string()
            .contains("repeated")
    );
    let unused = Mesh::new(mesh.vertices().to_vec(), vec![0, 1, 2]);
    assert!(face_offsets(&unused).is_err());
    let tangents = mesh.with_tangents(vec![[1., 0., 0., 1.]; 6]).unwrap();
    assert!(face_offsets(&tangents).is_err());
    assert_eq!(mesh.indices(), [5, 3, 4, 2, 0, 1]);
}

#[test]
fn payload_admission_checks_topology_and_output_before_allocation() {
    let limits = GpuDeformationLimits {
        max_source_bytes: 64,
        max_output_bytes: 384,
    };
    let memory = GpuFlatNormalsMemory::plan(6, limits).unwrap();
    let mesh = corners();
    assert_eq!(
        memory.face_offsets_bytes as usize,
        std::mem::size_of_val(face_offsets(&mesh).unwrap().as_slice())
    );
    assert_eq!(
        memory.index_bytes as usize,
        std::mem::size_of_val(mesh.indices())
    );
    assert_eq!(
        memory.output_bytes as usize,
        mesh.vertex_count() * std::mem::size_of::<crate::GpuDeformationVertex>()
    );
    assert!(
        GpuFlatNormalsMemory::plan(
            6,
            GpuDeformationLimits {
                max_source_bytes: 63,
                ..limits
            }
        )
        .is_err()
    );
    assert!(
        GpuFlatNormalsMemory::plan(
            6,
            GpuDeformationLimits {
                max_output_bytes: 383,
                ..limits
            }
        )
        .is_err()
    );
    for vertices in [0, 1, 4, usize::MAX] {
        assert!(GpuFlatNormalsMemory::plan(vertices, limits).is_err());
    }
}

#[test]
fn shader_validates_deformation_layout_and_topology_bindings() {
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
        globals.iter().map(|v| v.0).collect::<Vec<_>>(),
        [0, 1, 2, 3, 4]
    );
    for index in [0, 3] {
        assert!(matches!(
            globals[index].1,
            TypeInner::Array { stride: 64, .. }
        ));
    }
    for index in [1, 2] {
        assert!(matches!(
            globals[index].1,
            TypeInner::Array { stride: 4, .. }
        ));
    }
    assert!(matches!(globals[4].1, TypeInner::Struct { span: 16, .. }));
    assert_eq!(module.entry_points[0].workgroup_size, [64, 1, 1]);
}

#[test]
#[ignore = "requires a compute-capable GPU with SHADER_F64"]
fn gpu_flat_normals_compose_morph_and_skin_and_preserve_retained_outputs() -> Result<()> {
    let context = WgpuContext::new_headless()?;
    let limits = GpuDeformationLimits::default();
    let base = corners()
        .with_uv_set(2, vec![[0.2, 0.7]; 6])?
        .with_vertex_colors(vec![[0.1, 0.3, 0.5, 0.7]; 6])?;
    let source = MorphTargets::new(
        base.clone(),
        [MorphTarget {
            positions: Some(
                vec![
                    [0., 0., 0.],
                    [0., 0., 0.5],
                    [0., 0., 1.],
                    [0., 0., 0.],
                    [1., 0., 0.],
                    [0.2, 0., 0.],
                ]
                .into(),
            ),
            ..Default::default()
        }],
    )?;
    let morph = GpuMorph::new(context.clone(), source.clone(), limits)?;
    let normals = GpuFlatNormals::new(context.clone(), base.clone(), limits)?;
    let binding = Skin::new(
        [AffineTransform::IDENTITY],
        (0..6).map(|_| {
            [SkinInfluence {
                joint: 0,
                weight: 1.,
            }]
        }),
    )?;
    let skin = GpuSkin::new(context.clone(), binding.clone(), limits)?;
    let pose = AffineTransform::from_trs([2., -1., 0.5], [0., 0., 0., 1.], [1.5, 0.75, 2.])?;
    let palette = skin.palette(AffineTransform::IDENTITY, &[pose])?;
    let mut outputs = Vec::new();
    for weight in [0., 1., -0.5] {
        let input = morph.evaluate(&[weight])?;
        let rebuilt = normals.evaluate(&input)?;
        assert!(Arc::ptr_eq(&rebuilt.base_mesh().0, &base.0));
        let skinned = skin.evaluate(&rebuilt, &palette)?;
        outputs.push((weight, rebuilt, skinned));
    }
    let foreign = GpuDeformationOutput::upload(context.clone(), corners(), limits)?;
    assert!(normals.evaluate(&foreign).is_err());
    for expected_status in [[4, 0, 0, 0], [1, 0, 0, 0], [0, 0, 7, 0]] {
        let mut records = crate::geometry::gpu_deformation::pack_mesh(&base);
        if expected_status[0] == 4 {
            for record in &mut records[..3] {
                record.position = [0.; 4];
            }
        } else if expected_status[0] == 1 {
            records[2].position[0] = f32::INFINITY;
        } else {
            records[2].status = expected_status;
        }
        let invalid = GpuDeformationOutput {
            context: context.clone(),
            base: base.clone(),
            buffer: context.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("normal failures"),
                contents: bytemuck::cast_slice(&records),
                usage: wgpu::BufferUsages::STORAGE,
            }),
        };
        let error = normals
            .evaluate(&invalid)?
            .readback()
            .unwrap_err()
            .to_string();
        assert!(
            error.contains(&format!("vertex 0 failed with status {expected_status:?}")),
            "{error}"
        );
    }
    drop(normals);
    drop(morph);
    drop(skin);
    for (weight, rebuilt, skinned) in outputs {
        let expected = source
            .evaluate(&[weight])?
            .generate_normals(NormalMode::Flat)?;
        let actual = rebuilt.readback()?;
        assert_eq!(actual.indices(), base.indices());
        assert_eq!(actual.vertex_colors(), base.vertex_colors());
        for (corner, &source_index) in expected.source_vertices().iter().enumerate() {
            let a = actual.vertices()[source_index as usize];
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
                actual.uv_at(2, source_index as usize),
                base.uv_at(2, source_index as usize)
            );
        }
        let expected_skin = binding.evaluate_world(&actual, AffineTransform::IDENTITY, &[pose])?;
        for (a, b) in skinned
            .readback()?
            .vertices()
            .iter()
            .zip(expected_skin.vertices())
        {
            for (a, b) in a
                .position
                .into_iter()
                .chain(a.normal)
                .zip(b.position.into_iter().chain(b.normal))
            {
                assert!((a - b).abs() < 1e-5, "{a} != {b}");
            }
        }
    }
    Ok(())
}

#[test]
#[ignore = "requires a compute-capable GPU with SHADER_F64"]
fn gpu_flat_normals_preserve_finite_face_directions_across_coordinate_ranges() -> Result<()> {
    let context = WgpuContext::new_headless()?;
    let make_mesh = |positions: [[f32; 3]; 3]| {
        Mesh::new(
            positions
                .into_iter()
                .map(|position| Vertex {
                    position,
                    normal: [0., 1., 0.],
                    uv: [0.; 2],
                })
                .collect(),
            vec![0, 1, 2],
        )
    };
    let base = make_mesh([[0.; 3], [1., 0., 0.], [0., 1., 0.]]);
    let normals = GpuFlatNormals::new(context.clone(), base.clone(), Default::default())?;
    let tiny = f32::from_bits(1);
    let large = f32::MAX;
    let mut triangles = vec![
        [[-large, 0., 0.], [large, 0., 0.], [0., large, 0.]],
        [
            [0.; 3],
            [16_777_216., 16_777_215., 0.],
            [16_777_215., 16_777_214., 0.],
        ],
        [[0.; 3], [tiny, 0., 0.], [0., tiny, 0.]],
        [[0.; 3], [large, tiny, 0.], [large, 0., tiny]],
    ];
    for scale in [1e-30, 1., 1e30] {
        triangles.push([[0.; 3], [2. * scale, 0., 0.], [0., 3. * scale, 4. * scale]]);
    }
    let mut outputs = Vec::new();
    for positions in triangles {
        let mesh = make_mesh(positions);
        let expected = mesh.generate_normals(NormalMode::Flat)?;
        let buffer = context.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::cast_slice(&super::super::pack_mesh(&mesh)),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        });
        let input = GpuDeformationOutput::from_buffer(
            context.clone(),
            base.clone(),
            buffer,
            Default::default(),
        )?;
        outputs.push((normals.evaluate(&input)?, expected, mesh));
    }
    drop(normals);
    for (output, expected, input) in outputs {
        let actual = output.readback()?;
        for (corner, &source) in expected.source_vertices().iter().enumerate() {
            let actual = actual.vertices()[source as usize];
            assert_eq!(
                actual.position.map(f32::to_bits),
                input.vertices()[source as usize].position.map(f32::to_bits)
            );
            for (actual, expected) in actual
                .normal
                .into_iter()
                .zip(expected.mesh().vertices()[corner].normal)
            {
                assert!((actual - expected).abs() < 2e-6, "{actual} != {expected}");
            }
        }
    }
    Ok(())
}
