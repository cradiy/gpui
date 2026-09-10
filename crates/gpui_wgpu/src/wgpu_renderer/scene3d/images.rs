use collections::{HashMap, HashSet};
use gpui::{
    AtlasTile, MeshDraw3d, MeshTexture3d, TextureAddressMode3d, TextureColorSpace3d,
    TextureFilter3d, TextureMipFilter3d, TextureSampling3d,
};
use wgpu::util::DeviceExt;

use crate::WgpuAtlas;

#[derive(Clone, PartialEq, Eq, Hash)]
struct Key {
    source: wgpu::TextureView,
    generation: u64,
    rect: [u32; 4],
    srgb: bool,
}

struct Entry {
    tile: AtlasTile,
    view: wgpu::TextureView,
}

pub(super) struct Image {
    pub view: wgpu::TextureView,
    pub sampler: wgpu::Sampler,
}

#[derive(Default)]
pub(super) struct ImageCache {
    pipeline: Option<wgpu::RenderPipeline>,
    images: HashMap<Key, Entry>,
    samplers: HashMap<[u32; 6], wgpu::Sampler>,
}

impl ImageCache {
    pub fn retain<'a>(&mut self, objects: impl Iterator<Item = &'a MeshDraw3d>) {
        let mut required = HashSet::default();
        for object in objects {
            if let MeshTexture3d::Image(tile) = object.texture {
                if object.sampling.mip_filter != TextureMipFilter3d::None {
                    required.insert(tile_key(
                        tile,
                        object.image_color_space == TextureColorSpace3d::Srgb,
                    ));
                }
            }
            let pbr = object.pbr.is_some() && !object.unlit;
            for (map, srgb, active) in [
                (object.metallic_roughness_texture, false, pbr),
                (object.emissive_texture, true, pbr),
                (
                    object.normal_texture,
                    false,
                    pbr && object.normal_scale > 0.,
                ),
                (
                    object.occlusion_texture,
                    false,
                    !object.unlit && object.occlusion_strength > 0.,
                ),
            ] {
                if let Some(map) = map
                    && active
                    && map.sampling.mip_filter != TextureMipFilter3d::None
                {
                    required.insert(tile_key(map.tile, srgb));
                }
            }
        }
        self.images
            .retain(|key, entry| required.contains(&tile_key(entry.tile, key.srgb)));
    }

    pub fn get(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        atlas: &WgpuAtlas,
        tile: AtlasTile,
        color_space: TextureColorSpace3d,
        sampling: TextureSampling3d,
    ) -> Image {
        assert!(sampling.is_valid(), "invalid 3D texture sampling");
        let (source, generation) = atlas.get_tile_info(tile);
        let sampler_key = [
            sampling.address_u as u32,
            sampling.address_v as u32,
            sampling.filter as u32,
            sampling.magnification_filter() as u32,
            sampling.mip_filter as u32,
            u32::from(sampling.max_anisotropy),
        ];
        let sampler = self
            .samplers
            .entry(sampler_key)
            .or_insert_with(|| device.create_sampler(&sampler_descriptor(sampling)))
            .clone();
        if sampling.mip_filter == TextureMipFilter3d::None {
            return Image {
                view: source,
                sampler,
            };
        }
        let key = Key {
            source,
            generation,
            rect: [
                tile.bounds.origin.x.0 as u32,
                tile.bounds.origin.y.0 as u32,
                tile.bounds.size.width.0 as u32,
                tile.bounds.size.height.0 as u32,
            ],
            srgb: color_space == TextureColorSpace3d::Srgb,
        };
        if let Some(entry) = self.images.get(&key) {
            return Image {
                view: entry.view.clone(),
                sampler,
            };
        }
        self.images
            .retain(|old, entry| entry.tile != tile || old.srgb != key.srgb);
        let pipeline = self.pipeline.get_or_insert_with(|| mip_pipeline(device));
        let view = generate(device, queue, pipeline, &key);
        self.images.insert(
            key,
            Entry {
                tile,
                view: view.clone(),
            },
        );
        Image { view, sampler }
    }
}

fn tile_key(tile: AtlasTile, srgb: bool) -> (gpui::AtlasTextureId, u32, [i32; 4], bool) {
    (
        tile.texture_id,
        tile.tile_id.0,
        [
            tile.bounds.origin.x.0,
            tile.bounds.origin.y.0,
            tile.bounds.size.width.0,
            tile.bounds.size.height.0,
        ],
        srgb,
    )
}

pub(super) fn filter_flags(sampling: TextureSampling3d) -> u32 {
    sampling.filter as u32 | ((sampling.magnification_filter() as u32) << 1)
}

