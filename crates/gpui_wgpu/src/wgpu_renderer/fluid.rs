use super::*;
use gpui::{FluidDraw, MAX_FLUID_SPLATS};

const UPDATE: &str = concat!(
    include_str!("../fluid.wgsl"),
    include_str!("../fluid_update.wgsl")
);
const DRAW: &str = concat!(
    include_str!("../fluid.wgsl"),
    include_str!("../fluid_draw.wgsl")
);
const STAGES: [&str; 7] = [
    "advect_velocity",
    "curl",
    "confine",
    "divergence",
    "pressure",
    "project",
    "advect_dye",
];

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Params {
    grid: [u32; 4],
    step: [f32; 4],
    viewport: [f32; 4],
    bounds: [f32; 4],
    clip: [f32; 4],
    logical: [f32; 4],
}
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Splat {
    line: [f32; 4],
    velocity: [f32; 4],
    color: [f32; 4],
}
#[derive(Clone, Copy)]
struct Snapshot {
    generation: u64,
    frame: u64,
    time: Duration,
    index: usize,
}
struct System {
    size: Size<ScaledPixels>,
    scale: f32,
    resolution: u32,
    grid: [u32; 2],
    uniform: wgpu::Buffer,
    splats: wgpu::Buffer,
    pressure_zero: wgpu::Buffer,
    advect_velocity: [wgpu::BindGroup; 2],
    curl: wgpu::BindGroup,
    confine: wgpu::BindGroup,
    divergence: wgpu::BindGroup,
    pressure: [wgpu::BindGroup; 2],
    project: [[wgpu::BindGroup; 2]; 2],
    advect_dye: [wgpu::BindGroup; 2],
    draw: [wgpu::BindGroup; 2],
    committed: Cell<Option<Snapshot>>,
    pending: Cell<Option<Snapshot>>,
}
pub(super) struct FluidRenderer {
    layout: wgpu::BindGroupLayout,
    compute: [wgpu::ComputePipeline; 7],
    draw: wgpu::RenderPipeline,
    systems: HashMap<gpui::EffectHistoryId, System>,
}
impl FluidRenderer {
    pub(super) fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("fluid_compute"),
            entries: &(0..5)
                .map(|binding| wgpu::BindGroupLayoutEntry {
                    binding,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: if binding == 0 {
                            wgpu::BufferBindingType::Uniform
                        } else {
                            wgpu::BufferBindingType::Storage {
                                read_only: binding != 3,
                            }
                        },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                })
                .collect::<Vec<_>>(),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("fluid_compute"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("fluid_compute"),
            source: wgpu::ShaderSource::Wgsl(UPDATE.into()),
        });
        let compute = STAGES.map(|stage| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(stage),
                layout: Some(&pipeline_layout),
                module: &shader,
                entry_point: Some(stage),
                compilation_options: Default::default(),
                cache: None,
            })
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("fluid_draw"),
            source: wgpu::ShaderSource::Wgsl(DRAW.into()),
        });
        let draw = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("fluid_draw"),
            layout: None,
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vertex"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
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
        Self {
            layout,
            compute,
            draw,
            systems: HashMap::new(),
        }
    }

    pub(super) fn ensure(&mut self, device: &wgpu::Device, scene: &Scene) {
        let mut required = HashMap::new();
        scene.visit(&mut |scene| {
            for draw in &scene.fluids {
                assert!(
                    required
                        .insert(
                            draw.frame.id,
                            (
                                draw.bounds.size,
                                draw.scale_factor,
                                draw.frame.options.normalized().resolution
                            )
                        )
                        .is_none(),
                    "a fluid identity may only occur once in a scene"
                );
            }
        });
        self.systems.retain(|id, s| {
            required.get(id).is_some_and(|&(size, scale, resolution)| {
                s.size == size && s.scale == scale && s.resolution == resolution
            })
        });
        for (id, (size, scale, resolution)) in required {
            if self.systems.contains_key(&id) {
                continue;
            }
            let longest = size.width.0.max(size.height.0).max(1.);
            let grid = [
                (size.width.0 / longest * resolution as f32).round().max(2.) as u32,
                (size.height.0 / longest * resolution as f32)
                    .round()
                    .max(2.) as u32,
            ];
            let buffer = |label, size, usage| {
                device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some(label),
                    size,
                    usage,
                    mapped_at_creation: false,
                })
            };
            let uniform = buffer(
                "fluid_params",
                std::mem::size_of::<Params>() as u64,
                wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            );
            let splats = buffer(
                "fluid_splats",
                (std::mem::size_of::<Splat>() * MAX_FLUID_SPLATS) as u64,
                wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            );
            let field = || {
                buffer(
                    "fluid_field",
                    u64::from(grid[0]) * u64::from(grid[1]) * 16,
                    wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                )
            };
            let velocity = [field(), field()];
            let dye = [field(), field()];
            let scratch = [field(), field()];
            let scalar = field();
            let pressure = [field(), field()];
            let bind = |a: &wgpu::Buffer, b: &wgpu::Buffer, out: &wgpu::Buffer| {
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("fluid_compute"),
                    layout: &self.layout,
                    entries: &[&uniform, a, b, out, &splats]
                        .into_iter()
                        .enumerate()
                        .map(|(binding, buffer)| wgpu::BindGroupEntry {
                            binding: binding as u32,
                            resource: buffer.as_entire_binding(),
                        })
                        .collect::<Vec<_>>(),
                })
            };
            let advect_velocity =
                std::array::from_fn(|read| bind(&velocity[read], &dye[read], &scratch[0]));
            let curl = bind(&scratch[0], &dye[0], &scalar);
            let confine = bind(&scratch[0], &scalar, &scratch[1]);
            let divergence = bind(&scratch[1], &dye[0], &scalar);
            let pressure_groups =
                std::array::from_fn(|read| bind(&pressure[read], &scalar, &pressure[1 - read]));
            let project = std::array::from_fn(|read| {
                std::array::from_fn(|p| bind(&scratch[1], &pressure[p], &velocity[1 - read]))
            });
            let advect_dye =
                std::array::from_fn(|read| bind(&velocity[1 - read], &dye[read], &dye[1 - read]));
            let draw = std::array::from_fn(|index| {
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("fluid_draw"),
                    layout: &self.draw.get_bind_group_layout(0),
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: uniform.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: dye[index].as_entire_binding(),
                        },
                    ],
                })
            });
            let [pressure_zero, _] = pressure;
            self.systems.insert(
                id,
                System {
                    size,
                    scale,
                    resolution,
                    grid,
                    uniform,
                    splats,
                    pressure_zero,
                    advect_velocity,
                    curl,
                    confine,
                    divergence,
                    pressure: pressure_groups,
                    project,
                    advect_dye,
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
            for draw in &scene.fluids {
                let s = &self.systems[&draw.frame.id];
                let frame = &draw.frame;
                let previous = s.committed.get().filter(|p| {
                    p.generation == frame.generation
                        && p.frame <= frame.frame
                        && p.time <= frame.time
                });
                let update = previous.is_none_or(|p| p.frame != frame.frame);
                let read = previous.map_or(0, |p| p.index);
                let splats = frame
                    .splats
                    .iter()
                    .take(MAX_FLUID_SPLATS)
                    .filter(|s| {
                        [
                            s.from.x,
                            s.from.y,
                            s.to.x,
                            s.to.y,
                            s.velocity.x,
                            s.velocity.y,
                            s.radius,
                        ]
                        .iter()
                        .all(|v| f32::from(*v).is_finite())
                    })
                    .map(|s| Splat {
                        line: [
                            s.from.x.into(),
                            s.from.y.into(),
                            s.to.x.into(),
                            s.to.y.into(),
                        ],
                        velocity: [
                            f32::from(s.velocity.x).clamp(-2000., 2000.),
                            f32::from(s.velocity.y).clamp(-2000., 2000.),
                            f32::from(s.radius).max(0.5),
                            s.amount.clamp(0., 4.).max(0.),
                        ],
                        color: [
                            s.color.r.clamp(0., 1.).max(0.),
                            s.color.g.clamp(0., 1.).max(0.),
                            s.color.b.clamp(0., 1.).max(0.),
                            s.color.a.clamp(0., 1.).max(0.),
                        ],
                    })
                    .collect::<Vec<_>>();
                let options = frame.options.normalized();
                let params = Params {
                    grid: [
                        s.grid[0],
                        s.grid[1],
                        splats.len() as u32,
                        u32::from(previous.is_none()),
                    ],
                    step: [
                        previous.map_or(0., |p| frame.time.saturating_sub(p.time).as_secs_f32()),
                        options.velocity_decay,
                        options.dye_decay,
                        options.vorticity,
                    ],
                    viewport: [viewport[0], viewport[1], draw.scale_factor, draw.opacity],
                    bounds: [
                        draw.bounds.origin.x.0,
                        draw.bounds.origin.y.0,
                        s.size.width.0,
                        s.size.height.0,
                    ],
                    clip: [
                        draw.content_mask.bounds.origin.x.0,
                        draw.content_mask.bounds.origin.y.0,
                        draw.content_mask.bounds.size.width.0,
                        draw.content_mask.bounds.size.height.0,
                    ],
                    logical: [s.size.width.0 / s.scale, s.size.height.0 / s.scale, 0., 0.],
                };
                queue.write_buffer(&s.uniform, 0, bytemuck::bytes_of(&params));
                if update {
                    if !splats.is_empty() {
                        queue.write_buffer(&s.splats, 0, bytemuck::cast_slice(&splats));
                    }
                    encoder.clear_buffer(&s.pressure_zero, 0, None);
                    let mut dispatch = |stage: usize, group: &wgpu::BindGroup| {
                        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                            label: Some(STAGES[stage]),
                            timestamp_writes: None,
                        });
                        pass.set_pipeline(&self.compute[stage]);
                        pass.set_bind_group(0, group, &[]);
                        pass.dispatch_workgroups(s.grid[0].div_ceil(8), s.grid[1].div_ceil(8), 1);
                    };
                    dispatch(0, &s.advect_velocity[read]);
                    dispatch(1, &s.curl);
                    dispatch(2, &s.confine);
                    dispatch(3, &s.divergence);
                    for i in 0..options.pressure_iterations {
                        dispatch(4, &s.pressure[i as usize % 2]);
                    }
                    dispatch(
                        5,
                        &s.project[read][options.pressure_iterations as usize % 2],
                    );
                    dispatch(6, &s.advect_dye[read]);
                    s.pending.set(Some(Snapshot {
                        generation: frame.generation,
                        frame: frame.frame,
                        time: frame.time,
                        index: 1 - read,
                    }));
                } else {
                    s.pending.set(previous);
                }
            }
        });
    }
    pub(super) fn draw(&self, draw: &FluidDraw, pass: &mut wgpu::RenderPass<'_>) {
        let s = &self.systems[&draw.frame.id];
        if let Some(snapshot) = s.pending.get() {
            pass.set_pipeline(&self.draw);
            pass.set_bind_group(0, &s.draw[snapshot.index], &[]);
            pass.draw(0..4, 0..1);
        }
    }
    pub(super) fn commit(&self, success: bool) {
        for s in self.systems.values() {
            if let Some(snapshot) = s.pending.take()
                && success
            {
                s.committed.set(Some(snapshot));
            }
        }
    }
}
