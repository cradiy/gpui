use super::*;
mod parity;
use crate::{
    AffineTransform, GpuMorph, GpuSkin, GpuTangentAdjacency, GpuTangentDerivatives,
    GpuTangentFrames, GpuTangentGroups, GpuTangentWeld, MorphTarget, MorphTargets, Skin,
    SkinInfluence, Vertex,
};

fn mesh() -> Mesh {
    Mesh::new(
        [[0., 0., 0.], [2., 0., 0.], [0., 1., 0.]]
            .into_iter()
            .map(|position| Vertex {
                position,
                normal: [0., 0., 2.],
                uv: [0.; 2],
            })
            .collect(),
        vec![2, 0, 1],
    )
    .with_uv_set(2, vec![[0., 0.], [1., 0.], [0., 1.]])
    .unwrap()
    .with_vertex_colors(vec![[0.2, 0.3, 0.4, 1.]; 3])
    .unwrap()
}

#[test]
fn preparation_preserves_vertex_order_attributes_and_selected_tangent_basis() {
    let base = mesh();
    let (topology, output) = prepare(&base, 2, TangentGenerationMode::Strict).unwrap();
    assert_eq!(
        topology.iter().map(|v| v.vertex).collect::<Vec<_>>(),
        base.indices()
    );
    for item in &topology {
        assert_eq!(Some(item.uv), base.uv_at(2, item.vertex as usize));
    }
    for (output, source) in output.vertices().iter().zip(base.vertices()) {
        assert_eq!(output.position, source.position);
        assert_eq!(output.normal, source.normal);
        assert_eq!(output.uv, source.uv);
    }
    assert_eq!(output.indices(), base.indices());
    assert_eq!(output.vertex_colors(), base.vertex_colors());
    assert_eq!(output.tangent_uv_set(), Some(2));
    assert!(base.tangents().is_none());
    let expected = base
        .generate_tangents_for_uv_set(2, TangentGenerationMode::Strict)
        .unwrap();
    for (&source, tangent) in expected
        .source_vertices()
        .iter()
        .zip(expected.mesh().tangents().unwrap())
    {
        assert_eq!(&output.tangents().unwrap()[source as usize], tangent);
        assert_eq!(
            output.uv_at(2, source as usize),
            base.uv_at(2, source as usize)
        );
    }
}

#[test]
fn preparation_requires_fixed_corners_and_explicit_initial_repair() {
    let base = mesh();
    for indices in [vec![0, 1, 1], vec![0, 1, 2, 0, 1, 2]] {
        let shared = Mesh::new(base.vertices().to_vec(), indices);
        assert!(prepare(&shared, 0, TangentGenerationMode::Repair).is_err());
    }
    assert!(prepare(&base, 3, TangentGenerationMode::Repair).is_err());
    for mode in [
        TangentGenerationMode::Strict,
        TangentGenerationMode::Inherit,
    ] {
        assert!(prepare(&base, 0, mode).is_err());
    }
    let (_, repaired) = prepare(&base, 0, TangentGenerationMode::Repair).unwrap();
    assert_eq!(repaired.tangent_uv_set(), Some(0));
    assert_eq!(repaired.tangents().unwrap(), [[1., 0., 0., 1.]; 3]);
}

#[test]
fn admission_includes_repair_output_and_shader_matches_host_layout() {
    let limits = GpuDeformationLimits {
        max_source_bytes: 64,
        max_output_bytes: 204,
    };
    let memory = GpuTangentsMemory::plan(3, limits).unwrap();
    assert_eq!(
        memory.topology_bytes as usize,
        std::mem::size_of_val(
            prepare(&mesh(), 2, TangentGenerationMode::Strict)
                .unwrap()
                .0
                .as_slice()
        )
    );
    for limits in [
        GpuDeformationLimits {
            max_source_bytes: 63,
            ..limits
        },
        GpuDeformationLimits {
            max_output_bytes: 203,
            ..limits
        },
    ] {
        assert!(GpuTangentsMemory::plan(3, limits).is_err());
    }
    for count in [0, 1, 4, usize::MAX] {
        assert!(GpuTangentsMemory::plan(count, limits).is_err());
    }
    use wgpu::naga::{
        TypeInner,
        front::wgsl,
        valid::{Capabilities, ValidationFlags, Validator},
    };
    let module = wgsl::parse_str(include_str!("../tangents.wgsl")).unwrap();
    Validator::new(ValidationFlags::all(), Capabilities::empty())
        .validate(&module)
        .unwrap();
    for (_, variable) in module.global_variables.iter() {
        let binding = variable.binding.as_ref().unwrap();
        assert_eq!(binding.group, 0);
        let ty = &module.types[variable.ty].inner;
        if binding.binding == 5 {
            assert!(matches!(ty, TypeInner::Struct { span: 16, .. }));
        } else {
            let TypeInner::Array { base, stride, .. } = ty else {
                panic!("storage array required")
            };
            assert_eq!(*stride, [64, 64, 16, 64, 4][binding.binding as usize]);
            if binding.binding == 2 {
                let TypeInner::Struct { members, .. } = &module.types[*base].inner else {
                    panic!("topology record required")
                };
                assert_eq!(
                    members
                        .iter()
                        .map(|m| m.offset as usize)
                        .collect::<Vec<_>>(),
                    [
                        std::mem::offset_of!(Topology, vertex),
                        std::mem::offset_of!(Topology, reserved),
                        std::mem::offset_of!(Topology, uv)
                    ]
                );
            }
        }
    }
    assert_eq!(module.entry_points[0].workgroup_size, [64, 1, 1]);
}

