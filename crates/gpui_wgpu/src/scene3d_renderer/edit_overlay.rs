use super::{Scene3dCapabilities, Scene3dGpuOutput};
use crate::wgpu_renderer::scene3d::RenderRegion;
use crate::{WgpuContext, WgpuScene3dPickFrame};
use anyhow::{Result, ensure};
use gpui::{EditHiddenStyle3d, OcclusionGroup3d, Scene3dFrame};
use std::sync::{Arc, atomic::AtomicBool};
use wgpu::util::DeviceExt;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Instance {
    a: [f32; 4],
    b: [f32; 4],
    color: [f32; 4],
    hidden: [f32; 4],
    shape: [f32; 4],
    params: [f32; 4],
    ids: [u32; 4],
}

pub(crate) fn payload_bytes(group: &OcclusionGroup3d) -> Result<u64> {
    group
        .points
        .len()
        .checked_add(group.lines.len())
        .and_then(|n| (n as u64).checked_mul(std::mem::size_of::<Instance>() as u64))
        .ok_or_else(|| anyhow::anyhow!("edit overlay instance size overflow"))
}

pub(crate) struct EditOverlay {
    buffer: wgpu::Buffer,
    count: u32,
    bindings: wgpu::BindGroup,
    color: wgpu::RenderPipeline,
    data: wgpu::RenderPipeline,
    output: Arc<WgpuScene3dPickFrame>,
}

impl EditOverlay {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        context: &WgpuContext,
        capabilities: Scene3dCapabilities,
        frame: &Arc<Scene3dFrame>,
        definition: &OcclusionGroup3d,
        region: RenderRegion,
        occlusion: &Scene3dGpuOutput,
        samples: u32,
        busy: Arc<AtomicBool>,
    ) -> Result<Self> {
        let device = &context.device;
        ensure!(
            device.limits().max_storage_buffers_per_shader_stage >= 1
                && definition.occluders.len().max(1) as u64 * 4
                    <= device.limits().max_storage_buffer_binding_size
                && device.limits().max_color_attachments >= 2
                && device.limits().max_color_attachment_bytes_per_sample >= 8,
            "edit overlays exceed device storage or color attachment limits"
        );
        let instances = instances(frame, definition, region)?;
        let bytes = (instances.len() as u64)
            .checked_mul(std::mem::size_of::<Instance>() as u64)
            .ok_or_else(|| anyhow::anyhow!("edit overlay instance size overflow"))?;
        ensure!(
            bytes <= context.device.limits().max_buffer_size
                && instances.len() <= u32::MAX as usize,
            "edit overlay exceeds device vertex buffer limits"
        );
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("edit_overlay.instances"),
            contents: bytemuck::cast_slice(&instances),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let mut output = WgpuScene3dPickFrame::allocate(
            context.clone(),
            capabilities,
            frame,
            region.size,
            region.rect,
            region.source_rect,
            busy,
        )?;
        output.set_edit_payload(bytes, instances.len() as u64);
        let output = Arc::new(output);
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("edit_overlay"),
            source: wgpu::ShaderSource::Wgsl(include_str!("edit_overlay.wgsl").into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("edit_overlay"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(16),
                    },
                    count: None,
                },
                texture_entry(1, wgpu::TextureSampleType::Float { filterable: false }),
                texture_entry(2, wgpu::TextureSampleType::Uint),
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(4),
                    },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("edit_overlay"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let make_pipeline = |entry: &str, targets: &[Option<wgpu::ColorTargetState>], samples| {
            const ATTRIBUTES: [wgpu::VertexAttribute; 7] = wgpu::vertex_attr_array![0=>Float32x4,1=>Float32x4,2=>Float32x4,3=>Float32x4,4=>Float32x4,5=>Float32x4,6=>Uint32x4];
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("edit_overlay"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("vertex_main"),
                    compilation_options: Default::default(),
                    buffers: &[Some(wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<Instance>() as u64,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: &ATTRIBUTES,
                    })],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    targets,
                }),
                primitive: Default::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState {
                    count: samples,
                    ..Default::default()
                },
                multiview_mask: None,
                cache: None,
            })
        };
        let color = make_pipeline(
            "color_main",
            &[Some(wgpu::ColorTargetState {
                format: wgpu::TextureFormat::Rgba16Float,
                blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
            samples,
        );
        let data = make_pipeline(
            "data_main",
            &[
                Some(wgpu::TextureFormat::R32Uint.into()),
                Some(wgpu::TextureFormat::R32Float.into()),
            ],
            1,
        );
        let density = [
            region.size[0] as f32 / region.output_size[0] as f32,
            region.size[1] as f32 / region.output_size[1] as f32,
        ];
        let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("edit_overlay.view"),
            contents: bytemuck::cast_slice(&[
                region.size[0] as f32,
                region.size[1] as f32,
                density[0],
                density[1],
            ]),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let depth = occlusion
            .linear_depth()
            .unwrap()
            .create_view(&Default::default());
        let ids = occlusion
            .object_ids()
            .unwrap()
            .create_view(&Default::default());
        let mut auxiliary_ids: Vec<_> = definition.occluders.iter().map(|o| o.output_id).collect();
        auxiliary_ids.sort_unstable();
        if auxiliary_ids.is_empty() {
            auxiliary_ids.push(0);
        }
        let auxiliary = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("edit_overlay.auxiliary_ids"),
            contents: bytemuck::cast_slice(&auxiliary_ids),
            usage: wgpu::BufferUsages::STORAGE,
        });
        let bindings = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("edit_overlay"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&depth),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&ids),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: auxiliary.as_entire_binding(),
                },
            ],
        });
        Ok(Self {
            buffer,
            count: instances.len() as u32,
            bindings,
            color,
            data,
            output,
        })
    }

    pub(crate) fn output(&self) -> Arc<WgpuScene3dPickFrame> {
        self.output.clone()
    }
    pub(crate) fn encode_data(&self, encoder: &mut wgpu::CommandEncoder) {
        let gpu = self.output.gpu();
        let ids = gpu.object_ids().unwrap().create_view(&Default::default());
        let depth = gpu.linear_depth().unwrap().create_view(&Default::default());
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("edit_overlay.identities"),
            color_attachments: &[
                Some(attachment(
                    &ids,
                    wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                )),
                Some(attachment(
                    &depth,
                    wgpu::LoadOp::Clear(wgpu::Color {
                        r: f64::from(gpu.depth_background().value()),
                        ..wgpu::Color::TRANSPARENT
                    }),
                )),
            ],
            ..Default::default()
        });
        self.draw(&mut pass, &self.data);
    }
    pub(crate) fn encode_color(
        &self,
        target: &wgpu::TextureView,
        encoder: &mut wgpu::CommandEncoder,
    ) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("edit_overlay.color"),
            color_attachments: &[Some(attachment(target, wgpu::LoadOp::Load))],
            ..Default::default()
        });
        self.draw(&mut pass, &self.color);
    }
    fn draw<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>, pipeline: &'a wgpu::RenderPipeline) {
        if self.count == 0 {
            return;
        }
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &self.bindings, &[]);
        pass.set_vertex_buffer(0, self.buffer.slice(..));
        pass.draw(0..6, 0..self.count);
    }
}

