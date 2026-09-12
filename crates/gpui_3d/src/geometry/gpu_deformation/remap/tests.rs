use super::*;
use crate::{
    GpuMorph, GpuSmoothNormals, GpuTangentGeneration, MorphTarget, MorphTargets, NormalMode,
    TangentGenerationMode,
};

fn base() -> Mesh {
    let plane = Mesh::plane();
    Mesh::new(plane.vertices().to_vec(), plane.indices().to_vec())
}

#[test]
fn mapping_admission_checks_complete_correspondence_and_tangent_metadata() -> Result<()> {
    let source = base();
    let expanded = source.expand_corners(6)?;
    validate_mapping(&source, expanded.mesh(), expanded.source_vertices())?;
    validate_mapping(&source, &source, &[2, 0, 2, 1])?;
    let subset = Mesh::new(source.vertices()[..3].to_vec(), vec![0, 1, 2]);
    validate_mapping(&source, &subset, &[3, 1, 0])?;
    for mapping in [
        vec![],
        vec![0; 3],
        vec![0; 5],
        vec![0, 1, 2, 4],
        vec![0, 1, 2, u32::MAX],
    ] {
        assert!(validate_mapping(&source, &source, &mapping).is_err());
    }
    let tangent = Mesh::plane();
    assert!(validate_mapping(&source, &tangent, &[0, 1, 2, 3]).is_err());
    validate_mapping(&tangent, &source, &[0, 1, 2, 3])?;
    let other_set = source
        .with_uv_set(1, vec![[0.; 2]; 4])?
        .with_tangents_for_uv_set(1, vec![[1., 0., 0., 1.]; 4])?;
    assert!(validate_mapping(&tangent, &other_set, &[0, 1, 2, 3]).is_err());
    Ok(())
}

#[test]
fn memory_admission_separates_mapping_and_output_counts() {
    for (source, output) in [(4, 6), (6, 3), (4, 65)] {
        let memory = GpuDeformationRemapMemory::plan(source, output, Default::default()).unwrap();
        let limits = GpuDeformationLimits {
            max_source_bytes: memory.mapping_bytes + memory.uniform_bytes,
            max_output_bytes: memory.output_bytes,
        };
        assert_eq!(
            GpuDeformationRemapMemory::plan(source, output, limits).unwrap(),
            memory
        );
        assert!(
            GpuDeformationRemapMemory::plan(
                source,
                output,
                GpuDeformationLimits {
                    max_source_bytes: limits.max_source_bytes - 1,
                    ..limits
                }
            )
            .is_err()
        );
        assert!(
            GpuDeformationRemapMemory::plan(
                source,
                output,
                GpuDeformationLimits {
                    max_output_bytes: limits.max_output_bytes - 1,
                    ..limits
                }
            )
            .is_err()
        );
    }
    for (source, output) in [(0, 3), (3, 0)] {
        assert!(GpuDeformationRemapMemory::plan(source, output, Default::default()).is_err());
    }
    if let Some(too_many) = (u32::MAX as usize).checked_add(1) {
        assert!(GpuDeformationRemapMemory::plan(too_many, 3, Default::default()).is_err());
        assert!(GpuDeformationRemapMemory::plan(3, too_many, Default::default()).is_err());
    }
}

#[test]
fn shader_validates_integer_record_copy_without_optional_features() {
    use wgpu::naga::{
        TypeInner,
        front::wgsl,
        valid::{Capabilities, ValidationFlags, Validator},
    };
    let module = wgsl::parse_str(include_str!("../remap.wgsl")).unwrap();
    Validator::new(ValidationFlags::all(), Capabilities::empty())
        .validate(&module)
        .unwrap();
    for (_, global) in module.global_variables.iter() {
        match global.binding.as_ref().unwrap().binding {
            0 | 2 => assert!(matches!(
                module.types[global.ty].inner,
                TypeInner::Array { stride: 64, .. }
            )),
            1 => assert!(matches!(
                module.types[global.ty].inner,
                TypeInner::Array { stride: 4, .. }
            )),
            3 => assert!(matches!(
                module.types[global.ty].inner,
                TypeInner::Struct { span: 16, .. }
            )),
            _ => panic!("unexpected binding"),
        }
    }
    assert_eq!(module.entry_points[0].workgroup_size, [64, 1, 1]);
}

fn read_words(context: &WgpuContext, source: &wgpu::Buffer) -> Result<Vec<u32>> {
    let buffer = context.device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: source.size(),
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = context.device.create_command_encoder(&Default::default());
    encoder.copy_buffer_to_buffer(source, 0, &buffer, 0, source.size());
    let submission = context.queue.submit([encoder.finish()]);
    let (send, receive) = std::sync::mpsc::channel();
    buffer
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |result| {
            let _ = send.send(result);
        });
    let timeout = std::time::Duration::from_secs(10);
    context.device.poll(wgpu::PollType::Wait {
        submission_index: Some(submission),
        timeout: Some(timeout),
    })?;
    receive.recv_timeout(timeout)??;
    let words = buffer
        .slice(..)
        .get_mapped_range()?
        .chunks_exact(4)
        .map(|word| u32::from_ne_bytes(word.try_into().unwrap()))
        .collect();
    buffer.unmap();
    Ok(words)
}

