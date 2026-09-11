use super::*;
use Scene3dVertexStreamValue::{Bytes, CopiedBuffer, SharedBuffer};

fn declarations() -> Vec<Scene3dVertexAttribute> {
    vec![
        Scene3dVertexAttribute::new("direction", wgpu::VertexFormat::Float32x3),
        Scene3dVertexAttribute::new("tag", wgpu::VertexFormat::Uint32),
    ]
}

#[test]
fn stream_admission_preserves_order_packed_stride_and_full_snapshot_budget() {
    let directions = [[0.25_f32, -1., 2.]; 3];
    let tags = [u32::MAX, 0, 7];
    let values = [
        ("tag", Bytes(bytemuck::cast_slice(&tags))),
        ("direction", Bytes(bytemuck::cast_slice(&directions))),
    ];
    let declarations = declarations();
    let limits = wgpu::Limits::default();
    let plan = StreamPlan::new(&declarations, 3, &values, false, &limits, 48).unwrap();
    assert_eq!(plan.mapping, [Some(1), Some(0)]);
    assert_eq!(plan.payload_bytes, 48);
    let partial = StreamPlan::new(&declarations, 3, &values[..1], true, &limits, 48).unwrap();
    assert_eq!(partial.mapping, [None, Some(0)]);
    assert!(StreamPlan::new(&declarations, 3, &values[..1], true, &limits, 47).is_err());
    assert!(StreamPlan::new(&declarations, 3, &[], true, &limits, 47).is_err());
    assert!(StreamPlan::new(&declarations, 3, &values[..1], false, &limits, 48).is_err());
    assert!(
        StreamPlan::new(
            &declarations,
            3,
            &[("direction", Bytes(&[0; 48])), values[0]],
            false,
            &limits,
            64
        )
        .is_err()
    );
    assert!(StreamPlan::new(&declarations, 3, &[values[0], values[0]], true, &limits, 48).is_err());
    assert!(
        StreamPlan::new(
            &declarations,
            3,
            &[("unknown", values[0].1)],
            true,
            &limits,
            48
        )
        .is_err()
    );
    let small = wgpu::Limits {
        max_storage_buffer_binding_size: 35,
        ..limits
    };
    assert!(StreamPlan::new(&declarations, 3, &values, false, &small, 48).is_err());
}

#[test]
fn stream_admission_rejects_nonfinite_floats_and_invalid_addressing() {
    let declarations = declarations();
    let limits = wgpu::Limits::default();
    for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        let vector = [0_f32, value, 1.];
        assert!(
            StreamPlan::new(
                &declarations,
                1,
                &[("direction", Bytes(bytemuck::cast_slice(&vector)))],
                true,
                &limits,
                16
            )
            .is_err()
        );
    }
    for count in [0, usize::MAX] {
        assert!(StreamPlan::new(&declarations, count, &[], true, &limits, u64::MAX).is_err());
    }
    assert!(StreamPlan::new(&[], 1, &[], false, &limits, 16).is_err());
}

