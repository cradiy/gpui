use super::*;
use gpui::{Mesh3d, MeshTexture3d, SubtreeLayer};
use wgpu::util::DeviceExt;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Vertex {
    position: [f32; 3],
    normal: [f32; 3],
    uv: [f32; 2],
    tangent: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ImageParams {
    rect: [f32; 4],
    uv_u: [f32; 4],
    uv_v: [f32; 4],
    sampling: [u32; 4],
}

impl ImageParams {
    fn new(map: Option<gpui::MaterialTexture3d>, color_space: gpui::TextureColorSpace3d) -> Self {
        let (rect, sampling) = map.map_or(([0., 0., 1., 1.], Default::default()), |map| {
            let r = map.tile.bounds;
            (
                [
                    r.origin.x.0 as f32,
                    r.origin.y.0 as f32,
                    r.size.width.0 as f32,
                    r.size.height.0 as f32,
                ],
                map.sampling,
            )
        });
        let rows = sampling.transform.rows();
        Self {
            rect,
            uv_u: [rows[0][0], rows[0][1], rows[0][2], 0.],
            uv_v: [rows[1][0], rows[1][1], rows[1][2], 0.],
            sampling: [
                sampling.address_u as u32,
                sampling.address_v as u32,
                sampling.filter as u32,
                color_space as u32,
            ],
        }
    }
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
    ids: [u32; 4],
    uv_u: [f32; 4],
    uv_v: [f32; 4],
    sampling: [u32; 4],
    view: [f32; 4],
    pbr: [f32; 4],
    emissive: [f32; 4],
    metallic_roughness_map: ImageParams,
    emissive_map: ImageParams,
    normal_map: ImageParams,
    normal_settings: [f32; 4],
    depth_plane: [f32; 4],
    environment_sh: [[f32; 4]; 9],
    environment: [f32; 4],
}

struct Geometry {
    _mesh: Arc<Mesh3d>,
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    count: u32,
}

struct Targets {
    depth: wgpu::Texture,
    hdr: Option<wgpu::Texture>,
}

pub(crate) struct Scene3dRenderer {
    pipeline: wgpu::RenderPipeline,
    blend_pipeline: Option<wgpu::RenderPipeline>,
    display_pipeline: Option<wgpu::RenderPipeline>,
    sampler: wgpu::Sampler,
    white: wgpu::TextureView,
    geometry: HashMap<usize, Arc<Geometry>>,
    slots: Vec<wgpu::Buffer>,
    offsets: HashMap<usize, usize>,
    targets: Option<Targets>,
    format: wgpu::TextureFormat,
    samples: u32,
}

impl Scene3dRenderer {
    pub(crate) fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
        samples: u32,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("scene3d"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../scene3d.wgsl").into()),
        });
        let data_output = matches!(
            format,
            wgpu::TextureFormat::R32Uint
                | wgpu::TextureFormat::R32Float
                | wgpu::TextureFormat::Rgba32Float
        );
        let fragment = match format {
            wgpu::TextureFormat::R32Uint => "object_id",
            wgpu::TextureFormat::R32Float => "linear_depth",
            wgpu::TextureFormat::Rgba32Float => "world_normal",
            _ => "fragment",
        };
        let mesh_format = if data_output {
            format
        } else {
            wgpu::TextureFormat::Rgba16Float
        };
        let mut bindings = vec![
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(std::mem::size_of::<Params>() as u64),
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
        ];
        for binding in if data_output {
            &[1][..]
        } else {
            &[1, 3, 4, 5][..]
        } {
            bindings.push(wgpu::BindGroupLayoutEntry {
                binding: *binding,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            });
        }
        let material_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("scene3d_material"),
            entries: &bindings,
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("scene3d"),
            bind_group_layouts: &[Some(&material_layout)],
            immediate_size: 0,
        });
        let create_pipeline = |blend: bool| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("scene3d"), layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader, entry_point: Some("vertex"), compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout { array_stride: 48, step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x2, 3 => Float32x4] })],
            },
            fragment: Some(wgpu::FragmentState { module: &shader, entry_point: Some(fragment), compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState { format: mesh_format, blend: blend.then_some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING), write_mask: wgpu::ColorWrites::ALL })] }),
            primitive: wgpu::PrimitiveState { cull_mode: None, ..Default::default() },
            depth_stencil: Some(wgpu::DepthStencilState { format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(!blend), depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: Default::default(), bias: Default::default() }),
            multisample: wgpu::MultisampleState { count: samples, ..Default::default() },
            multiview_mask: None, cache: None,
        })
        };
        let pipeline = create_pipeline(false);
        let blend_pipeline = (!data_output).then(|| create_pipeline(true));
        let display_pipeline = (!data_output).then(|| {
            let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("scene3d_display"),
                source: wgpu::ShaderSource::Wgsl(include_str!("../scene3d_display.wgsl").into()),
            });
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("scene3d_display"),
                layout: None,
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vertex"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(if samples > 1 {
                        "fragment_msaa"
                    } else {
                        "fragment"
                    }),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
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
            blend_pipeline,
            display_pipeline,
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
        let mut frames = Vec::new();
        let mut slot_count = 0;
        scene.visit(&mut |scene| {
            for layer in &scene.subtree_layers {
                let Some(frame) = &layer.scene3d else {
                    continue;
                };
                self.offsets.insert(layer as *const _ as usize, slot_count);
                slot_count += frame.objects.len();
                frames.push(frame.clone());
            }
        });
        self.prepare_frames(device, frames.iter().map(AsRef::as_ref), width, height);
    }

    pub(crate) fn prepare_frames<'a>(
        &mut self,
        device: &wgpu::Device,
        frames: impl IntoIterator<Item = &'a gpui::Scene3dFrame>,
        width: u32,
        height: u32,
    ) {
        let mut used = HashSet::new();
        let mut slot_count = 0;
        let mut has_frame = false;
        for frame in frames {
            has_frame = true;
            slot_count += frame.objects.len();
            for object in frame.objects.iter() {
                let key = Arc::as_ptr(&object.mesh) as usize;
                used.insert(key);
                self.geometry.entry(key).or_insert_with(|| {
                    let vertices = object
                        .mesh
                        .vertices()
                        .iter()
                        .enumerate()
                        .map(|(index, v)| Vertex {
                            position: v.position,
                            normal: v.normal,
                            uv: v.uv,
                            tangent: object.mesh.tangents().map_or([0.; 4], |t| t[index]),
                        })
                        .collect::<Vec<_>>();
                    Arc::new(Geometry {
                        _mesh: object.mesh.clone(),
                        vertices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                            label: Some("mesh_vertices"),
                            contents: bytemuck::cast_slice(&vertices),
                            usage: wgpu::BufferUsages::VERTEX,
                        }),
                        indices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                            label: Some("mesh_indices"),
                            contents: bytemuck::cast_slice(object.mesh.indices()),
                            usage: wgpu::BufferUsages::INDEX,
                        }),
                        count: object.mesh.indices().len() as u32,
                    })
                });
            }
        }
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
        if !has_frame {
            self.targets = None;
            return;
        }
        if self.targets.as_ref().is_none_or(|targets| {
            targets.depth.width() != width || targets.depth.height() != height
        }) {
            let texture = |format, label, sample_count, usage| {
                device.create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d {
                        width,
                        height,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage,
                    view_formats: &[],
                })
            };
            let attachment = wgpu::TextureUsages::RENDER_ATTACHMENT;
            self.targets = Some(Targets {
                depth: texture(
                    wgpu::TextureFormat::Depth32Float,
                    "scene3d_depth",
                    self.samples,
                    attachment,
                ),
                hdr: self.display_pipeline.as_ref().map(|_| {
                    texture(
                        wgpu::TextureFormat::Rgba16Float,
                        "scene3d_hdr",
                        self.samples,
                        attachment | wgpu::TextureUsages::TEXTURE_BINDING,
                    )
                }),
            });
        }
    }

    pub(crate) fn reuse_geometry_from(&mut self, other: &Self) {
        self.geometry.clone_from(&other.geometry);
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
        let bounds = layer.composite.bounds;
        let rect = [
            bounds.origin.x.0,
            bounds.origin.y.0,
            bounds.size.width.0,
            bounds.size.height.0,
        ];
        let start = self.offsets[&(layer as *const _ as usize)];
        self.encode_frame(
            device,
            queue,
            atlas,
            frame,
            rect,
            start,
            Some(source),
            destination,
            encoder,
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn encode_frame(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        atlas: &WgpuAtlas,
        frame: &gpui::Scene3dFrame,
        rect: [f32; 4],
        start: usize,
        source: Option<&wgpu::TextureView>,
        destination: &wgpu::TextureView,
        encoder: &mut wgpu::CommandEncoder,
    ) {
        let targets = self.targets.as_ref().unwrap();
        let depth = &targets.depth;
        let width = depth.width() as f32;
        let height = depth.height() as f32;
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
                    (
                        source.expect("UI texture requires a captured subtree"),
                        texture_rect,
                        1. + f32::from(self.format.is_srgb()),
                    )
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
            let rows = object.sampling.transform.rows();
            let maps_enabled = object.pbr.is_some() && !object.unlit;
            let metallic_roughness_map = object.metallic_roughness_texture.filter(|_| maps_enabled);
            let emissive_map = object.emissive_texture.filter(|_| maps_enabled);
            let normal_map = object
                .normal_texture
                .filter(|_| maps_enabled && object.normal_scale > 0.);
            let normal_image = normal_map.map(|map| atlas.get_texture_info(map.tile.texture_id));
            let metallic_roughness_image =
                metallic_roughness_map.map(|map| atlas.get_texture_info(map.tile.texture_id));
            let emissive_image =
                emissive_map.map(|map| atlas.get_texture_info(map.tile.texture_id));
            let pbr = object.pbr.unwrap_or_default();
            let view = frame
                .orthographic_view_direction
                .unwrap_or(frame.camera_position);
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
                    f32::from(matches!(object.texture, MeshTexture3d::Image(_))),
                ],
                ids: [object.output_id, object.alpha_mode as u32, 0, 0],
                uv_u: [rows[0][0], rows[0][1], rows[0][2], 0.],
                uv_v: [rows[1][0], rows[1][1], rows[1][2], 0.],
                sampling: [
                    object.sampling.address_u as u32,
                    object.sampling.address_v as u32,
                    object.sampling.filter as u32,
                    object.image_color_space as u32,
                ],
                view: [
                    view[0],
                    view[1],
                    view[2],
                    f32::from(frame.orthographic_view_direction.is_none()),
                ],
                pbr: [
                    pbr.metallic,
                    pbr.roughness,
                    f32::from(object.pbr.is_some()),
                    0.,
                ],
                emissive: [pbr.emissive[0], pbr.emissive[1], pbr.emissive[2], 0.],
                metallic_roughness_map: ImageParams::new(
                    metallic_roughness_map,
                    gpui::TextureColorSpace3d::Linear,
                ),
                emissive_map: ImageParams::new(emissive_map, gpui::TextureColorSpace3d::Srgb),
                normal_map: ImageParams::new(normal_map, gpui::TextureColorSpace3d::Linear),
                normal_settings: [object.normal_scale, f32::from(normal_map.is_some()), 0., 0.],
                depth_plane: frame.world_to_view.map(|column| -column[2]),
                environment_sh: frame
                    .diffuse_environment
                    .map_or([[0.; 4]; 9], |environment| {
                        environment
                            .coefficients
                            .map(|rgb| [rgb[0], rgb[1], rgb[2], 0.])
                    }),
                environment: frame
                    .diffuse_environment
                    .map_or([1., 0., 0., 0.], |environment| {
                        let (sin, cos) = environment.rotation_y.sin_cos();
                        [cos, sin, environment.intensity, 0.]
                    }),
            };
            let buffer = &self.slots[start + index];
            queue.write_buffer(buffer, 0, bytemuck::bytes_of(&params));
            let mut entries = vec![
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
            ];
            if self.display_pipeline.is_some() {
                entries.extend([
                    wgpu::BindGroupEntry {
                        binding: 5,
                        resource: wgpu::BindingResource::TextureView(
                            normal_image
                                .as_ref()
                                .map_or(&self.white, |image| &image.view),
                        ),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::TextureView(
                            metallic_roughness_image
                                .as_ref()
                                .map_or(&self.white, |image| &image.view),
                        ),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: wgpu::BindingResource::TextureView(
                            emissive_image
                                .as_ref()
                                .map_or(&self.white, |image| &image.view),
                        ),
                    },
                ]);
            }
            groups.push(device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("mesh_material"),
                layout: &layout,
                entries: &entries,
            }));
        }
        let depth_view = depth.create_view(&Default::default());
        let hdr_view = targets
            .hdr
            .as_ref()
            .map(|texture| texture.create_view(&Default::default()));
        let mesh_destination = hdr_view.as_ref().unwrap_or(destination);
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("scene3d"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: mesh_destination,
                resolve_target: None,
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
        if right > x && bottom > y {
            pass.set_scissor_rect(x, y, right - x, bottom - y);
            let order = if self.blend_pipeline.is_some() {
                color_draw_order(&frame.objects)
            } else {
                (0..frame.objects.len()).collect()
            };
            for index in order {
                let object = &frame.objects[index];
                let group = &groups[index];
                let pipeline = if object.alpha_mode == gpui::AlphaMode3d::Blend {
                    self.blend_pipeline.as_ref().unwrap_or(&self.pipeline)
                } else {
                    &self.pipeline
                };
                pass.set_pipeline(pipeline);
                let geometry = &self.geometry[&(Arc::as_ptr(&object.mesh) as usize)];
                pass.set_bind_group(0, group, &[]);
                pass.set_vertex_buffer(0, geometry.vertices.slice(..));
                pass.set_index_buffer(geometry.indices.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..geometry.count, 0, 0..1);
            }
        }
        drop(pass);
        if let (Some(pipeline), Some(hdr)) = (&self.display_pipeline, &hdr_view) {
            let settings = [
                2.0_f32.powf(frame.color_output.exposure),
                frame.color_output.tone_mapping as u32 as f32,
                f32::from(self.format.is_srgb()),
                0.,
            ];
            let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("scene3d_display_params"),
                contents: bytemuck::cast_slice(&settings),
                usage: wgpu::BufferUsages::UNIFORM,
            });
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("scene3d_display"),
                layout: &pipeline.get_bind_group_layout(0),
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: if self.samples > 1 { 2 } else { 0 },
                        resource: wgpu::BindingResource::TextureView(hdr),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: buffer.as_entire_binding(),
                    },
                ],
            });
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("scene3d_display"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: destination,
                    resolve_target: None,
                    depth_slice: None,
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
    }
}

