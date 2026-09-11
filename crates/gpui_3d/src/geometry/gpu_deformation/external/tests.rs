use super::*;
use crate::{GpuDeformationBounds, GpuFlatNormals, Vertex};

const PRODUCER: &str = r#"
struct Vertex { position: vec4<f32>, normal: vec4<f32>, tangent: vec4<f32>, status: vec4<u32> }
@group(0) @binding(0) var<storage, read_write> vertices: array<Vertex>;
@compute @workgroup_size(1) fn deform() { vertices[1].position.z = 0.5; }
"#;

#[test]
fn external_buffer_admission_checks_layout_usage_and_both_payload_limits() {
    let module = wgpu::naga::front::wgsl::parse_str(PRODUCER).unwrap();
    wgpu::naga::valid::Validator::new(
        wgpu::naga::valid::ValidationFlags::all(),
        wgpu::naga::valid::Capabilities::empty(),
    )
    .validate(&module)
    .unwrap();
    let device = wgpu::Limits::default();
    let limits = GpuDeformationLimits::default();
    let usage = wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC;
    admit(&device, 3, 192, usage, usage, limits).unwrap();
    for (count, size) in [(0, 0), (usize::MAX, 192), (3, 191), (3, 193), (4, 192)] {
        assert!(admit(&device, count, size, usage, usage, limits).is_err());
    }
    let copy = wgpu::BufferUsages::COPY_SRC;
    assert!(admit(&device, 3, 192, copy, usage, limits).is_err());
    admit(&device, 3, 192, copy, copy, limits).unwrap();
    assert!(admit(&device, 3, 192, wgpu::BufferUsages::STORAGE, copy, limits).is_err());
    for (source, output) in [(191, 192), (192, 191)] {
        assert!(
            admit(
                &device,
                3,
                192,
                usage,
                usage,
                GpuDeformationLimits {
                    max_source_bytes: source,
                    max_output_bytes: output,
                }
            )
            .is_err()
        );
    }
    for limited in [
        wgpu::Limits {
            max_buffer_size: 191,
            ..device
        },
        wgpu::Limits {
            max_storage_buffer_binding_size: 191,
            ..device
        },
        wgpu::Limits {
            max_compute_workgroups_per_dimension: 0,
            ..device
        },
    ] {
        assert!(admit(&limited, 3, 192, usage, usage, limits).is_err());
    }
}

#[test]
#[ignore = "requires a compute-capable GPU"]
fn external_buffers_share_processing_and_copy_snapshots_survive_producer_reuse() -> Result<()> {
    let context = WgpuContext::new_headless()?;
    let limits = GpuDeformationLimits::default();
    let base = Mesh::new(
        vec![
            Vertex {
                position: [0., 0., 0.],
                normal: [0., 0., 1.],
                uv: [0., 0.],
            },
            Vertex {
                position: [1., 0., 0.],
                normal: [0., 0., 1.],
                uv: [1., 0.],
            },
            Vertex {
                position: [0., 1., 0.],
                normal: [0., 0., 1.],
                uv: [0., 1.],
            },
        ],
        vec![0, 1, 2],
    );
    let mut records = super::super::pack_mesh(&base);
    let usage =
        wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST;
    let buffer = context.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("external deformation"),
        contents: bytemuck::cast_slice(&records),
        usage,
    });
    let shader = context
        .device
        .create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("external producer"),
            source: wgpu::ShaderSource::Wgsl(PRODUCER.into()),
        });
    let pipeline = context
        .device
        .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("external producer"),
            layout: None,
            module: &shader,
            entry_point: Some("deform"),
            compilation_options: Default::default(),
            cache: None,
        });
    let group = context
        .device
        .create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("external producer"),
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: buffer.as_entire_binding(),
            }],
        });
    let mut encoder = context.device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_compute_pass(&Default::default());
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.dispatch_workgroups(1, 1, 1);
    }
    context.queue.submit(Some(encoder.finish()));
    let copied =
        GpuDeformationOutput::copy_from_buffer(context.clone(), base.clone(), &buffer, limits)?;
    records[1].position[2] = 1.0;
    context
        .queue
        .write_buffer(&buffer, 0, bytemuck::cast_slice(&records));
    context.queue.submit([]);
    let adopted =
        GpuDeformationOutput::from_buffer(context.clone(), base.clone(), buffer.clone(), limits)?;
    assert_eq!(adopted.buffer().raw(), buffer.raw());
    assert_ne!(copied.buffer().raw(), buffer.raw());
    adopted.buffer().check_device(&context.device)?;
    copied.buffer().check_device(&context.device)?;
    let old = copied.readback()?;
    let new = adopted.readback()?;
    assert_eq!(old.vertices()[1].position[2], 0.5);
    assert_eq!(new.vertices()[1].position[2], 1.0);

    let normals = GpuFlatNormals::new(context.clone(), base.clone(), limits)?;
    let rebuilt = normals.evaluate(&adopted)?;
    let reduced = GpuDeformationBounds::new(context.clone())?;
    let mut bounds = reduced.request(&rebuilt, None)?;
    let mesh = rebuilt.readback()?;
    assert!(
        mesh.vertices()
            .iter()
            .all(|vertex| vertex.normal[0] < -0.6 && vertex.normal[2] > 0.6)
    );
    context.device.poll(wgpu::PollType::Wait {
        submission_index: None,
        timeout: Some(std::time::Duration::from_secs(10)),
    })?;
    assert_eq!(bounds.try_read()?.unwrap(), mesh.bounds());
    let source = rebuilt.render_source([0; 5], None)?;
    let packed = rebuilt.render_geometry(&source)?;
    assert!(std::sync::Arc::ptr_eq(packed.base_mesh(), &base.0));

    let foreign = WgpuContext::new_headless()?;
    assert!(
        GpuDeformationOutput::from_buffer(foreign.clone(), base.clone(), buffer.clone(), limits)
            .err()
            .unwrap()
            .to_string()
            .contains("different device")
    );
    assert!(
        GpuDeformationOutput::copy_from_buffer(foreign, base.clone(), &buffer, limits)
            .err()
            .unwrap()
            .to_string()
            .contains("different device")
    );
    records[0].status[0] = 7;
    let invalid = context.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("invalid deformation"),
        contents: bytemuck::cast_slice(&records),
        usage,
    });
    let invalid = GpuDeformationOutput::from_buffer(context.clone(), base, invalid, limits)?;
    assert!(invalid.readback().is_err());
    let mut invalid_bounds = reduced.request(&invalid, None)?;
    context.device.poll(wgpu::PollType::Wait {
        submission_index: None,
        timeout: Some(std::time::Duration::from_secs(10)),
    })?;
    assert!(invalid_bounds.try_read().is_err());
    drop((adopted, invalid, buffer));
    assert_eq!(copied.readback()?.vertices()[1].position[2], 0.5);
    Ok(())
}
