use super::*;
use crate::{
    GpuDeformationOutput, GpuMorph, GpuTangentAdjacency, GpuTangentDerivatives, GpuTangentGroups,
    GpuTangentWeld, MorphTarget, MorphTargets, TangentGenerationMode, Vertex,
};

#[test]
fn frame_shader_matches_host_records_and_binding_layout() {
    use wgpu::naga::{
        TypeInner,
        front::wgsl,
        valid::{Capabilities, ValidationFlags, Validator},
    };
    let module = wgsl::parse_str(include_str!("../tangent_frames.wgsl")).unwrap();
    Validator::new(ValidationFlags::all(), Capabilities::empty())
        .validate(&module)
        .unwrap();
    for (_, variable) in module.global_variables.iter() {
        let binding = variable.binding.as_ref().unwrap();
        assert_eq!(binding.group, 0);
        let ty = &module.types[variable.ty].inner;
        if binding.binding == 4 {
            assert!(matches!(ty, TypeInner::Struct { span: 16, .. }));
        } else {
            let TypeInner::Array {
                base, stride: 64, ..
            } = ty
            else {
                panic!("64-byte record required")
            };
            if binding.binding == 0 || binding.binding == 3 {
                let TypeInner::Struct { members, span: 64 } = &module.types[*base].inner else {
                    panic!("frame required")
                };
                assert_eq!(std::mem::size_of::<GpuTangentFrame>(), 64);
                assert_eq!(
                    members
                        .iter()
                        .map(|member| member.offset as usize)
                        .collect::<Vec<_>>(),
                    [
                        std::mem::offset_of!(GpuTangentFrame, tangent),
                        std::mem::offset_of!(GpuTangentFrame, bitangent),
                        std::mem::offset_of!(GpuTangentFrame, identity),
                        std::mem::offset_of!(GpuTangentFrame, angle_weight),
                        std::mem::offset_of!(GpuTangentFrame, status),
                    ]
                );
            }
        }
    }
    assert_eq!(
        module
            .entry_points
            .iter()
            .map(|entry| (entry.name.as_str(), entry.workgroup_size))
            .collect::<Vec<_>>(),
        [
            ("initialize", [64, 1, 1]),
            ("sort_pairs", [64, 1, 1]),
            ("accumulate", [64, 1, 1])
        ]
    );
}

#[test]
fn frame_budget_covers_padded_sort_buffers_and_all_pass_uniforms() {
    for count in [3, 6, 63, 66, 129, 258] {
        let memory = GpuTangentFramesMemory::plan(count, GpuDeformationLimits::default()).unwrap();
        let passes = super::super::tangent_weld::passes(count as u32);
        assert_eq!(memory.uniform_bytes, passes.len() as u64 * 16);
        assert_eq!(memory.sort_passes as usize, passes.len() - 2);
        assert_eq!(
            memory.output_bytes,
            (count * std::mem::size_of::<GpuTangentFrame>()) as u64
        );
        assert_eq!(memory.scratch_bytes, u64::from(passes[0][3]) * 128);
        let limits = GpuDeformationLimits {
            max_source_bytes: memory.uniform_bytes,
            max_output_bytes: memory.scratch_bytes + memory.output_bytes,
        };
        assert!(GpuTangentFramesMemory::plan(count, limits).is_ok());
        assert!(
            GpuTangentFramesMemory::plan(
                count,
                GpuDeformationLimits {
                    max_source_bytes: limits.max_source_bytes - 1,
                    ..limits
                }
            )
            .is_err()
        );
        assert!(
            GpuTangentFramesMemory::plan(
                count,
                GpuDeformationLimits {
                    max_output_bytes: limits.max_output_bytes - 1,
                    ..limits
                }
            )
            .is_err()
        );
    }
    for count in [0, 1, 4, (1 << 30) + 1, usize::MAX] {
        assert!(GpuTangentFramesMemory::plan(count, GpuDeformationLimits::default()).is_err());
    }
}

fn mesh(opposed: bool) -> Mesh {
    let mut positions = vec![[0., 0., 0.], [2., 0., 0.], [0., 1., 0.], [-1., 1., 0.]];
    let mut uv = vec![[0., 0.], [1., 0.], [0., 1.], [-1., 2.]];
    if opposed {
        positions[3][0] = 1.;
        uv[3][1] = 1.;
    }
    Mesh::new(
        positions
            .into_iter()
            .map(|position| Vertex {
                position,
                normal: [0., 0., 2.],
                uv: [0.; 2],
            })
            .collect(),
        vec![0, 1, 2, 0, 2, 3],
    )
    .with_uv_set(2, uv)
    .unwrap()
}

