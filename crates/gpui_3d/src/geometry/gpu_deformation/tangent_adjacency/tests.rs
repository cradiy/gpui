use super::*;
use crate::{
    GpuDeformationOutput, GpuMorph, GpuTangentDerivatives, GpuTangentWeld, MorphTarget,
    MorphTargets, Vertex,
};

#[test]
fn adjacency_admission_includes_sort_scratch_and_every_pass_uniform() {
    for corners in [3, 27, 81, 129, 1023] {
        let memory =
            GpuTangentAdjacencyMemory::plan(corners, GpuDeformationLimits::default()).unwrap();
        let passes = super::super::tangent_weld::passes(corners as u32);
        assert_eq!(
            memory.uniform_bytes as usize,
            std::mem::size_of_val(passes.as_slice())
        );
        assert_eq!(memory.sort_passes as usize + 2, passes.len());
        assert_eq!(
            memory.output_bytes as usize,
            corners * std::mem::size_of::<GpuTangentEdge>()
        );
        let limits = GpuDeformationLimits {
            max_source_bytes: memory.uniform_bytes,
            max_output_bytes: memory.scratch_bytes + memory.output_bytes,
        };
        assert!(GpuTangentAdjacencyMemory::plan(corners, limits).is_ok());
        assert!(
            GpuTangentAdjacencyMemory::plan(
                corners,
                GpuDeformationLimits {
                    max_source_bytes: limits.max_source_bytes - 1,
                    ..limits
                }
            )
            .is_err()
        );
        assert!(
            GpuTangentAdjacencyMemory::plan(
                corners,
                GpuDeformationLimits {
                    max_output_bytes: limits.max_output_bytes - 1,
                    ..limits
                }
            )
            .is_err()
        );
    }
    for corners in [0, 1, 4, (1 << 30) + 2, usize::MAX] {
        assert!(GpuTangentAdjacencyMemory::plan(corners, GpuDeformationLimits::default()).is_err());
    }
}

#[test]
fn adjacency_shader_validates_edge_face_and_corner_layouts() {
    use wgpu::naga::{
        TypeInner,
        front::wgsl,
        valid::{Capabilities, ValidationFlags, Validator},
    };
    let module = wgsl::parse_str(include_str!("../tangent_adjacency.wgsl")).unwrap();
    Validator::new(ValidationFlags::all(), Capabilities::empty())
        .validate(&module)
        .unwrap();
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
    for (_, ty) in &globals[..4] {
        assert!(matches!(ty, TypeInner::Array { stride: 64, .. }));
    }
    let TypeInner::Array { base, .. } = globals[3].1 else {
        panic!("expected edge array")
    };
    let TypeInner::Struct { members, span } = &module.types[*base].inner else {
        panic!("expected edge record")
    };
    assert_eq!(*span as usize, std::mem::size_of::<GpuTangentEdge>());
    assert_eq!(
        members
            .iter()
            .map(|m| m.offset as usize)
            .collect::<Vec<_>>(),
        [
            std::mem::offset_of!(GpuTangentEdge, edge),
            std::mem::offset_of!(GpuTangentEdge, adjacency),
            std::mem::offset_of!(GpuTangentEdge, classification),
            std::mem::offset_of!(GpuTangentEdge, status)
        ]
    );
    assert_eq!(
        members[2].offset as usize,
        std::mem::offset_of!(crate::GpuTangentWeldRecord, identity)
    );
    assert_eq!(
        members[2].offset as usize,
        std::mem::offset_of!(crate::GpuTangentDerivative, classification)
    );
    assert!(matches!(globals[4].1, TypeInner::Struct { span: 16, .. }));
    assert_eq!(
        module
            .entry_points
            .iter()
            .map(|e| (e.name.as_str(), e.workgroup_size))
            .collect::<Vec<_>>(),
        [
            ("initialize", [64, 1, 1]),
            ("sort_pairs", [64, 1, 1]),
            ("resolve", [64, 1, 1])
        ]
    );
}

fn mesh() -> Mesh {
    let a = [0., 0.];
    let b = [1., 0.];
    let mut vertices = Vec::new();
    for offset in [0., 10., 20.] {
        for (positions, uvs) in [
            ([a, b, [0., 1.]], [a, b, [0., 1.]]),
            ([b, a, [0., -1.]], [b, a, [0., -1.]]),
            ([a, b, [2., 1.]], [a, b, [2., 1.]]),
            ([b, a, [2., -1.]], [b, a, [2., 1.]]),
            ([a, [-1., 0.], [-1., -1.]], [a, [-1., 0.], [-1., -1.]]),
            ([a, b, [3., 1.]], [a, b, [3., 0.]]),
            ([b, a, [3., -1.]], [b, a, [3., -1.]]),
            ([a, b, a], [a, b, [2., 0.]]),
            ([b, a, [4., 0.]], [b, a, [4., -1.]]),
        ] {
            vertices.extend(positions.into_iter().zip(uvs).map(|(p, uv)| Vertex {
                position: [p[0] + offset, p[1], 0.],
                normal: [0., 0., 1.],
                uv,
            }));
        }
    }
    let uv = vertices.iter().map(|v| v.uv).collect();
    Mesh::new(vertices, (0..81).collect())
        .with_uv_set(2, uv)
        .unwrap()
}

