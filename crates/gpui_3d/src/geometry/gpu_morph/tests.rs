use super::*;
use crate::Mesh;
use crate::geometry::gpu_deformation::decode;
use crate::{MorphTarget, PlaneOptions};

fn source() -> MorphTargets {
    let mesh = Mesh::subdivided_plane(PlaneOptions {
        segments: [9, 8],
        ..Default::default()
    })
    .unwrap();
    let count = mesh.vertex_count();
    let mesh = mesh
        .with_uv_set(3, mesh.vertices().iter().map(|v| v.uv).collect())
        .unwrap()
        .with_tangents_for_uv_set(3, mesh.tangents().unwrap().to_vec())
        .unwrap()
        .with_vertex_colors(vec![[0.2, 0.5, 0.8, 0.7]; count])
        .unwrap();
    MorphTargets::new(
        mesh,
        [
            MorphTarget {
                positions: Some(vec![[0.2, 0.3, 0.4]; count].into()),
                normals: Some(vec![[0.1, 0.2, 0.]; count].into()),
                tangents: Some(vec![[0., 0.1, 0.]; count].into()),
            },
            MorphTarget {
                positions: Some(vec![[-0.3, 0., 0.2]; count].into()),
                ..Default::default()
            },
        ],
    )
    .unwrap()
}

fn records(mesh: &Mesh) -> Vec<GpuDeformationVertex> {
    mesh.vertices()
        .iter()
        .enumerate()
        .map(|(index, v)| GpuDeformationVertex {
            position: pad(v.position),
            normal: pad(v.normal),
            tangent: mesh.tangents().map_or([0.; 4], |t| t[index]),
            status: [0; 4],
        })
        .collect()
}

fn near_mesh(actual: &Mesh, expected: &Mesh) {
    assert_eq!(actual.vertex_count(), expected.vertex_count());
    assert_eq!(actual.indices(), expected.indices());
    assert_eq!(actual.vertex_colors(), expected.vertex_colors());
    assert_eq!(actual.tangent_uv_set(), expected.tangent_uv_set());
    assert_eq!(
        actual.uv_sets().collect::<Vec<_>>(),
        expected.uv_sets().collect::<Vec<_>>()
    );
    for set in actual.uv_sets() {
        for vertex in 0..actual.vertex_count() {
            assert_eq!(actual.uv_at(set, vertex), expected.uv_at(set, vertex));
        }
    }
    for (a, b) in actual.vertices().iter().zip(expected.vertices()) {
        assert_eq!(a.uv, b.uv);
        for (a, b) in a
            .position
            .into_iter()
            .chain(a.normal)
            .zip(b.position.into_iter().chain(b.normal))
        {
            assert!((a - b).abs() < 2e-5, "{a} != {b}");
        }
    }
    if let Some(tangents) = actual.tangents() {
        for (a, b) in tangents
            .iter()
            .flatten()
            .zip(expected.tangents().unwrap().iter().flatten())
        {
            assert!((a - b).abs() < 2e-5);
        }
    } else {
        assert!(expected.tangents().is_none());
    }
}

#[test]
fn shader_storage_layout_matches_host_records_and_bindings() {
    use wgpu::naga::{
        ShaderStage, TypeInner,
        front::wgsl,
        valid::{Capabilities, ValidationFlags, Validator},
    };
    let module = wgsl::parse_str(include_str!("../gpu_morph.wgsl")).unwrap();
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
        panic!("vertex record is not a struct")
    };
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
    assert_eq!(module.entry_points[0].stage, ShaderStage::Compute);
    assert_eq!(module.entry_points[0].workgroup_size, [64, 1, 1]);
    let bindings: Vec<_> = module
        .global_variables
        .iter()
        .filter_map(|(_, v)| v.binding.as_ref().map(|b| (b.group, b.binding)))
        .collect();
    assert_eq!(bindings, [(0, 0), (0, 1), (0, 2), (0, 3), (0, 4)]);
}

#[test]
fn admission_checks_padding_overflow_and_device_dispatch_before_allocation() {
    let limits = GpuDeformationLimits::default();
    let memory = GpuMorphMemory::plan(4, 2, limits).unwrap();
    assert_eq!(memory.base_bytes, 256);
    assert_eq!(memory.delta_bytes, 512);
    assert_eq!(memory.weight_bytes, 8);
    assert_eq!(memory.uniform_bytes, 16);
    assert_eq!(memory.output_bytes, 256);
    let empty = GpuMorphMemory::plan(4, 0, limits).unwrap();
    assert_eq!(empty.delta_bytes, 64);
    assert_eq!(empty.weight_bytes, 4);
    assert!(GpuMorphMemory::plan(0, 1, limits).is_err());
    assert!(GpuMorphMemory::plan(usize::MAX, 2, limits).is_err());
    assert!(GpuMorphMemory::plan(65536, 65536, limits).is_err());
    assert!(
        GpuMorphMemory::plan(
            4,
            2,
            GpuDeformationLimits {
                max_source_bytes: 783,
                ..limits
            }
        )
        .is_err()
    );
    assert!(
        GpuMorphMemory::plan(
            4,
            2,
            GpuDeformationLimits {
                max_output_bytes: 255,
                ..limits
            }
        )
        .is_err()
    );
    let device = wgpu::Limits {
        max_storage_buffer_binding_size: 511,
        ..Default::default()
    };
    assert!(memory.validate_device(&device).is_err());
    let device = wgpu::Limits {
        max_compute_workgroups_per_dimension: 1,
        ..Default::default()
    };
    assert!(
        GpuMorphMemory::plan(65, 1, limits)
            .unwrap()
            .validate_device(&device)
            .is_err()
    );
}