fn texture_entry(binding: u32, sample_type: wgpu::TextureSampleType) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type,
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}
fn attachment(
    view: &wgpu::TextureView,
    load: wgpu::LoadOp<wgpu::Color>,
) -> wgpu::RenderPassColorAttachment<'_> {
    wgpu::RenderPassColorAttachment {
        view,
        resolve_target: None,
        depth_slice: None,
        ops: wgpu::Operations {
            load,
            store: wgpu::StoreOp::Store,
        },
    }
}

fn instances(
    frame: &Scene3dFrame,
    group: &OcclusionGroup3d,
    region: RenderRegion,
) -> Result<Vec<Instance>> {
    ensure!(
        group.pixel_scale.is_finite() && group.pixel_scale > 0.,
        "invalid edit overlay pixel scale"
    );
    let mut ids = std::collections::HashSet::new();
    let mut instances = Vec::new();
    let primitives = group
        .lines
        .iter()
        .map(|l| (l.id, l.start, l.end, l.style))
        .chain(
            group
                .points
                .iter()
                .map(|p| (p.id, p.position, p.position, p.style)),
        );
    for (id, a, b, style) in primitives {
        ensure!(
            id != 0 && ids.insert(id),
            "edit element IDs must be unique and nonzero within a group"
        );
        ensure!(
            style.is_valid() && a.iter().chain(&b).all(|v| v.is_finite()),
            "invalid edit overlay geometry or style"
        );
        ensure!(
            (style.size * group.pixel_scale).is_finite(),
            "edit overlay size overflow"
        );
        if let Some((a, b, phase)) = project(
            frame,
            region,
            a,
            b,
            style.size as f64 * group.pixel_scale as f64 * 0.5,
        ) {
            let rgba = |c: gpui::Rgba| [c.r, c.g, c.b, c.a];
            let (mode, dash, gap) = match style.hidden {
                EditHiddenStyle3d::Hide => (0., 1., 1.),
                EditHiddenStyle3d::Solid => (1., 1., 1.),
                EditHiddenStyle3d::Dashed { dash, gap } => {
                    (2., dash * group.pixel_scale, gap * group.pixel_scale)
                }
            };
            let phase = (phase % (f64::from(dash) + f64::from(gap))) as f32;
            ensure!(
                [dash, gap, phase].iter().all(|v| v.is_finite()),
                "edit overlay dash size overflow"
            );
            instances.push(Instance {
                a,
                b,
                color: rgba(style.color),
                hidden: rgba(style.hidden_color),
                shape: [style.size * group.pixel_scale * 0.5, mode, dash, gap],
                params: [style.depth_tolerance, phase, 0., 0.],
                ids: [id, 0, 0, 0],
            });
        }
    }
    Ok(instances)
}

