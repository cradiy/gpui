use super::*;
use wgpu::util::DeviceExt;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Params {
    region: [u32; 4],
    jump: u32,
    threshold: f32,
    pad: [u32; 2],
}

pub(super) struct DistanceFieldRenderer {
    layouts: [wgpu::BindGroupLayout; 2],
    pipelines: [wgpu::ComputePipeline; 3],
    seeds: [wgpu::Texture; 2],
    field: wgpu::Texture,
}

pub(super) fn region(bounds: Bounds<ScaledPixels>, viewport: [u32; 2]) -> [u32; 4] {
    let x = bounds.origin.x.0.floor().max(0.) as u32;
    let y = bounds.origin.y.0.floor().max(0.) as u32;
    let right = (bounds.origin.x.0 + bounds.size.width.0).ceil().max(0.) as u32;
    let bottom = (bounds.origin.y.0 + bounds.size.height.0).ceil().max(0.) as u32;
    [
        x,
        y,
        right.min(viewport[0]).saturating_sub(x),
        bottom.min(viewport[1]).saturating_sub(y),
    ]
}

impl DistanceFieldRenderer {
    pub(super) fn new(device: &wgpu::Device, size: [u32; 2]) -> Self {
        let formats = [
            wgpu::TextureFormat::Rgba32Float,
            wgpu::TextureFormat::Rgba16Float,
        ];
        let layouts = formats.map(|format| {
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("distance_field_compute"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: false },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: false },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::StorageTexture {
                            access: wgpu::StorageTextureAccess::WriteOnly,
                            format,
                            view_dimension: wgpu::TextureViewDimension::D2,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 3,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: true,
                            min_binding_size: NonZeroU64::new(std::mem::size_of::<Params>() as u64),
                        },
                        count: None,
                    },
                ],
            })
        });
        let modules = ["rgba32float", "rgba16float"].map(|format| {
            device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("distance_field_compute"),
                source: wgpu::ShaderSource::Wgsl(
                    include_str!("../distance_field.wgsl")
                        .replace("OUTPUT_FORMAT", format)
                        .into(),
                ),
            })
        });
        let pipelines = ["seed", "jump", "resolve"].map(|entry| {
            let index = usize::from(entry == "resolve");
            let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("distance_field_compute"),
                bind_group_layouts: &[Some(&layouts[index])],
                immediate_size: 0,
            });
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("distance_field_compute"),
                layout: Some(&layout),
                module: &modules[index],
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        });
        let (seeds, field) = Self::textures(device, size);
        Self {
            layouts,
            pipelines,
            seeds,
            field,
        }
    }

    fn textures(device: &wgpu::Device, size: [u32; 2]) -> ([wgpu::Texture; 2], wgpu::Texture) {
        let texture = |format| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some("distance_field_scratch"),
                size: wgpu::Extent3d {
                    width: size[0],
                    height: size[1],
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
        };
        (
            std::array::from_fn(|_| texture(wgpu::TextureFormat::Rgba32Float)),
            texture(wgpu::TextureFormat::Rgba16Float),
        )
    }

    pub(super) fn reserve(&mut self, device: &wgpu::Device, required: [u32; 2]) {
        let current = self.size();
        if current[0] < required[0] || current[1] < required[1] {
            (self.seeds, self.field) = Self::textures(
                device,
                [current[0].max(required[0]), current[1].max(required[1])],
            );
        }
    }

    pub(super) fn size(&self) -> [u32; 2] {
        [self.field.width(), self.field.height()]
    }

    pub(super) fn encode(
        &self,
        device: &wgpu::Device,
        source: &wgpu::TextureView,
        region: [u32; 4],
        threshold: f32,
        encoder: &mut wgpu::CommandEncoder,
    ) -> wgpu::TextureView {
        let seeds = self
            .seeds
            .each_ref()
            .map(|texture| texture.create_view(&Default::default()));
        let output = self.field.create_view(&Default::default());
        let mut stages = vec![(0, 0)];
        let mut jump = region[2].max(region[3]).next_power_of_two() / 2;
        while jump > 0 {
            stages.push((1, jump));
            jump /= 2;
        }
        stages.push((1, 1));
        stages.push((2, 0));
        let alignment = device.limits().min_uniform_buffer_offset_alignment as usize;
        let stride = std::mem::size_of::<Params>().div_ceil(alignment) * alignment;
        let mut uniforms = vec![0u8; stride * stages.len()];
        for (index, (_, jump)) in stages.iter().enumerate() {
            let params = Params {
                region,
                jump: *jump,
                threshold: if threshold.is_finite() {
                    threshold.clamp(0.001, 0.999)
                } else {
                    0.5
                },
                pad: [0; 2],
            };
            uniforms[index * stride..index * stride + std::mem::size_of::<Params>()]
                .copy_from_slice(bytemuck::bytes_of(&params));
        }
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("distance_field_steps"),
            contents: &uniforms,
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let mut current = 1;
        for (index, (kind, _)) in stages.into_iter().enumerate() {
            let destination = if kind == 2 {
                &output
            } else {
                &seeds[1 - current]
            };
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("distance_field_step"),
                layout: &self.layouts[usize::from(kind == 2)],
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(source),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&seeds[current]),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(destination),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                            buffer: &buffer,
                            offset: 0,
                            size: NonZeroU64::new(std::mem::size_of::<Params>() as u64),
                        }),
                    },
                ],
            });
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("distance_field_step"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.pipelines[kind]);
            pass.set_bind_group(0, &group, &[(index * stride) as u32]);
            pass.dispatch_workgroups(region[2].div_ceil(8), region[3].div_ceil(8), 1);
            current = 1 - current;
        }
        output
    }
}