fn color_draw_order(objects: &[gpui::MeshDraw3d]) -> Vec<usize> {
    let mut order: Vec<_> = (0..objects.len()).collect();
    order.sort_by(|&a, &b| {
        let a_blend = objects[a].alpha_mode == gpui::AlphaMode3d::Blend;
        let b_blend = objects[b].alpha_mode == gpui::AlphaMode3d::Blend;
        a_blend.cmp(&b_blend).then_with(|| {
            if a_blend && b_blend {
                objects[b].sort_depth.total_cmp(&objects[a].sort_depth)
            } else {
                std::cmp::Ordering::Equal
            }
        })
    });
    order
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scene3d_color_order_keeps_depth_writers_first_and_blend_ties_stable() {
        use gpui::AlphaMode3d::{Blend, Mask, Opaque};
        let objects: Vec<_> = [
            (Blend, 2.),
            (Mask, 9.),
            (Blend, 5.),
            (Opaque, 1.),
            (Blend, 5.),
        ]
        .into_iter()
        .map(|(alpha_mode, sort_depth)| gpui::MeshDraw3d {
            output_id: 1,
            mesh: gpui::Mesh3d::new(
                [[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]]
                    .map(|position| gpui::MeshVertex3d {
                        position,
                        normal: [0., 0., 1.],
                        uv: [0.; 2],
                    })
                    .to_vec(),
                vec![0, 1, 2],
            ),
            model: [[0.; 4]; 4],
            normal: [[0.; 4]; 4],
            color: gpui::rgb(0xffffff),
            texture: gpui::MeshTexture3d::None,
            sampling: Default::default(),
            image_color_space: Default::default(),
            pbr: None,
            metallic_roughness_texture: None,
            emissive_texture: None,
            normal_texture: None,
            normal_scale: 1.,
            unlit: true,
            alpha_cutoff: 0.5,
            alpha_mode,
            sort_depth,
        })
        .collect();
        assert_eq!(color_draw_order(&objects), vec![1, 3, 2, 4, 0]);
    }

    #[test]
    fn scene3d_display_shader_validates() {
        let module = naga::front::wgsl::parse_str(include_str!("../scene3d_display.wgsl")).unwrap();
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap();
    }

    #[test]
    fn scene3d_shader_validates_and_matches_uniform_layout() {
        let module = naga::front::wgsl::parse_str(include_str!("../scene3d.wgsl")).unwrap();
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap();
        let (members, span) = module
            .types
            .iter()
            .find_map(|(_, ty)| {
                if ty.name.as_deref() != Some("Params") {
                    return None;
                }
                if let naga::TypeInner::Struct { members, span } = &ty.inner {
                    Some((members, *span))
                } else {
                    None
                }
            })
            .unwrap();
        assert_eq!(span as usize, std::mem::size_of::<Params>());
        for (name, offset) in [
            ("ids", std::mem::offset_of!(Params, ids)),
            ("uv_u", std::mem::offset_of!(Params, uv_u)),
            ("uv_v", std::mem::offset_of!(Params, uv_v)),
            ("sampling", std::mem::offset_of!(Params, sampling)),
            ("view", std::mem::offset_of!(Params, view)),
            ("depth_plane", std::mem::offset_of!(Params, depth_plane)),
            (
                "environment_sh",
                std::mem::offset_of!(Params, environment_sh),
            ),
            ("environment", std::mem::offset_of!(Params, environment)),
            ("pbr", std::mem::offset_of!(Params, pbr)),
            ("emissive", std::mem::offset_of!(Params, emissive)),
            (
                "metallic_roughness_map",
                std::mem::offset_of!(Params, metallic_roughness_map),
            ),
            ("emissive_map", std::mem::offset_of!(Params, emissive_map)),
            ("normal_map", std::mem::offset_of!(Params, normal_map)),
            (
                "normal_settings",
                std::mem::offset_of!(Params, normal_settings),
            ),
        ] {
            let member = members
                .iter()
                .find(|member| member.name.as_deref() == Some(name))
                .unwrap();
            assert_eq!(member.offset as usize, offset, "{name}");
        }
    }
}
