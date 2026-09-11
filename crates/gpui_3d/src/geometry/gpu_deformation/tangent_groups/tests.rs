use super::*;
use crate::{
    GpuDeformationOutput, GpuMorph, GpuTangentAdjacency, GpuTangentDerivatives, GpuTangentWeld,
    MorphTarget, MorphTargets, Vertex,
};

#[test]
fn pass_bound_covers_bidirectional_chains_cycles_and_disconnected_vertices() {
    for count in [3usize, 6, 63, 66, 129, 258] {
        let memory = GpuTangentGroupsMemory::plan(count, GpuDeformationLimits::default()).unwrap();
        let mut order: Vec<_> = (0..count).rev().collect();
        order.rotate_left(count / 3);
        let mut links = vec![[usize::MAX; 2]; count];
        for (part, cycle) in [
            (&order[..count / 2], false),
            (&order[count / 2..count - 1], true),
        ] {
            for (index, &node) in part.iter().enumerate() {
                if cycle || index + 1 < part.len() {
                    links[node][0] = part[(index + 1) % part.len()];
                }
                if cycle || index > 0 {
                    links[node][1] = part[(index + part.len() - 1) % part.len()];
                }
            }
        }
        let mut state: Vec<_> = links
            .iter()
            .enumerate()
            .map(|(id, &links)| (links, id))
            .collect();
        for _ in 0..memory.propagation_passes {
            state = state
                .iter()
                .map(|&(jumps, mut label)| {
                    let mut next = jumps;
                    for direction in 0..2 {
                        if jumps[direction] != usize::MAX {
                            let other = state[jumps[direction]];
                            label = label.min(other.1);
                            next[direction] = other.0[direction];
                        }
                    }
                    (next, label)
                })
                .collect();
        }
        for (node, &(_, label)) in state.iter().enumerate() {
            let mut visited = vec![false; count];
            let mut stack = vec![node];
            let mut expected = node;
            while let Some(next) = stack.pop() {
                if next == usize::MAX || visited[next] {
                    continue;
                }
                visited[next] = true;
                expected = expected.min(next);
                stack.extend(links[next]);
            }
            assert_eq!(label, expected, "count {count}, corner {node}");
        }
        let limits = GpuDeformationLimits {
            max_source_bytes: memory.uniform_bytes,
            max_output_bytes: memory.scratch_bytes + memory.output_bytes,
        };
        assert_eq!(
            memory.output_bytes as usize,
            count * std::mem::size_of::<GpuTangentGroup>()
        );
        assert_eq!(
            memory.inheritance_flags_bytes,
            ((count as u64).div_ceil(64) * 4).max(64)
        );
        assert_eq!(
            memory.scratch_bytes,
            memory.output_bytes + memory.inheritance_flags_bytes
        );
        assert!(GpuTangentGroupsMemory::plan(count, limits).is_ok());
        assert!(
            GpuTangentGroupsMemory::plan(
                count,
                GpuDeformationLimits {
                    max_source_bytes: 15,
                    ..limits
                }
            )
            .is_err()
        );
        assert!(
            GpuTangentGroupsMemory::plan(
                count,
                GpuDeformationLimits {
                    max_output_bytes: limits.max_output_bytes - 1,
                    ..limits
                }
            )
            .is_err()
        );
    }
    for count in [0, 1, 4, usize::MAX] {
        assert!(GpuTangentGroupsMemory::plan(count, GpuDeformationLimits::default()).is_err());
    }
}

