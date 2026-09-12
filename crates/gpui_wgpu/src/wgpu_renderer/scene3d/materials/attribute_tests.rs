use crate::wgpu_renderer::scene3d::tests::{frame, object};
use crate::{
    Scene3dChannels, Scene3dMaterialProgram, Scene3dMaterialSource, Scene3dOutputConfig,
    Scene3dVertexUpdate as Update, WgpuContext, WgpuScene3dGeometry, WgpuScene3dRenderer,
};
use anyhow::{Context as _, Result};
use gpui::PlatformAtlas as _;
use std::{borrow::Cow, sync::Arc};

#[test]
#[ignore = "requires a compute-capable GPU"]
fn attribute_revisions_and_repairs_preserve_all_render_channels() -> Result<()> {
    let context = WgpuContext::new_headless()?;
    let mut renderer = WgpuScene3dRenderer::new(context.clone())?;
    let tile = renderer
        .sprite_atlas()
        .get_or_insert_with(
            &gpui::RenderImageParams {
                image_id: gpui::ImageId(79013),
                frame_index: 0,
            }
            .into(),
            &mut || {
                Ok(Some((
                    gpui::size(gpui::DevicePixels(1), gpui::DevicePixels(1)),
                    Cow::Borrowed(&[255; 4]),
                )))
            },
        )?
        .unwrap();
    let material = Scene3dMaterialSource::new(
        context.clone(),
        Scene3dMaterialProgram::compile(
            r#"
        fn material_surface(input: SurfaceInput, gradients: mat2x2<f32>) -> vec4<f32> {
            let uv = input.uv.xy * 0.125 + input.uv.zw * 0.25
                + input.detail_uv.xy * 0.5 + input.detail_uv.zw * 0.0625
                + input.occlusion_uv * 0.03125;
            let covered = input.uv.x >= 0.2 && input.detail_uv.z >= 0.2;
            return vec4<f32>(uv * input.color.rg, input.color.b,
                select(0.0, input.color.a, covered));
        }
        fn material_shading(base: vec3<f32>, input: SurfaceInput,
            gradients: SurfaceGradients, face_sign: f32) -> vec3<f32> { return base; }
    "#,
        )?,
    )?
    .bind([], Default::default())?;
    let mut draw = object();
    draw.model[3][2] = 0.5;
    draw.alpha_mode = gpui::AlphaMode3d::Mask;
    draw.custom_material = Some(gpui::MeshMaterial3d::new(Arc::new(material)));
    draw.texture = gpui::MeshTexture3d::Image(tile);
    let map = |uv_set| {
        Some(gpui::MaterialTexture3d {
            tile,
            sampling: Default::default(),
            uv_set,
        })
    };
    draw.metallic_roughness_texture = map(2);
    draw.emissive_texture = map(0);
    draw.normal_texture = map(1);
    draw.occlusion_texture = map(2);
    let base_uv = [[0_f32, 0.], [1., 0.], [0., 1.]];
    let detail_uv = [[0.25_f32, 0.25], [0.75, 0.25], [0.25, 0.75]];
    let old_uv = [[0.25_f32, 0.75]; 3];
    draw.mesh = draw
        .mesh
        .with_uv_set(0, base_uv.to_vec())?
        .with_uv_set(1, detail_uv.to_vec())?
        .with_uv_set(2, old_uv.to_vec())?
        .with_tangents_for_uv_set(1, vec![[1., 0., 0., 1.]; 3])?;
    let base = draw.mesh.clone();
    let source =
        WgpuScene3dGeometry::new(context.clone(), base.clone(), draw.texture_uv_sets(), None)?;
    let buffer = |bytes: &[u8], usage| {
        context.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytes,
            usage,
        })
    };
    let records: Vec<_> = base
        .vertices()
        .iter()
        .map(|vertex| {
            let mut record = [0_f32; 16];
            record[..3].copy_from_slice(&vertex.position);
            record[4..7].copy_from_slice(&vertex.normal);
            record[8] = 1.;
            record[11] = 1.;
            record
        })
        .collect();
    let deformation = buffer(bytemuck::cast_slice(&records), wgpu::BufferUsages::STORAGE);
    let copy_usage = wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST;
    let uv = [[0.75_f32, 0.125], [0.125, 0.875], [0.5, 0.375]];
    let colors = [
        [0.5_f32, 0.75, 0.25, 0.],
        [1., 0.5, 0.75, 1.],
        [0.75, 1., 0.5, 1.],
    ];
    let uv_input = buffer(bytemuck::cast_slice(&old_uv), copy_usage);
    context
        .queue
        .write_buffer(&uv_input, 0, bytemuck::cast_slice(&uv));
    let changed = source.with_attributes(
        &[
            Update::UvBuffer {
                set: 2,
                buffer: &uv_input,
            },
            Update::Color(&colors),
        ],
        None,
    )?;
    context
        .queue
        .write_buffer(&uv_input, 0, bytemuck::cast_slice(&[[f32::NAN, 0.]; 3]));
    let expected = base
        .with_uv_set(2, uv.to_vec())?
        .with_vertex_colors(colors.to_vec())?;
    let next_colors = [[0.25_f32, 0.5, 1., 1.]; 3];
    let color_input = buffer(bytemuck::cast_slice(&next_colors), copy_usage);
    let next = changed.with_attributes(&[Update::ColorBuffer(&color_input)], None)?;
    context.queue.write_buffer(
        &color_input,
        0,
        bytemuck::cast_slice(&[[-0.25_f32, 1., 1., 1.]; 3]),
    );
    let expected_next = expected.with_vertex_colors(next_colors.to_vec())?;
    let invalid_uv = next.with_attributes(
        &[Update::UvBuffer {
            set: 1,
            buffer: &uv_input,
        }],
        None,
    )?;
    let invalid_color = next.with_attributes(&[Update::ColorBuffer(&color_input)], None)?;
    let invalid_both = invalid_uv.with_attributes(&[Update::ColorBuffer(&color_input)], None)?;
    let half_repaired = invalid_both.with_attributes(
        &[Update::Uv {
            set: 1,
            coordinates: &detail_uv,
        }],
        None,
    )?;
    let repaired = half_repaired.with_attributes(&[Update::Color(&next_colors)], None)?;
    let foreign_context = WgpuContext::new_headless()?;
    let foreign = foreign_context.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: 48,
        usage: copy_usage,
        mapped_at_creation: false,
    });
    assert!(
        next.with_attributes(&[Update::ColorBuffer(&foreign)], None)
            .is_err()
    );
    let versions = [
        (source, Some(base.clone())),
        (changed, Some(expected)),
        (next, Some(expected_next.clone())),
        (invalid_uv, None),
        (invalid_color, None),
        (invalid_both, None),
        (half_repaired, None),
        (repaired, Some(expected_next)),
    ];
    let mut frames = Vec::new();
    for (source, expected) in versions {
        let geometry = source.evaluate(&deformation)?;
        let mut gpu = draw.clone();
        gpu.gpu_geometry = Some(gpui::MeshGpuGeometry3d::new(Arc::new(geometry)));
        gpu.render_bounds = Some([[0., 0., 0.], [1., 1., 0.]]);
        let mut gpu = frame(&[gpu]);
        gpu.world_to_view[2][2] = -1.;
        let mut cpu = draw.clone();
        if let Some(mesh) = &expected {
            cpu.mesh = mesh.clone();
        }
        let mut cpu = frame(&if expected.is_some() {
            vec![cpu]
        } else {
            vec![]
        });
        cpu.world_to_view[2][2] = -1.;
        frames.push((gpu, cpu, expected.is_some()));
    }
    drop((
        draw,
        base,
        deformation,
        uv_input,
        color_input,
        foreign,
        foreign_context,
    ));
    let mut outputs = Vec::new();
    for samples in [1, 4] {
        let config = Scene3dOutputConfig {
            size: [64, 64],
            channels: Scene3dChannels::all(),
            color_samples: samples,
        };
        for (gpu, cpu, visible) in frames.iter().rev().chain(frames.iter()) {
            outputs.push((
                renderer.render(gpu, config)?,
                renderer.render(cpu, config)?,
                *visible,
            ));
        }
    }
    renderer.clear_caches();
    drop((renderer, frames));
    for (gpu, cpu, visible) in outputs {
        let mut actual = gpu.readback()?;
        context.device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_secs(15)),
        })?;
        let actual = actual.try_read()?.context("attribute output not ready")?;
        let mut expected = cpu.readback()?;
        context.device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_secs(15)),
        })?;
        let expected = expected.try_read()?.context("reference output not ready")?;
        assert_eq!(actual.object_ids, expected.object_ids);
        assert_eq!(actual.linear_depth, expected.linear_depth);
        assert_eq!(actual.world_normals, expected.world_normals);
        assert_eq!(actual.rgba, expected.rgba);
        assert_eq!(actual.linear_rgba, expected.linear_rgba);
        if visible {
            assert!(
                actual
                    .object_ids
                    .as_ref()
                    .unwrap()
                    .iter()
                    .filter(|id| **id == 1)
                    .count()
                    > 100
            );
        } else {
            assert!(
                actual
                    .object_ids
                    .as_ref()
                    .unwrap()
                    .iter()
                    .all(|id| *id == 0)
            );
        }
    }
    Ok(())
}
