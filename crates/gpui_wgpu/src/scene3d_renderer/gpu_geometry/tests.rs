use super::*;

#[test]
fn packing_preflight_separates_adapter_limits_enabled_limits_and_indirect_support() {
    let adapter = wgpu::Limits::default();
    let mut device = wgpu::Limits::downlevel_defaults();
    let flags = wgpu::DownlevelFlags::all();
    let error = validate_support(&device, &adapter, flags)
        .unwrap_err()
        .to_string();
    assert!(error.contains("max_storage_buffers_per_shader_stage >= 5"));
    assert!(error.contains("device enabled 4"));
    assert!(error.contains(&format!(
        "adapter supports {}",
        adapter.max_storage_buffers_per_shader_stage
    )));
    device.max_storage_buffers_per_shader_stage = 5;
    validate_support(&device, &adapter, flags).unwrap();
    for flag in [
        wgpu::DownlevelFlags::COMPUTE_SHADERS,
        wgpu::DownlevelFlags::INDIRECT_EXECUTION,
    ] {
        let error = validate_support(&device, &adapter, flags - flag)
            .unwrap_err()
            .to_string();
        assert!(error.contains(&format!("{flag:?}")));
    }
    device.max_bindings_per_bind_group = 4;
    assert!(
        validate_support(&device, &adapter, flags)
            .unwrap_err()
            .to_string()
            .contains("max_bindings_per_bind_group")
    );
}

pub(super) fn mesh() -> Arc<Mesh3d> {
    let mesh = Mesh3d::new(
        vec![
            gpui::MeshVertex3d {
                position: [-1., -1., 0.],
                normal: [0., 0., 1.],
                uv: [0., 0.],
            },
            gpui::MeshVertex3d {
                position: [1., -1., 0.],
                normal: [0., 0., 1.],
                uv: [1., 0.],
            },
            gpui::MeshVertex3d {
                position: [0., 1., 0.],
                normal: [0., 0., 1.],
                uv: [0.5, 1.],
            },
        ],
        vec![0, 1, 2],
    );
    mesh.with_uv_set(2, vec![[0.3, 0.7], [0.6, 0.8], [0.9, 0.4]])
        .unwrap()
        .with_tangents(vec![[1., 0., 0., 1.]; 3])
        .unwrap()
        .with_vertex_colors(vec![[0.2, 0.4, 0.8, 0.6]; 3])
        .unwrap()
}

#[test]
fn gpu_geometry_shader_offsets_match_the_render_vertex_layout() {
    use wgpu::naga::{
        Expression, Literal,
        front::wgsl,
        valid::{Capabilities, ValidationFlags, Validator},
    };
    let module = wgsl::parse_str(include_str!("../gpu_geometry.wgsl")).unwrap();
    Validator::new(ValidationFlags::all(), Capabilities::empty())
        .validate(&module)
        .unwrap();
    let constant = |name| {
        let constant = module
            .constants
            .iter()
            .find(|(_, value)| value.name.as_deref() == Some(name))
            .unwrap()
            .1;
        let Expression::Literal(Literal::U32(value)) = module.global_expressions[constant.init]
        else {
            panic!("expected u32 layout constant")
        };
        u64::from(value) * 4
    };
    let layout = Scene3dGpuGeometry::vertex_layout();
    assert_eq!(layout.array_stride, constant("VERTEX_WORDS"));
    assert_eq!(layout.array_stride as usize, std::mem::size_of::<Vertex>());
    for (location, constant_name) in [(1, "NORMAL_WORD"), (3, "TANGENT_WORD")] {
        assert_eq!(
            layout
                .attributes
                .iter()
                .find(|a| a.shader_location == location)
                .unwrap()
                .offset,
            constant(constant_name)
        );
    }
    assert_eq!(
        layout
            .attributes
            .iter()
            .find(|a| a.shader_location == 0)
            .unwrap()
            .offset,
        0
    );
    let inputs: Vec<_> = module
        .global_variables
        .iter()
        .filter_map(|(_, v)| v.binding.as_ref().map(|b| (b.group, b.binding)))
        .collect();
    assert_eq!(inputs, [(0, 0), (0, 1), (0, 2), (0, 3), (0, 4)]);
    assert_eq!(module.entry_points[0].workgroup_size, [64, 1, 1]);
}

