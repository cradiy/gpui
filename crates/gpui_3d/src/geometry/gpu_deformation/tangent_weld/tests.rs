use super::*;
use crate::{
    GpuDeformationOutput, GpuMorph, GpuTangentDerivatives, MorphTarget, MorphTargets, Vertex,
};

#[test]
fn sort_schedule_orders_duplicate_keys_and_padding_with_bounded_working_memory() {
    for corners in [3, 6, 9, 63, 66, 129, 255, 1023] {
        let schedule = passes(corners);
        let plan = GpuTangentWeldMemory::plan(
            corners as usize,
            corners as usize,
            GpuDeformationLimits::default(),
        )
        .unwrap();
        assert_eq!(schedule.len() as u32, plan.sort_passes + 2);
        assert_eq!(
            std::mem::size_of_val(schedule.as_slice()) as u64,
            plan.uniform_bytes
        );
        let original: Vec<_> = (0..plan.padded_corners)
            .map(|corner| {
                if corner < corners {
                    (0, (corner * 17 + 9) % 11, corner)
                } else {
                    (2, 0, u32::MAX)
                }
            })
            .collect();
        let mut sorted = original.clone();
        for &[_, distance, width, capacity] in &schedule[1..schedule.len() - 1] {
            let mut next = sorted.clone();
            for index in 0..capacity {
                let a = sorted[index as usize];
                let b = sorted[(index ^ distance) as usize];
                let want_low = (index & distance == 0) == (index & width == 0);
                next[index as usize] = if want_low { a.min(b) } else { a.max(b) };
            }
            sorted = next;
        }
        let mut expected = original;
        expected.sort();
        assert_eq!(sorted, expected);
        let source_bytes = plan.uv_bytes + plan.index_bytes + plan.uniform_bytes;
        let working_bytes = plan.scratch_bytes + plan.output_bytes;
        let limits = GpuDeformationLimits {
            max_source_bytes: source_bytes,
            max_output_bytes: working_bytes,
        };
        assert!(GpuTangentWeldMemory::plan(corners as usize, corners as usize, limits).is_ok());
        for limits in [
            GpuDeformationLimits {
                max_source_bytes: source_bytes - 1,
                ..limits
            },
            GpuDeformationLimits {
                max_output_bytes: working_bytes - 1,
                ..limits
            },
        ] {
            assert!(
                GpuTangentWeldMemory::plan(corners as usize, corners as usize, limits).is_err()
            );
        }
    }
    for (vertices, corners) in [(0, 3), (3, 0), (3, 4), (3, (1 << 30) + 2), (usize::MAX, 3)] {
        assert!(
            GpuTangentWeldMemory::plan(vertices, corners, GpuDeformationLimits::default()).is_err()
        );
    }
}

#[test]
fn weld_shader_validates_record_layout_and_all_dispatch_stages() {
    use wgpu::naga::{
        TypeInner,
        front::wgsl,
        valid::{Capabilities, ValidationFlags, Validator},
    };
    let module = wgsl::parse_str(SHADER).unwrap();
    assert!(
        Validator::new(ValidationFlags::all(), Capabilities::empty())
            .validate(&module)
            .is_err()
    );
    let info = Validator::new(ValidationFlags::all(), Capabilities::FLOAT64)
        .validate(&module)
        .unwrap();
    #[cfg(target_os = "linux")]
    wgpu::naga::back::spv::write_vec(&module, &info, &Default::default(), None).unwrap();
    #[cfg(not(target_os = "linux"))]
    let _ = info;
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
    for (binding, stride) in [(0, 64), (1, 8), (2, 4), (3, 64)] {
        assert!(
            matches!(globals[binding].1, TypeInner::Array { stride: actual, .. } if *actual == stride)
        );
    }
    let TypeInner::Array { base, .. } = globals[3].1 else {
        panic!("expected record array")
    };
    let TypeInner::Struct { members, span } = &module.types[*base].inner else {
        panic!("expected weld record")
    };
    assert_eq!(*span as usize, std::mem::size_of::<GpuTangentWeldRecord>());
    assert_eq!(
        members
            .iter()
            .map(|m| m.offset as usize)
            .collect::<Vec<_>>(),
        [
            0,
            16,
            std::mem::offset_of!(GpuTangentWeldRecord, identity),
            std::mem::offset_of!(GpuTangentWeldRecord, status)
        ]
    );
    assert!(matches!(globals[4].1, TypeInner::Struct { span: 16, .. }));
    assert_eq!(
        module
            .entry_points
            .iter()
            .map(|entry| (entry.name.as_str(), entry.workgroup_size))
            .collect::<Vec<_>>(),
        [
            ("initialize", [64, 1, 1]),
            ("sort_pairs", [64, 1, 1]),
            ("resolve", [64, 1, 1])
        ]
    );
}

