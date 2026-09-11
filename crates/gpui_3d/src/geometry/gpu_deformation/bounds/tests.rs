use super::*;
use crate::{GpuDeformationLimits, GpuMorph, Mesh, MorphTarget, MorphTargets, PlaneOptions};

#[test]
fn shader_validates_and_matches_deformation_record_layout() {
    use wgpu::naga::{
        TypeInner,
        front::wgsl,
        valid::{Capabilities, ValidationFlags, Validator},
    };
    let module = wgsl::parse_str(include_str!("../bounds.wgsl")).unwrap();
    Validator::new(ValidationFlags::all(), Capabilities::empty())
        .validate(&module)
        .unwrap();
    let vertex = module
        .types
        .iter()
        .find(|(_, ty)| ty.name.as_deref() == Some("Vertex"))
        .unwrap()
        .1;
    let TypeInner::Struct { members, span } = &vertex.inner else {
        panic!("vertex must be a struct")
    };
    use crate::GpuDeformationVertex;
    assert_eq!(*span as usize, std::mem::size_of::<GpuDeformationVertex>());
    assert_eq!(
        members
            .iter()
            .map(|member| member.offset as usize)
            .collect::<Vec<_>>(),
        [
            std::mem::offset_of!(GpuDeformationVertex, position),
            std::mem::offset_of!(GpuDeformationVertex, normal),
            std::mem::offset_of!(GpuDeformationVertex, tangent),
            std::mem::offset_of!(GpuDeformationVertex, status),
        ]
    );
    assert_eq!(module.entry_points[0].workgroup_size, [64, 1, 1]);
    let bounds = module
        .global_variables
        .iter()
        .find(|(_, v)| v.name.as_deref() == Some("bounds"))
        .unwrap()
        .1;
    let TypeInner::Array {
        size: wgpu::naga::ArraySize::Constant(size),
        stride,
        ..
    } = module.types[bounds.ty].inner
    else {
        panic!("bounds must be a fixed-size array")
    };
    assert_eq!(u64::from(size.get()) * u64::from(stride), RESULT_BYTES);
}

#[test]
fn bounds_decode_signed_extents_and_reject_invalid_results() {
    let encoded: [u32; 8] = [
        0x3fffffff, 0x7fffffff, 0xc0400000, 0, 0xbf800000, 0x80000000, 0xc0a00000, 0,
    ];
    let bounds = decode(bytemuck::cast_slice(&encoded)).unwrap();
    assert_eq!(bounds, Aabb::new([-2., -0., 3.], [1., 0., 5.]).unwrap());
    assert_eq!(bounds.min()[1].to_bits(), (-0_f32).to_bits());
    assert_eq!(bounds.max()[1].to_bits(), 0_f32.to_bits());
    assert!(decode(&[0; 31]).is_err());
    assert!(decode(&[0; 33]).is_err());
    assert!(decode(bytemuck::cast_slice(&INITIAL)).is_err());
    for (lane, value) in [
        (3, 1),
        (7, 2),
        (0, 0xffffffff),
        (4, 0xff800000),
        (0, 0xc0000000),
    ] {
        let mut invalid = encoded;
        invalid[lane] = value;
        assert!(decode(bytemuck::cast_slice(&invalid)).is_err());
    }
}

fn read(request: &mut GpuDeformationBoundsReadback) -> Result<Aabb> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        if let Some(bounds) = request.try_read()? {
            return Ok(bounds);
        }
        ensure!(
            std::time::Instant::now() < deadline,
            "GPU bounds readback timed out"
        );
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}

#[test]
#[ignore = "requires a supported GPU adapter"]
fn gpu_bounds_match_retained_morph_outputs_and_reject_invalid_vertices() {
    let context = WgpuContext::new_headless().unwrap();
    let reducer = GpuDeformationBounds::new(context.clone()).unwrap();
    let mesh = Mesh::subdivided_plane(PlaneOptions {
        segments: [12, 9],
        ..Default::default()
    })
    .unwrap();
    let count = mesh.vertex_count();
    let source = MorphTargets::new(
        mesh.clone(),
        [MorphTarget {
            positions: Some(
                (0..count)
                    .map(|i| {
                        if i == count - 1 {
                            [-5., 6., -7.]
                        } else {
                            [i as f32 / count as f32, -0.75, 0.25]
                        }
                    })
                    .collect::<Vec<_>>()
                    .into(),
            ),
            ..Default::default()
        }],
    )
    .unwrap();
    let gpu = GpuMorph::new(
        context.clone(),
        source.clone(),
        GpuDeformationLimits::default(),
    )
    .unwrap();
    let mut requests = Vec::new();
    for weight in [0., 0.5, -1.] {
        let output = gpu.evaluate(&[weight]).unwrap();
        assert!(reducer.request(&output, Some(63)).is_err());
        let canceled = reducer.request(&output, Some(64)).unwrap();
        drop(canceled);
        requests.push((
            reducer.request(&output, Some(64)).unwrap(),
            source.evaluate(&[weight]).unwrap().bounds(),
        ));
    }
    let mut records = crate::geometry::gpu_deformation::pack_mesh(&mesh);
    let mut failures = Vec::new();
    for reserved in [false, true] {
        let record = &mut records[count - 1];
        if reserved {
            record.position[0] = 0.;
            record.status[2] = 1;
        } else {
            record.position[0] = f32::NAN;
        }
        let buffer = context
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("invalid bounds input"),
                contents: bytemuck::cast_slice(&records),
                usage: wgpu::BufferUsages::STORAGE,
            });
        let output = GpuDeformationOutput {
            context: context.clone(),
            base: mesh.clone(),
            buffer,
        };
        failures.push(reducer.request(&output, None).unwrap());
    }
    drop(gpu);
    drop(reducer);
    for (mut request, expected) in requests {
        let actual = read(&mut request).unwrap();
        for (a, b) in actual
            .min()
            .into_iter()
            .chain(actual.max())
            .zip(expected.min().into_iter().chain(expected.max()))
        {
            assert!((a - b).abs() < 1e-5, "{actual:?} != {expected:?}");
        }
        assert!(request.try_read().is_err());
    }
    for mut request in failures {
        assert!(
            read(&mut request)
                .unwrap_err()
                .to_string()
                .contains("status")
        );
        assert!(request.try_read().is_err());
    }
}
