use anyhow::Result;
use gpui::{EffectShader, EffectTextureOptions, EffectUniforms};
use gpui_wgpu::{TextureEffectConfig, WgpuContext, WgpuTextureEffect, wgpu};

fn upload(
    context: &WgpuContext,
    size: [u32; 2],
    format: wgpu::TextureFormat,
    data: &[f32],
) -> wgpu::Texture {
    let texture = context.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("texture effect input"),
        size: wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    context.queue.write_texture(
        texture.as_image_copy(),
        bytemuck::cast_slice(data),
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(size[0] * format.block_copy_size(None).unwrap()),
            rows_per_image: Some(size[1]),
        },
        texture.size(),
    );
    texture
}

fn read(context: &WgpuContext, texture: &wgpu::Texture) -> Result<Vec<[f32; 4]>> {
    assert_eq!(texture.format(), wgpu::TextureFormat::Rgba32Float);
    let stride = (texture.width() * 16).div_ceil(256) * 256;
    let buffer = context.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("texture effect readback"),
        size: u64::from(stride) * u64::from(texture.height()),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = context.device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(stride),
                rows_per_image: Some(texture.height()),
            },
        },
        texture.size(),
    );
    context.queue.submit(Some(encoder.finish()));
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    buffer.map_async(wgpu::MapMode::Read, .., move |result| {
        let _ = tx.send(result);
    });
    context.device.poll(wgpu::PollType::Wait {
        submission_index: None,
        timeout: Some(std::time::Duration::from_secs(10)),
    })?;
    rx.recv_timeout(std::time::Duration::from_secs(10))??;
    let mapped = buffer.get_mapped_range(..)?;
    let mut pixels = Vec::new();
    for row in mapped.chunks_exact(stride as usize) {
        for pixel in row[..texture.width() as usize * 16].chunks_exact(16) {
            pixels.push(std::array::from_fn(|i| {
                f32::from_le_bytes(pixel[i * 4..i * 4 + 4].try_into().unwrap())
            }));
        }
    }
    drop(mapped);
    buffer.unmap();
    Ok(pixels)
}

fn near(actual: [f32; 4], expected: [f32; 4]) {
    for (a, b) in actual.into_iter().zip(expected) {
        assert!((a - b).abs() < 0.0001, "{actual:?} != {expected:?}");
    }
}

