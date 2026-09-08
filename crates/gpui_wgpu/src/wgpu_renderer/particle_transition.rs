use super::*;
use wgpu::util::DeviceExt;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Params {
    bounds: [f32; 4],
    viewport: [f32; 4],
    motion: [f32; 4],
    shape: [f32; 4],
    grid: [u32; 4],
}

pub(super) struct ParticleTransitionRenderer {
    pipeline: wgpu::RenderPipeline,
    sampler: wgpu::Sampler,
}

impl ParticleTransitionRenderer {
    pub(super) fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("particle_transition"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../particle_transition.wgsl").into()),
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("particle_transition"),
            layout: None,
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vertex"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fragment"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("particle_transition_sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Self { pipeline, sampler }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn encode(
        &self,
        device: &wgpu::Device,
        quad: &EffectQuad,
        transition: &gpui::SubtreeParticleTransitionPass,
        source: &wgpu::TextureView,
        destination: &wgpu::TextureView,
        viewport: [f32; 2],
        encoder: &mut wgpu::CommandEncoder,
    ) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("particle_transition"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: destination,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
                depth_slice: None,
            })],
            ..Default::default()
        });
        let progress = if transition.progress.is_finite() {
            transition.progress.clamp(0., 1.)
        } else {
            0.
        };
        if progress >= 1. || quad.bounds.size.width.0 <= 0. || quad.bounds.size.height.0 <= 0. {
            return;
        }
        let options = transition.options.normalized();
        let scale = transition.scale_factor.max(0.001);
        let mut cell = (f32::from(options.cell_size) * scale).max(1.);
        let dimensions = |cell: f32| {
            [
                (quad.bounds.size.width.0 / cell).ceil() as u32,
                (quad.bounds.size.height.0 / cell).ceil() as u32,
            ]
        };
        let mut grid = dimensions(cell);
        while u64::from(grid[0]) * u64::from(grid[1]) > 131_072 {
            cell *= 1.1;
            grid = dimensions(cell);
        }
        let params = Params {
            bounds: [
                quad.bounds.origin.x.0,
                quad.bounds.origin.y.0,
                quad.bounds.size.width.0,
                quad.bounds.size.height.0,
            ],
            viewport: [viewport[0], viewport[1], cell, progress],
            motion: [
                f32::from(options.scatter.x) * scale,
                f32::from(options.scatter.y) * scale,
                f32::from(options.spread) * scale,
                0.,
            ],
            shape: [
                f32::from(options.radius) * scale,
                f32::from(options.streak) * scale,
                0.,
                0.,
            ],
            grid: [grid[0], grid[1], options.seed, 0],
        };
        let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("particle_transition_params"),
            contents: bytemuck::bytes_of(&params),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("particle_transition_input"),
            layout: &self.pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(source),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.draw(0..4, 0..grid[0] * grid[1]);
    }
}