#[test]
fn gpu_geometry_admission_includes_material_inputs_indices_and_draw_arguments() {
    let memory = Scene3dGpuGeometryMemory::plan(90, 432).unwrap();
    let limits = wgpu::Limits::default();
    assert!(
        memory
            .validate(&limits, 90, 432, Some(memory.total_bytes))
            .is_ok()
    );
    assert!(
        memory
            .validate(&limits, 90, 432, Some(memory.total_bytes - 1))
            .is_err()
    );
    assert_eq!(memory.total_bytes, 90 * 96 * 2 + 432 * 4 + 32);
    assert!(
        memory
            .validate(
                &wgpu::Limits {
                    max_compute_workgroups_per_dimension: 1,
                    ..limits
                },
                90,
                432,
                None
            )
            .is_err()
    );
    assert!(
        memory
            .validate(
                &wgpu::Limits {
                    max_storage_buffer_binding_size: memory.vertex_bytes - 1,
                    ..limits
                },
                90,
                432,
                None
            )
            .is_err()
    );
    for (vertices, indices) in [(0, 3), (3, 0), (3, 4), (usize::MAX, 3), (3, usize::MAX)] {
        assert!(Scene3dGpuGeometryMemory::plan(vertices, indices).is_err());
    }
}

pub(super) fn read(context: &WgpuContext, buffer: &wgpu::Buffer) -> Vec<u32> {
    let staging = context.device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: buffer.size(),
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = context.device.create_command_encoder(&Default::default());
    encoder.copy_buffer_to_buffer(buffer, 0, &staging, 0, buffer.size());
    let submission = context.queue.submit([encoder.finish()]);
    let (send, receive) = std::sync::mpsc::channel();
    staging
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |result| {
            send.send(result).unwrap();
        });
    context
        .device
        .poll(wgpu::PollType::Wait {
            submission_index: Some(submission),
            timeout: Some(std::time::Duration::from_secs(30)),
        })
        .unwrap();
    receive
        .recv_timeout(std::time::Duration::from_secs(1))
        .unwrap()
        .unwrap();
    let mapped = staging.slice(..).get_mapped_range().unwrap();
    let result = mapped
        .chunks_exact(4)
        .map(|v| u32::from_ne_bytes(v.try_into().unwrap()))
        .collect();
    drop(mapped);
    staging.unmap();
    result
}

#[test]
#[ignore = "requires a compute-capable GPU"]
fn source_rebinding_shares_kernels_and_topology_without_inheriting_stream_updates() {
    let context = WgpuContext::new_headless().unwrap();
    let mesh = mesh();
    let source = WgpuScene3dGeometry::new(context.clone(), mesh.clone(), [0; 5], None).unwrap();
    let updated = source
        .with_attributes(&[Scene3dVertexUpdate::Color(&[[1., 0., 0., 1.]; 3])], None)
        .unwrap();
    let old_values = read(&context, &updated.source);
    let replacement = mesh.with_uv_set(2, vec![[0.1, 0.9]; 3]).unwrap();
    let rebound = updated
        .with_mesh(replacement.clone(), [2; 5], None)
        .unwrap();
    assert!(Arc::ptr_eq(rebound.base_mesh(), &replacement));
    assert_eq!(rebound.pipeline, source.pipeline);
    assert_eq!(rebound.layout, source.layout);
    assert_eq!(rebound.indices, source.indices);
    assert!(Arc::ptr_eq(
        &rebound.attribute_kernel,
        &updated.attribute_kernel
    ));
    assert_ne!(rebound.source, updated.source);
    let expected: Vec<_> = (0..3)
        .map(|i| Vertex::new(&replacement, i, [2; 5]))
        .collect();
    assert_eq!(
        read(&context, &rebound.source),
        bytemuck::cast_slice::<_, u32>(&expected)
    );
    assert_ne!(read(&context, &rebound.source), old_values);
    assert_eq!(read(&context, &updated.source), old_values);
    assert!(
        updated
            .with_mesh(replacement.clone(), [9; 5], None)
            .is_err()
    );
    assert!(
        updated
            .with_mesh(replacement, [2; 5], Some(rebound.memory.total_bytes - 1))
            .is_err()
    );

    let reindexed = Mesh3d::new(mesh.vertices().to_vec(), vec![0, 2, 1]);
    let different = rebound.with_mesh(reindexed, [0; 5], None).unwrap();
    assert_eq!(different.pipeline, rebound.pipeline);
    assert_ne!(different.indices, rebound.indices);
    drop(source);
    drop(updated);
    drop(rebound);
    assert_eq!(read(&context, &different.indices), [0, 2, 1]);
}