fn read(output: &GpuTangentFramesOutput) -> Result<Vec<GpuTangentFrame>> {
    let context = output.groups().adjacency().weld().derivatives().context();
    let staging = context.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("frame verification"),
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
        .map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
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

fn sources(
    context: &WgpuContext,
    base: &Mesh,
) -> Result<(
    GpuTangentDerivatives,
    GpuTangentWeld,
    GpuTangentAdjacency,
    GpuTangentGroups,
    GpuTangentFrames,
)> {
    let limits = GpuDeformationLimits::default();
    Ok((
        GpuTangentDerivatives::new(context.clone(), base.clone(), 2, limits)?,
        GpuTangentWeld::new(context.clone(), base.clone(), 2, limits)?,
        GpuTangentAdjacency::new(context.clone(), base.clone(), 2, limits)?,
        GpuTangentGroups::new(context.clone(), base.clone(), 2, limits)?,
        GpuTangentFrames::new(context.clone(), base.clone(), 2, limits)?,
    ))
}

fn close(actual: f32, expected: f32) {
    assert!((actual - expected).abs() < 2e-5, "{actual} != {expected}");
}

#[test]
#[ignore = "requires a compute-capable GPU"]
fn gpu_frames_weight_corners_preserve_subgroups_and_retain_deformed_inputs() -> Result<()> {
    let context = WgpuContext::new_headless()?;
    let limits = GpuDeformationLimits::default();
    let base = mesh(false);
    let (derivatives, weld, adjacency, groups, frames) = sources(&context, &base)?;
    let mut delta = vec![[0.; 3]; base.vertex_count()];
    delta[3][1] = 1.;
    let targets = MorphTargets::new(
        base.clone(),
        [MorphTarget {
            positions: Some(delta.into()),
            normals: Some(vec![[0.2, -0.3, 0.]; base.vertex_count()].into()),
            ..Default::default()
        }],
    )?;
    let morph = GpuMorph::new(context.clone(), targets.clone(), limits)?;
    let grouped = |input: &GpuDeformationOutput| {
        groups.evaluate(&adjacency.evaluate(&weld.evaluate(&derivatives.evaluate(input)?)?)?)
    };
    let mut retained = Vec::new();
    for weight in [0., 0.5, 1.] {
        let input = morph.evaluate(&[weight])?;
        let group = grouped(&input)?;
        let output = frames.evaluate(&group)?;
        assert_eq!(output.groups().buffer(), group.buffer());
        assert_eq!(
            output
                .groups()
                .adjacency()
                .weld()
                .derivatives()
                .input_buffer(),
            input.buffer()
        );
        retained.push((weight, output));
    }
    let group = grouped(&morph.evaluate(&[0.])?)?;
    assert!(
        GpuTangentFrames::new(context.clone(), mesh(false), 2, limits)?
            .evaluate(&group)
            .is_err()
    );
    assert!(
        GpuTangentFrames::new(context.clone(), base.clone(), 0, limits)?
            .evaluate(&group)
            .is_err()
    );
    let mut records = super::super::pack_mesh(&base);
    records[3].position = records[2].position;
    let collapsed = GpuDeformationOutput {
        context: context.clone(),
        base: base.clone(),
        buffer: buffer(
            &context.device,
            "collapsed frame input",
            bytemuck::cast_slice(&records),
            wgpu::BufferUsages::STORAGE,
        ),
    };
    let collapsed = read(&frames.evaluate(&grouped(&collapsed)?)?)?;
    for record in &collapsed[3..] {
        assert_eq!(record.identity[1..], [u32::MAX; 2]);
        assert_eq!(record.tangent, [0.; 4]);
        assert_eq!(record.angle_weight, 0.);
        assert_eq!(record.status, [0; 4]);
    }
    records[2].normal = [0.; 4];
    let invalid = GpuDeformationOutput {
        context: context.clone(),
        base,
        buffer: buffer(
            &context.device,
            "invalid frame input",
            bytemuck::cast_slice(&records),
            wgpu::BufferUsages::STORAGE,
        ),
    };
    for record in read(&frames.evaluate(&grouped(&invalid)?)?)? {
        assert_eq!(record.status, [2, 0, 0, 0]);
    }
    drop((frames, groups, adjacency, weld, derivatives, morph));
    for (weight, output) in retained {
        let cpu = targets
            .evaluate(&[weight])?
            .generate_tangents_for_uv_set(2, TangentGenerationMode::Strict)?;
        let records = read(&output)?;
        for (corner, record) in records.iter().enumerate() {
            assert_eq!(record.identity[0] as usize, corner);
            assert_eq!(record.identity[2], 1);
            assert_eq!(record.status, [0; 4]);
            let tangent = cpu.mesh().tangents().unwrap()[cpu.mesh().indices()[corner] as usize];
            for (a, b) in record.tangent[..3].iter().zip(tangent) {
                close(*a, b);
            }
            let normal = cpu.mesh().vertices()[cpu.mesh().indices()[corner] as usize].normal;
            close(
                record.bitangent[..3]
                    .iter()
                    .zip(normal)
                    .map(|(a, b)| a * b)
                    .sum(),
                0.,
            );
            close(record.bitangent[..3].iter().map(|v| v * v).sum(), 1.);
        }
        assert_eq!(records[0].tangent, records[3].tangent);
        if weight == 0. {
            let v = [
                2. + std::f32::consts::FRAC_1_SQRT_2,
                std::f32::consts::FRAC_1_SQRT_2,
            ];
            let length = v[0].hypot(v[1]);
            close(records[0].tangent[0], v[0] / length);
            close(records[0].tangent[1], v[1] / length);
            close(records[0].tangent[3], (4. + std::f32::consts::SQRT_2) / 3.);
            close(records[0].angle_weight, std::f32::consts::FRAC_PI_4 * 3.);
        }
    }
    let base = mesh(true);
    let (derivatives, weld, adjacency, groups, frames) = sources(&context, &base)?;
    let input = GpuDeformationOutput::upload(context, base, limits)?;
    let output =
        frames
            .evaluate(&groups.evaluate(
                &adjacency.evaluate(&weld.evaluate(&derivatives.evaluate(&input)?)?)?,
            )?)?;
    let records = read(&output)?;
    assert_eq!(records[0].identity[1], records[3].identity[1]);
    close(records[0].tangent[0], 1.);
    close(records[3].tangent[0], -1.);
    close(records[0].angle_weight, std::f32::consts::FRAC_PI_2);
    close(records[3].angle_weight, std::f32::consts::FRAC_PI_4);
    Ok(())
}
