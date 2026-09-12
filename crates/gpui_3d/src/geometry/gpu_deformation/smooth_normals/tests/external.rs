use super::*;
use crate::{AffineTransform, GpuDeformationBounds, GpuSkin, Skin, SkinInfluence};

#[test]
#[ignore = "requires a compute-capable GPU with SHADER_F64"]
fn external_normal_snapshots_preserve_extreme_coordinates_and_downstream_geometry() -> Result<()> {
    let context = WgpuContext::new_headless()?;
    let block = mesh();
    let base = Mesh::new(
        block.vertices().repeat(17),
        (0..17)
            .flat_map(|block_index| {
                block
                    .indices()
                    .iter()
                    .map(move |index| index + block_index * 8)
            })
            .collect(),
    )
    .with_uv_set(2, vec![[0.25, 0.75]; 136])?
    .with_vertex_colors(vec![[0.2, 0.4, 0.6, 0.8]; 136])?;
    let normals = GpuSmoothNormals::new(context.clone(), base.clone(), Default::default())?;
    let bounds = GpuDeformationBounds::new(context.clone())?;
    let initial = GpuDeformationOutput::upload(context.clone(), base.clone(), Default::default())?;
    let packing = initial.render_source([2; 5], None)?;
    let producer = context.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: initial.buffer().size(),
        usage: wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut inputs = Vec::new();
    for scale in [
        f32::from_bits(1),
        f32::MIN_POSITIVE,
        1e-30,
        1.,
        1e30,
        f32::MAX / 16.,
    ] {
        inputs.push((
            scale == 1.,
            base.with_vertices(
                base.vertices()
                    .iter()
                    .map(|vertex| Vertex {
                        position: vertex.position.map(|value| value * scale),
                        ..*vertex
                    })
                    .collect(),
                None,
            )?,
        ));
    }
    let mut vertices = base.vertices().to_vec();
    for block in vertices.chunks_exact_mut(8) {
        block[0].position = [-0., 0., -0.];
        block[1].position = [16_777_216., 16_777_215., 0.];
        block[2].position = [16_777_215., 16_777_214., 0.];
        block[3].position = [0., 0., 1.];
    }
    inputs.push((false, base.with_vertices(vertices, None)?));

    let binding = Skin::new(
        [AffineTransform::IDENTITY],
        (0..base.vertex_count()).map(|_| {
            [SkinInfluence {
                joint: 0,
                weight: 1.,
            }]
        }),
    )?;
    let skin = GpuSkin::new(context.clone(), binding.clone(), Default::default())?;
    let pose = AffineTransform::from_trs([0.5, -0.25, 0.75], [0., 0.6, 0., 0.8], [-1.5, 0.75, 2.])?;
    let palette = skin.palette(AffineTransform::IDENTITY, &[pose])?;
    let mut retained = Vec::new();
    for (check_skin, input) in inputs {
        let generated = input.generate_normals(NormalMode::Smooth)?;
        let mut expected_vertices = input.vertices().to_vec();
        for (corner, source) in generated.source_vertices().iter().enumerate() {
            expected_vertices[*source as usize].normal = generated.mesh().vertices()[corner].normal;
        }
        let expected = input.with_vertices(expected_vertices, None)?;
        let records = crate::geometry::gpu_deformation::pack_mesh(&input);
        context
            .queue
            .write_buffer(&producer, 0, bytemuck::cast_slice(&records));
        let external = GpuDeformationOutput::copy_from_buffer(
            context.clone(),
            base.clone(),
            &producer,
            Default::default(),
        )?;
        context
            .queue
            .write_buffer(&producer, 0, &vec![0xff; producer.size() as usize]);
        let output = normals.evaluate(&external)?;
        let preparation = output.prepare_render_geometry(&packing, &bounds, None)?;
        let skinned = if check_skin {
            let output = skin.evaluate(&output, &palette)?;
            let prepared = output.prepare_render_geometry(&packing, &bounds, None)?;
            Some((
                output,
                prepared,
                binding.evaluate_world(&expected, AffineTransform::IDENTITY, &[pose])?,
            ))
        } else {
            None
        };
        retained.push((output, preparation, expected, skinned));
    }
    drop((producer, initial, normals, skin, palette, bounds, packing));
    context.device.poll(wgpu::PollType::Wait {
        submission_index: None,
        timeout: Some(std::time::Duration::from_secs(30)),
    })?;
    for (output, mut preparation, expected, skinned) in retained.into_iter().rev() {
        let actual = output.readback()?;
        assert!(output.base_mesh().ptr_eq(&base));
        assert_eq!(actual.indices(), base.indices());
        assert_eq!(actual.vertex_colors(), base.vertex_colors());
        for (index, (actual, expected)) in actual
            .vertices()
            .iter()
            .zip(expected.vertices())
            .enumerate()
        {
            assert_eq!(
                actual.position.map(f32::to_bits),
                expected.position.map(f32::to_bits)
            );
            for (actual, expected) in actual.normal.into_iter().zip(expected.normal) {
                assert!(
                    (actual - expected).abs() < 2e-6,
                    "vertex {index}: {actual} != {expected}"
                );
            }
            assert_eq!(actual.uv, expected.uv);
        }
        for index in 0..base.vertex_count() {
            assert_eq!(actual.uv_at(2, index), base.uv_at(2, index));
        }
        let prepared = preparation.try_read()?.expect("normal preparation pending");
        assert_eq!(prepared.bounds(), expected.bounds());
        assert_eq!(prepared.geometry().uv_sets(), [2; 5]);
        assert!(preparation.try_read().is_err());
        if let Some((skinned, mut preparation, expected)) = skinned {
            let actual = skinned.readback()?;
            for (actual, expected) in actual.vertices().iter().zip(expected.vertices()) {
                for (actual, expected) in actual
                    .position
                    .into_iter()
                    .chain(actual.normal)
                    .zip(expected.position.into_iter().chain(expected.normal))
                {
                    assert!((actual - expected).abs() < 4e-6, "{actual} != {expected}");
                }
            }
            let prepared = preparation.try_read()?.expect("skin preparation pending");
            for (actual, expected) in prepared
                .bounds()
                .min()
                .into_iter()
                .chain(prepared.bounds().max())
                .zip(
                    expected
                        .bounds()
                        .min()
                        .into_iter()
                        .chain(expected.bounds().max()),
                )
            {
                assert!((actual - expected).abs() < 4e-6);
            }
        }
    }
    Ok(())
}
