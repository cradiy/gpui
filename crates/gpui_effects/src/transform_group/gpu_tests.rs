use super::*;
use gpui::{point, px, size};
use wgpu::util::DeviceExt;

#[test]
#[ignore = "requires a GPU adapter"]
fn transform_group_gpu_preserves_alpha_and_matches_transformed_targets() {
    pollster::block_on(async {
        let instance = wgpu::Instance::default();
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        let (device, queue) = adapter.request_device(&Default::default()).await.unwrap();
        let source = format!(
            "{}\n{}",
            gpui::compose_subtree_effect_wgsl(&transform_group_shader()),
            r#"
            @group(0) @binding(1) var<uniform> matrix: EffectParams;
            @group(2) @binding(0) var output: texture_storage_2d<rgba8unorm, write>;
            @compute @workgroup_size(8, 8)
            fn check_transform(@builtin(global_invocation_id) id: vec3<u32>) {
                let dimensions = textureDimensions(output);
                if (any(id.xy >= dimensions)) { return; }
                var input: EffectInput;
                input.size = vec2<f32>(dimensions);
                input.image_size = input.size;
                input.uv = (vec2<f32>(id.xy) + vec2<f32>(0.5)) / input.size;
                let color = effect(input, matrix);
                textureStore(output, id.xy, vec4<f32>(color.rgb * color.a, color.a));
            }
        "#
        );
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("transform group pixel regression"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: None,
            layout: None,
            module: &shader,
            entry_point: Some("check_transform"),
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
        let mut pixels = vec![0; (SIZE * SIZE * 4) as usize];
        for y in 12..20 {
            for x in 8..16 {
                let index = ((y * SIZE + x) * 4) as usize;
                pixels[index..index + 4].copy_from_slice(&[128, 0, 0, 128]);
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
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: u64::from(SIZE * SIZE * 4),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let translate = |x, y| TransformationMatrix {
            translation: [x, y],
            ..TransformationMatrix::unit()
        };
        for (matrices, sample) in [
            (
                vec![TransformationMatrix {
                    rotation_scale: [[2., 0.], [0., 2.]],
                    translation: [4., 6.],
                }],
                Some((28, 38)),
            ),
            (
                vec![TransformationMatrix {
                    rotation_scale: [[0., -1.], [1., 0.]],
                    translation: [48., 0.],
                }],
                Some((32, 12)),
            ),
            (
                vec![TransformationMatrix {
                    rotation_scale: [[-1., 0.], [0., 1.]],
                    translation: [40., 0.],
                }],
                Some((28, 16)),
            ),
            (
                vec![
                    translate(4., 6.),
                    TransformationMatrix {
                        rotation_scale: [[2., 0.], [0., 2.]],
                        translation: [0., 0.],
                    },
                ],
                Some((32, 44)),
            ),
            (vec![translate(80., 0.)], None),
        ] {
            let mut encoder = device.create_command_encoder(&Default::default());
            for (index, matrix) in matrices.iter().enumerate() {
                let params = uniforms(
                    *matrix,
                    Bounds::new(point(px(0.), px(0.)), size(px(64.), px(64.))),
                    1.,
                );
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
                let resources = [
                    buffer.as_entire_binding(),
                    wgpu::BindingResource::TextureView(&views[index]),
                    wgpu::BindingResource::TextureView(&views[index + 1]),
                ];
                let groups: Vec<_> = resources
                    .into_iter()
                    .enumerate()
                    .map(|(group, resource)| {
                        device.create_bind_group(&wgpu::BindGroupDescriptor {
                            label: None,
                            layout: &pipeline.get_bind_group_layout(group as u32),
                            entries: &[wgpu::BindGroupEntry {
                                binding: if group == 2 { 0 } else { 1 },
                                resource,
                            }],
                        })
                    })
                    .collect();
                let mut pass = encoder.begin_compute_pass(&Default::default());
                pass.set_pipeline(&pipeline);
                for (index, group) in groups.iter().enumerate() {
                    pass.set_bind_group(index as u32, group, &[]);
                }
                pass.dispatch_workgroups(SIZE / 8, SIZE / 8, 1);
            }
            encoder.copy_texture_to_buffer(
                textures[matrices.len()].as_image_copy(),
                wgpu::TexelCopyBufferInfo {
                    buffer: &readback,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(SIZE * 4),
                        rows_per_image: None,
                    },
                },
                textures[0].size(),
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
            if let Some((x, y)) = sample {
                let index = ((y * SIZE + x) * 4) as usize;
                assert_eq!(&output[index..index + 4], &[128, 0, 0, 128]);
                assert_eq!(&output[0..4], &[0, 0, 0, 0]);
            } else {
                assert!(output.iter().all(|v| *v == 0));
            }
            // Translucent red must remain premultiplied, without dark or colored fringes.
            for pixel in output.chunks_exact(4) {
                assert_eq!(pixel[0], pixel[3]);
                assert_eq!(&pixel[1..3], &[0, 0]);
            }
            drop(output);
            readback.unmap();
        }
    });
}
