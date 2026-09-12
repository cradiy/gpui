use super::*;
use crate::{GpuDeformationLimits, GpuMorph, Mesh, MorphTarget, MorphTargets, Scene3dVertexUpdate};
use gpui_wgpu::{WgpuContext, wgpu};

#[test]
#[ignore = "requires a supported GPU adapter"]
fn prepared_geometry_pairs_retained_bounds_and_rejects_invalid_attributes() -> Result<()> {
    let context = WgpuContext::new_headless()?;
    let mesh = Mesh::cube();
    let morphs = MorphTargets::new(
        mesh.clone(),
        [MorphTarget {
            positions: Some(vec![[2., -4., 6.]; mesh.vertex_count()].into()),
            ..Default::default()
        }],
    )?;
    let morph = GpuMorph::new(
        context.clone(),
        morphs.clone(),
        GpuDeformationLimits::default(),
    )?;
    let bounds = GpuDeformationBounds::new(context.clone())?;
    let initial = morph.evaluate(&[0.])?;
    let source = initial.render_source([0; 5], None)?;
    let bytes = source.memory().vertex_bytes + source.memory().draw_bytes + 96;
    assert!(
        initial
            .prepare_render_geometry(&source, &bounds, Some(bytes - 1))
            .is_err()
    );
    drop(initial.prepare_render_geometry(&source, &bounds, Some(bytes))?);
    let mut requests = Vec::new();
    let mut outputs = Vec::new();
    for weight in [-1., 0.5, 2.] {
        let output = morph.evaluate(&[weight])?;
        let request = output.prepare_render_geometry(&source, &bounds, Some(bytes))?;
        assert_eq!(request.working_bytes(), bytes);
        requests.push((request, morphs.evaluate(&[weight])?.bounds()));
        outputs.push(output);
    }
    let inputs: Vec<_> = outputs.iter().map(|output| (output, &source)).collect();
    let batch_bytes = bytes * inputs.len() as u64;
    assert!(GpuGeometryBatchPreparation::new(&inputs, &bounds, Some(batch_bytes - 1)).is_err());
    let mut batch = GpuGeometryBatchPreparation::new(&inputs, &bounds, Some(batch_bytes))?;
    assert_eq!(batch.working_bytes(), batch_bytes);
    let mut empty = GpuGeometryBatchPreparation::new(&[], &bounds, Some(0))?;
    assert!(empty.try_read()?.unwrap().is_empty());
    assert!(empty.try_read().is_err());
    let different =
        GpuDeformationOutput::upload(context.clone(), Mesh::plane(), Default::default())?;
    let different_source = different.render_source([0; 5], None)?;
    let error = GpuGeometryBatchPreparation::new(
        &[(&initial, &source), (&initial, &different_source)],
        &bounds,
        None,
    )
    .err()
    .unwrap();
    assert_eq!(error.to_string(), "GPU preparation input 1");
    assert!(format!("{error:#}").contains("source mesh mismatch"));
    let colors = context.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: None,
        contents: bytemuck::cast_slice(&vec![[1_f32, 1., 1., -0.5]; mesh.vertex_count()]),
        usage: wgpu::BufferUsages::COPY_SRC,
    });
    let invalid_source =
        source.with_attributes(&[Scene3dVertexUpdate::ColorBuffer(&colors)], None)?;
    let mut invalid = initial.prepare_render_geometry(&invalid_source, &bounds, None)?;
    let mut invalid_batch = GpuGeometryBatchPreparation::new(
        &[(&initial, &source), (&initial, &invalid_source)],
        &bounds,
        None,
    )?;
    drop(outputs);
    drop(invalid_source);
    drop(initial);
    drop(source);
    drop(morph);
    drop(bounds);
    context.device.poll(wgpu::PollType::Wait {
        submission_index: None,
        timeout: Some(std::time::Duration::from_secs(30)),
    })?;
    let prepared = batch.try_read()?.context("batch is still pending")?;
    assert_eq!(prepared.len(), requests.len());
    for (prepared, (_, expected)) in prepared.iter().zip(&requests) {
        assert_eq!(prepared.bounds(), *expected);
    }
    assert!(batch.try_read().is_err());
    assert_eq!(batch.working_bytes(), batch_bytes);
    let error = invalid_batch
        .try_read()
        .err()
        .context("invalid batch was admitted")?;
    assert_eq!(error.to_string(), "GPU preparation input 1");
    assert!(format!("{error:#}").contains("INVALID_COLOR"));
    assert!(invalid_batch.try_read().is_err());
    for (mut request, expected_bounds) in requests {
        let prepared = request
            .try_read()?
            .context("preparation is still pending")?;
        assert_eq!(prepared.bounds(), expected_bounds);
        assert!(Arc::ptr_eq(prepared.geometry().base_mesh(), &mesh.0));
        let retained = prepared.clone();
        assert!(Arc::ptr_eq(prepared.geometry(), retained.geometry()));
        assert!(request.try_read().is_err());
    }
    let error = match invalid.try_read() {
        Err(error) => error,
        Ok(_) => anyhow::bail!("invalid geometry was admitted"),
    };
    assert!(error.to_string().contains("INVALID_COLOR"), "{error:#}");
    assert!(invalid.try_read().is_err());
    Ok(())
}