#[test]
#[ignore = "requires a GPU adapter"]
fn hdr_depth_processing_preserves_texels_alpha_and_owned_results() -> Result<()> {
    let context = WgpuContext::new_headless()?;
    let color = upload(
        &context,
        [3, 2],
        wgpu::TextureFormat::Rgba32Float,
        &[2., 0.5, 0.25, 0.5].repeat(6),
    );
    let depth = upload(
        &context,
        [3, 2],
        wgpu::TextureFormat::R32Float,
        &[0., 2., 4., 6., 8., 10.],
    );
    let config = TextureEffectConfig {
        output_format: wgpu::TextureFormat::Rgba32Float,
        ..Default::default()
    };
    let identity = WgpuTextureEffect::new(
        context.clone(),
        &gpui_effects::subtree_identity_shader(),
        config.clone(),
    )?;
    let retained = identity.render(&[&color], [3, 2], EffectUniforms::default(), 0.)?;
    let texels = [[0.2, 0., 0., 0.2], [0., 0.4, 0., 0.4], [0., 0., 2., 0.5]];
    let varied = upload(
        &context,
        [3, 1],
        wgpu::TextureFormat::Rgba32Float,
        texels.as_flattened(),
    );
    let exact = identity.render(&[&varied], [3, 1], EffectUniforms::default(), 0.)?;
    let fog = WgpuTextureEffect::new(
        context.clone(),
        &gpui_effects::depth_fog_shader(),
        TextureEffectConfig {
            inputs: vec![
                config.inputs[0],
                EffectTextureOptions {
                    premultiplied_alpha: false,
                    nearest: true,
                },
            ],
            ..config.clone()
        },
    )?;
    let uniforms = EffectUniforms::new()
        .with_slot(0, [2., 6., 0., 0.])
        .with_slot(1, [0., 0., 1., 1.]);
    let fogged = fog.render(&[&retained, &depth], [3, 2], uniforms, 0.)?;
    let rows = upload(&context, [1, 2], wgpu::TextureFormat::R32Float, &[0., 8.]);
    let row_fogged = fog.render(&[&retained, &rows], [3, 2], uniforms, 0.)?;
    let tone = WgpuTextureEffect::new(
        context.clone(),
        &gpui_effects::hdr_tone_map_shader(),
        config.clone(),
    )?;
    let display = tone.render(
        &[&retained],
        [3, 2],
        EffectUniforms::new().with_slot(0, [0., 1., 0., 0.]),
        0.,
    )?;
    let resized = identity.render(&[&retained], [6, 4], EffectUniforms::default(), 0.)?;
    drop((identity, fog, tone, color, depth, rows));

    for pixel in read(&context, &retained)? {
        near(pixel, [2., 0.5, 0.25, 0.5]);
    }
    for (actual, expected) in read(&context, &exact)?.into_iter().zip(texels) {
        near(actual, expected);
    }
    for pixel in read(&context, &resized)? {
        near(pixel, [2., 0.5, 0.25, 0.5]);
    }
    let pixels = read(&context, &fogged)?;
    near(pixels[0], [2., 0.5, 0.25, 0.5]);
    near(pixels[1], pixels[0]);
    near(pixels[2], [1., 0.25, 0.375, 0.5]);
    for pixel in &pixels[3..] {
        near(*pixel, [0., 0., 0.5, 0.5]);
    }
    let pixels = read(&context, &row_fogged)?;
    for pixel in &pixels[..3] {
        near(*pixel, [2., 0.5, 0.25, 0.5]);
    }
    for pixel in &pixels[3..] {
        near(*pixel, [0., 0., 0.5, 0.5]);
    }
    let expected = [4_f32, 1., 0.5].map(|v| (1.055 * (v / (1. + v)).powf(1. / 2.4) - 0.055) * 0.5);
    for pixel in read(&context, &display)? {
        near(pixel, [expected[0], expected[1], expected[2], 0.5]);
    }

    let a = upload(&context, [1, 1], wgpu::TextureFormat::R32Float, &[2.]);
    let b = upload(&context, [1, 1], wgpu::TextureFormat::R32Float, &[3.]);
    let c = upload(&context, [1, 1], wgpu::TextureFormat::R32Float, &[4.]);
    let d = upload(&context, [1, 1], wgpu::TextureFormat::R32Float, &[0.5]);
    let shader = EffectShader::wgsl_four_images(
        "fn effect(i: EffectInput, p: EffectParams) -> vec4<f32> { return vec4<f32>(sample_effect_image(i, i.uv).r, sample_effect_second_image(i, i.uv).r, sample_effect_third_image(i, i.uv).r, sample_effect_fourth_image(i, i.uv).r); }",
    );
    let four = WgpuTextureEffect::new(
        context.clone(),
        &shader,
        TextureEffectConfig {
            inputs: vec![EffectTextureOptions::default(); 4],
            premultiplied_alpha: false,
            ..config
        },
    )?;
    let result = four.render(&[&a, &b, &c, &d], [1, 1], EffectUniforms::default(), 0.)?;
    near(read(&context, &result)?[0], [2., 3., 4., 0.5]);
    Ok(())
}