#[test]
fn inheritance_shaders_validate_and_match_group_scratch_layout() {
    use wgpu::naga::{
        TypeInner,
        front::wgsl,
        valid::{Capabilities, ValidationFlags, Validator},
    };
    for (source, flag_binding, names) in [
        (
            include_str!("../tangent_group_flags.wgsl"),
            3,
            "detect_inheritance",
        ),
        (include_str!("../tangent_group_inherit.wgsl"), 0, "inherit"),
    ] {
        let module = wgsl::parse_str(source).unwrap();
        Validator::new(ValidationFlags::all(), Capabilities::empty())
            .validate(&module)
            .unwrap();
        for (_, variable) in module.global_variables.iter() {
            let Some(binding) = &variable.binding else {
                continue;
            };
            assert_eq!(binding.group, 0);
            let ty = &module.types[variable.ty].inner;
            if binding.binding == 4 {
                assert!(matches!(ty, TypeInner::Struct { span: 16, .. }));
            } else {
                let TypeInner::Array { base, stride, .. } = ty else {
                    panic!("array required")
                };
                assert_eq!(
                    *stride,
                    if binding.binding == flag_binding {
                        4
                    } else {
                        64
                    }
                );
                if binding.binding == 3 && flag_binding == 0 {
                    let TypeInner::Struct { members, span: 64 } = &module.types[*base].inner else {
                        panic!("group scratch record required")
                    };
                    assert_eq!(
                        members
                            .iter()
                            .map(|v| v.offset as usize)
                            .collect::<Vec<_>>(),
                        [
                            std::mem::offset_of!(GpuTangentGroup, identity),
                            std::mem::offset_of!(GpuTangentGroup, neighbors),
                            std::mem::offset_of!(GpuTangentGroup, reserved),
                            std::mem::offset_of!(GpuTangentGroup, status),
                        ]
                    );
                }
            }
        }
        assert_eq!(module.entry_points.len(), 1);
        assert_eq!(module.entry_points[0].name, names);
        assert_eq!(module.entry_points[0].workgroup_size, [64, 1, 1]);
    }
}

#[test]
fn group_shader_layout_matches_adjacency_and_corner_records() {
    use wgpu::naga::{
        TypeInner,
        front::wgsl,
        valid::{Capabilities, ValidationFlags, Validator},
    };
    let module = wgsl::parse_str(include_str!("../tangent_groups.wgsl")).unwrap();
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
        panic!("expected group array")
    };
    let TypeInner::Struct { members, span } = &module.types[*base].inner else {
        panic!("expected group record")
    };
    assert_eq!(*span as usize, std::mem::size_of::<GpuTangentGroup>());
    assert_eq!(
        members
            .iter()
            .map(|m| m.offset as usize)
            .collect::<Vec<_>>(),
        [
            std::mem::offset_of!(GpuTangentGroup, identity),
            std::mem::offset_of!(GpuTangentGroup, neighbors),
            std::mem::offset_of!(GpuTangentGroup, reserved),
            std::mem::offset_of!(GpuTangentGroup, status)
        ]
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
            ("propagate", [64, 1, 1]),
            ("finalize", [64, 1, 1])
        ]
    );
}

const SEGMENTS: usize = 65;
fn mesh() -> Mesh {
    let mut vertices = vec![Vertex {
        position: [0.; 3],
        normal: [0., 0., 1.],
        uv: [0.; 2],
    }];
    for index in 0..SEGMENTS {
        let angle = index as f64 / SEGMENTS as f64 * std::f64::consts::TAU;
        let uv = [angle.cos() as f32, angle.sin() as f32];
        vertices.push(Vertex {
            position: [uv[0], uv[1], 0.],
            normal: [0., 0., 1.],
            uv,
        });
    }
    let mut indices = Vec::new();
    for index in 0..SEGMENTS {
        indices.extend([0, index as u32 + 1, ((index + 1) % SEGMENTS) as u32 + 1]);
    }
    let u = vertices.len() as u32;
    for (position, uv) in [
        ([2., 0., 0.], [2., 0.]),
        ([2., 1., 0.], [2., 1.]),
        ([2., -1., 0.], [2., 1.]),
        ([3., 0., 0.], [3., 0.]),
        ([3., 1., 0.], [4., 0.]),
    ] {
        vertices.push(Vertex {
            position,
            normal: [0., 0., 1.],
            uv,
        });
    }
    indices.extend([0, u, u + 1, u, 0, u + 2, 0, u + 3, u + 4, 0, u, u]);
    let uv = vertices.iter().map(|v| v.uv).collect();
    Mesh::new(vertices, indices).with_uv_set(2, uv).unwrap()
}