#[test]
#[ignore = "requires a compute-capable GPU"]
fn gpu_remap_preserves_record_bits_and_retained_copies_after_producer_reuse() -> Result<()> {
    let context = WgpuContext::new_headless()?;
    let source = base();
    let destination = source.with_vertex_colors(vec![[0.2, 0.3, 0.4, 1.]; 4])?;
    let mapping = [2, 0, 2, 1];
    let remap = GpuDeformationRemap::new(
        context.clone(),
        source.clone(),
        destination.clone(),
        &mapping,
        Default::default(),
    )?;
    let mut records = super::super::pack_mesh(&source);
    records[1].position[0] = f32::from_bits(0x7fc00123);
    records[0].normal[3] = -0.;
    records[2].status = [0, 7, 11, 13];
    let buffer = context.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: None,
        contents: bytemuck::cast_slice(&records),
        usage: wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
    });
    let input = GpuDeformationOutput::copy_from_buffer(
        context.clone(),
        source.clone(),
        &buffer,
        Default::default(),
    )?;
    let first = remap.evaluate(&input)?;
    let second = remap.evaluate(&input)?;
    let subset_mesh = Mesh::new(source.vertices()[..3].to_vec(), vec![0, 1, 2]);
    let subset = GpuDeformationRemap::new(
        context.clone(),
        source.clone(),
        subset_mesh,
        &[0, 3, 0],
        Default::default(),
    )?;
    let valid = subset.evaluate(&input)?.readback()?;
    assert_eq!(valid.vertex_count(), 3);
    assert_eq!(valid.vertices()[1].position, source.vertices()[3].position);
    assert_ne!(first.buffer().raw(), second.buffer().raw());
    assert!(first.base_mesh().ptr_eq(&destination));
    let expected: Vec<u32> = mapping
        .iter()
        .flat_map(|&index| {
            bytemuck::cast_slice::<_, u32>(&records[index as usize..index as usize + 1]).to_vec()
        })
        .collect();
    context.queue.write_buffer(
        &buffer,
        0,
        bytemuck::cast_slice(&super::super::pack_mesh(&source)),
    );
    let newer = GpuDeformationOutput::copy_from_buffer(
        context.clone(),
        source.clone(),
        &buffer,
        Default::default(),
    )?;
    remap.evaluate(&newer)?.readback()?;
    let mismatched = GpuDeformationOutput::upload(context.clone(), base(), Default::default())?;
    assert!(remap.evaluate(&mismatched).is_err());
    let foreign = WgpuContext::new_headless()?;
    let foreign = GpuDeformationOutput::upload(foreign, source, Default::default())?;
    assert!(remap.evaluate(&foreign).is_err());
    drop((remap, input, buffer));
    for output in [first, second] {
        assert_eq!(read_words(&context, output.buffer().raw())?, expected);
        assert!(
            output
                .readback()
                .unwrap_err()
                .to_string()
                .contains("[0, 7, 11, 13]")
        );
    }
    Ok(())
}

#[test]
#[ignore = "requires a compute-capable GPU with SHADER_F64"]
fn gpu_remap_connects_shared_smooth_normals_to_corner_tangent_generation() -> Result<()> {
    let context = WgpuContext::new_headless()?;
    let base = base();
    let expanded = base.expand_corners(6)?;
    let targets = MorphTargets::new(
        base.clone(),
        [MorphTarget {
            positions: Some(
                vec![[0., 0., 0.], [0., 0., 0.2], [0., 0., 0.7], [0., 0., -0.1]].into(),
            ),
            ..Default::default()
        }],
    )?;
    let morph = GpuMorph::new(context.clone(), targets.clone(), Default::default())?;
    let normals = GpuSmoothNormals::new(context.clone(), base.clone(), Default::default())?;
    let remap = GpuDeformationRemap::new(
        context.clone(),
        base,
        expanded.mesh().clone(),
        expanded.source_vertices(),
        Default::default(),
    )?;
    let tangents = GpuTangentGeneration::new(
        context,
        expanded.mesh().clone(),
        0,
        TangentGenerationMode::Strict,
        Default::default(),
    )?;
    let mut retained = Vec::new();
    for weight in [0., 1., -0.5] {
        let deformed = morph.evaluate(&[weight])?;
        let smooth = normals.evaluate(&deformed)?;
        let corners = remap.evaluate(&smooth)?;
        assert!(corners.base_mesh().ptr_eq(expanded.mesh()));
        retained.push((weight, tangents.evaluate(&corners)?));
    }
    drop((morph, normals, remap, tangents));
    for (weight, output) in retained {
        let expected = targets
            .evaluate(&[weight])?
            .generate_normals(NormalMode::Smooth)?;
        let expected = expected.mesh().expand_corners(6)?;
        let expected = expected
            .mesh()
            .generate_tangents_with_mode(TangentGenerationMode::Strict)?;
        let actual = output.deformation().readback()?;
        assert_eq!(actual.indices(), expanded.mesh().indices());
        for (index, &source) in expected.source_vertices().iter().enumerate() {
            let a = actual.vertices()[source as usize];
            let b = expected.mesh().vertices()[index];
            for (a, b) in a
                .position
                .into_iter()
                .chain(a.normal)
                .chain(actual.tangents().unwrap()[source as usize])
                .zip(
                    b.position
                        .into_iter()
                        .chain(b.normal)
                        .chain(expected.mesh().tangents().unwrap()[index]),
                )
            {
                assert!((a - b).abs() < 2e-5, "{a} != {b}");
            }
        }
        let source = output.deformation().render_source([0; 5], None)?;
        output.deformation().render_geometry(&source)?;
    }
    Ok(())
}