fn sampler_descriptor(sampling: TextureSampling3d) -> wgpu::SamplerDescriptor<'static> {
    let address = |value| match value {
        TextureAddressMode3d::Clamp => wgpu::AddressMode::ClampToEdge,
        TextureAddressMode3d::Repeat => wgpu::AddressMode::Repeat,
        TextureAddressMode3d::Mirror => wgpu::AddressMode::MirrorRepeat,
    };
    let filter = |value| match value {
        TextureFilter3d::Nearest => wgpu::FilterMode::Nearest,
        TextureFilter3d::Linear => wgpu::FilterMode::Linear,
    };
    wgpu::SamplerDescriptor {
        label: Some("scene3d_image"),
        address_mode_u: address(sampling.address_u),
        address_mode_v: address(sampling.address_v),
        mag_filter: filter(sampling.magnification_filter()),
        min_filter: filter(sampling.filter),
        mipmap_filter: match sampling.mip_filter {
            TextureMipFilter3d::Linear => wgpu::MipmapFilterMode::Linear,
            _ => wgpu::MipmapFilterMode::Nearest,
        },
        anisotropy_clamp: sampling.max_anisotropy,
        ..Default::default()
    }
}

fn mip_pipeline(device: &wgpu::Device) -> wgpu::RenderPipeline {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("scene3d_mipmap"),
        source: wgpu::ShaderSource::Wgsl(include_str!("../../scene3d_mipmap.wgsl").into()),
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("scene3d_mipmap"),
        layout: None,
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vertex"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fragment"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: wgpu::TextureFormat::Rgba16Float,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: Default::default(),
        depth_stencil: None,
        multisample: Default::default(),
        multiview_mask: None,
        cache: None,
    })
}

fn mip_sizes(width: u32, height: u32) -> Vec<[u32; 2]> {
    let mut sizes = vec![[width, height]];
    let mut size = [width, height];
    while size != [1, 1] {
        size = size.map(|value| (value / 2).max(1));
        sizes.push(size);
    }
    sizes
}