#[test]
fn readback_decoding_rebuilds_bounds_and_rejects_invalid_records() {
    let source = source();
    let original = source.base_mesh().clone();
    let evaluated = source.evaluate(&[0.75, -0.5]).unwrap();
    let mut output = records(&evaluated);
    let decoded = decode(&original, bytemuck::cast_slice(&output)).unwrap();
    near_mesh(&decoded, &evaluated);
    assert_eq!(decoded.bounds(), evaluated.bounds());
    assert_ne!(decoded.bounds(), original.bounds());
    assert_eq!(original.bounds(), source.base_mesh().bounds());
    output[1].status[0] = 2;
    assert!(decode(&original, bytemuck::cast_slice(&output)).is_err());
    output[1].status = [0; 4];
    output[1].position[0] = f32::NAN;
    assert!(decode(&original, bytemuck::cast_slice(&output)).is_err());
    assert!(decode(&original, &[]).is_err());
}

#[test]
#[ignore = "requires a compute-capable GPU"]
fn compute_morph_matches_cpu_and_retains_independent_outputs() {
    let read = |request: &mut crate::GpuDeformationReadback| -> anyhow::Result<Mesh> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        loop {
            if let Some(mesh) = request.try_read()? {
                return Ok(mesh);
            }
            anyhow::ensure!(std::time::Instant::now() < deadline, "readback timed out");
            std::thread::yield_now();
        }
    };
    let source = source();
    let context = WgpuContext::new_headless().unwrap();
    let gpu = GpuMorph::new(
        context.clone(),
        source.clone(),
        GpuDeformationLimits::default(),
    )
    .unwrap();
    assert!(gpu.evaluate(&[1.]).is_err());
    assert!(gpu.evaluate(&[f32::NAN, 0.]).is_err());
    let first = gpu.evaluate(&[0.75, -0.5]).unwrap();
    let zero = gpu.evaluate(&[0., 0.]).unwrap();
    let second = gpu.evaluate(&[-0.2, 0.9]).unwrap();
    let bytes = first.buffer().size();
    assert!(first.request_readback(Some(bytes - 1)).is_err());
    let mut request = first.request_readback(Some(bytes)).unwrap();
    assert_eq!(request.staging_bytes(), bytes);
    let canceled = second.request_readback(None).unwrap();
    drop(canceled);
    let mut second_request = second.request_readback(None).unwrap();
    drop(first);
    drop(second);
    drop(gpu);
    near_mesh(
        &read(&mut request).unwrap(),
        &source.evaluate(&[0.75, -0.5]).unwrap(),
    );
    assert!(
        request
            .try_read()
            .unwrap_err()
            .to_string()
            .contains("finished")
    );
    near_mesh(
        &read(&mut second_request).unwrap(),
        &source.evaluate(&[-0.2, 0.9]).unwrap(),
    );
    near_mesh(&zero.readback().unwrap(), source.base_mesh());
    let empty = MorphTargets::new(source.base_mesh().clone(), []).unwrap();
    near_mesh(
        &GpuMorph::new(context.clone(), empty, GpuDeformationLimits::default())
            .unwrap()
            .evaluate(&[])
            .unwrap()
            .readback()
            .unwrap(),
        source.base_mesh(),
    );
    let count = source.base_mesh().vertex_count();
    let invalid = MorphTargets::new(
        source.base_mesh().clone(),
        [MorphTarget {
            normals: Some(vec![[0., 0., -1.]; count].into()),
            ..Default::default()
        }],
    )
    .unwrap();
    let gpu = GpuMorph::new(context, invalid, GpuDeformationLimits::default()).unwrap();
    let mut failed = gpu.evaluate(&[1.]).unwrap().request_readback(None).unwrap();
    assert!(
        read(&mut failed)
            .unwrap_err()
            .to_string()
            .contains("status")
    );
    assert!(
        failed
            .try_read()
            .unwrap_err()
            .to_string()
            .contains("finished")
    );
}
