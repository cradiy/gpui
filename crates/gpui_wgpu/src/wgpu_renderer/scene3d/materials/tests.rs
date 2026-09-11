use super::*;
use crate::wgpu_renderer::scene3d::tests::{frame, object};
use crate::{
    Scene3dChannels, Scene3dMaterialBindingLimits, Scene3dMaterialProgram, Scene3dMaterialValue,
    Scene3dOutputConfig, Scene3dPixels, WgpuContext, WgpuScene3dRenderer,
};
use std::sync::Arc;

const MATERIAL: &str = r#"
struct Controls { color: vec4<f32>, crop: vec4<f32> }
@group(1) @binding(0) var<uniform> controls: Controls;
fn material_surface(input: SurfaceInput, gradients: mat2x2<f32>) -> vec4<f32> {
    return vec4<f32>(controls.color.rgb, select(0.0, 1.0, input.world.x < controls.crop.x));
}
fn material_shading(base: vec3<f32>, input: SurfaceInput,
    gradients: SurfaceGradients, face_sign: f32) -> vec3<f32> {
    return base;
}
"#;

#[test]
fn scene3d_custom_surface_program_is_valid_and_requires_typed_snapshots() {
    let program = Scene3dMaterialProgram::compile(MATERIAL).unwrap();
    assert!(program.resources()[0].coverage);
    let mut object = object();
    object.custom_material = Some(gpui::MeshMaterial3d::new(Arc::new(())));
    assert!(snapshot(&object).is_err());
}

#[test]
#[ignore = "requires a GPU adapter"]
fn scene3d_custom_material_preserves_coverage_and_retained_uniforms() -> Result<()> {
    let context = WgpuContext::new_headless()?;
    let source =
        Scene3dMaterialSource::new(context.clone(), Scene3dMaterialProgram::compile(MATERIAL)?)?;
    let limits = Scene3dMaterialBindingLimits::default();
    let value = |red, blue, crop| {
        Scene3dMaterialValue::Uniform(
            bytemuck::cast_slice(&[red, 0.0_f32, blue, 1.0, crop, 0.0, 0.0, 0.0]).into(),
        )
    };
    let original = source.bind([(0, value(1.0, 0.0, 0.3))], limits)?;
    let changed = original.with_values([(0, value(0.0, 1.0, 0.8))], limits)?;
    let mut object = object();
    object.model[3][2] = 0.5;
    object.alpha_mode = gpui::AlphaMode3d::Mask;
    object.alpha_cutoff = 0.5;
    object.custom_material = Some(gpui::MeshMaterial3d::new(Arc::new(original.clone())));
    let mut input = frame(&[object]);
    input.world_to_view[2][2] = -1.0;
    let retained = input.clone();
    Arc::make_mut(&mut input.objects)[0].custom_material =
        Some(gpui::MeshMaterial3d::new(Arc::new(changed)));

    let mut cache = MaterialCache::default();
    cache.prepare(
        &context.device,
        &[&retained],
        wgpu::TextureFormat::Rgba8Unorm,
        1,
    )?;
    let pipeline = cache.get(&original).mesh.clone();
    cache.prepare(
        &context.device,
        &[&input],
        wgpu::TextureFormat::Rgba8Unorm,
        1,
    )?;
    assert_eq!(cache.get(&original).mesh, pipeline);
    let foreign = WgpuContext::new_headless()?;
    assert!(
        cache
            .prepare(
                &foreign.device,
                &[&input],
                wgpu::TextureFormat::Rgba8Unorm,
                1
            )
            .is_err()
    );

    let mut renderer = WgpuScene3dRenderer::new(context.clone())?;
    for samples in [1, 4] {
        let config = Scene3dOutputConfig {
            size: [64, 64],
            channels: Scene3dChannels::all(),
            color_samples: samples,
        };
        let old = renderer.render(&retained, config)?;
        let new = renderer.render(&input, config)?;
        for (output, changed) in [(old, false), (new, true)] {
            let mut pending = output.readback()?;
            context.device.poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(std::time::Duration::from_secs(10)),
            })?;
            let pixels = pending.try_read()?.context("material readback not ready")?;
            assert_pixel(&pixels, 36, true, changed);
            assert_pixel(&pixels, 48, changed, changed);
        }
    }
    cache.prepare(&context.device, &[], wgpu::TextureFormat::Rgba8Unorm, 1)?;
    assert!(cache.0.is_empty());
    Ok(())
}

fn assert_pixel(pixels: &Scene3dPixels, x: usize, covered: bool, blue: bool) {
    let index = 24 * 64 + x;
    assert_eq!(
        pixels.object_ids.as_ref().unwrap()[index],
        u32::from(covered)
    );
    let rgba = pixels.linear_rgba.as_ref().unwrap()[index];
    let expected = if !covered {
        [0.0; 4]
    } else if blue {
        [0.0, 0.0, 1.0, 1.0]
    } else {
        [1.0, 0.0, 0.0, 1.0]
    };
    for (actual, expected) in rgba.into_iter().zip(expected) {
        assert!((actual - expected).abs() < 0.001);
    }
    let normal = pixels.world_normals.as_ref().unwrap()[index];
    assert_eq!(normal[3], f32::from(covered));
    if covered {
        assert!((pixels.linear_depth.as_ref().unwrap()[index] - 0.5).abs() < 0.001);
        assert_eq!(&normal[..3], &[0.0, 0.0, 1.0]);
    }
}