fn generate(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    pipeline: &wgpu::RenderPipeline,
    key: &Key,
) -> wgpu::TextureView {
    let sizes = mip_sizes(key.rect[2], key.rect[3]);
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("scene3d_image_mips"),
        size: wgpu::Extent3d {
            width: key.rect[2],
            height: key.rect[3],
            depth_or_array_layers: 1,
        },
        mip_level_count: sizes.len() as u32,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba16Float,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("scene3d_image_mips"),
    });
    let mut source = key.source.clone();
    let mut rect = key.rect;
    for (level, size) in sizes.iter().enumerate() {
        let target = texture.create_view(&wgpu::TextureViewDescriptor {
            base_mip_level: level as u32,
            mip_level_count: Some(1),
            ..Default::default()
        });
        let params = [
            rect,
            [size[0], size[1], u32::from(level == 0 && key.srgb), 0],
        ];
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("scene3d_mip_params"),
            contents: bytemuck::cast_slice(&params),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("scene3d_mip"),
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&source),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: buffer.as_entire_binding(),
                },
            ],
        });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("scene3d_mip"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &group, &[]);
            pass.draw(0..3, 0..1);
        }
        source = target;
        rect = [0, 0, size[0], size[1]];
    }
    // Resource initialization is submitted independently of replayable frame encoders.
    queue.submit([encoder.finish()]);
    texture.create_view(&Default::default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(not(target_family = "wasm"))]
    #[ignore = "requires a GPU adapter"]
    fn scene3d_mip_cache_separates_color_spaces_and_tracks_atlas_generations() -> anyhow::Result<()>
    {
        use gpui::{DevicePixels, PlatformAtlas, size};
        let context = crate::WgpuContext::new_headless()?;
        let atlas = WgpuAtlas::from_context(&context);
        let key = gpui::AtlasKey::Image(gpui::RenderImageParams {
            image_id: gpui::ImageId(99002),
            frame_index: 0,
        });
        let allocate = || {
            atlas.get_or_insert_with(&key, &mut || {
                Ok(Some((
                    size(DevicePixels(5), DevicePixels(3)),
                    std::borrow::Cow::Owned([128, 64, 32, 255].repeat(15)),
                )))
            })
        };
        let tile = allocate()?.unwrap();
        atlas.before_frame();
        let generation = atlas.get_tile_info(tile).1;
        let mut cache = ImageCache::default();
        let sampling = TextureSampling3d {
            mip_filter: TextureMipFilter3d::Linear,
            ..Default::default()
        };
        let first = cache.get(
            &context.device,
            &context.queue,
            &atlas,
            tile,
            TextureColorSpace3d::Srgb,
            sampling,
        );
        let different_sampling = TextureSampling3d {
            address_u: TextureAddressMode3d::Repeat,
            max_anisotropy: 8,
            transform: gpui::UvTransform3d::from_scale_rotation_translation(
                [4.; 2], 0.2, [0.5; 2],
            )?,
            ..sampling
        };
        let reused = cache.get(
            &context.device,
            &context.queue,
            &atlas,
            tile,
            TextureColorSpace3d::Srgb,
            different_sampling,
        );
        assert_eq!(first.view, reused.view);
        assert_ne!(first.sampler, reused.sampler);
        let linear = cache.get(
            &context.device,
            &context.queue,
            &atlas,
            tile,
            TextureColorSpace3d::Linear,
            sampling,
        );
        assert_ne!(first.view, linear.view);
        assert_eq!(cache.images.len(), 2);
        for clear in [false, true] {
            if clear {
                atlas.clear();
            } else {
                atlas.remove(&key);
            }
            let tile = allocate()?.unwrap();
            assert_ne!(atlas.get_tile_info(tile).1, generation);
            atlas.before_frame();
            let replacement = cache.get(
                &context.device,
                &context.queue,
                &atlas,
                tile,
                TextureColorSpace3d::Srgb,
                sampling,
            );
            assert_ne!(first.view, replacement.view);
        }
        cache.retain(std::iter::empty());
        assert!(cache.images.is_empty());
        // Previously returned views remain owned independently of cache eviction.
        assert_eq!(first.view.texture().width(), 5);
        assert_eq!(first.view.texture().mip_level_count(), 3);
        Ok(())
    }

    #[test]
    fn scene3d_mip_chain_covers_odd_and_single_axis_images() {
        for width in [1u32, 2, 3, 5, 64, 127, 8192] {
            for height in [1u32, 2, 3, 7, 128, 8191] {
                let sizes = mip_sizes(width, height);
                assert_eq!(
                    sizes.len(),
                    (u32::BITS - width.max(height).leading_zeros()) as usize
                );
                assert_eq!(sizes[0], [width, height]);
                assert_eq!(sizes.last(), Some(&[1, 1]));
                for (level, size) in sizes.iter().enumerate() {
                    assert_eq!(*size, [(width >> level).max(1), (height >> level).max(1)]);
                }
            }
        }
    }

    #[test]
    fn scene3d_sampler_descriptors_preserve_independent_addressing_and_filters() {
        for address_u in [
            TextureAddressMode3d::Clamp,
            TextureAddressMode3d::Repeat,
            TextureAddressMode3d::Mirror,
        ] {
            for address_v in [
                TextureAddressMode3d::Clamp,
                TextureAddressMode3d::Repeat,
                TextureAddressMode3d::Mirror,
            ] {
                for filter in [TextureFilter3d::Nearest, TextureFilter3d::Linear] {
                    for mag_filter in [
                        None,
                        Some(TextureFilter3d::Nearest),
                        Some(TextureFilter3d::Linear),
                    ] {
                        for mip_filter in [
                            TextureMipFilter3d::None,
                            TextureMipFilter3d::Nearest,
                            TextureMipFilter3d::Linear,
                        ] {
                            for max_anisotropy in [1, 2, 4, 8, 16] {
                                let sampling = TextureSampling3d {
                                    address_u,
                                    address_v,
                                    filter,
                                    mag_filter,
                                    mip_filter,
                                    max_anisotropy,
                                    ..Default::default()
                                };
                                if !sampling.is_valid() {
                                    continue;
                                }
                                let descriptor = sampler_descriptor(sampling);
                                let address = |value| match value {
                                    TextureAddressMode3d::Clamp => wgpu::AddressMode::ClampToEdge,
                                    TextureAddressMode3d::Repeat => wgpu::AddressMode::Repeat,
                                    TextureAddressMode3d::Mirror => wgpu::AddressMode::MirrorRepeat,
                                };
                                assert_eq!(descriptor.address_mode_u, address(address_u));
                                assert_eq!(descriptor.address_mode_v, address(address_v));
                                assert_eq!(descriptor.anisotropy_clamp, max_anisotropy);
                                assert_eq!(
                                    descriptor.mag_filter == wgpu::FilterMode::Linear,
                                    mag_filter.unwrap_or(filter) == TextureFilter3d::Linear
                                );
                                assert_eq!(
                                    descriptor.min_filter == wgpu::FilterMode::Linear,
                                    filter == TextureFilter3d::Linear
                                );
                                let flags = filter_flags(sampling);
                                assert_eq!(
                                    flags & 1 != 0,
                                    descriptor.min_filter == wgpu::FilterMode::Linear
                                );
                                assert_eq!(
                                    flags & 2 != 0,
                                    descriptor.mag_filter == wgpu::FilterMode::Linear
                                );
                                assert_eq!(
                                    descriptor.mipmap_filter == wgpu::MipmapFilterMode::Linear,
                                    mip_filter == TextureMipFilter3d::Linear
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn scene3d_mipmap_shader_validates_and_matches_upload_layout() {
        let module =
            naga::front::wgsl::parse_str(include_str!("../../scene3d_mipmap.wgsl")).unwrap();
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap();
        let params = module
            .types
            .iter()
            .find(|(_, ty)| ty.name.as_deref() == Some("Params"))
            .unwrap()
            .1;
        let naga::TypeInner::Struct { members, span } = &params.inner else {
            panic!("missing mip uniform struct");
        };
        assert_eq!(*span as usize, std::mem::size_of::<[[u32; 4]; 2]>());
        assert_eq!(
            members.iter().map(|m| m.offset).collect::<Vec<_>>(),
            [0, 16]
        );
    }
}