fn read(output: &GpuTangentGroupsOutput) -> Result<Vec<GpuTangentGroup>> {
    let context = output.adjacency().weld().derivatives().context();
    let staging = context.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("group verification"),
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

#[test]
#[ignore = "requires a compute-capable GPU"]
fn gpu_groups_cover_fan_cycles_mirror_boundaries_degeneracy_and_retained_deformation() -> Result<()>
{
    let context = WgpuContext::new_headless()?;
    let limits = GpuDeformationLimits::default();
    let base = mesh();
    let derivatives = GpuTangentDerivatives::new(context.clone(), base.clone(), 0, limits)?;
    let weld = GpuTangentWeld::new(context.clone(), base.clone(), 0, limits)?;
    let adjacency = GpuTangentAdjacency::new(context.clone(), base.clone(), 0, limits)?;
    let groups = GpuTangentGroups::new(context.clone(), base.clone(), 0, limits)?;
    let edges_for = |input: &GpuDeformationOutput| {
        adjacency.evaluate(&weld.evaluate(&derivatives.evaluate(input)?)?)
    };
    let input = GpuDeformationOutput::upload(context.clone(), base.clone(), limits)?;
    let edges = edges_for(&input)?;
    let initial = groups.evaluate(&edges)?;
    assert_eq!(initial.adjacency().buffer(), edges.buffer());
    assert_eq!(
        initial.adjacency().weld().derivatives().input_buffer(),
        input.buffer()
    );
    let mut delta = vec![[0.; 3]; base.vertex_count()];
    delta[1] = [-1., 0., 0.];
    let targets = MorphTargets::new(
        base.clone(),
        [MorphTarget {
            positions: Some(delta.into()),
            ..Default::default()
        }],
    )?;
    let morph = GpuMorph::new(context.clone(), targets, limits)?;
    let moving = groups.evaluate(&edges_for(&morph.evaluate(&[1.])?)?)?;
    assert!(
        GpuTangentGroups::new(context.clone(), mesh(), 0, limits)?
            .evaluate(&edges)
            .is_err()
    );
    assert!(
        GpuTangentGroups::new(context.clone(), base.clone(), 2, limits)?
            .evaluate(&edges)
            .is_err()
    );
    let mut records = crate::geometry::gpu_deformation::pack_mesh(&base);
    records[2].normal = [0.; 4];
    let invalid = GpuDeformationOutput {
        context: context.clone(),
        base,
        buffer: buffer(
            &context.device,
            "group failures",
            bytemuck::cast_slice(&records),
            wgpu::BufferUsages::STORAGE,
        ),
    };
    let invalid = read(&groups.evaluate(&edges_for(&invalid)?)?)?;
    for record in &invalid[..6] {
        assert_eq!(record.identity[2..], [u32::MAX; 2]);
        assert_eq!(record.neighbors, [u32::MAX, u32::MAX, 3, 0]);
        assert_eq!(record.status, [2, 0, 0, 0]);
    }
    for face in 2..SEGMENTS {
        assert_eq!(invalid[face * 3].identity[2], 6);
    }
    drop(groups);
    drop(adjacency);
    drop(weld);
    drop(derivatives);
    drop(morph);
    let initial = read(&initial)?;
    for face in 0..SEGMENTS {
        assert_eq!(initial[face * 3].identity[2], 0);
        let previous_corner = ((face + SEGMENTS - 1) % SEGMENTS) * 3 + 2;
        assert_eq!(
            initial[face * 3 + 1].identity[2] as usize,
            (face * 3 + 1).min(previous_corner)
        );
    }
    let extra = SEGMENTS * 3;
    for corner in extra..extra + 6 {
        assert_eq!(initial[corner].identity[2] as usize, corner);
    }
    for corner in extra + 6..extra + 12 {
        assert_eq!(initial[corner].identity[2..], [u32::MAX; 2]);
        assert_eq!(
            initial[corner].neighbors[2],
            if corner < extra + 9 { 1 } else { 2 }
        );
    }
    for (corner, record) in initial.iter().enumerate() {
        assert_eq!(record.identity[0] as usize, corner);
        assert_eq!(record.status, [0; 4]);
        assert_eq!(record.reserved, [0; 4]);
        for &neighbor in &record.neighbors[..2] {
            if neighbor != u32::MAX {
                assert_eq!(record.identity[2], initial[neighbor as usize].identity[2]);
                assert_eq!(record.identity[1], initial[neighbor as usize].identity[1]);
                assert_eq!(record.identity[3], initial[neighbor as usize].identity[3]);
            }
        }
    }
    let moving = read(&moving)?;
    for face in 0..SEGMENTS {
        if face == 0 || face == SEGMENTS - 1 {
            for record in &moving[face * 3..face * 3 + 3] {
                assert_eq!(record.identity[2..], [u32::MAX; 2]);
                assert_eq!(record.neighbors[2], 2);
            }
        } else {
            assert_eq!(moving[face * 3].identity[2], 3);
        }
    }
    Ok(())
}
