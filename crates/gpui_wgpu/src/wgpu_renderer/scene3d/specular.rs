use gpui::{SpecularEnvironment3d, SpecularEnvironmentMap3d};
use std::collections::{HashMap, HashSet};
use wgpu::util::DeviceExt;

struct Map {
    _source: SpecularEnvironmentMap3d,
    view: wgpu::TextureView,
}
pub(super) struct SpecularRenderer {
    maps: HashMap<usize, Map>,
    black: wgpu::TextureView,
    brdf: Option<wgpu::TextureView>,
    pub sampler: wgpu::Sampler,
}
impl SpecularRenderer {
    pub fn new(device: &wgpu::Device) -> Self {
        let black = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("scene3d_specular_black"),
                size: wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 6,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba16Float,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
            .create_view(&wgpu::TextureViewDescriptor {
                dimension: Some(wgpu::TextureViewDimension::Cube),
                ..Default::default()
            });
        Self {
            maps: HashMap::new(),
            black,
            brdf: None,
            sampler: device.create_sampler(&wgpu::SamplerDescriptor {
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                mipmap_filter: wgpu::MipmapFilterMode::Linear,
                ..Default::default()
            }),
        }
    }
    pub fn prepare<'a>(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        environments: impl Iterator<Item = &'a SpecularEnvironment3d>,
    ) {
        let mut used = HashSet::new();
        for environment in environments {
            assert!(environment.is_valid(), "invalid specular environment");
            if environment.intensity == 0. {
                continue;
            }
            let map = &environment.map;
            let size = map.size();
            assert!(
                size <= device.limits().max_texture_dimension_2d,
                "specular environment exceeds device limits"
            );
            let key = map.levels().as_ptr() as usize;
            used.insert(key);
            self.maps.entry(key).or_insert_with(|| {
                let pixels: Vec<u8> = map
                    .levels()
                    .iter()
                    .flatten()
                    .flat_map(|p| {
                        [p[0], p[1], p[2], 1.]
                            .into_iter()
                            .flat_map(|v| half::f16::from_f32(v).to_bits().to_le_bytes())
                    })
                    .collect();
                let texture = device.create_texture_with_data(
                    queue,
                    &wgpu::TextureDescriptor {
                        label: Some("scene3d_specular_environment"),
                        size: wgpu::Extent3d {
                            width: size,
                            height: size,
                            depth_or_array_layers: 6,
                        },
                        mip_level_count: map.levels().len() as u32,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format: wgpu::TextureFormat::Rgba16Float,
                        usage: wgpu::TextureUsages::TEXTURE_BINDING,
                        view_formats: &[],
                    },
                    wgpu::util::TextureDataOrder::MipMajor,
                    &pixels,
                );
                Map {
                    _source: map.clone(),
                    view: texture.create_view(&wgpu::TextureViewDescriptor {
                        dimension: Some(wgpu::TextureViewDimension::Cube),
                        ..Default::default()
                    }),
                }
            });
        }
        if !used.is_empty() && self.brdf.is_none() {
            self.brdf = Some(integrate_brdf(device, queue));
        }
        self.maps.retain(|key, _| used.contains(key));
    }
    pub fn map(&self, environment: Option<&SpecularEnvironment3d>) -> &wgpu::TextureView {
        environment.map_or(&self.black, |e| {
            &self.maps[&(e.map.levels().as_ptr() as usize)].view
        })
    }
    pub fn brdf<'a>(&'a self, fallback: &'a wgpu::TextureView) -> &'a wgpu::TextureView {
        self.brdf.as_ref().unwrap_or(fallback)
    }
}
fn integrate_brdf(device: &wgpu::Device, queue: &wgpu::Queue) -> wgpu::TextureView {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("scene3d_specular_brdf"),
        size: wgpu::Extent3d {
            width: 128,
            height: 128,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rg16Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let view = texture.create_view(&Default::default());
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("scene3d_specular_brdf"),
        source: wgpu::ShaderSource::Wgsl(include_str!("../../scene3d_brdf.wgsl").into()),
    });
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("scene3d_specular_brdf"),
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
                format: wgpu::TextureFormat::Rg16Float,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: Default::default(),
        depth_stencil: None,
        multisample: Default::default(),
        multiview_mask: None,
        cache: None,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("scene3d_specular_brdf"),
    });
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("scene3d_specular_brdf"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        pass.set_pipeline(&pipeline);
        pass.draw(0..3, 0..1);
    }
    queue.submit([encoder.finish()]);
    view
}

#[cfg(test)]
mod tests {
    #[test]
    fn scene3d_brdf_shader_validates() {
        let module = naga::front::wgsl::parse_str(include_str!("../../scene3d_brdf.wgsl")).unwrap();
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap();
    }
}
