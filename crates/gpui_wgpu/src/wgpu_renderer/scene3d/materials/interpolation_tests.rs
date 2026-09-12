use crate::wgpu_renderer::scene3d::tests::{frame, object};
use crate::{
    Scene3dChannels, Scene3dMaterialProgram, Scene3dMaterialSource, Scene3dMaterialValue,
    Scene3dOutputConfig, Scene3dVertexAttribute, Scene3dVertexInterpolation as Interpolation,
    Scene3dVertexStreamValue::Bytes, WgpuContext, WgpuScene3dGeometry, WgpuScene3dRenderer,
};
use anyhow::{Context as _, Result};
use std::sync::Arc;

fn values(
    tags: [u32; 3],
    signed: [i32; 3],
    flat: [f32; 2],
    cutoffs: [f32; 2],
) -> [(u32, Scene3dMaterialValue); 1] {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(bytemuck::cast_slice(&[tags[0], tags[1], tags[2], 0]));
    bytes.extend_from_slice(bytemuck::cast_slice(&[signed[0], signed[1], signed[2], 0]));
    bytes.extend_from_slice(bytemuck::cast_slice(&[
        flat[0], flat[1], cutoffs[0], cutoffs[1],
    ]));
    [(0, Scene3dMaterialValue::Uniform(bytes.into()))]
}

