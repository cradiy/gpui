use crate::wgpu_renderer::scene3d::tests::{frame, object};
use crate::{
    Scene3dChannels, Scene3dMaterialProgram, Scene3dMaterialSource, Scene3dMaterialValue,
    Scene3dOutputConfig, WgpuContext, WgpuResource, WgpuScene3dRenderer,
};
use anyhow::{Context as _, Result};
use std::sync::Arc;

fn texture(context: &WgpuContext, pixels: &[u8; 8]) -> WgpuResource<wgpu::Texture> {
    let texture = context.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width: 2,
            height: 1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[wgpu::TextureFormat::Rgba8UnormSrgb],
    });
    context.queue.write_texture(
        texture.as_image_copy(),
        pixels,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(8),
            rows_per_image: Some(1),
        },
        texture.size(),
    );
    texture
}

#[test]
#[ignore = "requires a GPU adapter"]
fn material_texture_sampler_and_uniform_updates_preserve_retained_pixels_and_coverage() -> Result<()>
{
    let context = WgpuContext::new_headless()?;
    let source = Scene3dMaterialSource::new(
        context.clone(),
        Scene3dMaterialProgram::compile(
            r#"
        struct Controls { tint: vec4<f32> }
        @group(1) @binding(0) var<uniform> controls: Controls;
        @group(1) @binding(1) var picture: texture_2d<f32>;
        @group(1) @binding(2) var picture_sampler: sampler;
        fn material_surface(input: SurfaceInput, gradients: mat2x2<f32>) -> vec4<f32> {
            return textureSampleLevel(picture, picture_sampler,
                vec2<f32>(input.world.x * 2.0, 0.5), 0.0) * controls.tint;
        }
        fn material_shading(base: vec3<f32>, input: SurfaceInput,
            gradients: SurfaceGradients, face_sign: f32) -> vec3<f32> { return base; }
    "#,
        )?,
    )?;
    let image = texture(&context, &[128, 64, 32, 255, 0, 128, 255, 0]);
    let replacement = texture(&context, &[255, 0, 0, 0, 32, 64, 128, 255]);
    let tint = |gain: f32| {
        Scene3dMaterialValue::Uniform(bytemuck::cast_slice(&[gain, gain, gain, 1.]).into())
    };
    let limits = Default::default();
    let original = source.bind(
        [
            (0, tint(1.)),
            (
                1,
                Scene3dMaterialValue::Texture(image.create_view(&Default::default())),
            ),
            (
                2,
                Scene3dMaterialValue::Sampler(context.create_sampler(&Default::default())),
            ),
        ],
        limits,
    )?;
    let dimmed = original.with_values([(0, tint(0.5))], limits)?;
    let srgb = dimmed.with_values(
        [(
            1,
            Scene3dMaterialValue::Texture(image.create_view(&wgpu::TextureViewDescriptor {
                format: Some(wgpu::TextureFormat::Rgba8UnormSrgb),
                ..Default::default()
            })),
        )],
        limits,
    )?;
    let repeated = srgb.with_values(
        [(
            2,
            Scene3dMaterialValue::Sampler(context.create_sampler(&wgpu::SamplerDescriptor {
                address_mode_u: wgpu::AddressMode::Repeat,
                ..Default::default()
            })),
        )],
        limits,
    )?;
    let replaced = repeated.with_values(
        [(
            1,
            Scene3dMaterialValue::Texture(replacement.create_view(&Default::default())),
        )],
        limits,
    )?;

    let wrong = image.create_view(&wgpu::TextureViewDescriptor {
        dimension: Some(wgpu::TextureViewDimension::D2Array),
        ..Default::default()
    });
    assert!(
        repeated
            .with_values(
                [(0, tint(0.)), (1, Scene3dMaterialValue::Texture(wrong))],
                limits
            )
            .is_err()
    );
    drop(image);
    drop(replacement);
    drop(source);

    let linear = [128_f32 / 255., 64. / 255., 32. / 255.];
    let decoded = linear.map(|value| ((value + 0.055) / 1.055).powf(2.4) * 0.5);
    let snapshots = [
        (original.clone(), [true, false, false], linear),
        (
            dimmed,
            [true, false, false],
            linear.map(|value| value * 0.5),
        ),
        (srgb, [true, false, false], decoded),
        (repeated, [true, false, true], decoded),
        (
            replaced,
            [false, true, false],
            [16. / 255., 32. / 255., 64. / 255.],
        ),
        (original, [true, false, false], linear),
    ];
    let mut renderer = WgpuScene3dRenderer::new(context.clone())?;
    let mut outputs = Vec::new();
    for samples in [1, 4] {
        for (snapshot, covered, expected) in &snapshots {
            let mut object = object();
            object.model[3][2] = 0.5;
            object.alpha_mode = gpui::AlphaMode3d::Mask;
            object.custom_material = Some(gpui::MeshMaterial3d::new(Arc::new(snapshot.clone())));
            let mut input = frame(&[object]);
            input.world_to_view[2][2] = -1.;
            outputs.push((
                renderer.render(
                    &input,
                    Scene3dOutputConfig {
                        size: [64, 64],
                        channels: Scene3dChannels::all(),
                        color_samples: samples,
                    },
                )?,
                *covered,
                *expected,
            ));
        }
        renderer.clear_caches();
    }
    drop(snapshots);
    drop(renderer);
    for (output, covered, expected) in outputs {
        let mut pending = output.readback()?;
        context.device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_secs(15)),
        })?;
        let pixels = pending
            .try_read()?
            .context("material texture readback not ready")?;
        for (x, covered) in [38, 46, 54].into_iter().zip(covered) {
            let index = 28 * 64 + x;
            assert_eq!(
                pixels.object_ids.as_ref().unwrap()[index],
                u32::from(covered)
            );
            let color = pixels.linear_rgba.as_ref().unwrap()[index];
            if covered {
                for (actual, expected) in color[..3].iter().zip(expected) {
                    assert!(
                        (actual - expected).abs() < 0.001,
                        "x={x}: {color:?}, expected {expected}"
                    );
                }
                assert_eq!(color[3], 1.);
                assert!((pixels.linear_depth.as_ref().unwrap()[index] - 0.5).abs() < 0.001);
                assert_eq!(
                    pixels.world_normals.as_ref().unwrap()[index],
                    [0., 0., 1., 1.]
                );
            } else {
                assert_eq!(color, [0.; 4]);
                assert_eq!(pixels.linear_depth.as_ref().unwrap()[index], 0.);
                assert_eq!(pixels.world_normals.as_ref().unwrap()[index], [0.; 4]);
            }
        }
    }
    Ok(())
}