#[test]
#[ignore = "requires a compute-capable GPU"]
fn gpu_geometry_preserves_material_attributes_and_gates_invalid_draws() {
    let context = WgpuContext::new_headless().unwrap();
    let mesh = mesh();
    let sets = [2, 0, 2, 0, 2];
    let geometry =
        WgpuScene3dGeometry::new(context.clone(), mesh.clone(), sets, Some(4096)).unwrap();
    let mut attributes: Vec<[u32; 16]> = mesh
        .vertices()
        .iter()
        .map(|v| {
            let mut record = [0; 16];
            for axis in 0..3 {
                record[axis] = (v.position[axis] + 0.25).to_bits();
                record[4 + axis] = v.normal[axis].to_bits();
            }
            record[8] = 1_f32.to_bits();
            record[11] = 1_f32.to_bits();
            record
        })
        .collect();
    let evaluate = |records: &[[u32; 16]]| {
        let buffer = context
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: None,
                contents: bytemuck::cast_slice(records),
                usage: wgpu::BufferUsages::STORAGE,
            });
        geometry.evaluate(&buffer).unwrap()
    };
    let valid = evaluate(&attributes);
    let mut expected: Vec<_> = (0..3)
        .map(|index| Vertex::new(&mesh, index, sets))
        .collect();
    let expected_words: &mut [u32] = bytemuck::cast_slice_mut(&mut expected);
    for index in 0..3 {
        expected_words[index * 24..index * 24 + 3].copy_from_slice(&attributes[index][..3]);
    }
    assert_eq!(read(&context, valid.vertices()), expected_words);
    assert_eq!(&read(&context, valid.draw())[..5], [3, 1, 0, 0, 0]);
    attributes[1][12] = 3;
    let invalid = evaluate(&attributes);
    attributes[1][12] = 0;
    attributes[1][11] = (-1_f32).to_bits();
    let mixed_sign = evaluate(&attributes);
    drop(geometry);
    assert_eq!(&read(&context, invalid.draw())[..5], [3, 0, 0, 0, 0]);
    assert_eq!(&read(&context, mixed_sign.draw())[..5], [3, 0, 0, 0, 0]);
    assert_eq!(&read(&context, valid.draw())[..5], [3, 1, 0, 0, 0]);
    assert!(valid.request_status(Some(31)).is_err());
    let mut requests = [
        valid.request_status(Some(32)).unwrap(),
        invalid.request_status(None).unwrap(),
        mixed_sign.request_status(None).unwrap(),
    ];
    drop(valid);
    drop(invalid);
    drop(mixed_sign);
    context
        .device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_secs(30)),
        })
        .unwrap();
    let [clean, failed, tangent] = requests
        .each_mut()
        .map(|request| request.try_read().unwrap().unwrap());
    assert!(clean.is_drawable());
    assert_eq!(failed.issues, Scene3dGeometryIssues::DEFORMATION_STATUS);
    assert_eq!(failed.first_invalid_vertex, Some(1));
    assert_eq!(tangent.issues, Scene3dGeometryIssues::TRIANGLE_TANGENT_SIGN);
    assert_eq!(tangent.first_invalid_triangle, Some(0));
    assert!(
        requests
            .iter_mut()
            .all(|request| request.try_read().is_err())
    );
}
