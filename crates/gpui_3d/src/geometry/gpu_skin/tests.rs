use super::*;
use crate::{GpuMorph, Mesh, MorphTarget, MorphTargets, PlaneOptions, SkinInfluence};

fn binding(vertices: usize) -> Skin {
    Skin::new(
        [AffineTransform::IDENTITY; 3],
        (0..vertices).map(|vertex| {
            (0..6).map(move |index| SkinInfluence {
                joint: (vertex + index) % 3,
                weight: (index + 1) as f32,
            })
        }),
    )
    .unwrap()
}

#[test]
fn skin_shader_validates_storage_and_matrix_layout() {
    use wgpu::naga::{
        ShaderStage, TypeInner,
        front::wgsl,
        valid::{Capabilities, ValidationFlags, Validator},
    };
    let module = wgsl::parse_str(include_str!("../gpu_skin.wgsl")).unwrap();
    Validator::new(ValidationFlags::all(), Capabilities::empty())
        .validate(&module)
        .unwrap();
    let globals: Vec<_> = module
        .global_variables
        .iter()
        .filter_map(|(_, v)| {
            v.binding
                .as_ref()
                .map(|b| (b.group, b.binding, &module.types[v.ty].inner))
        })
        .collect();
    assert_eq!(
        globals.iter().map(|&(g, b, _)| (g, b)).collect::<Vec<_>>(),
        [(0, 0), (0, 1), (0, 2), (0, 3), (0, 4)]
    );
    assert!(matches!(globals[0].2, TypeInner::Array { stride: 64, .. }));
    assert!(matches!(globals[1].2, TypeInner::Array { stride: 4, .. }));
    assert!(matches!(globals[2].2, TypeInner::Array { stride: 64, .. }));
    assert!(matches!(globals[3].2, TypeInner::Array { stride: 64, .. }));
    assert_eq!(module.entry_points[0].stage, ShaderStage::Compute);
    assert_eq!(module.entry_points[0].workgroup_size, [64, 1, 1]);
}

#[test]
fn compact_influences_preserve_variable_ranges_joint_order_and_normalized_weights() {
    let source = Skin::new(
        [AffineTransform::IDENTITY; 3],
        [
            vec![SkinInfluence {
                joint: 2,
                weight: 1.,
            }],
            (0..7)
                .map(|index| SkinInfluence {
                    joint: index % 3,
                    weight: index as f32,
                })
                .collect(),
        ],
    )
    .unwrap();
    let memory = GpuSkinMemory::plan(2, 3, 7, GpuDeformationLimits::default()).unwrap();
    let words = pack_binding(&source, memory).unwrap();
    assert_eq!(words.len() as u64 * 4, memory.binding_bytes);
    for vertex in 0..2 {
        let start = 3 + 2 * words[vertex] as usize;
        let end = 3 + 2 * words[vertex + 1] as usize;
        let records: Vec<_> = words[start..end].chunks_exact(2).collect();
        let expected = source.vertex_influences(vertex).unwrap();
        assert_eq!(records.len(), expected.len());
        for (record, expected) in records.iter().zip(expected) {
            assert_eq!(record[0] as usize, expected.joint);
            assert!((f64::from(f32::from_bits(record[1])) - expected.weight).abs() < 1e-7);
        }
    }
    let tiny = Skin::new(
        [AffineTransform::IDENTITY],
        [vec![
            SkinInfluence {
                joint: 0,
                weight: f32::from_bits(1),
            },
            SkinInfluence {
                joint: 0,
                weight: f32::MAX,
            },
        ]],
    )
    .unwrap();
    assert!(
        pack_binding(
            &tiny,
            GpuSkinMemory::plan(1, 1, 2, GpuDeformationLimits::default()).unwrap()
        )
        .is_err()
    );
}

#[test]
fn skin_admission_bounds_flat_indexing_palette_storage_and_dispatch() {
    let limits = GpuDeformationLimits::default();
    let memory = GpuSkinMemory::plan(90, 3, 540, limits).unwrap();
    assert!(
        GpuSkinMemory::plan(
            90,
            3,
            540,
            GpuDeformationLimits {
                max_source_bytes: memory.binding_bytes
                    + memory.palette_bytes
                    + memory.uniform_bytes
                    - 1,
                ..limits
            }
        )
        .is_err()
    );
    assert!(
        GpuSkinMemory::plan(
            90,
            3,
            540,
            GpuDeformationLimits {
                max_output_bytes: memory.output_bytes - 1,
                ..limits
            }
        )
        .is_err()
    );
    for (vertices, joints, influences) in [
        (0, 1, 0),
        (1, 0, 1),
        (2, 1, 1),
        (1, 1, u32::MAX as usize),
        (usize::MAX, 1, usize::MAX),
    ] {
        assert!(GpuSkinMemory::plan(vertices, joints, influences, limits).is_err());
    }
    assert!(
        memory
            .validate_device(&wgpu::Limits {
                max_compute_workgroups_per_dimension: 1,
                ..Default::default()
            })
            .is_err()
    );
    let many_joints = GpuSkinMemory::plan(1, 1000, 1, limits).unwrap();
    assert!(
        many_joints
            .validate_device(&wgpu::Limits {
                max_storage_buffer_binding_size: 1024,
                ..Default::default()
            })
            .is_err()
    );
}

