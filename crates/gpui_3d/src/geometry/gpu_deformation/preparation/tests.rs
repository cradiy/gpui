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
    for weight in [-1., 0.5, 2.] {
        let output = morph.evaluate(&[weight])?;
        let request = output.prepare_render_geometry(&source, &bounds, Some(bytes))?;
        assert_eq!(request.working_bytes(), bytes);
        requests.push((request, morphs.evaluate(&[weight])?.bounds()));
    }
    let colors = context.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: None,
        contents: bytemuck::cast_slice(&vec![[1., 1., 1., -0.5]; mesh.vertex_count()]),
        usage: wgpu::BufferUsages::COPY_SRC,
    });
    let invalid_source =
        source.with_attributes(&[Scene3dVertexUpdate::ColorBuffer(&colors)], None)?;
    let mut invalid = initial.prepare_render_geometry(&invalid_source, &bounds, None)?;
    drop(invalid_source);
    drop(initial);
    drop(source);
    drop(morph);
    drop(bounds);
    context.device.poll(wgpu::PollType::Wait {
        submission_index: None,
        timeout: Some(std::time::Duration::from_secs(30)),
    })?;
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
