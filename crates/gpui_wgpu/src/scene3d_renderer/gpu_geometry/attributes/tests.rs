use super::*;

#[test]
fn sparse_attribute_upload_shares_selected_uv_slots_and_admits_peak_payload() {
    let coordinates = [[0.25, -1.], [2., 0.75], [1., 0.]];
    let colors = [[0.1, 0.2, 0.3, 0.4]; 3];
    let updates = [
        Scene3dVertexUpdate::Uv {
            set: 2,
            coordinates: &coordinates,
        },
        Scene3dVertexUpdate::Color(&colors),
    ];
    let limits = wgpu::Limits::default();
    let upload_bytes = 32 + 3 * (8 + 16);
    let working_bytes = 3 * 96 + upload_bytes;
    let plan =
        AttributePlan::new(3, [2, 0, 2, 0, 2], &updates, &limits, Some(working_bytes)).unwrap();
    assert_eq!(plan.upload_bytes, upload_bytes);
    assert_eq!(plan.header, [3, 8, 0, 8, 0, 8, 14, 0]);
    assert!(
        AttributePlan::new(
            3,
            [2, 0, 2, 0, 2],
            &updates,
            &limits,
            Some(working_bytes - 1)
        )
        .is_err()
    );
    let limits = wgpu::Limits {
        max_storage_buffer_binding_size: 287,
        ..limits
    };
    assert!(AttributePlan::new(3, [2; 5], &updates, &limits, None).is_err());
    let empty = AttributePlan::new(3, [0; 5], &[], &limits, Some(0)).unwrap();
    assert_eq!(empty.upload_bytes, 0);
}

#[test]
fn external_attribute_buffers_require_exact_copyable_records() {
    let copy = wgpu::BufferUsages::COPY_SRC;
    for bytes in [24, 48] {
        validate_buffer(bytes, bytes, copy).unwrap();
        for actual in [0, bytes - 4, bytes + 4] {
            assert!(validate_buffer(bytes, actual, copy).is_err());
        }
        assert!(validate_buffer(bytes, bytes, wgpu::BufferUsages::STORAGE).is_err());
    }
}

#[test]
fn attribute_admission_rejects_ambiguous_streams_invalid_values_and_dispatch_overflow() {
    let uv = [[0., 0.]; 3];
    let color = [[1.; 4]; 3];
    let valid_uv = Scene3dVertexUpdate::Uv {
        set: 0,
        coordinates: &uv,
    };
    let valid_color = Scene3dVertexUpdate::Color(&color);
    let limits = wgpu::Limits::default();
    for updates in [
        vec![valid_uv, valid_uv],
        vec![valid_color, valid_color],
        vec![Scene3dVertexUpdate::Uv {
            set: 1,
            coordinates: &uv,
        }],
        vec![Scene3dVertexUpdate::Uv {
            set: 0,
            coordinates: &uv[..2],
        }],
        vec![Scene3dVertexUpdate::Color(&color[..2])],
    ] {
        assert!(AttributePlan::new(3, [0; 5], &updates, &limits, None).is_err());
    }
    for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        let uv = [[value, 0.]; 3];
        let update = Scene3dVertexUpdate::Uv {
            set: 0,
            coordinates: &uv,
        };
        assert!(AttributePlan::new(3, [0; 5], &[update], &limits, None).is_err());
    }
    for value in [f32::NAN, f32::INFINITY, -0.1, 1.1] {
        let colors = [[1., 1., 1., value]; 3];
        assert!(
            AttributePlan::new(
                3,
                [0; 5],
                &[Scene3dVertexUpdate::Color(&colors)],
                &limits,
                None
            )
            .is_err()
        );
    }
    let colors = [[1.; 4]; 65];
    let updates = [Scene3dVertexUpdate::Color(&colors)];
    let limits = wgpu::Limits {
        max_compute_workgroups_per_dimension: 1,
        ..limits
    };
    assert!(AttributePlan::new(65, [0; 5], &updates, &limits, None).is_err());
}