fn near_mesh(actual: &Mesh, expected: &Mesh) {
    assert_eq!(actual.indices(), expected.indices());
    assert_eq!(actual.vertex_count(), expected.vertex_count());
    assert_eq!(actual.vertex_colors(), expected.vertex_colors());
    assert_eq!(actual.tangent_uv_set(), expected.tangent_uv_set());
    for (a, b) in actual.vertices().iter().zip(expected.vertices()) {
        assert_eq!(a.uv, b.uv);
        for (a, b) in a
            .position
            .into_iter()
            .chain(a.normal)
            .zip(b.position.into_iter().chain(b.normal))
        {
            assert!((a - b).abs() < 5e-5, "{a} != {b}");
        }
    }
    for (a, b) in actual
        .tangents()
        .unwrap()
        .iter()
        .flatten()
        .zip(expected.tangents().unwrap().iter().flatten())
    {
        assert!((a - b).abs() < 5e-5, "{a} != {b}");
    }
}

#[test]
#[ignore = "requires a compute-capable GPU"]
fn compute_skin_composes_morph_and_retains_outputs_with_shared_palettes() {
    let context = WgpuContext::new_headless().unwrap();
    let limits = GpuDeformationLimits::default();
    let mesh = Mesh::subdivided_plane(PlaneOptions {
        segments: [9, 8],
        ..Default::default()
    })
    .unwrap();
    let source = binding(mesh.vertex_count());
    let skin = GpuSkin::new(context.clone(), source.clone(), limits).unwrap();
    let morph_source = MorphTargets::new(
        mesh.clone(),
        [MorphTarget {
            positions: Some(vec![[0.1, 0.2, -0.3]; mesh.vertex_count()].into()),
            ..Default::default()
        }],
    )
    .unwrap();
    let morph = GpuMorph::new(context.clone(), morph_source.clone(), limits).unwrap();
    let input = morph.evaluate(&[0.75]).unwrap();
    let joints = [
        AffineTransform::from_trs([0.5, 0.2, -0.1], [0., 0., 0.1, 1.], [1., 2., 0.5]).unwrap(),
        AffineTransform::from_matrix([
            [1., 0., 0., 0.],
            [0.3, 2., 0., 0.],
            [0., 0., 1., 0.],
            [0., 1., 0., 1.],
        ])
        .unwrap(),
        AffineTransform::IDENTITY,
    ];
    let world = AffineTransform::from_translation([3., 2., 1.]).unwrap();
    let prepared = source.palette(world, &joints).unwrap();
    let palette = skin.upload_palette(&prepared).unwrap();
    let unrelated = binding(mesh.vertex_count());
    let wrong = unrelated.palette(world, &joints).unwrap();
    assert!(skin.upload_palette(&wrong).is_err());
    let first = skin.evaluate(&input, &palette).unwrap();
    let uploaded = GpuDeformationOutput::upload(context.clone(), mesh.clone(), limits).unwrap();
    let bind_result = skin.evaluate(&uploaded, &palette).unwrap();
    let reflection = AffineTransform::from_trs([0.; 3], [0., 0., 0., 1.], [-1., 1., 1.]).unwrap();
    let reflected_palette = skin
        .palette(AffineTransform::IDENTITY, &[reflection; 3])
        .unwrap();
    let reflected = skin.evaluate(&input, &reflected_palette).unwrap();
    let foreign = GpuSkin::new(context.clone(), source.clone(), limits).unwrap();
    let shared = foreign.upload_palette(&prepared).unwrap();
    near_mesh(
        &foreign
            .evaluate(&input, &shared)
            .unwrap()
            .readback()
            .unwrap(),
        &source
            .evaluate_with_palette(&morph_source.evaluate(&[0.75]).unwrap(), &prepared)
            .unwrap(),
    );
    assert!(foreign.evaluate(&input, &palette).is_err());
    let mismatched = GpuDeformationOutput::upload(context.clone(), Mesh::cube(), limits).unwrap();
    assert!(skin.evaluate(&mismatched, &palette).is_err());
    drop((skin, morph, input, palette, reflected_palette, uploaded));
    let morphed = morph_source.evaluate(&[0.75]).unwrap();
    near_mesh(
        &first.readback().unwrap(),
        &source.evaluate_world(&morphed, world, &joints).unwrap(),
    );
    near_mesh(
        &bind_result.readback().unwrap(),
        &source.evaluate_world(&mesh, world, &joints).unwrap(),
    );
    near_mesh(
        &reflected.readback().unwrap(),
        &source.evaluate(&morphed, &[reflection; 3]).unwrap(),
    );

    let collapsed = Skin::new(
        [AffineTransform::IDENTITY; 2],
        (0..mesh.vertex_count()).map(|_| {
            [
                SkinInfluence {
                    joint: 0,
                    weight: 1.,
                },
                SkinInfluence {
                    joint: 1,
                    weight: 1.,
                },
            ]
        }),
    )
    .unwrap();
    let gpu = GpuSkin::new(context.clone(), collapsed, limits).unwrap();
    let palette = gpu
        .palette(
            AffineTransform::IDENTITY,
            &[AffineTransform::IDENTITY, reflection],
        )
        .unwrap();
    let input = GpuDeformationOutput::upload(context.clone(), mesh.clone(), limits).unwrap();
    assert!(gpu.evaluate(&input, &palette).unwrap().readback().is_err());

    let invalid = GpuMorph::new(
        context,
        MorphTargets::new(
            mesh.clone(),
            [MorphTarget {
                normals: Some(vec![[0., 0., -1.]; mesh.vertex_count()].into()),
                ..Default::default()
            }],
        )
        .unwrap(),
        limits,
    )
    .unwrap()
    .evaluate(&[1.])
    .unwrap();
    assert!(
        foreign
            .evaluate(
                &invalid,
                &foreign
                    .palette(AffineTransform::IDENTITY, &[AffineTransform::IDENTITY; 3])
                    .unwrap()
            )
            .unwrap()
            .readback()
            .is_err()
    );
}