#[test]
#[ignore = "requires a GPU adapter"]
fn scene3d_custom_vertex_streams_preserve_versions_across_mesh_and_gpu_draws() -> Result<()> {
    use crate::{Scene3dVertexAttribute, WgpuScene3dGeometry};
    use wgpu::util::DeviceExt as _;
    let context = WgpuContext::new_headless()?;
    let program = Scene3dMaterialProgram::compile_with_attributes(
        r#"
        struct Controls { tint: vec4<f32> }
        @group(1) @binding(0) var<uniform> controls: Controls;
        fn material_surface(input: SurfaceInput, gradients: mat2x2<f32>) -> vec4<f32> {
            return vec4<f32>(input.attributes.tint, input.attributes.coverage) * controls.tint;
        }
        fn material_shading(base: vec3<f32>, input: SurfaceInput, gradients: SurfaceGradients, face_sign: f32) -> vec3<f32> {
            return base;
        }
    "#,
        &[
            Scene3dVertexAttribute::new("tint", wgpu::VertexFormat::Float32x3),
            Scene3dVertexAttribute::new("coverage", wgpu::VertexFormat::Float32),
        ],
    )?;
    let source = Scene3dMaterialSource::new(context.clone(), program)?;
    let red = [[1_f32, 0., 0.]; 3];
    let blue = [[0_f32, 0., 1.]; 3];
    let streams = source.bind_vertex_streams(
        3,
        &[
            ("tint", bytemuck::cast_slice(&red)),
            ("coverage", bytemuck::cast_slice(&[1_f32; 3])),
        ],
        48,
    )?;
    let blue_streams = streams.with_values(&[("tint", bytemuck::cast_slice(&blue))], 48)?;
    let hidden_streams =
        blue_streams.with_values(&[("coverage", bytemuck::cast_slice(&[0_f32; 3]))], 48)?;
    let limits = Scene3dMaterialBindingLimits::default();
    let original = source
        .bind(
            [(
                0,
                Scene3dMaterialValue::Uniform(bytemuck::cast_slice(&[1_f32; 4]).into()),
            )],
            limits,
        )?
        .with_vertex_streams(streams)?;
    let recolored = original.with_vertex_streams(blue_streams)?;
    let dimmed = recolored.with_values(
        [(
            0,
            Scene3dMaterialValue::Uniform(bytemuck::cast_slice(&[0.5_f32, 0.5, 0.5, 1.]).into()),
        )],
        limits,
    )?;
    let hidden = recolored.with_vertex_streams(hidden_streams)?;
    let mut object = object();
    object.model[3][2] = 0.5;
    object.alpha_mode = gpui::AlphaMode3d::Mask;
    let mut records = Vec::<[f32; 16]>::new();
    for vertex in object.mesh.vertices() {
        let mut record = [0.; 16];
        record[..3].copy_from_slice(&vertex.position);
        record[4..7].copy_from_slice(&vertex.normal);
        records.push(record);
    }
    let attributes = context
        .device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::cast_slice(&records),
            usage: wgpu::BufferUsages::STORAGE,
        });
    let geometry = Arc::new(
        WgpuScene3dGeometry::new(context.clone(), object.mesh.clone(), [0; 5], None)?
            .evaluate(&attributes)?,
    );
    let mut renderer = WgpuScene3dRenderer::new(context.clone())?;
    for gpu in [false, true] {
        object.gpu_geometry = gpu.then(|| gpui::MeshGpuGeometry3d::new(geometry.clone()));
        object.render_bounds = gpu.then_some([[0., 0., 0.], [1., 1., 0.]]);
        for (material, expected) in [
            (&original, [1., 0., 0., 1.]),
            (&recolored, [0., 0., 1., 1.]),
            (&dimmed, [0., 0., 0.5, 1.]),
            (&hidden, [0.; 4]),
        ] {
            object.custom_material = Some(gpui::MeshMaterial3d::new(Arc::new(material.clone())));
            let mut input = frame(&[object.clone()]);
            input.world_to_view[2][2] = -1.;
            let output = renderer.render(
                &input,
                Scene3dOutputConfig {
                    size: [64, 64],
                    channels: Scene3dChannels::all(),
                    color_samples: 4,
                },
            )?;
            let mut pending = output.readback()?;
            context.device.poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(std::time::Duration::from_secs(10)),
            })?;
            let pixels = pending
                .try_read()?
                .context("vertex stream readback not ready")?;
            let index = 24 * 64 + 36;
            let covered = expected[3] > 0.;
            assert_eq!(
                pixels.object_ids.as_ref().unwrap()[index],
                u32::from(covered)
            );
            for (actual, expected) in pixels.linear_rgba.as_ref().unwrap()[index]
                .into_iter()
                .zip(expected)
            {
                assert!((actual - expected).abs() < 0.001);
            }
            assert_eq!(
                pixels.world_normals.as_ref().unwrap()[index][3],
                f32::from(covered)
            );
            assert_eq!(
                pixels.rgba.as_ref().unwrap()[index * 4 + 3],
                if covered { 255 } else { 0 }
            );
            let depth = pixels.linear_depth.as_ref().unwrap()[index];
            if covered {
                assert!((depth - 0.5).abs() < 0.001);
            } else {
                assert!(pixels.depth_background.is_background(depth));
            }
        }
    }
    Ok(())
}
