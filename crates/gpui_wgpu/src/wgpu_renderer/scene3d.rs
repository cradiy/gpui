use super::*;
use gpui::{Mesh3d, MeshTexture3d, SubtreeLayer};
use wgpu::util::DeviceExt;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Vertex {
    position: [f32; 3],
    normal: [f32; 3],
    uv: [f32; 2],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Params {
    model: [[f32; 4]; 4],
    normal: [[f32; 4]; 4],
    camera: [[f32; 4]; 4],
    bounds: [f32; 4],
    viewport: [f32; 4],
    direction: [f32; 4],
    light: [f32; 4],
    color: [f32; 4],
    texture_rect: [f32; 4],
    flags: [f32; 4],
}

struct Geometry {
    _mesh: Arc<Mesh3d>,
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    count: u32,
}

pub(super) struct Scene3dRenderer {
    pipeline: wgpu::RenderPipeline,
    sampler: wgpu::Sampler,
    white: wgpu::TextureView,
    geometry: HashMap<usize, Geometry>,
    slots: Vec<wgpu::Buffer>,
    offsets: HashMap<usize, usize>,
    targets: Option<(wgpu::Texture, wgpu::Texture)>,
    format: wgpu::TextureFormat,
    samples: u32,
}

impl Scene3dRenderer {
    pub(super) fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
        samples: u32,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("scene3d"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../scene3d.wgsl").into()),
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("scene3d"), layout: None,
            vertex: wgpu::VertexState {
                module: &shader, entry_point: Some("vertex"), compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout { array_stride: 32, step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x2] })],
            },
            fragment: Some(wgpu::FragmentState { module: &shader, entry_point: Some("fragment"), compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState { format, blend: None, write_mask: wgpu::ColorWrites::ALL })] }),
            primitive: wgpu::PrimitiveState { cull_mode: None, ..Default::default() },
            depth_stencil: Some(wgpu::DepthStencilState { format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(true), depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: Default::default(), bias: Default::default() }),
            multisample: wgpu::MultisampleState { count: samples, ..Default::default() },
            multiview_mask: None, cache: None,
        });
        let white = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("scene3d_white"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            white.as_image_copy(),
            &[255; 4],
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4),
                rows_per_image: Some(1),
            },
            white.size(),
        );
        Self {
            pipeline,
            sampler: device.create_sampler(&wgpu::SamplerDescriptor {
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            }),
            white: white.create_view(&Default::default()),
            geometry: HashMap::new(),
            slots: Vec::new(),
            offsets: HashMap::new(),
            targets: None,
            format,
            samples,
        }
    }

    pub(super) fn prepare(
        &mut self,
        device: &wgpu::Device,
        scene: &Scene,
        width: u32,
        height: u32,
    ) {
        self.offsets.clear();
        let mut used = HashSet::new();
        let mut slot_count = 0;
        scene.visit(&mut |scene| {
            for layer in &scene.subtree_layers {
                let Some(frame) = &layer.scene3d else {
                    continue;
                };
                self.offsets.insert(layer as *const _ as usize, slot_count);
                slot_count += frame.objects.len();
                for object in frame.objects.iter() {
                    let key = Arc::as_ptr(&object.mesh) as usize;
                    used.insert(key);
                    self.geometry.entry(key).or_insert_with(|| {
                        let vertices = object
                            .mesh
                            .vertices()
                            .iter()
                            .map(|v| Vertex {
                                position: v.position,
                                normal: v.normal,
                                uv: v.uv,
                            })
                            .collect::<Vec<_>>();
                        Geometry {
                            _mesh: object.mesh.clone(),
                            vertices: device.create_buffer_init(
                                &wgpu::util::BufferInitDescriptor {
                                    label: Some("mesh_vertices"),
                                    contents: bytemuck::cast_slice(&vertices),
                                    usage: wgpu::BufferUsages::VERTEX,
                                },
                            ),
                            indices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                                label: Some("mesh_indices"),
                                contents: bytemuck::cast_slice(object.mesh.indices()),
                                usage: wgpu::BufferUsages::INDEX,
                            }),
                            count: object.mesh.indices().len() as u32,
                        }
                    });
                }
            }
        });
        self.geometry.retain(|key, _| used.contains(key));
        self.slots.truncate(slot_count);
        while self.slots.len() < slot_count {
            self.slots
                .push(device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("mesh_params"),
                    size: std::mem::size_of::<Params>() as u64,
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }));
        }
        if self.offsets.is_empty() {
            self.targets = None;
            return;
        }
        if self
            .targets
            .as_ref()
            .is_none_or(|(depth, _)| depth.width() != width || depth.height() != height)
        {
            let texture = |format, label| {
                device.create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d {
                        width,
                        height,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: self.samples,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                    view_formats: &[],
                })
            };
            self.targets = Some((
                texture(wgpu::TextureFormat::Depth32Float, "scene3d_depth"),
                texture(self.format, "scene3d_color"),
            ));
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn encode(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        atlas: &WgpuAtlas,
        layer: &SubtreeLayer,
        source: &wgpu::TextureView,
        destination: &wgpu::TextureView,
        encoder: &mut wgpu::CommandEncoder,
    ) {
        let frame = layer.scene3d.as_ref().unwrap();
        let (depth, color) = self.targets.as_ref().unwrap();
        let width = depth.width() as f32;
        let height = depth.height() as f32;
        let bounds = layer.composite.bounds;
        let rect = [
            bounds.origin.x.0,
            bounds.origin.y.0,
            bounds.size.width.0,
            bounds.size.height.0,
        ];
        let start = self.offsets[&(layer as *const _ as usize)];
        let layout = self.pipeline.get_bind_group_layout(0);
        let mut groups = Vec::with_capacity(frame.objects.len());
        for (index, object) in frame.objects.iter().enumerate() {
            let atlas_texture = match object.texture {
                MeshTexture3d::Image(tile) => Some(atlas.get_texture_info(tile.texture_id)),
                _ => None,
            };
            let (texture, texture_rect, premultiplied) = match object.texture {
                MeshTexture3d::None => (&self.white, [0., 0., 1., 1.], 0.),
                MeshTexture3d::Subtree => {
                    let texture_rect = frame.ui_texture.map_or(rect, |texture| {
                        let size = texture.pixel_size();
                        [0., 0., size.width.0 as f32, size.height.0 as f32]
                    });
                    (source, texture_rect, 1.)
                }
                MeshTexture3d::Image(tile) => {
                    let r = tile.bounds;
                    (
                        &atlas_texture.as_ref().unwrap().view,
                        [
                            r.origin.x.0 as f32,
                            r.origin.y.0 as f32,
                            r.size.width.0 as f32,
                            r.size.height.0 as f32,
                        ],
                        0.,
                    )
                }
            };
            let params = Params {
                model: object.model,
                normal: object.normal,
                camera: frame.view_projection,
                bounds: rect,
                viewport: [width, height, 0., 0.],
                direction: [
                    frame.light_direction[0],
                    frame.light_direction[1],
                    frame.light_direction[2],
                    frame.ambient,
                ],
                light: frame.light,
                color: [
                    object.color.r,
                    object.color.g,
                    object.color.b,
                    object.color.a,
                ],
                texture_rect,
                flags: [
                    object.alpha_cutoff.clamp(0.001, 1.),
                    f32::from(object.unlit),
                    premultiplied,
                    0.,
                ],
            };
            let buffer = &self.slots[start + index];
            queue.write_buffer(buffer, 0, bytemuck::bytes_of(&params));
            groups.push(device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("mesh_material"),
                layout: &layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: buffer.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(texture),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                ],
            }));
        }
        let depth_view = depth.create_view(&Default::default());
        let color_view = color.create_view(&Default::default());
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("scene3d"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: if self.samples > 1 {
                    &color_view
                } else {
                    destination
                },
                resolve_target: (self.samples > 1).then_some(destination),
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &depth_view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.),
                    store: wgpu::StoreOp::Discard,
                }),
                stencil_ops: None,
            }),
            ..Default::default()
        });
        let x = rect[0].max(0.).floor() as u32;
        let y = rect[1].max(0.).floor() as u32;
        let right = (rect[0] + rect[2]).min(width).ceil().max(0.) as u32;
        let bottom = (rect[1] + rect[3]).min(height).ceil().max(0.) as u32;
        if right <= x || bottom <= y {
            return;
        }
        pass.set_scissor_rect(x, y, right - x, bottom - y);
        pass.set_pipeline(&self.pipeline);
        for (object, group) in frame.objects.iter().zip(&groups) {
            let geometry = &self.geometry[&(Arc::as_ptr(&object.mesh) as usize)];
            pass.set_bind_group(0, group, &[]);
            pass.set_vertex_buffer(0, geometry.vertices.slice(..));
            pass.set_index_buffer(geometry.indices.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..geometry.count, 0, 0..1);
        }
    }
}