fn mesh() -> Mesh {
    let mut vertices = Vec::new();
    for face in 0..6 {
        for (corner, position) in [[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]]
            .into_iter()
            .enumerate()
        {
            vertices.push(Vertex {
                position,
                normal: [0., 0., if face == 1 { 2. } else { 1. }],
                uv: [[0., 0.], [1., 0.], [0., 1.]][corner],
            });
        }
    }
    vertices[6].uv[0] = 0.25;
    vertices[9].normal = [1., 0., 0.];
    vertices[12].position[0] = -0.;
    vertices[15].position[0] = 1.;
    let uv = vertices.iter().map(|v| v.uv).collect();
    Mesh::new(
        vertices,
        [3, 4, 5, 0, 1, 2].into_iter().chain(6..18).collect(),
    )
    .with_uv_set(2, uv)
    .unwrap()
}

fn read(output: &GpuTangentWeldOutput) -> Result<Vec<GpuTangentWeldRecord>> {
    let context = output.derivatives().context();
    let staging = context.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("weld verification"),
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

fn expected(mesh: &Mesh) -> Vec<u32> {
    let mut first = std::collections::HashMap::new();
    mesh.indices()
        .iter()
        .enumerate()
        .map(|(corner, &vertex)| {
            let v = mesh.vertices()[vertex as usize];
            let length = v
                .normal
                .iter()
                .map(|&n| f64::from(n).powi(2))
                .sum::<f64>()
                .sqrt();
            let normal = v.normal.map(|n| (f64::from(n) / length) as f32);
            let uv = mesh.uv_at(2, vertex as usize).unwrap();
            let key: Vec<_> = v
                .position
                .into_iter()
                .chain(normal)
                .chain(uv)
                .map(f32::to_bits)
                .collect();
            *first.entry(key).or_insert(corner as u32)
        })
        .collect()
}

fn normal_variants(normals: &[[f32; 3]]) -> Mesh {
    let vertices: Vec<_> = normals
        .iter()
        .flat_map(|&normal| {
            [
                ([0., 0., 0.], [0., 0.]),
                ([1., 0., 0.], [1., 0.]),
                ([0., 1., 0.], [0., 1.]),
            ]
            .map(|(position, uv)| Vertex {
                position,
                normal,
                uv,
            })
        })
        .collect();
    let indices = (0..vertices.len() as u32).collect();
    let uv = vertices.iter().map(|vertex| vertex.uv).collect();
    Mesh::new(vertices, indices).with_uv_set(2, uv).unwrap()
}

#[test]
fn direction_generation_stages_require_device_enabled_float64() {
    use gpui_wgpu::Scene3dDeviceCapabilities;
    let mut capabilities = Scene3dDeviceCapabilities {
        adapter_info: wgpu::AdapterInfo {
            name: String::new(),
            vendor: 0,
            device: 0,
            device_type: wgpu::DeviceType::Other,
            device_pci_bus_id: String::new(),
            driver: String::new(),
            driver_info: String::new(),
            backend: wgpu::Backend::Vulkan,
            subgroup_min_size: 0,
            subgroup_max_size: 0,
            transient_saves_memory: None,
            limit_bucket: None,
        },
        adapter_features: wgpu::Features::SHADER_F64,
        enabled_features: wgpu::Features::empty(),
        adapter_limits: wgpu::Limits::default(),
        limits: wgpu::Limits::default(),
        downlevel: wgpu::DownlevelCapabilities::default(),
        color_atlas_format: wgpu::TextureFormat::Rgba8Unorm,
        formats: Vec::new(),
        max_image_anisotropy: 1,
    };
    let error = GpuTangentWeld::check_support(&capabilities)
        .unwrap_err()
        .to_string();
    assert!(error.contains("enabled SHADER_F64"));
    assert!(error.contains("adapter support: true"));
    assert!(
        GpuTangentDerivatives::check_support(&capabilities)
            .unwrap_err()
            .to_string()
            .contains("enabled SHADER_F64")
    );
    assert!(
        crate::GpuTangents::check_support(&capabilities)
            .unwrap_err()
            .to_string()
            .contains("enabled SHADER_F64")
    );
    GpuMorph::check_support(&capabilities).unwrap();
    assert!(crate::GpuFlatNormals::check_support(&capabilities).is_err());
    assert!(crate::GpuSmoothNormals::check_support(&capabilities).is_err());
    capabilities.enabled_features = wgpu::Features::SHADER_F64;
    crate::GpuFlatNormals::check_support(&capabilities).unwrap();
    crate::GpuSmoothNormals::check_support(&capabilities).unwrap();
    GpuTangentWeld::check_support(&capabilities).unwrap();
    GpuTangentDerivatives::check_support(&capabilities).unwrap();
    crate::GpuTangents::check_support(&capabilities).unwrap();
    capabilities.enabled_features = wgpu::Features::empty();
    capabilities.adapter_features = wgpu::Features::empty();
    assert!(
        GpuTangentWeld::check_support(&capabilities)
            .unwrap_err()
            .to_string()
            .contains("adapter support: false")
    );
}

#[test]
#[ignore = "requires a compute-capable GPU with SHADER_F64"]
fn gpu_weld_normal_keys_match_cpu_across_component_exponents() -> Result<()> {
    let mut normals = vec![
        [1., 1., 1.],
        [f32::MAX, f32::MAX, f32::MAX],
        [f32::MIN_POSITIVE; 3],
        [f32::from_bits(1), -f32::from_bits(2), f32::from_bits(3)],
        [f32::MAX, f32::from_bits(1), -f32::from_bits(1)],
    ];
    for exponent in [0, 1, 2, 63, 126, 127, 128, 190, 253, 254] {
        for fraction in [1, 0x234567, 0x7fffff] {
            let value = f32::from_bits(exponent << 23 | fraction);
            normals.extend([
                [value, 0.75, -0.375],
                [value, -value, value],
                [value, -0., 0.],
            ]);
        }
    }
    let base = normal_variants(&normals);
    let context = WgpuContext::new_headless()?;
    let limits = GpuDeformationLimits::default();
    let input = GpuDeformationOutput::upload(context.clone(), base.clone(), limits)?;
    let faces =
        GpuTangentDerivatives::new(context.clone(), base.clone(), 2, limits)?.evaluate(&input)?;
    let output = GpuTangentWeld::new(context, base.clone(), 2, limits)?.evaluate(&faces)?;
    let actual = read(&output)?;
    assert_eq!(
        actual.iter().map(|r| r.identity[2]).collect::<Vec<_>>(),
        expected(&base)
    );
    for (corner, record) in actual.iter().enumerate() {
        let normal = normals[corner / 3];
        let length = normal
            .into_iter()
            .map(|value| f64::from(value).powi(2))
            .sum::<f64>()
            .sqrt();
        let expected = normal.map(|value| ((f64::from(value) / length) as f32).to_bits());
        assert_eq!(record.status, [0; 4]);
        assert_eq!(&record.key[3..6], &expected, "normal {normal:?}");
    }
    Ok(())
}

#[test]
#[ignore = "requires a compute-capable GPU"]
fn gpu_weld_preserves_normal_zero_signs_and_proportional_axis_keys() -> Result<()> {
    let normals = [
        [0., 0., 1.],
        [-0., 0., 1.],
        [0., -0., 1.],
        [-0., -0., 1.],
        [0., 0., 2.],
        [-0., 0., 2.],
        [1., 0., 0.],
        [1., -0., 0.],
        [1., 0., -0.],
        [0., 1., -0.],
        [-0., 1., -0.],
    ];
    let base = normal_variants(&normals);
    let context = WgpuContext::new_headless()?;
    let limits = GpuDeformationLimits::default();
    let input = GpuDeformationOutput::upload(context.clone(), base.clone(), limits)?;
    let faces =
        GpuTangentDerivatives::new(context.clone(), base.clone(), 2, limits)?.evaluate(&input)?;
    let output = GpuTangentWeld::new(context, base.clone(), 2, limits)?.evaluate(&faces)?;
    let actual = read(&output)?;
    assert_eq!(
        actual.iter().map(|r| r.identity[2]).collect::<Vec<_>>(),
        expected(&base)
    );
    for (corner, record) in actual.iter().enumerate() {
        assert_eq!(record.status, [0; 4]);
        for (axis, value) in normals[corner / 3].into_iter().enumerate() {
            if value == 0. {
                assert_eq!(
                    record.key[3 + axis],
                    value.to_bits(),
                    "corner {corner}, axis {axis}"
                );
            }
        }
    }
    Ok(())
}

#[test]
#[ignore = "requires a compute-capable GPU; CPU/GPU normal-key rounding parity"]
fn gpu_weld_matches_cpu_normal_rounding_equivalence_classes() -> Result<()> {
    let base = normal_variants(&[
        [f32::from_bits(0x3f80000c), 0.75, 0.375],
        [f32::from_bits(0x3f80000d), 0.75, 0.375],
        [f32::from_bits(0x3f800013), 0.75, 0.375],
        [f32::from_bits(0x3f800014), 0.75, 0.375],
    ]);
    let representatives = expected(&base);
    assert_ne!(representatives[0], representatives[3]);
    assert_eq!(representatives[6], representatives[9]);
    let context = WgpuContext::new_headless()?;
    let limits = GpuDeformationLimits::default();
    let input = GpuDeformationOutput::upload(context.clone(), base.clone(), limits)?;
    let faces =
        GpuTangentDerivatives::new(context.clone(), base.clone(), 2, limits)?.evaluate(&input)?;
    let output = GpuTangentWeld::new(context, base, 2, limits)?.evaluate(&faces)?;
    let actual = read(&output)?;
    for record in &actual {
        assert_eq!(record.status, [0; 4]);
    }
    assert_eq!(
        actual.iter().map(|r| r.identity[2]).collect::<Vec<_>>(),
        representatives
    );
    Ok(())
}

#[test]
#[ignore = "requires a compute-capable GPU"]
fn gpu_weld_tracks_deformed_keys_seams_failures_and_retained_frames() -> Result<()> {
    let context = WgpuContext::new_headless()?;
    let limits = GpuDeformationLimits::default();
    let base = mesh();
    let mut positions = vec![[0.; 3]; 18];
    positions[15] = [-1., 0., 0.];
    let mut normals = vec![[0.; 3]; 18];
    normals[16] = [0., 1., 0.];
    let targets = MorphTargets::new(
        base.clone(),
        [MorphTarget {
            positions: Some(positions.into()),
            normals: Some(normals.into()),
            ..Default::default()
        }],
    )?;
    let morph = GpuMorph::new(context.clone(), targets.clone(), limits)?;
    let derivatives = GpuTangentDerivatives::new(context.clone(), base.clone(), 2, limits)?;
    let weld = GpuTangentWeld::new(context.clone(), base.clone(), 2, limits)?;
    let mut retained = Vec::new();
    // Upload checks normalization and signed zero independently of Morph arithmetic.
    let uploaded = GpuDeformationOutput::upload(context.clone(), base.clone(), limits)?;
    let initial_faces = derivatives.evaluate(&uploaded)?;
    let initial = weld.evaluate(&initial_faces)?;
    assert_eq!(
        initial.derivatives().input_buffer(),
        uploaded.buffer().raw()
    );
    assert_eq!(initial.derivatives().buffer(), initial_faces.buffer());
    assert_eq!(
        read(&initial)?
            .iter()
            .map(|r| r.identity[2])
            .collect::<Vec<_>>(),
        expected(&base)
    );
    let repeated = Mesh::new(
        base.vertices().to_vec(),
        base.indices().iter().copied().cycle().take(162).collect(),
    )
    .with_uv_set(2, base.vertices().iter().map(|v| v.uv).collect())?;
    let repeated_input = GpuDeformationOutput::upload(context.clone(), repeated.clone(), limits)?;
    let repeated_faces = GpuTangentDerivatives::new(context.clone(), repeated.clone(), 2, limits)?
        .evaluate(&repeated_input)?;
    let repeated_output = GpuTangentWeld::new(context.clone(), repeated.clone(), 2, limits)?
        .evaluate(&repeated_faces)?;
    assert_eq!(
        read(&repeated_output)?
            .iter()
            .map(|r| r.identity[2])
            .collect::<Vec<_>>(),
        expected(&repeated)
    );
    for weight in [1., -0.5] {
        let input = morph.evaluate(&[weight])?;
        let faces = derivatives.evaluate(&input)?;
        retained.push((targets.evaluate(&[weight])?, weld.evaluate(&faces)?));
    }
    let wrong_uv = GpuTangentDerivatives::new(context.clone(), base.clone(), 0, limits)?
        .evaluate(&uploaded)?;
    assert!(weld.evaluate(&wrong_uv).is_err());
    let foreign_base = mesh();
    let foreign = GpuDeformationOutput::upload(context.clone(), foreign_base.clone(), limits)?;
    let foreign =
        GpuTangentDerivatives::new(context.clone(), foreign_base, 2, limits)?.evaluate(&foreign)?;
    assert!(weld.evaluate(&foreign).is_err());
    let mut records = crate::geometry::gpu_deformation::pack_mesh(&base);
    records[0].status = [0, 0, 9, 0];
    records[3].status = [0, 0, 9, 0];
    records[4].normal = [0.; 4];
    records[5].position[0] = f32::NAN;
    let invalid = GpuDeformationOutput {
        context: context.clone(),
        base: base.clone(),
        buffer: context.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("weld failures"),
            contents: bytemuck::cast_slice(&records),
            usage: wgpu::BufferUsages::STORAGE,
        }),
    };
    let result = read(&weld.evaluate(&derivatives.evaluate(&invalid)?)?)?;
    for (corner, status) in [
        (0, [0, 0, 9, 0]),
        (3, [0, 0, 9, 0]),
        (1, [2, 0, 0, 0]),
        (2, [1, 0, 0, 0]),
    ] {
        assert_eq!(result[corner].status, status);
        assert_eq!(result[corner].identity[2], corner as u32);
        assert_eq!(result[corner].identity[3], 1);
    }
    drop(weld);
    drop(derivatives);
    drop(morph);
    for (cpu, output) in retained {
        let actual = read(&output)?;
        assert_eq!(actual.len(), output.corner_count());
        assert_eq!(
            actual
                .iter()
                .map(|record| record.identity[2])
                .collect::<Vec<_>>(),
            expected(&cpu)
        );
        for (corner, record) in actual.iter().enumerate() {
            assert_eq!(record.identity[0], corner as u32);
            assert_eq!(record.identity[1], base.indices()[corner]);
            assert_eq!(record.status, [0; 4]);
            assert_eq!(record.identity[3], 0);
        }
    }
    Ok(())
}