fn frames(input: &GpuDeformationOutput, set: u32) -> Result<GpuTangentFramesOutput> {
    let context = input.context();
    let base = input.base_mesh();
    let limits = GpuDeformationLimits::default();
    let derivatives =
        GpuTangentDerivatives::new(context.clone(), base.clone(), set, limits)?.evaluate(input)?;
    let weld =
        GpuTangentWeld::new(context.clone(), base.clone(), set, limits)?.evaluate(&derivatives)?;
    let adjacency =
        GpuTangentAdjacency::new(context.clone(), base.clone(), set, limits)?.evaluate(&weld)?;
    let groups =
        GpuTangentGroups::new(context.clone(), base.clone(), set, limits)?.evaluate(&adjacency)?;
    GpuTangentFrames::new(context.clone(), base.clone(), set, limits)?.evaluate(&groups)
}

fn repair_tags(output: &GpuTangentsOutput) -> Result<Vec<u32>> {
    let context = output.deformation().context();
    let source = output.repair_buffer();
    let staging = context.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("tangent repair verification"),
        size: source.size(),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = context.device.create_command_encoder(&Default::default());
    encoder.copy_buffer_to_buffer(source, 0, &staging, 0, source.size());
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
    let values = mapped
        .chunks_exact(4)
        .map(bytemuck::pod_read_unaligned)
        .collect();
    drop(mapped);
    staging.unmap();
    Ok(values)
}

fn compare(actual: &Mesh, expected: &Mesh) {
    assert_eq!(actual.indices(), expected.indices());
    assert_eq!(actual.tangent_uv_set(), expected.tangent_uv_set());
    for ((a, ta), (b, tb)) in actual
        .vertices()
        .iter()
        .zip(actual.tangents().unwrap())
        .zip(expected.vertices().iter().zip(expected.tangents().unwrap()))
    {
        for (a, b) in a
            .position
            .into_iter()
            .chain(a.normal)
            .chain(*ta)
            .zip(b.position.into_iter().chain(b.normal).chain(*tb))
        {
            assert!((a - b).abs() < 2e-5, "{a} != {b}");
        }
    }
}

#[test]
#[ignore = "requires a compute-capable GPU"]
fn gpu_publication_preserves_morph_skin_correspondence_and_retained_outputs() -> Result<()> {
    let context = WgpuContext::new_headless()?;
    let limits = GpuDeformationLimits::default();
    let base = mesh();
    let targets = MorphTargets::new(
        base.clone(),
        [MorphTarget {
            positions: Some(vec![[0.; 3], [0.5, 0.2, 0.], [-0.2, 0.1, 0.]].into()),
            normals: Some(vec![[0.2, 0.1, 0.]; 3].into()),
            ..Default::default()
        }],
    )?;
    let morph = GpuMorph::new(context.clone(), targets.clone(), limits)?;
    let source = GpuTangents::new(
        context.clone(),
        base,
        2,
        TangentGenerationMode::Strict,
        limits,
    )?;
    let binding = Skin::new(
        [AffineTransform::IDENTITY],
        (0..3).map(|_| {
            [SkinInfluence {
                joint: 0,
                weight: 1.,
            }]
        }),
    )?;
    let skin = GpuSkin::new(context.clone(), binding.clone(), limits)?;
    let pose = AffineTransform::from_trs([2., -1., 0.5], [0., 0., 0., 1.], [-1.5, 0.75, 2.])?;
    let palette = skin.palette(AffineTransform::IDENTITY, &[pose])?;
    let mut retained = Vec::new();
    for weight in [0., 0.5, 1.] {
        let input = morph.evaluate(&[weight])?;
        let output = source.evaluate(&frames(&input, 2)?)?;
        let skinned = skin.evaluate(output.deformation(), &palette)?;
        let render_source = skinned.render_source([2; 5], None)?;
        let packed = skinned.render_geometry(&render_source)?;
        assert!(Arc::ptr_eq(
            &output.deformation().base_mesh().0,
            &source.output_mesh().0
        ));
        retained.push((weight, output, skinned, packed));
    }
    let input = GpuDeformationOutput::upload(context, mesh(), limits)?;
    assert!(source.evaluate(&frames(&input, 2)?).is_err());
    drop(source);
    drop(morph);
    drop(skin);
    for (weight, output, skinned, _packed) in retained {
        let (_, expected) = prepare(
            &targets.evaluate(&[weight])?,
            2,
            TangentGenerationMode::Strict,
        )?;
        compare(&output.deformation().readback()?, &expected);
        assert_eq!(repair_tags(&output)?, [0; 3]);
        let expected = binding.evaluate_world(&expected, AffineTransform::IDENTITY, &[pose])?;
        compare(&skinned.readback()?, &expected);
    }
    Ok(())
}