fn project(
    frame: &Scene3dFrame,
    region: RenderRegion,
    a: [f32; 3],
    b: [f32; 3],
    radius: f64,
) -> Option<([f32; 4], [f32; 4], f64)> {
    let transform = |matrix: [[f32; 4]; 4], p: [f32; 3]| -> [f64; 4] {
        std::array::from_fn(|r| {
            (0..3)
                .map(|c| f64::from(matrix[c][r]) * f64::from(p[c]))
                .sum::<f64>()
                + f64::from(matrix[3][r])
        })
    };
    let ca = transform(frame.view_projection, a);
    let cb = transform(frame.view_projection, b);
    let da = -transform(frame.world_to_view, a)[2];
    let db = -transform(frame.world_to_view, b)[2];
    let mut t0: f64 = 0.;
    let mut t1: f64 = 1.;
    // Clip depth before division; side clipping retains the dash anchor.
    for (a, b) in [(ca[2], cb[2]), (ca[3] - ca[2], cb[3] - cb[2])] {
        if a < 0. && b < 0. {
            return None;
        }
        if a < 0. {
            t0 = t0.max(a / (a - b));
        } else if b < 0. {
            t1 = t1.min(a / (a - b));
        }
    }
    if t0 > t1 {
        return None;
    }
    let density = [
        region.size[0] as f64 / region.output_size[0] as f64,
        region.size[1] as f64 / region.output_size[1] as f64,
    ];
    let screen = |t: f64| -> Option<[f64; 4]> {
        let p: [f64; 4] = std::array::from_fn(|i| ca[i] + (cb[i] - ca[i]) * t);
        if p[3] <= 0. {
            return None;
        }
        Some([
            (f64::from(region.rect[0]) + (p[0] / p[3] * 0.5 + 0.5) * f64::from(region.rect[2]))
                / density[0],
            (f64::from(region.rect[1]) + (0.5 - p[1] / p[3] * 0.5) * f64::from(region.rect[3]))
                / density[1],
            da + (db - da) * t,
            1. / p[3],
        ])
    };
    let a = screen(t0)?;
    let b = screen(t1)?;
    let mut s0: f64 = 0.;
    let mut s1: f64 = 1.;
    let padding = radius + 2. / density[0].min(density[1]);
    for axis in 0..2 {
        let delta = b[axis] - a[axis];
        let lo = -padding;
        let hi = region.output_size[axis] as f64 + padding;
        if delta == 0. {
            if a[axis] < lo || a[axis] > hi {
                return None;
            }
        } else {
            let x = (lo - a[axis]) / delta;
            let y = (hi - a[axis]) / delta;
            s0 = s0.max(x.min(y));
            s1 = s1.min(x.max(y));
        }
    }
    if s0 > s1 {
        return None;
    }
    let at = |t: f64| -> [f32; 4] {
        let iw = a[3] + (b[3] - a[3]) * t;
        [
            (a[0] + (b[0] - a[0]) * t) as f32,
            (a[1] + (b[1] - a[1]) * t) as f32,
            ((a[2] * a[3] + (b[2] * b[3] - a[2] * a[3]) * t) / iw) as f32,
            iw as f32,
        ]
    };
    let phase = ((b[0] - a[0]).hypot(b[1] - a[1])) * s0;
    let (a, b) = (at(s0), at(s1));
    if !phase.is_finite() || !a.iter().chain(&b).all(|v| v.is_finite()) || a[3] <= 0. || b[3] <= 0.
    {
        return None;
    }
    Some((a, b, phase))
}