#[test]
#[ignore = "requires a GPU with five vertex storage bindings"]
fn custom_interpolation_and_packed_integer_streams_match_projected_triangles() -> Result<()> {
    let context = WgpuContext::new_headless()?;
    let source = Scene3dMaterialSource::new(
        context.clone(),
        Scene3dMaterialProgram::compile_with_attributes(
            r#"
        struct Expected { tags: vec4<u32>, signed_tags: vec4<i32>, constant_value: vec4<f32> }
        @group(1) @binding(0) var<uniform> expected: Expected;
        fn material_surface(input: SurfaceInput, gradients: mat2x2<f32>) -> vec4<f32> {
            let weights = vec3<f32>(1.0, 0.5, 0.25);
            let interpolated = vec2<f32>(dot(input.attributes.perspective_value, weights),
                dot(input.attributes.linear_value, weights));
            let covered = all(interpolated >= expected.constant_value.zw)
                && all(input.attributes.tags == expected.tags.xyz)
                && all(input.attributes.signed_tags == expected.signed_tags.xyz)
                && all(input.attributes.constant_value == expected.constant_value.xy);
            return vec4<f32>(interpolated, input.attributes.constant_value.x,
                select(0.0, 1.0, covered));
        }
        fn material_shading(base: vec3<f32>, input: SurfaceInput,
            gradients: SurfaceGradients, face_sign: f32) -> vec3<f32> { return base; }
        "#,
            &[
                Scene3dVertexAttribute::new("perspective_value", wgpu::VertexFormat::Float32x3),
                Scene3dVertexAttribute::new("linear_value", wgpu::VertexFormat::Float32x3)
                    .interpolation(Interpolation::Linear),
                Scene3dVertexAttribute::new("tags", wgpu::VertexFormat::Uint32x3),
                Scene3dVertexAttribute::new("signed_tags", wgpu::VertexFormat::Sint32x3),
                Scene3dVertexAttribute::new("constant_value", wgpu::VertexFormat::Float32x2)
                    .interpolation(Interpolation::Flat),
            ],
        )?,
    )?;
    let smooth = [[1_f32, 0., 0.], [0., 1., 0.], [0., 0., 1.]];
    let tags = [
        [0x80000001_u32, u32::MAX, 17],
        [19, 0xf0000001, 23],
        [29, 31, 0x80000003],
    ];
    let signed = [
        [-2_000_000_001_i32, -1, 257],
        [-33, 513, i32::MIN],
        [i32::MAX, -1025, -7],
    ];
    let flat = [[0.25_f32, 0.5], [0.5, 0.75], [0.75, 1.]];
    let streams = source.bind_vertex_streams(
        3,
        &[
            ("perspective_value", Bytes(bytemuck::cast_slice(&smooth))),
            ("linear_value", Bytes(bytemuck::cast_slice(&smooth))),
            ("tags", Bytes(bytemuck::cast_slice(&tags))),
            ("signed_tags", Bytes(bytemuck::cast_slice(&signed))),
            ("constant_value", Bytes(bytemuck::cast_slice(&flat))),
        ],
        168,
    )?;
    let original = source
        .bind(
            values(tags[0], signed[0], flat[0], [0.; 2]),
            Default::default(),
        )?
        .with_vertex_streams(streams)?;
    let positions = [[-0.75_f32, -0.75, 1.], [1.5, -1.5, 2.], [0., 3., 4.]];
    let mut submissions = Vec::new();
    for first in 0..3 {
        let mut object = object();
        object.alpha_mode = gpui::AlphaMode3d::Mask;
        object.mesh = gpui::Mesh3d::new(
            positions
                .map(|position| gpui::MeshVertex3d {
                    position,
                    normal: [0., 0., 1.],
                    uv: [0.; 2],
                })
                .to_vec(),
            vec![
                first as u32,
                ((first + 1) % 3) as u32,
                ((first + 2) % 3) as u32,
            ],
        );
        let mut records = Vec::new();
        for position in positions {
            let mut record = [0_f32; 16];
            record[..3].copy_from_slice(&[
                position[0] + 0.125 * position[2],
                position[1],
                position[2],
            ]);
            record[6] = 1.;
            records.push(record);
        }
        let buffer = context.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::cast_slice(&records),
            usage: wgpu::BufferUsages::STORAGE,
        });
        let geometry = Arc::new(
            WgpuScene3dGeometry::new(context.clone(), object.mesh.clone(), [0; 5], None)?
                .evaluate(&buffer)?,
        );
        for gpu in [false, true] {
            object.gpu_geometry = gpu.then(|| gpui::MeshGpuGeometry3d::new(geometry.clone()));
            object.render_bounds = gpu.then_some([[-0.625, -1.5, 1.], [1.75, 3., 4.]]);
            for (visible, cutoffs) in [
                (false, [0., 0.]),
                (true, [0., 0.]),
                (true, [0.65, 0.]),
                (true, [0., 0.65]),
            ] {
                let mut expected = tags[first];
                if !visible {
                    expected[0] ^= 1;
                }
                let material = original.with_values(
                    values(expected, signed[first], flat[first], cutoffs),
                    Default::default(),
                )?;
                object.custom_material = Some(gpui::MeshMaterial3d::new(Arc::new(material)));
                let mut input = frame(&[object.clone()]);
                input.view_projection = [
                    [1., 0., 0., 0.],
                    [0., 1., 0., 0.],
                    [0., 0., 1., 1.],
                    [0., 0., -0.5, 0.],
                ];
                input.world_to_view[2][2] = -1.;
                submissions.push((input, first, gpu, visible, cutoffs));
            }
        }
    }
    let mut renderer = WgpuScene3dRenderer::new(context.clone())?;
    for samples in [1, 4] {
        for (input, first, gpu, visible, cutoffs) in &submissions {
            let output = renderer.render(
                input,
                Scene3dOutputConfig {
                    size: [64, 64],
                    channels: Scene3dChannels::all(),
                    color_samples: samples,
                },
            )?;
            let mut pending = output.readback()?;
            context.device.poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(std::time::Duration::from_secs(15)),
            })?;
            let pixels = pending
                .try_read()?
                .context("interpolation readback not ready")?;
            let mut checked = 0;
            let mut separated = 0;
            let mut covered_count = 0;
            for y in 0..64 {
                for x in 0..64 {
                    let nx = (x as f32 + 0.5) / 32. - 1. - if *gpu { 0.125 } else { 0. };
                    let ny = 1. - (y as f32 + 0.5) / 32.;
                    let c = (ny + 0.75) / 1.5;
                    let b = (nx + 0.75 - c * 0.75) / 1.5;
                    let a = 1. - b - c;
                    if a.min(b).min(c) < 0.05 {
                        continue;
                    }
                    let denominator = a + b / 2. + c / 4.;
                    let smooth = (a + b / 4. + c / 16.) / denominator;
                    let linear = a + b * 0.5 + c * 0.25;
                    if (smooth - cutoffs[0]).abs() < 0.01 || (linear - cutoffs[1]).abs() < 0.01 {
                        continue;
                    }
                    let covered = *visible && smooth >= cutoffs[0] && linear >= cutoffs[1];
                    let index = y * 64 + x;
                    let color = pixels.linear_rgba.as_ref().unwrap()[index];
                    assert_eq!(
                        pixels.object_ids.as_ref().unwrap()[index],
                        u32::from(covered),
                        "first {first}, gpu {gpu}, samples {samples}, cutoffs {cutoffs:?}, pixel {x},{y}"
                    );
                    assert_eq!(
                        pixels.rgba.as_ref().unwrap()[index * 4 + 3],
                        if covered { 255 } else { 0 }
                    );
                    if covered {
                        covered_count += 1;
                        let expected = [smooth, linear, flat[*first][0], 1.];
                        for (actual, expected) in color.iter().zip(expected) {
                            assert!(
                                (actual - expected).abs() < 0.001,
                                "first {first}, gpu {gpu}, {x},{y}: {color:?} != {expected}"
                            );
                        }
                        assert!(
                            (pixels.linear_depth.as_ref().unwrap()[index] - 1. / denominator).abs()
                                < 0.001
                        );
                        assert_eq!(
                            pixels.world_normals.as_ref().unwrap()[index],
                            [0., 0., 1., 1.]
                        );
                        if (smooth - linear).abs() > 0.05 {
                            separated += 1;
                        }
                    } else {
                        assert_eq!(color, [0.; 4]);
                        assert_eq!(pixels.linear_depth.as_ref().unwrap()[index], 0.);
                        assert_eq!(pixels.world_normals.as_ref().unwrap()[index], [0.; 4]);
                    }
                    checked += 1;
                }
            }
            assert!(checked > 500);
            if *visible {
                assert!(separated > 100);
                if cutoffs != &[0.; 2] {
                    assert!(checked - covered_count > 100);
                }
            }
        }
    }
    Ok(())
}