#[test]
#[ignore = "requires a GPU adapter"]
fn stream_snapshots_retain_buffers_and_compose_with_material_updates() -> Result<()> {
    let context = WgpuContext::new_headless()?;
    let source = Scene3dMaterialSource::new(
        context.clone(),
        MaterialProgram::compile_with_attributes(
            crate::scene3d_material::DEFAULT,
            &declarations(),
        )?,
    )?;
    let directions = [[1_f32, 0., 0.]; 3];
    let tags = [1_u32, 2, 3];
    let original = source.bind_vertex_streams(
        3,
        &[
            ("direction", Bytes(bytemuck::cast_slice(&directions))),
            ("tag", Bytes(bytemuck::cast_slice(&tags))),
        ],
        48,
    )?;
    let tags = [4_u32, 5, 6];
    let changed = original.with_values(&[("tag", Bytes(bytemuck::cast_slice(&tags)))], 48)?;
    assert_eq!(original.0.buffers[0], changed.0.buffers[0]);
    assert_ne!(original.0.buffers[1], changed.0.buffers[1]);
    assert_ne!(original.bind_group(), changed.bind_group());
    assert!(Arc::ptr_eq(&original.0, &original.with_values(&[], 48)?.0));
    let material = source.bind([], Scene3dMaterialBindingLimits::default())?;
    assert!(material.validate_vertex_count(3).is_err());
    let attached = material.with_vertex_streams(original.clone())?;
    attached.validate_vertex_count(3)?;
    assert!(attached.validate_vertex_count(4).is_err());
    let updated = attached.with_vertex_streams(changed)?;
    assert_eq!(attached.bind_group(), updated.bind_group());
    let other_source = Scene3dMaterialSource::new(context, source.program().clone())?;
    let other = other_source.bind([], Scene3dMaterialBindingLimits::default())?;
    assert!(other.with_vertex_streams(original).is_err());
    drop(source);
    assert_eq!(attached.vertex_streams().unwrap().payload_bytes(), 48);
    Ok(())
}

#[test]
fn external_stream_admission_distinguishes_storage_from_copy_sources() {
    use wgpu::BufferUsages as Usage;
    for (copied, required) in [(false, Usage::STORAGE), (true, Usage::COPY_SRC)] {
        input::validate_buffer("direction", 36, 36, required, copied).unwrap();
        for size in [0, 32, 40, 48] {
            assert!(input::validate_buffer("direction", 36, size, required, copied).is_err());
        }
        assert!(input::validate_buffer("direction", 36, 36, Usage::COPY_DST, copied).is_err());
    }
    assert!(input::validate_buffer("tag", 12, 12, Usage::COPY_SRC, false).is_err());
    assert!(input::validate_buffer("tag", 12, 12, Usage::STORAGE, true).is_err());
}

#[test]
#[ignore = "requires a GPU adapter"]
fn external_streams_check_devices_and_share_only_declared_buffers() -> Result<()> {
    let context = WgpuContext::new_headless()?;
    let source = Scene3dMaterialSource::new(
        context.clone(),
        MaterialProgram::compile_with_attributes(
            crate::scene3d_material::DEFAULT,
            &declarations(),
        )?,
    )?;
    let direction = context.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: None,
        contents: bytemuck::cast_slice(&[[1_f32, 0., 0.]; 3]),
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
    });
    let tags = [1_u32, 2, 3];
    let shared = source.bind_vertex_streams(
        3,
        &[
            ("direction", SharedBuffer(&direction)),
            ("tag", Bytes(bytemuck::cast_slice(&tags))),
        ],
        48,
    )?;
    assert_eq!(&shared.0.buffers[0], direction.raw());
    let copied = shared.with_values(&[("direction", CopiedBuffer(&direction))], 48)?;
    assert_ne!(&copied.0.buffers[0], direction.raw());
    assert_eq!(copied.0.buffers[1], shared.0.buffers[1]);
    assert!(
        shared
            .with_values(&[("direction", SharedBuffer(&direction))], 47)
            .is_err()
    );
    let foreign = WgpuContext::new_headless()?;
    let buffer = foreign.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: None,
        contents: bytemuck::cast_slice(&[[1_f32, 0., 0.]; 3]),
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
    });
    for value in [SharedBuffer(&buffer), CopiedBuffer(&buffer)] {
        let error = shared
            .with_values(&[("direction", value)], 48)
            .err()
            .unwrap();
        assert!(error.to_string().contains("different device"), "{error}");
        let error = source
            .bind_vertex_streams(
                3,
                &[
                    ("direction", value),
                    ("tag", Bytes(bytemuck::cast_slice(&tags))),
                ],
                48,
            )
            .err()
            .unwrap();
        assert!(error.to_string().contains("different device"), "{error}");
    }
    Ok(())
}