#[test]
#[ignore = "requires a compute-capable GPU"]
fn gpu_publication_reports_repairs_without_clearing_input_failures() -> Result<()> {
    let context = WgpuContext::new_headless()?;
    let limits = GpuDeformationLimits::default();
    let base = mesh();
    let strict = GpuTangents::new(
        context.clone(),
        base.clone(),
        2,
        TangentGenerationMode::Strict,
        limits,
    )?;
    let inherit = GpuTangents::new(
        context.clone(),
        base.clone(),
        2,
        TangentGenerationMode::Inherit,
        limits,
    )?;
    let repair = GpuTangents::new(
        context.clone(),
        base.clone(),
        2,
        TangentGenerationMode::Repair,
        limits,
    )?;
    let mut records = super::super::pack_mesh(&base);
    for record in &mut records {
        record.position = [0.; 4];
    }
    let input = |records: &[crate::GpuDeformationVertex]| GpuDeformationOutput {
        context: context.clone(),
        base: base.clone(),
        buffer: buffer(
            &context.device,
            "tangent input",
            bytemuck::cast_slice(records),
            wgpu::BufferUsages::STORAGE,
        ),
    };
    let collapsed = frames(&input(&records), 2)?;
    assert!(
        strict
            .evaluate(&collapsed)?
            .deformation()
            .readback()
            .is_err()
    );
    assert!(
        inherit
            .evaluate(&collapsed)?
            .deformation()
            .readback()
            .is_err()
    );
    let repaired = repair.evaluate(&collapsed)?;
    assert_eq!(repair_tags(&repaired)?, [2; 3]);
    assert_eq!(
        repaired.deformation().readback()?.tangents().unwrap(),
        [[1., 0., 0., 1.]; 3]
    );
    records[1].position = [2., 0., 0., 0.];
    records[2].position = [1., 0., 0., 0.];
    let collinear = repair.evaluate(&frames(&input(&records), 2)?)?;
    assert_eq!(repair_tags(&collinear)?, [1, 1, 0]);
    assert_eq!(
        collinear.deformation().readback()?.tangents().unwrap(),
        [[1., 0., 0., 1.]; 3]
    );
    records[0].normal = [0.; 4];
    let invalid_normal = repair.evaluate(&frames(&input(&records), 2)?)?;
    assert_eq!(repair_tags(&invalid_normal)?, [0; 3]);
    assert!(invalid_normal.deformation().readback().is_err());
    records[0].normal = [0., 0., 2., 0.];
    records[0].status = [0, 0, 7, 0];
    let failed = repair.evaluate(&frames(&input(&records), 2)?)?;
    assert_eq!(repair_tags(&failed)?, [0; 3]);
    assert!(
        failed
            .deformation()
            .readback()
            .unwrap_err()
            .to_string()
            .contains("[0, 0, 7, 0]")
    );
    Ok(())
}

#[test]
#[ignore = "requires a compute-capable GPU"]
fn gpu_publication_rejects_conflicting_inherited_handedness() -> Result<()> {
    let context = WgpuContext::new_headless()?;
    let limits = GpuDeformationLimits::default();
    let positions = [
        [0., 0., 0.],
        [1., 0., 0.],
        [0., 1., 0.],
        [2., 0., 0.],
        [3., 0., 0.],
        [2., 1., 0.],
        [4., 0., 0.],
        [5., 0., 0.],
        [4., 1., 0.],
    ];
    let uv = [
        [0., 0.],
        [1., 0.],
        [0., 1.],
        [1., 0.],
        [0., 0.],
        [1., 1.],
        [0., 0.],
        [1., 0.],
        [0., 0.],
    ];
    let base = Mesh::new(
        positions
            .into_iter()
            .zip(uv)
            .map(|(position, uv)| Vertex {
                position,
                normal: [0., 0., 1.],
                uv,
            })
            .collect(),
        (0..9).collect(),
    );
    let source = GpuTangents::new(
        context.clone(),
        base.clone(),
        0,
        TangentGenerationMode::Repair,
        limits,
    )?;
    let mut records = super::super::pack_mesh(&base);
    records[6].position = records[0].position;
    records[7].position = records[3].position;
    records[8].position = records[0].position;
    let input = GpuDeformationOutput {
        context: context.clone(),
        base,
        buffer: buffer(
            &context.device,
            "mixed tangent orientation",
            bytemuck::cast_slice(&records),
            wgpu::BufferUsages::STORAGE,
        ),
    };
    let output = source.evaluate(&frames(&input, 0)?)?;
    assert_eq!(repair_tags(&output)?, [0; 9]);
    let error = output.deformation().readback().unwrap_err().to_string();
    assert!(
        error.contains("vertex 6 failed with status [2, 0, 0, 0]"),
        "{error}"
    );
    Ok(())
}