#[test]
fn attribute_shader_offsets_match_render_vertex_attributes() {
    use wgpu::naga::{
        Expression, Literal,
        front::wgsl,
        valid::{Capabilities, ValidationFlags, Validator},
    };
    for source in [
        include_str!("../attributes.wgsl"),
        include_str!("../../gpu_geometry.wgsl"),
    ] {
        let module = wgsl::parse_str(source).unwrap();
        Validator::new(ValidationFlags::all(), Capabilities::empty())
            .validate(&module)
            .unwrap();
        let constant = |name| {
            module
                .constants
                .iter()
                .find(|(_, c)| c.name.as_deref() == Some(name))
                .unwrap()
                .1
                .init
        };
        let value = |handle| {
            let Expression::Literal(Literal::U32(v)) = module.global_expressions[handle] else {
                panic!("expected u32");
            };
            u64::from(v) * 4
        };
        let layout = Vertex::layout();
        let offset = |location| {
            layout
                .attributes
                .iter()
                .find(|a| a.shader_location == location)
                .unwrap()
                .offset
        };
        assert_eq!(value(constant("VERTEX_WORDS")), layout.array_stride);
        assert_eq!(value(constant("COLOR_WORD")), offset(11));
        let Expression::Compose { components, .. } =
            &module.global_expressions[constant("UV_WORDS")]
        else {
            panic!("expected UV offsets");
        };
        let uv_offsets: Vec<_> = components.iter().map(|&handle| value(handle)).collect();
        assert_eq!(
            uv_offsets,
            [
                offset(2),
                offset(2) + 8,
                offset(14),
                offset(14) + 8,
                offset(15)
            ]
        );
    }
}

