use super::*;

fn fan(segments: usize, closed: bool) -> Mesh {
    let center = Vertex {
        position: [0.; 3],
        normal: [0., 0., 1.],
        uv: [0.; 2],
    };
    let ring: Vec<_> = (0..segments)
        .map(|index| {
            let angle = index as f64 / segments as f64 * std::f64::consts::TAU;
            let uv = [angle.cos() as f32, angle.sin() as f32];
            Vertex {
                position: [uv[0], uv[1], 0.],
                uv,
                ..center
            }
        })
        .collect();
    let mut vertices = Vec::new();
    for slot in 0..segments {
        let face = slot * (segments - 2) % segments;
        if closed || face != segments - 1 {
            vertices.extend([center, ring[face], ring[(face + 1) % segments]]);
        }
    }
    let edge = Vertex {
        position: [2., 0., 0.],
        uv: [2., 0.],
        ..center
    };
    vertices.extend([
        center,
        edge,
        Vertex {
            position: [2., 1., 0.],
            uv: [2., 1.],
            ..center
        },
        edge,
        center,
        Vertex {
            position: [2., -1., 0.],
            uv: [2., 1.],
            ..center
        },
    ]);
    let indices = (0..vertices.len() as u32).collect();
    Mesh::new(vertices, indices)
}

fn expected(mesh: &Mesh) -> Vec<[u32; 2]> {
    let vertices: Vec<_> = mesh
        .indices()
        .iter()
        .map(|&index| mesh.vertices()[index as usize])
        .collect();
    let keys: Vec<Vec<_>> = vertices
        .iter()
        .map(|v| {
            v.position
                .into_iter()
                .chain(v.normal)
                .chain(v.uv)
                .map(f32::to_bits)
                .collect()
        })
        .collect();
    let orientation: Vec<_> = vertices
        .chunks_exact(3)
        .map(|face| {
            let u = [face[1].uv[0] - face[0].uv[0], face[1].uv[1] - face[0].uv[1]];
            let v = [face[2].uv[0] - face[0].uv[0], face[2].uv[1] - face[0].uv[1]];
            u32::from(u[0] * v[1] - u[1] * v[0] > 0.)
        })
        .collect();
    let next = |corner: usize| corner / 3 * 3 + (corner + 1) % 3;
    let mut links = vec![Vec::new(); vertices.len()];
    for a in 0..vertices.len() {
        for b in a + 1..vertices.len() {
            if orientation[a / 3] == orientation[b / 3]
                && keys[a] == keys[next(b)]
                && keys[next(a)] == keys[b]
            {
                links[a].push(next(b));
                links[next(b)].push(a);
                links[b].push(next(a));
                links[next(a)].push(b);
            }
        }
    }
    let mut result = vec![[u32::MAX; 2]; vertices.len()];
    for seed in 0..vertices.len() {
        if result[seed][0] != u32::MAX {
            continue;
        }
        let mut pending = vec![seed];
        while let Some(corner) = pending.pop() {
            if result[corner][0] != u32::MAX {
                continue;
            }
            result[corner] = [seed as u32, orientation[corner / 3]];
            pending.extend(&links[corner]);
        }
    }
    result
}

#[test]
#[ignore = "requires a compute-capable GPU with SHADER_F64"]
fn regular_groups_match_edge_components_across_workgroups_and_split_snapshots() -> Result<()> {
    let context = WgpuContext::new_headless()?;
    let limits = GpuDeformationLimits::default();
    let mut retained = Vec::new();
    for segments in [63, 65, 129, 257] {
        for closed in [false, true] {
            let base = fan(segments, closed);
            let derivatives = GpuTangentDerivatives::new(context.clone(), base.clone(), 0, limits)?;
            let weld = GpuTangentWeld::new(context.clone(), base.clone(), 0, limits)?;
            let adjacency = GpuTangentAdjacency::new(context.clone(), base.clone(), 0, limits)?;
            let groups = GpuTangentGroups::new(context.clone(), base.clone(), 0, limits)?;
            let producer = context.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: base.vertex_count() as u64 * 64,
                usage: wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            for split in [false, true, false] {
                let mut vertices = base.vertices().to_vec();
                if split {
                    let faces = vertices.len() / 3 - 2;
                    for (index, face) in vertices.chunks_exact_mut(3).take(faces).enumerate() {
                        if index % 5 == 0 {
                            face[0].position[2] = 0.25;
                            face[0].normal = [0., 1., 0.];
                        }
                    }
                }
                let mesh = base.with_vertices(vertices, None)?;
                let reference = expected(&mesh);
                let records = crate::geometry::gpu_deformation::pack_mesh(&mesh);
                context
                    .queue
                    .write_buffer(&producer, 0, bytemuck::cast_slice(&records));
                let input = GpuDeformationOutput::copy_from_buffer(
                    context.clone(),
                    base.clone(),
                    &producer,
                    limits,
                )?;
                context
                    .queue
                    .write_buffer(&producer, 0, &vec![0xff; producer.size() as usize]);
                let output = groups.evaluate(
                    &adjacency.evaluate(&weld.evaluate(&derivatives.evaluate(&input)?)?)?,
                )?;
                retained.push((segments, closed, split, output, reference));
            }
        }
    }
    for (segments, closed, split, output, expected) in retained.into_iter().rev() {
        let actual = read(&output)?;
        assert_eq!(actual.len(), expected.len());
        for (corner, (actual, expected)) in actual.iter().zip(expected).enumerate() {
            assert_eq!(actual.status, [0; 4]);
            assert_eq!(actual.neighbors[2], 0);
            assert_eq!(
                actual.identity[2..],
                expected,
                "segments {segments}, closed {closed}, split {split}, corner {corner}"
            );
        }
    }
    Ok(())
}
