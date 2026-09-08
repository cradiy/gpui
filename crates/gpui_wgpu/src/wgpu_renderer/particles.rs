use super::*;
use gpui::{MAX_GPU_PARTICLES, MAX_PARTICLE_SPAWNS, ParticleDraw};

const UPDATE: &str = concat!(
    include_str!("../particles.wgsl"),
    include_str!("../particles_update.wgsl")
);
const DRAW: &str = concat!(
    include_str!("../particles.wgsl"),
    include_str!("../particles_draw.wgsl")
);
const MAX_MASK_SAMPLES: u32 = 131_072;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Params {
    counts: [u32; 4],
    timing: [f32; 4],
    viewport: [f32; 4],
    bounds: [f32; 4],
    clip: [f32; 4],
    acceleration: [f32; 4],
    field: [f32; 4],
    mask: [u32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct MaskParams {
    bounds: [f32; 4],
    region: [u32; 4],
    settings: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Spawn {
    range: [u32; 4],
    line: [f32; 4],
    velocity: [f32; 4],
    life: [f32; 4],
    color: [f32; 4],
    shape: [f32; 4],
}

#[derive(Clone, Copy)]
struct Snapshot {
    generation: u64,
    frame: u64,
    time: Duration,
    index: usize,
    cursor: u32,
}

struct System {
    capacity: u32,
    size: Size<ScaledPixels>,
    scale: f32,
    masked: bool,
    candidates: wgpu::Buffer,
    candidate_group: wgpu::BindGroup,
    mask_uniform: wgpu::Buffer,
    uniform: wgpu::Buffer,
    emissions: wgpu::Buffer,
    update: [wgpu::BindGroup; 2],
    draw: [wgpu::BindGroup; 2],
    committed: Cell<Option<Snapshot>>,
    pending: Cell<Option<Snapshot>>,
}

pub(super) struct ParticleRenderer {
    update: wgpu::ComputePipeline,
    draw: wgpu::RenderPipeline,
    mask: wgpu::ComputePipeline,
    systems: HashMap<gpui::EffectHistoryId, System>,
}

impl ParticleRenderer {
    pub(super) fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let compute = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("particles_update"),
            source: wgpu::ShaderSource::Wgsl(UPDATE.into()),
        });
        let render = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("particles_draw"),
            source: wgpu::ShaderSource::Wgsl(DRAW.into()),
        });
        let update = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("particles_update"),
            layout: None,
            module: &compute,
            entry_point: Some("update"),
            compilation_options: Default::default(),
            cache: None,
        });
        let mask_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("particle_mask"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../particles_mask.wgsl").into()),
        });
        let mask = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("particle_mask"),
            layout: None,
            module: &mask_module,
            entry_point: Some("sample_mask"),
            compilation_options: Default::default(),
            cache: None,
        });
        let draw = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("particles_draw"),
            layout: None,
            vertex: wgpu::VertexState {
                module: &render,
                entry_point: Some("vertex"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &render,
                entry_point: Some("fragment"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
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
        Self {
            update,
            draw,
            mask,
            systems: HashMap::new(),
        }
    }

    pub(super) fn ensure(&mut self, device: &wgpu::Device, scene: &Scene) {
        let mut required = HashMap::new();
        let mut require = |draw: &ParticleDraw, masked: bool| {
            assert!(
                required
                    .insert(
                        draw.frame.id,
                        (
                            draw.frame.capacity.clamp(1, MAX_GPU_PARTICLES),
                            draw.bounds.size,
                            draw.scale_factor,
                            masked,
                        )
                    )
                    .is_none(),
                "a particle identity may only occur once in a scene"
            );
        };
        scene.visit(&mut |scene| {
            for draw in &scene.particles {
                require(draw, false);
            }
            for layer in &scene.subtree_layers {
                for effect in layer.intermediate_effects.iter() {
                    if let Some(particles) = &effect.particles {
                        require(&Self::masked_draw(&layer.composite, particles), true);
                    }
                }
            }
        });
        self.systems.retain(|id, system| {
            required
                .get(id)
                .is_some_and(|&(capacity, size, scale, masked)| {
                    system.capacity == capacity
                        && system.size == size
                        && system.scale == scale
                        && system.masked == masked
                })
        });
        for (id, (capacity, size, scale, masked)) in required {
            if self.systems.contains_key(&id) {
                continue;
            }
            let state: [_; 2] = std::array::from_fn(|_| {
                device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("particle_state"),
                    size: u64::from(capacity) * 48,
                    usage: wgpu::BufferUsages::STORAGE,
                    mapped_at_creation: false,
                })
            });
            let uniform = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("particle_params"),
                size: std::mem::size_of::<Params>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let emissions = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("particle_emissions"),
                size: (std::mem::size_of::<Spawn>() * MAX_PARTICLE_SPAWNS) as u64,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let update = std::array::from_fn(|read| {
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("particle_update"),
                    layout: &self.update.get_bind_group_layout(0),
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: uniform.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: state[read].as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: state[1 - read].as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 3,
                            resource: emissions.as_entire_binding(),
                        },
                    ],
                })
            });
            let candidates = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("particle_mask_candidates"),
                size: 16 + 32 * u64::from(if masked { MAX_MASK_SAMPLES } else { 1 }),
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let candidate_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("particle_mask_samples"),
                layout: &self.update.get_bind_group_layout(1),
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: candidates.as_entire_binding(),
                }],
            });
            let mask_uniform = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("particle_mask_params"),
                size: std::mem::size_of::<MaskParams>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let draw = std::array::from_fn(|index| {
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("particle_draw"),
                    layout: &self.draw.get_bind_group_layout(0),
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: uniform.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: state[index].as_entire_binding(),
                        },
                    ],
                })
            });
            self.systems.insert(
                id,
                System {
                    capacity,
                    size,
                    scale,
                    masked,
                    candidates,
                    candidate_group,
                    mask_uniform,
                    uniform,
                    emissions,
                    update,
                    draw,
                    committed: Cell::new(None),
                    pending: Cell::new(None),
                },
            );
        }
    }

    pub(super) fn encode(
        &self,
        queue: &wgpu::Queue,
        scene: &Scene,
        viewport: [f32; 2],
        encoder: &mut wgpu::CommandEncoder,
    ) {
        scene.visit(&mut |scene| {
            for draw in &scene.particles {
                self.encode_draw(queue, draw, viewport, None, encoder);
            }
        });
    }

    fn encode_draw(
        &self,
        queue: &wgpu::Queue,
        draw: &ParticleDraw,
        viewport: [f32; 2],
        mask: Option<gpui::ParticleMask>,
        encoder: &mut wgpu::CommandEncoder,
    ) {
        let system = &self.systems[&draw.frame.id];
        let frame = &draw.frame;
        let previous = system.committed.get().filter(|snapshot| {
            snapshot.generation == frame.generation
                && snapshot.frame <= frame.frame
                && snapshot.time <= frame.time
        });
        let update = previous.is_none_or(|snapshot| snapshot.frame != frame.frame);
        let read = previous.map_or(0, |snapshot| snapshot.index);
        let cursor = previous.map_or(0, |snapshot| snapshot.cursor);
        let mut total = 0;
        let emissions = frame
            .spawns
            .iter()
            .take(MAX_PARTICLE_SPAWNS)
            .map(|spawn| {
                let count = spawn.count.min(system.capacity);
                let start = total;
                total += count;
                let min_life = spawn.lifetime.start.as_secs_f32().max(0.001);
                let min_radius = f32::from(spawn.radius.start).max(0.25);
                let min_speed = f32::from(spawn.speed.start).max(0.);
                Spawn {
                    range: [start, count, 0, 0],
                    line: [
                        spawn.from.x.into(),
                        spawn.from.y.into(),
                        spawn.to.x.into(),
                        spawn.to.y.into(),
                    ],
                    velocity: [
                        spawn.velocity.x.into(),
                        spawn.velocity.y.into(),
                        min_speed,
                        f32::from(spawn.speed.end).max(min_speed),
                    ],
                    life: [
                        min_life,
                        spawn.lifetime.end.as_secs_f32().max(min_life),
                        min_radius,
                        f32::from(spawn.radius.end).max(min_radius),
                    ],
                    color: [spawn.color.r, spawn.color.g, spawn.color.b, spawn.color.a],
                    shape: [spawn.stretch.clamp(0., 0.25).max(0.), 0., 0., 0.],
                }
            })
            .collect::<Vec<_>>();
        let physics = frame.physics;
        let params = Params {
            mask: [
                u32::from(mask.is_some()),
                u32::from(mask.is_some_and(|mask| mask.inherit_color)),
                0,
                0,
            ],
            counts: [
                system.capacity,
                cursor,
                total,
                (frame.frame as u32)
                    .wrapping_mul(1664525)
                    .wrapping_add(1013904223),
            ],
            timing: [
                previous.map_or(0., |snapshot| {
                    frame.time.saturating_sub(snapshot.time).as_secs_f32()
                }),
                if previous.is_none() { 1. } else { 0. },
                emissions.len() as f32,
                0.,
            ],
            viewport: [viewport[0], viewport[1], draw.scale_factor, draw.opacity],
            bounds: [
                draw.bounds.origin.x.0,
                draw.bounds.origin.y.0,
                draw.bounds.size.width.0,
                draw.bounds.size.height.0,
            ],
            clip: [
                draw.content_mask.bounds.origin.x.0,
                draw.content_mask.bounds.origin.y.0,
                draw.content_mask.bounds.size.width.0,
                draw.content_mask.bounds.size.height.0,
            ],
            acceleration: [
                physics.acceleration.x.into(),
                physics.acceleration.y.into(),
                physics.drag.max(0.),
                0.,
            ],
            field: [
                physics.attractor.x.into(),
                physics.attractor.y.into(),
                physics.strength.into(),
                f32::from(physics.radius).max(1.),
            ],
        };
        queue.write_buffer(&system.uniform, 0, bytemuck::bytes_of(&params));
        if update {
            if !emissions.is_empty() {
                queue.write_buffer(&system.emissions, 0, bytemuck::cast_slice(&emissions));
            }
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("particle_simulation"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.update);
            pass.set_bind_group(0, &system.update[read], &[]);
            pass.set_bind_group(1, &system.candidate_group, &[]);
            pass.dispatch_workgroups(system.capacity.div_ceil(64), 1, 1);
            system.pending.set(Some(Snapshot {
                generation: frame.generation,
                frame: frame.frame,
                time: frame.time,
                index: 1 - read,
                cursor: (cursor + total) % system.capacity,
            }));
        } else {
            system.pending.set(previous);
        }
    }

    pub(super) fn masked_draw(
        quad: &EffectQuad,
        particles: &gpui::SubtreeParticlePass,
    ) -> ParticleDraw {
        ParticleDraw {
            order: 0,
            bounds: quad.bounds,
            content_mask: gpui::ContentMask {
                bounds: quad.bounds,
            },
            scale_factor: particles.scale_factor,
            opacity: 1.,
            frame: particles.frame.clone(),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn encode_masked(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        draw: &ParticleDraw,
        mask: gpui::ParticleMask,
        source: &wgpu::TextureView,
        viewport: [f32; 2],
        encoder: &mut wgpu::CommandEncoder,
    ) {
        let system = &self.systems[&draw.frame.id];
        let replay = system.committed.get().is_some_and(|previous| {
            previous.generation == draw.frame.generation
                && previous.frame == draw.frame.frame
                && previous.time == draw.frame.time
        });
        if !replay && !draw.frame.spawns.is_empty() {
            let region =
                super::distance_field::region(draw.bounds, viewport.map(|value| value as u32));
            let mut step = 1u32;
            while u64::from(region[2].div_ceil(step)) * u64::from(region[3].div_ceil(step))
                > u64::from(MAX_MASK_SAMPLES)
            {
                step += 1;
            }
            let threshold = if mask.threshold.is_finite() {
                mask.threshold.clamp(0.001, 0.999)
            } else {
                0.5
            };
            let edge = f32::from(mask.edge_width);
            let edge = if edge.is_finite() {
                edge.clamp(0., 128.) * draw.scale_factor
            } else {
                0.
            };
            let params = MaskParams {
                bounds: [
                    draw.bounds.origin.x.0,
                    draw.bounds.origin.y.0,
                    draw.scale_factor,
                    step as f32,
                ],
                region,
                settings: [threshold, edge, 0., 0.],
            };
            queue.write_buffer(&system.mask_uniform, 0, bytemuck::bytes_of(&params));
            encoder.clear_buffer(&system.candidates, 0, Some(16));
            if region[2] > 0 && region[3] > 0 {
                let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("particle_mask_capture"),
                    layout: &self.mask.get_bind_group_layout(0),
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: system.mask_uniform.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::TextureView(source),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: system.candidates.as_entire_binding(),
                        },
                    ],
                });
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("particle_mask_sampling"),
                    timestamp_writes: None,
                });
                pass.set_pipeline(&self.mask);
                pass.set_bind_group(0, &group, &[]);
                pass.dispatch_workgroups(
                    region[2].div_ceil(step).div_ceil(8),
                    region[3].div_ceil(step).div_ceil(8),
                    1,
                );
            }
        }
        self.encode_draw(queue, draw, viewport, Some(mask), encoder);
    }

    pub(super) fn draw(&self, draw: &ParticleDraw, pass: &mut wgpu::RenderPass<'_>) {
        let system = &self.systems[&draw.frame.id];
        let Some(snapshot) = system.pending.get() else {
            return;
        };
        pass.set_pipeline(&self.draw);
        pass.set_bind_group(0, &system.draw[snapshot.index], &[]);
        pass.draw(0..4, 0..system.capacity);
    }

    pub(super) fn commit(&self, success: bool) {
        for system in self.systems.values() {
            if let Some(snapshot) = system.pending.take()
                && success
            {
                system.committed.set(Some(snapshot));
            }
        }
    }
}
