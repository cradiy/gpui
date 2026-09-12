use super::*;
use wgpu::util::DeviceExt;

#[test]
#[ignore = "requires a GPU adapter"]
fn progressive_blur_preserves_clear_pixels_and_filters_edges_without_color_fringes() {
    pollster::block_on(async {
        let instance = wgpu::Instance::default();
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions::default())
            .await
            .unwrap();
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor::default())
            .await
            .unwrap();
        let source = format!(
            "{}\n{}",
            gpui::compose_subtree_effect_wgsl(&progressive_blur_shader()),
            r#"
            @group(0) @binding(1) var<uniform> filter_params: EffectParams;
            @group(2) @binding(0) var output: texture_storage_2d<rgba8unorm, write>;
            @compute @workgroup_size(8, 8)
            fn filter_main(@builtin(global_invocation_id) id: vec3<u32>) {
                let dimensions = textureDimensions(output);
                if (any(id.xy >= dimensions)) { return; }
                var input: EffectInput;
                input.size = vec2<f32>(dimensions);
                input.image_size = input.size;
                input.uv = (vec2<f32>(id.xy) + vec2<f32>(0.5)) / input.size;
                let color = effect(input, filter_params);
                textureStore(output, id.xy, vec4<f32>(color.rgb * color.a, color.a));
            }
        "#
        );
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("progressive blur validation"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: None,
            layout: None,
            module: &shader,
            entry_point: Some("filter_main"),
            compilation_options: Default::default(),
            cache: None,
        });
        const SIZE: u32 = 64;
        let texture = || {
            device.create_texture(&wgpu::TextureDescriptor {
                label: None,
                size: wgpu::Extent3d {
                    width: SIZE,
                    height: SIZE,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::STORAGE_BINDING
                    | wgpu::TextureUsages::COPY_DST
                    | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            })
        };
        let textures = [texture(), texture(), texture()];
        let views = textures
            .each_ref()
            .map(|t| t.create_view(&Default::default()));
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: u64::from(SIZE * SIZE * 4),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        for edge in 0..4 {
            for translucent in [false, true] {
                let mut pixels = Vec::new();
                for y in 0..SIZE {
                    for x in 0..SIZE {
                        let cross = if edge < 2 { x } else { y };
                        let alpha = if cross % 8 < 4 {
                            255
                        } else if translucent {
                            0
                        } else {
                            255
                        };
                        let red = if cross % 8 < 4 { 255 } else { 0 };
                        pixels.extend_from_slice(&[red, 0, 0, alpha]);
                    }
                }
                queue.write_texture(
                    textures[0].as_image_copy(),
                    &pixels,
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(SIZE * 4),
                        rows_per_image: None,
                    },
                    textures[0].size(),
                );
                let mut encoder = device.create_command_encoder(&Default::default());
                for (index, stage) in stages(edge, px(32.), px(12.), true).iter().enumerate() {
                    let params = stage.prepare(1., 0.).uniforms;
                    let bytes: Vec<u8> = params
                        .slots()
                        .iter()
                        .flatten()
                        .flat_map(|v| v.to_ne_bytes())
                        .collect();
                    let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: None,
                        contents: &bytes,
                        usage: wgpu::BufferUsages::UNIFORM,
                    });
                    let groups = [
                        device.create_bind_group(&wgpu::BindGroupDescriptor {
                            label: None,
                            layout: &pipeline.get_bind_group_layout(0),
                            entries: &[wgpu::BindGroupEntry {
                                binding: 1,
                                resource: buffer.as_entire_binding(),
                            }],
                        }),
                        device.create_bind_group(&wgpu::BindGroupDescriptor {
                            label: None,
                            layout: &pipeline.get_bind_group_layout(1),
                            entries: &[wgpu::BindGroupEntry {
                                binding: 1,
                                resource: wgpu::BindingResource::TextureView(&views[index]),
                            }],
                        }),
                        device.create_bind_group(&wgpu::BindGroupDescriptor {
                            label: None,
                            layout: &pipeline.get_bind_group_layout(2),
                            entries: &[wgpu::BindGroupEntry {
                                binding: 0,
                                resource: wgpu::BindingResource::TextureView(&views[index + 1]),
                            }],
                        }),
                    ];
                    let mut pass = encoder.begin_compute_pass(&Default::default());
                    pass.set_pipeline(&pipeline);
                    for (group, bindings) in groups.iter().enumerate() {
                        pass.set_bind_group(group as u32, bindings, &[]);
                    }
                    pass.dispatch_workgroups(SIZE / 8, SIZE / 8, 1);
                }
                encoder.copy_texture_to_buffer(
                    textures[2].as_image_copy(),
                    wgpu::TexelCopyBufferInfo {
                        buffer: &readback,
                        layout: wgpu::TexelCopyBufferLayout {
                            offset: 0,
                            bytes_per_row: Some(SIZE * 4),
                            rows_per_image: None,
                        },
                    },
                    textures[2].size(),
                );
                queue.submit([encoder.finish()]);
                let (sender, receiver) = std::sync::mpsc::channel();
                readback
                    .slice(..)
                    .map_async(wgpu::MapMode::Read, move |result| {
                        sender.send(result).unwrap();
                    });
                device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
                receiver.recv().unwrap().unwrap();
                let output = readback.slice(..).get_mapped_range().unwrap();
                let sample = |depth: u32, cross: u32| {
                    let (x, y) = match edge {
                        0 => (cross, depth),
                        1 => (cross, SIZE - 1 - depth),
                        2 => (depth, cross),
                        _ => (SIZE - 1 - depth, cross),
                    };
                    ((y * SIZE + x) * 4) as usize
                };
                for depth in 32..SIZE {
                    for cross in 0..SIZE {
                        let index = sample(depth, cross);
                        assert_eq!(&output[index..index + 4], &pixels[index..index + 4]);
                    }
                }
                let contrast = |depth| {
                    (i32::from(output[sample(depth, 25)]) - i32::from(output[sample(depth, 29)]))
                        .abs()
                };
                assert!(
                    contrast(2) < contrast(22),
                    "blur must decrease inward: edge {edge}"
                );
                assert!(contrast(22) < contrast(40));
                for pixel in output.chunks_exact(4) {
                    assert_eq!(&pixel[1..3], &[0, 0]);
                    if translucent {
                        assert_eq!(pixel[0], pixel[3]);
                    } else {
                        assert_eq!(pixel[3], 255);
                    }
                }
                drop(output);
                readback.unmap();
            }
        }
    });
}