fn read(output: &GpuTangentAdjacencyOutput) -> Result<Vec<GpuTangentEdge>> {
    let context = output.weld().derivatives().context();
    let staging = context.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("adjacency verification"),
        size: output.buffer().size(),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = context.device.create_command_encoder(&Default::default());
    encoder.copy_buffer_to_buffer(output.buffer(), 0, &staging, 0, output.buffer().size());
    let submission = context.queue.submit(Some(encoder.finish()));
    let (sender, receiver) = std::sync::mpsc::channel();
    staging
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |value| {
            let _ = sender.send(value);
        });
    context.device.poll(wgpu::PollType::Wait {
        submission_index: Some(submission),
        timeout: Some(std::time::Duration::from_secs(30)),
    })?;
    receiver.recv_timeout(std::time::Duration::from_secs(1))??;
    let mapped = staging.slice(..).get_mapped_range()?;
    let values = mapped
        .chunks_exact(64)
        .map(bytemuck::pod_read_unaligned)
        .collect();
    drop(mapped);
    staging.unmap();
    Ok(values)
}

fn verify(edges: &[GpuTangentEdge], first_pairs: [(usize, usize); 3]) {
    assert_eq!(edges.len(), 81);
    for block in 0..3 {
        let start = block * 27;
        let pairs = if block == 0 {
            first_pairs
        } else {
            [(0, 3), (6, 9), (15, 18)]
        };
        let mut opposite = [u32::MAX; 27];
        for (a, b) in pairs {
            opposite[a] = (start + b) as u32;
            opposite[b] = (start + a) as u32;
        }
        for (index, edge) in edges[start..start + 27].iter().enumerate() {
            assert_eq!(edge.edge[0], (start + index) as u32);
            assert_eq!(edge.edge[3], opposite[index], "block {block} edge {index}");
            assert_eq!(edge.status, [0; 4]);
            assert_eq!(
                edge.adjacency[3],
                match index / 3 {
                    5 | 8 => 1,
                    7 => 2,
                    _ => 0,
                }
            );
            if opposite[index] != u32::MAX {
                let neighbor = opposite[index] as usize;
                assert_eq!(
                    edge.adjacency[0] as usize,
                    neighbor / 3 * 3 + (neighbor % 3 + 1) % 3
                );
                assert_eq!(edge.adjacency[1] as usize, neighbor);
                let other = edges[neighbor];
                assert_eq!(edge.edge[1], other.edge[2]);
                assert_eq!(edge.edge[2], other.edge[1]);
                assert_eq!(
                    edge.adjacency[2],
                    u32::from(
                        edge.adjacency[3] == 0
                            && other.adjacency[3] == 0
                            && edge.classification[2] == other.classification[2]
                    )
                );
            } else {
                assert_eq!(edge.adjacency[..3], [u32::MAX, u32::MAX, 0]);
            }
        }
    }
}

#[test]
#[ignore = "requires a compute-capable GPU"]
fn gpu_adjacency_preserves_nonmanifold_rank_mirrors_degeneracy_and_input_frames() -> Result<()> {
    let context = WgpuContext::new_headless()?;
    let limits = GpuDeformationLimits::default();
    let base = mesh();
    let derivatives = GpuTangentDerivatives::new(context.clone(), base.clone(), 0, limits)?;
    let weld = GpuTangentWeld::new(context.clone(), base.clone(), 0, limits)?;
    let adjacency = GpuTangentAdjacency::new(context.clone(), base.clone(), 0, limits)?;
    let input = GpuDeformationOutput::upload(context.clone(), base.clone(), limits)?;
    let corners = weld.evaluate(&derivatives.evaluate(&input)?)?;
    let initial = adjacency.evaluate(&corners)?;
    assert_eq!(initial.weld().buffer(), corners.buffer());
    assert_eq!(
        initial.weld().derivatives().input_buffer(),
        input.buffer().raw()
    );
    let mut delta = vec![[0.; 3]; base.vertex_count()];
    delta[3] = [0.25, 0., 0.];
    let morph = GpuMorph::new(
        context.clone(),
        MorphTargets::new(
            base.clone(),
            [MorphTarget {
                positions: Some(delta.into()),
                ..Default::default()
            }],
        )?,
        limits,
    )?;
    let moving =
        adjacency.evaluate(&weld.evaluate(&derivatives.evaluate(&morph.evaluate(&[1.])?)?)?)?;
    let foreign = GpuTangentAdjacency::new(context.clone(), mesh(), 0, limits)?;
    assert!(foreign.evaluate(&corners).is_err());
    let other_uv = GpuTangentAdjacency::new(context.clone(), base.clone(), 2, limits)?;
    assert!(other_uv.evaluate(&corners).is_err());
    let mut records = crate::geometry::gpu_deformation::pack_mesh(&base);
    records[1].normal = [0.; 4];
    let invalid = GpuDeformationOutput {
        context: context.clone(),
        base,
        buffer: context.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("adjacency failures"),
            contents: bytemuck::cast_slice(&records),
            usage: wgpu::BufferUsages::STORAGE,
        }),
    };
    let invalid = read(&adjacency.evaluate(&weld.evaluate(&derivatives.evaluate(&invalid)?)?)?)?;
    for edge in &invalid[..3] {
        assert_eq!(edge.status, [2, 0, 0, 0]);
        assert_eq!(edge.adjacency, [u32::MAX, u32::MAX, 0, 3]);
        assert_eq!(edge.edge[3], u32::MAX);
    }
    assert_eq!(invalid[6].edge[3], 3);
    assert_eq!(invalid[15].edge[3], 9);
    drop(adjacency);
    drop(weld);
    drop(derivatives);
    drop(morph);
    verify(&read(&initial)?, [(0, 3), (6, 9), (15, 18)]);
    verify(&read(&moving)?, [(0, 9), (6, 18), (15, 24)]);
    Ok(())
}