#[test]
#[ignore = "requires a compute-capable GPU"]
fn gpu_attribute_versions_preserve_geometry_and_share_packing_resources() {
    use super::super::tests::{mesh, read};
    let context = WgpuContext::new_headless().unwrap();
    let mesh = mesh();
    let sets = [2, 0, 2, 0, 2];
    let source = WgpuScene3dGeometry::new(context.clone(), mesh.clone(), sets, None).unwrap();
    let empty = source.with_attributes(&[], Some(0)).unwrap();
    assert_eq!(empty.source, source.source);
    assert!(source.attribute_kernel.lock().is_none());
    let coordinates = [[2., -1.], [0.5, 0.25], [3., 4.]];
    let colors = [[1., 0., 0., 0.]; 3];
    let updated = source
        .with_attributes(
            &[
                Scene3dVertexUpdate::Uv {
                    set: 2,
                    coordinates: &coordinates,
                },
                Scene3dVertexUpdate::Color(&colors),
            ],
            None,
        )
        .unwrap();
    assert_eq!(updated.indices, source.indices);
    assert_eq!(updated.pipeline, source.pipeline);
    assert!(Arc::ptr_eq(&updated.mesh, &source.mesh));
    assert!(Arc::ptr_eq(
        &updated.attribute_kernel,
        &source.attribute_kernel
    ));
    let next_colors = [[0.25, 0.5, 0.75, 1.]; 3];
    let next = updated
        .with_attributes(&[Scene3dVertexUpdate::Color(&next_colors)], None)
        .unwrap();
    let uv_buffer = context.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: None,
        contents: bytemuck::cast_slice(&coordinates),
        usage: wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
    });
    let color_buffer = context.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: None,
        contents: bytemuck::cast_slice(&next_colors),
        usage: wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
    });
    let external = source
        .with_attributes(
            &[
                Scene3dVertexUpdate::UvBuffer {
                    set: 2,
                    buffer: &uv_buffer,
                },
                Scene3dVertexUpdate::Color(&colors),
            ],
            None,
        )
        .unwrap()
        .with_attributes(&[Scene3dVertexUpdate::ColorBuffer(&color_buffer)], None)
        .unwrap();
    assert!(
        source
            .with_attributes(
                &[
                    Scene3dVertexUpdate::UvBuffer {
                        set: 2,
                        buffer: &uv_buffer
                    },
                    Scene3dVertexUpdate::Uv {
                        set: 2,
                        coordinates: &coordinates
                    },
                ],
                None
            )
            .is_err()
    );
    context
        .queue
        .write_buffer(&uv_buffer, 0, bytemuck::cast_slice(&[[f32::NAN, 0.]; 3]));
    context.queue.write_buffer(
        &color_buffer,
        0,
        bytemuck::cast_slice(&[[1., 1., 1., 2.]; 3]),
    );
    assert_eq!(
        read(&context, &external.source),
        read(&context, &next.source)
    );
    let invalid = updated.with_attributes(&[Scene3dVertexUpdate::Color(&[])], None);
    assert!(invalid.is_err());
    let base: Vec<_> = (0..3)
        .map(|index| Vertex::new(&mesh, index, sets))
        .collect();
    assert_eq!(
        read(&context, &source.source),
        bytemuck::cast_slice::<_, u32>(&base)
    );
    let expected_mesh = mesh
        .with_uv_set(2, coordinates.to_vec())
        .unwrap()
        .with_vertex_colors(colors.to_vec())
        .unwrap();
    let expected: Vec<_> = (0..3)
        .map(|index| Vertex::new(&expected_mesh, index, sets))
        .collect();
    assert_eq!(
        read(&context, &updated.source),
        bytemuck::cast_slice::<_, u32>(&expected)
    );
    let attributes: Vec<[u32; 16]> = mesh
        .vertices()
        .iter()
        .map(|v| {
            let mut record = [0; 16];
            for axis in 0..3 {
                record[axis] = v.position[axis].to_bits();
                record[4 + axis] = v.normal[axis].to_bits();
            }
            record[8] = 1_f32.to_bits();
            record[11] = 1_f32.to_bits();
            record
        })
        .collect();
    let buffer = context
        .device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::cast_slice(&attributes),
            usage: wgpu::BufferUsages::STORAGE,
        });
    let packed = next.evaluate(&buffer).unwrap();
    let external_packed = external.evaluate(&buffer).unwrap();
    for update in [
        Scene3dVertexUpdate::UvBuffer {
            set: 2,
            buffer: &uv_buffer,
        },
        Scene3dVertexUpdate::ColorBuffer(&color_buffer),
    ] {
        let invalid = external.with_attributes(&[update], None).unwrap();
        let rejected = read(&context, invalid.evaluate(&buffer).unwrap().draw());
        assert_eq!(&rejected[..5], [3, 0, 0, 0, 0]);
        let issue = match update {
            Scene3dVertexUpdate::UvBuffer { .. } => Scene3dGeometryIssues::INVALID_UV,
            Scene3dVertexUpdate::ColorBuffer(_) => Scene3dGeometryIssues::INVALID_COLOR,
            _ => unreachable!(),
        };
        assert_eq!(&rejected[5..], [issue.bits(), 0, u32::MAX]);
        let repaired = invalid
            .with_attributes(
                &[
                    Scene3dVertexUpdate::Uv {
                        set: 2,
                        coordinates: &coordinates,
                    },
                    Scene3dVertexUpdate::Color(&next_colors),
                ],
                None,
            )
            .unwrap()
            .evaluate(&buffer)
            .unwrap();
        assert_eq!(&read(&context, repaired.draw())[..5], [3, 1, 0, 0, 0]);
        assert_eq!(
            read(&context, repaired.vertices()),
            read(&context, external_packed.vertices())
        );
    }
    let foreign = WgpuContext::new_headless()
        .unwrap()
        .create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: 24,
            usage: wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
    assert!(
        source
            .with_attributes(
                &[Scene3dVertexUpdate::UvBuffer {
                    set: 2,
                    buffer: &foreign
                },],
                None
            )
            .is_err()
    );
    let expected_mesh = expected_mesh
        .with_vertex_colors(next_colors.to_vec())
        .unwrap();
    let expected: Vec<_> = (0..3)
        .map(|index| Vertex::new(&expected_mesh, index, sets))
        .collect();
    drop(next);
    drop(updated);
    drop(source);
    drop(external);
    drop(uv_buffer);
    drop(color_buffer);
    assert_eq!(
        read(&context, packed.vertices()),
        bytemuck::cast_slice::<_, u32>(&expected)
    );
    assert_eq!(&read(&context, packed.draw())[..5], [3, 1, 0, 0, 0]);
    assert_eq!(
        read(&context, external_packed.vertices()),
        read(&context, packed.vertices())
    );
    assert_eq!(
        &read(&context, external_packed.draw())[..5],
        [3, 1, 0, 0, 0]
    );
}