#[test]
#[ignore = "requires a GPU adapter"]
fn caller_encoding_preserves_pass_order_cancellation_and_parameter_isolation() -> Result<()> {
    let context = WgpuContext::new_headless()?;
    let source = upload(
        &context,
        [3, 2],
        wgpu::TextureFormat::Rgba32Float,
        &[2., 0.5, 0.25, 0.5].repeat(6),
    );
    let shader = EffectShader::wgsl_image(
        "fn effect(i: EffectInput, p: EffectParams) -> vec4<f32> { let c = sample_effect_image(i, i.uv); return vec4<f32>(c.rgb * p.slots[0].x, c.a); }",
    );
    let processor = WgpuTextureEffect::new(
        context.clone(),
        &shader,
        TextureEffectConfig {
            output_format: wgpu::TextureFormat::Rgba32Float,
            ..Default::default()
        },
    )?;
    let gain = |value| EffectUniforms::new().with_slot(0, [value, 0., 0., 0.]);
    let first = processor.render(&[&source], [3, 2], gain(2.), 0.)?;
    let expected = processor.render(&[&first], [3, 2], gain(0.25), 0.)?;

    let mut encoder = processor
        .context()
        .device
        .create_command_encoder(&Default::default());
    let pending_first = processor.encode(&mut encoder, &[&source], [3, 2], gain(2.), 0.)?;
    assert!(
        processor
            .encode(&mut encoder, &[], [3, 2], gain(3.), 0.)
            .is_err()
    );
    assert!(
        processor
            .encode(&mut encoder, &[&source], [0, 2], gain(3.), 0.)
            .is_err()
    );
    let pending_last = processor.encode(&mut encoder, &[&pending_first], [3, 2], gain(0.25), 0.)?;
    for output in [&pending_first, &pending_last] {
        for pixel in read(&context, output)? {
            near(pixel, [0.; 4]);
        }
    }
    context.queue.submit(Some(encoder.finish()));
    let expected_pixels = read(&context, &expected)?;
    assert_eq!(read(&context, &pending_last)?, expected_pixels);
    for pixel in read(&context, &pending_first)? {
        near(pixel, [4., 1., 0.5, 0.5]);
    }

    let mut abandoned = context.device.create_command_encoder(&Default::default());
    let unsubmitted = processor.encode(&mut abandoned, &[&source], [3, 2], gain(8.), 0.)?;
    drop(abandoned);
    for pixel in read(&context, &unsubmitted)? {
        near(pixel, [0.; 4]);
    }
    assert_eq!(read(&context, &pending_last)?, expected_pixels);

    let mut earlier = context.device.create_command_encoder(&Default::default());
    let mut later = context.device.create_command_encoder(&Default::default());
    let intermediate = processor.encode(&mut earlier, &[&source], [3, 2], gain(2.), 0.)?;
    let output = processor.encode(&mut later, &[&intermediate], [3, 2], gain(0.25), 0.)?;
    drop((processor, source, intermediate));
    context.queue.submit([earlier.finish(), later.finish()]);
    assert_eq!(read(&context, &output)?, expected_pixels);
    Ok(())
}

#[test]
#[ignore = "requires a GPU adapter"]
fn invalid_gpu_inputs_fail_without_poisoning_the_processor() -> Result<()> {
    let context = WgpuContext::new_headless()?;
    let other = WgpuContext::new_headless()?;
    let processor = WgpuTextureEffect::new(
        context.clone(),
        &gpui_effects::subtree_identity_shader(),
        TextureEffectConfig::default(),
    )?;
    let local = upload(
        &context,
        [1, 1],
        wgpu::TextureFormat::Rgba32Float,
        &[1., 1., 1., 1.],
    );
    let foreign = upload(
        &other,
        [1, 1],
        wgpu::TextureFormat::Rgba32Float,
        &[1., 1., 1., 1.],
    );
    assert!(
        processor
            .render(&[], [1, 1], EffectUniforms::default(), 0.)
            .is_err()
    );
    assert!(
        processor
            .render(&[&foreign], [1, 1], EffectUniforms::default(), 0.)
            .is_err()
    );
    assert!(
        processor
            .render(&[&local], [0, 1], EffectUniforms::default(), 0.)
            .is_err()
    );
    assert!(
        processor
            .render(&[&local], [1, 1], EffectUniforms::default(), f32::NAN)
            .is_err()
    );
    assert!(
        processor
            .render(&[&local], [1, 1], EffectUniforms::default(), 0.)
            .is_ok()
    );
    local.destroy();
    assert!(
        processor
            .render(&[&local], [1, 1], EffectUniforms::default(), 0.)
            .is_err()
    );
    Ok(())
}
