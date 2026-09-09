use gpui::{EnvironmentBackground3d, EnvironmentMap3d};
use std::collections::{HashMap, HashSet};
use wgpu::util::DeviceExt;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    rays: [[f32; 4]; 3],
    bounds: [f32; 4],
    settings: [f32; 4],
}

struct Map {
    _source: EnvironmentMap3d,
    view: wgpu::TextureView,
}

pub(super) struct BackgroundRenderer {
    pub pipeline: wgpu::RenderPipeline,
    maps: HashMap<usize, Map>,
    sampler: wgpu::Sampler,
}
impl BackgroundRenderer {
    pub fn new(device: &wgpu::Device, samples: u32) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("scene3d_background"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../../scene3d_background.wgsl").into()),
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("scene3d_background"),
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
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::Always),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: wgpu::MultisampleState {
                count: samples,
                ..Default::default()
            },
            multiview_mask: None,
            cache: None,
        });
        Self {
            pipeline,
            maps: HashMap::new(),
            sampler: device.create_sampler(&wgpu::SamplerDescriptor {
                address_mode_u: wgpu::AddressMode::Repeat,
                address_mode_v: wgpu::AddressMode::ClampToEdge,
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            }),
        }
    }

    pub fn prepare<'a>(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        backgrounds: impl Iterator<Item = &'a EnvironmentBackground3d>,
    ) {
        let mut used = HashSet::new();
        for background in backgrounds {
            assert!(background.is_valid(), "invalid environment background");
            let map = &background.map;
            let [width, height] = map.size();
            assert!(
                width <= device.limits().max_texture_dimension_2d
                    && height <= device.limits().max_texture_dimension_2d,
                "environment map exceeds device texture dimensions"
            );
            let key = map.pixels().as_ptr() as usize;
            used.insert(key);
            self.maps.entry(key).or_insert_with(|| {
                let pixels: Vec<u8> = map
                    .pixels()
                    .iter()
                    .flat_map(|p| {
                        [p[0], p[1], p[2], 1.]
                            .into_iter()
                            .flat_map(|v| half::f16::from_f32(v).to_bits().to_le_bytes())
                    })
                    .collect();
                let texture = device.create_texture_with_data(
                    queue,
                    &wgpu::TextureDescriptor {
                        label: Some("scene3d_environment"),
                        size: wgpu::Extent3d {
                            width,
                            height,
                            depth_or_array_layers: 1,
                        },
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format: wgpu::TextureFormat::Rgba16Float,
                        usage: wgpu::TextureUsages::TEXTURE_BINDING,
                        view_formats: &[],
                    },
                    wgpu::util::TextureDataOrder::LayerMajor,
                    &pixels,
                );
                Map {
                    _source: map.clone(),
                    view: texture.create_view(&Default::default()),
                }
            });
        }
        self.maps.retain(|key, _| used.contains(key));
    }

    pub fn bind(
        &self,
        device: &wgpu::Device,
        background: &EnvironmentBackground3d,
        bounds: [f32; 4],
    ) -> wgpu::BindGroup {
        let params = Params {
            rays: background.rays.map(|v| [v[0], v[1], v[2], 0.]),
            bounds,
            settings: [
                background.rotation_y.cos(),
                background.rotation_y.sin(),
                background.intensity,
                0.,
            ],
        };
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("scene3d_background_params"),
            contents: bytemuck::bytes_of(&params),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("scene3d_background"),
            layout: &self.pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(
                        &self.maps[&(background.map.pixels().as_ptr() as usize)].view,
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem;
    #[test]
    fn scene3d_background_shader_validates_and_matches_uniform_layout() {
        let module =
            naga::front::wgsl::parse_str(include_str!("../../scene3d_background.wgsl")).unwrap();
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap();
        let ty = module
            .types
            .iter()
            .find(|(_, ty)| ty.name.as_deref() == Some("Params"))
            .unwrap()
            .1;
        let naga::TypeInner::Struct { members, span } = &ty.inner else {
            panic!("expected Params struct")
        };
        assert_eq!(*span as usize, mem::size_of::<Params>());
        let offsets = [
            mem::offset_of!(Params, rays),
            mem::offset_of!(Params, bounds),
            mem::offset_of!(Params, settings),
        ];
        assert_eq!(
            members
                .iter()
                .map(|m| m.offset as usize)
                .collect::<Vec<_>>(),
            offsets
        );
    }
}
