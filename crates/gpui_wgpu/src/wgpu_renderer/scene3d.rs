use super::*;
use gpui::{Mesh3d, MeshTexture3d, SubtreeLayer};
use wgpu::util::DeviceExt;

mod background;
mod geometry;
mod instances;
mod specular;

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
struct Instance {
    model: [[f32; 4]; 4],
    normal: [[f32; 4]; 4],
    color: [f32; 4],
    ids: [u32; 4],
}

impl Instance {
    fn new(object: &gpui::MeshDraw3d) -> Self {
        Self {
            model: object.model,
            normal: object.normal,
            color: [
                object.color.r,
                object.color.g,
                object.color.b,
                object.color.a,
            ],
            ids: [object.output_id, 0, 0, 0],
        }
    }
    fn layout() -> wgpu::VertexBufferLayout<'static> {
        const ATTRIBUTES: [wgpu::VertexAttribute; 10] = wgpu::vertex_attr_array![
            4 => Float32x4, 5 => Float32x4, 6 => Float32x4, 7 => Float32x4,
            8 => Float32x4, 9 => Float32x4, 10 => Float32x4, 11 => Float32x4,
            12 => Float32x4, 13 => Uint32x4
        ];
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Self>() as u64,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &ATTRIBUTES,
        }
    }
}

struct BatchSlot {
    params: wgpu::Buffer,
    instances: wgpu::Buffer,
}

fn instance_limit(device: &wgpu::Device) -> usize {
    ((device.limits().max_buffer_size - std::mem::size_of::<Params>() as u64)
        / std::mem::size_of::<Instance>() as u64)
        .min(u64::from(u32::MAX)) as usize
}

fn instance_buffer(device: &wgpu::Device, count: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("mesh_instances"),
        size: (instances::capacity(count, instance_limit(device)) * std::mem::size_of::<Instance>())
            as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
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
struct DirectLight {
    position_kind: [f32; 4],
    direction_range: [f32; 4],
    color_intensity: [f32; 4],
    cone: [f32; 4],
}

impl From<gpui::PunctualLight3d> for DirectLight {
    fn from(light: gpui::PunctualLight3d) -> Self {
        let length = light
            .direction
            .iter()
            .map(|v| f64::from(*v).powi(2))
            .sum::<f64>()
            .sqrt();
        let direction = light
            .direction
            .map(|v| (f64::from(v) / length.max(f64::MIN_POSITIVE)) as f32);
        Self {
            position_kind: [
                light.position[0],
                light.position[1],
                light.position[2],
                match light.kind {
                    gpui::LightKind3d::Directional => 0.,
                    gpui::LightKind3d::Point => 1.,
                    gpui::LightKind3d::Spot => 2.,
                },
            ],
            direction_range: [
                direction[0],
                direction[1],
                direction[2],
                light.range.unwrap_or(0.),
            ],
            color_intensity: [light.color.r, light.color.g, light.color.b, light.intensity],
            cone: [
                light.inner_angle.cos(),
                light.outer_angle.cos(),
                light.minimum_distance,
                0.,
            ],
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Params {
    specular_environment: [f32; 4],
    camera: [[f32; 4]; 4],
    bounds: [f32; 4],
    viewport: [f32; 4],
    ambient: [f32; 4],
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
    occlusion_map: ImageParams,
    occlusion_settings: [f32; 4],
    lights: [DirectLight; gpui::MAX_PUNCTUAL_LIGHTS_3D],
    light_count: [u32; 4],
    shadow_camera: [[f32; 4]; 4],
    shadow_settings: [f32; 4],
    shadow_flags: [u32; 4],
}

struct Geometry {
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    count: u32,
    upload: Option<wgpu::Buffer>,
}

impl Geometry {
    fn prepare(device: &wgpu::Device, previous: Option<Self>, mesh: &Mesh3d) -> Self {
        let vertices = mesh
            .vertices()
            .iter()
            .enumerate()
            .map(|(index, v)| Vertex {
                position: v.position,
                normal: v.normal,
                uv: v.uv,
                tangent: mesh.tangents().map_or([0.; 4], |t| t[index]),
            })
            .collect::<Vec<_>>();
        let contents = bytemuck::cast_slice(&vertices);
        if let Some(mut previous) = previous {
            previous.upload = Some(
                device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("mesh_vertex_upload"),
                    contents,
                    usage: wgpu::BufferUsages::COPY_SRC,
                }),
            );
            previous
        } else {
            Self {
                vertices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("mesh_vertices"),
                    contents,
                    usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                }),
                indices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("mesh_indices"),
                    contents: bytemuck::cast_slice(mesh.indices()),
                    usage: wgpu::BufferUsages::INDEX,
                }),
                count: mesh.indices().len() as u32,
                upload: None,
            }
        }
    }

    fn encode_upload(&self, encoder: &mut wgpu::CommandEncoder) {
        if let Some(upload) = &self.upload {
            // Keep the copy in command order and replay it after abandoned encoders.
            encoder.copy_buffer_to_buffer(upload, 0, &self.vertices, 0, upload.size());
        }
    }
}

struct Targets {
    depth: wgpu::Texture,
    hdr: Option<wgpu::Texture>,
}

pub(crate) struct Scene3dRenderer {
    specular: Option<specular::SpecularRenderer>,
    background: Option<background::BackgroundRenderer>,
    pipeline: wgpu::RenderPipeline,
    blend_pipeline: Option<wgpu::RenderPipeline>,
    display_pipeline: Option<wgpu::RenderPipeline>,
    shadow_pipeline: Option<wgpu::RenderPipeline>,
    shadow_maps: HashMap<u32, wgpu::Texture>,
    shadow_fallback: wgpu::TextureView,
    shadow_sampler: wgpu::Sampler,
    sampler: wgpu::Sampler,
    white: wgpu::TextureView,
    geometry: geometry::GeometryCache<Geometry>,
    slots: Vec<BatchSlot>,
    offsets: HashMap<usize, usize>,
    plans: HashMap<usize, instances::BatchPlan>,
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
            &[1, 3, 4, 5, 6][..]
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
        if !data_output {
            for (binding, dimension) in [
                (9, wgpu::TextureViewDimension::Cube),
                (10, wgpu::TextureViewDimension::D2),
            ] {
                bindings.push(wgpu::BindGroupLayoutEntry {
                    binding,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: dimension,
                        multisampled: false,
                    },
                    count: None,
                });
            }
            bindings.push(wgpu::BindGroupLayoutEntry {
                binding: 11,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            });
            bindings.extend([
                wgpu::BindGroupLayoutEntry {
                    binding: 7,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 8,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
                    count: None,
                },
            ]);
        }
        let shadow_pipeline = (!data_output).then(|| {
            let material = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("scene3d_shadow_material"),
                entries: &bindings.iter().filter(|binding| binding.binding <= 2).cloned().collect::<Vec<_>>(),
            });
            let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("scene3d_shadow"), bind_group_layouts: &[Some(&material)], immediate_size: 0,
            });
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("scene3d_shadow"), layout: Some(&layout),
                vertex: wgpu::VertexState { module: &shader, entry_point: Some("shadow_vertex"), compilation_options: Default::default(),
                    buffers: &[Some(wgpu::VertexBufferLayout { array_stride: 48, step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x2, 3 => Float32x4] }), Some(Instance::layout())] },
                fragment: Some(wgpu::FragmentState { module: &shader, entry_point: Some("shadow_fragment"), compilation_options: Default::default(), targets: &[] }),
                primitive: wgpu::PrimitiveState { cull_mode: None, ..Default::default() },
                depth_stencil: Some(wgpu::DepthStencilState { format: wgpu::TextureFormat::Depth32Float,
                    depth_write_enabled: Some(true), depth_compare: Some(wgpu::CompareFunction::Less), stencil: Default::default(), bias: Default::default() }),
                multisample: Default::default(), multiview_mask: None, cache: None,
            })
        });
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
                    attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x2, 3 => Float32x4] }), Some(Instance::layout())],
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
            specular: (!data_output).then(|| specular::SpecularRenderer::new(device)),
            background: (!data_output)
                .then(|| background::BackgroundRenderer::new(device, samples)),
            shadow_pipeline,
            shadow_maps: HashMap::new(),
            shadow_fallback: shadow_texture(device, 1).create_view(&Default::default()),
            shadow_sampler: device.create_sampler(&wgpu::SamplerDescriptor {
                compare: Some(wgpu::CompareFunction::LessEqual),
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            }),
            pipeline,
            blend_pipeline,
            display_pipeline,
            sampler: device.create_sampler(&wgpu::SamplerDescriptor {
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            }),
            white: white.create_view(&Default::default()),
            geometry: geometry::GeometryCache::default(),
            slots: Vec::new(),
            offsets: HashMap::new(),
            plans: HashMap::new(),
            targets: None,
            format,
            samples,
        }
    }

    pub(super) fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        scene: &Scene,
        width: u32,
        height: u32,
    ) {
        self.offsets.clear();
        let mut frames = Vec::new();
        let mut layers = Vec::new();
        scene.visit(&mut |scene| {
            for layer in &scene.subtree_layers {
                let Some(frame) = &layer.scene3d else {
                    continue;
                };
                layers.push(layer as *const _ as usize);
                frames.push(frame.clone());
            }
        });
        self.prepare_frames(
            device,
            queue,
            frames.iter().map(AsRef::as_ref),
            width,
            height,
        );
        let mut slot_count = 0;
        for (layer, frame) in layers.into_iter().zip(&frames) {
            self.offsets.insert(layer, slot_count);
            slot_count += self.plan(frame).batches.len();
        }
    }

    fn plan(&self, frame: &gpui::Scene3dFrame) -> &instances::BatchPlan {
        &self.plans[&(frame as *const _ as usize)]
    }

    pub(crate) fn prepare_frames<'a>(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        frames: impl IntoIterator<Item = &'a gpui::Scene3dFrame>,
        width: u32,
        height: u32,
    ) {
        let frames: Vec<_> = frames.into_iter().collect();
        self.plans.clear();
        for frame in &frames {
            self.plans
                .entry(*frame as *const _ as usize)
                .or_insert_with(|| {
                    instances::BatchPlan::new(
                        frame,
                        self.blend_pipeline.is_some(),
                        instance_limit(device),
                    )
                });
        }
        if let Some(specular) = &mut self.specular {
            specular.prepare(
                device,
                queue,
                frames
                    .iter()
                    .filter_map(|f| f.specular_environment.as_ref()),
            );
        }
        if let Some(background) = &mut self.background {
            background.prepare(
                device,
                queue,
                frames.iter().filter_map(|frame| frame.background.as_ref()),
            );
        }
        let meshes: Vec<_> = frames
            .iter()
            .flat_map(|frame| {
                self.plan(frame)
                    .order
                    .iter()
                    .map(|&index| frame.objects[index].mesh.clone())
            })
            .collect();
        self.geometry.prepare(meshes, |previous, mesh| {
            Geometry::prepare(device, previous, mesh)
        });
        let mut shadow_sizes = HashSet::new();
        let mut batch_sizes = Vec::new();
        let mut has_frame = false;
        for frame in frames {
            assert!(
                frame.shadow_is_valid(),
                "invalid directional shadow parameters or source"
            );
            if self.shadow_pipeline.is_some()
                && let Some(shadow) = frame.directional_shadow
            {
                assert!(
                    shadow.resolution <= device.limits().max_texture_dimension_2d,
                    "shadow resolution exceeds device limits"
                );
                shadow_sizes.insert(shadow.resolution);
            }
            has_frame = true;
            batch_sizes.extend(self.plan(frame).batches.iter().map(|batch| batch.len()));
        }
        self.shadow_maps
            .retain(|size, _| shadow_sizes.contains(size));
        for resolution in shadow_sizes {
            self.shadow_maps
                .entry(resolution)
                .or_insert_with(|| shadow_texture(device, resolution));
        }
        self.slots.truncate(batch_sizes.len());
        for (index, count) in batch_sizes.into_iter().enumerate() {
            if index == self.slots.len() {
                self.slots.push(BatchSlot {
                    params: device.create_buffer(&wgpu::BufferDescriptor {
                        label: Some("mesh_params"),
                        size: std::mem::size_of::<Params>() as u64,
                        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                        mapped_at_creation: false,
                    }),
                    instances: instance_buffer(device, count),
                });
            } else if self.slots[index].instances.size()
                < (count * std::mem::size_of::<Instance>()) as u64
            {
                self.slots[index].instances = instance_buffer(device, count);
            }
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
                        attachment
                            | wgpu::TextureUsages::TEXTURE_BINDING
                            | if self.samples == 1 {
                                wgpu::TextureUsages::COPY_SRC
                            } else {
                                wgpu::TextureUsages::empty()
                            },
                    )
                }),
            });
        }
    }

    pub(crate) fn reuse_geometry_from(&mut self, other: &Self) {
        self.geometry.reuse_from(&other.geometry);
    }

    pub(crate) fn retain_geometry_for(&mut self, frame: Option<&gpui::Scene3dFrame>) {
        let shadows = self.shadow_pipeline.is_some();
        self.geometry.retain(frame.into_iter().flat_map(|frame| {
            frame
                .objects
                .iter()
                .filter(move |object| instances::Visibility::new(frame, object, shadows).any())
                .map(|object| object.mesh.clone())
        }));
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
            Some(destination),
            encoder,
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn encode_frame(
        &self,
        device: &wgpu::Device,
        _queue: &wgpu::Queue,
        atlas: &WgpuAtlas,
        frame: &gpui::Scene3dFrame,
        rect: [f32; 4],
        start: usize,
        source: Option<&wgpu::TextureView>,
        destination: Option<&wgpu::TextureView>,
        encoder: &mut wgpu::CommandEncoder,
    ) {
        let targets = self.targets.as_ref().unwrap();
        let plan = self.plan(frame);
        let mut uploaded = HashSet::new();
        for &index in &plan.order {
            let object = &frame.objects[index];
            if uploaded.insert(Arc::as_ptr(&object.mesh)) {
                self.geometry.get(&object.mesh).encode_upload(encoder);
            }
        }
        let specular_environment = frame
            .specular_environment
            .as_ref()
            .filter(|e| e.intensity > 0.);
        let depth = &targets.depth;
        let width = depth.width() as f32;
        let height = depth.height() as f32;
        let layout = self.pipeline.get_bind_group_layout(0);
        let mut groups = Vec::with_capacity(plan.batches.len());
        let shadow = frame
            .directional_shadow
            .filter(|_| self.shadow_pipeline.is_some());
        let shadow_view = shadow
            .map(|shadow| self.shadow_maps[&shadow.resolution].create_view(&Default::default()));
        let shadow_layout = self
            .shadow_pipeline
            .as_ref()
            .map(|pipeline| pipeline.get_bind_group_layout(0));
        let mut shadow_groups = Vec::with_capacity(plan.batches.len());
        let mut lights = [DirectLight::zeroed(); gpui::MAX_PUNCTUAL_LIGHTS_3D];
        let light_count = if let Some(sources) = &frame.lights {
            assert!(sources.len() <= lights.len(), "too many direct lights");
            for (target, source) in lights.iter_mut().zip(sources.iter()) {
                assert!(source.is_valid(), "invalid direct light");
                *target = (*source).into();
            }
            sources.len() as u32
        } else {
            lights[0] = DirectLight {
                position_kind: [0.; 4],
                direction_range: [
                    frame.light_direction[0],
                    frame.light_direction[1],
                    frame.light_direction[2],
                    0.,
                ],
                color_intensity: frame.light,
                cone: [0.; 4],
            };
            1
        };
        for (index, batch) in plan.batches.iter().enumerate() {
            let object = &frame.objects[plan.order[batch.start]];
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
            let occlusion_map = object
                .occlusion_texture
                .filter(|_| !object.unlit && object.occlusion_strength > 0.);
            let occlusion_image =
                occlusion_map.map(|map| atlas.get_texture_info(map.tile.texture_id));
            let metallic_roughness_image =
                metallic_roughness_map.map(|map| atlas.get_texture_info(map.tile.texture_id));
            let emissive_image =
                emissive_map.map(|map| atlas.get_texture_info(map.tile.texture_id));
            let pbr = object.pbr.unwrap_or_default();
            let view = frame
                .orthographic_view_direction
                .unwrap_or(frame.camera_position);
            let params = Params {
                specular_environment: specular_environment.map_or([0.; 4], |e| {
                    [
                        e.rotation_y.cos(),
                        e.rotation_y.sin(),
                        e.intensity,
                        (e.map.levels().len() - 1) as f32,
                    ]
                }),
                shadow_camera: shadow.map_or([[0.; 4]; 4], |s| s.view_projection),
                shadow_settings: shadow
                    .map_or([0.; 4], |s| [s.depth_bias, s.normal_bias, s.softness, 0.]),
                shadow_flags: [
                    u32::from(shadow.is_some() && object.receive_shadows),
                    shadow.map_or(0, |s| s.light_index),
                    0,
                    0,
                ],
                camera: frame.view_projection,
                bounds: rect,
                viewport: [width, height, 0., 0.],
                ambient: [frame.ambient, 0., 0., 0.],
                lights,
                light_count: [light_count, 0, 0, 0],
                texture_rect,
                flags: [
                    object.alpha_cutoff.clamp(0.001, 1.),
                    f32::from(object.unlit),
                    premultiplied,
                    f32::from(matches!(object.texture, MeshTexture3d::Image(_))),
                ],
                ids: [0, object.alpha_mode as u32, 0, 0],
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
                occlusion_map: ImageParams::new(occlusion_map, gpui::TextureColorSpace3d::Linear),
                occlusion_settings: [
                    if occlusion_map.is_some() {
                        object.occlusion_strength
                    } else {
                        0.
                    },
                    0.,
                    0.,
                    0.,
                ],
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
            let slot = &self.slots[start + index];
            let instance_data: Vec<_> = plan.order[batch.clone()]
                .iter()
                .map(|&i| Instance::new(&frame.objects[i]))
                .collect();
            let mut upload = bytemuck::bytes_of(&params).to_vec();
            upload.extend_from_slice(bytemuck::cast_slice(&instance_data));
            let upload = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("mesh_batch_upload"),
                contents: &upload,
                usage: wgpu::BufferUsages::COPY_SRC,
            });
            let params_size = std::mem::size_of::<Params>() as u64;
            encoder.copy_buffer_to_buffer(&upload, 0, &slot.params, 0, params_size);
            encoder.copy_buffer_to_buffer(
                &upload,
                params_size,
                &slot.instances,
                0,
                upload.size() - params_size,
            );
            let mut entries = vec![
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: slot.params.as_entire_binding(),
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
            if let Some(layout) = &shadow_layout
                && shadow.is_some()
            {
                shadow_groups.push(device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("mesh_shadow_material"),
                    layout,
                    entries: &entries,
                }));
            }
            if self.display_pipeline.is_some() {
                let specular = self.specular.as_ref().unwrap();
                entries.extend([
                    wgpu::BindGroupEntry {
                        binding: 9,
                        resource: wgpu::BindingResource::TextureView(
                            specular.map(specular_environment),
                        ),
                    },
                    wgpu::BindGroupEntry {
                        binding: 10,
                        resource: wgpu::BindingResource::TextureView(specular.brdf(&self.white)),
                    },
                    wgpu::BindGroupEntry {
                        binding: 11,
                        resource: wgpu::BindingResource::Sampler(&specular.sampler),
                    },
                ]);
                entries.extend([
                    wgpu::BindGroupEntry {
                        binding: 7,
                        resource: wgpu::BindingResource::TextureView(
                            shadow_view.as_ref().unwrap_or(&self.shadow_fallback),
                        ),
                    },
                    wgpu::BindGroupEntry {
                        binding: 8,
                        resource: wgpu::BindingResource::Sampler(&self.shadow_sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 6,
                        resource: wgpu::BindingResource::TextureView(
                            occlusion_image
                                .as_ref()
                                .map_or(&self.white, |image| &image.view),
                        ),
                    },
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
        if let (Some(pipeline), Some(view)) = (&self.shadow_pipeline, &shadow_view) {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("scene3d_shadow"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            pass.set_pipeline(pipeline);
            for (index, batch) in plan.batches.iter().enumerate() {
                let object = &frame.objects[plan.order[batch.start]];
                if !plan.passes[batch.start].shadow {
                    continue;
                }
                let geometry = self.geometry.get(&object.mesh);
                pass.set_bind_group(0, &shadow_groups[index], &[]);
                pass.set_vertex_buffer(0, geometry.vertices.slice(..));
                pass.set_vertex_buffer(1, self.slots[start + index].instances.slice(..));
                pass.set_index_buffer(geometry.indices.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..geometry.count, 0, 0..batch.len() as u32);
            }
        }
        let depth_view = depth.create_view(&Default::default());
        let hdr_view = targets
            .hdr
            .as_ref()
            .map(|texture| texture.create_view(&Default::default()));
        let mesh_destination = hdr_view
            .as_ref()
            .or(destination)
            .expect("missing mesh output");
        let background_group = self
            .background
            .as_ref()
            .zip(frame.background.as_ref())
            .map(|(renderer, background)| renderer.bind(device, background, rect));
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
            if let (Some(background), Some(group)) = (&self.background, &background_group) {
                pass.set_pipeline(&background.pipeline);
                pass.set_bind_group(0, group, &[]);
                pass.draw(0..3, 0..1);
            }
            for (index, batch) in plan.batches.iter().enumerate() {
                if !plan.passes[batch.start].camera {
                    continue;
                }
                let object = &frame.objects[plan.order[batch.start]];
                let group = &groups[index];
                let pipeline = if object.alpha_mode == gpui::AlphaMode3d::Blend {
                    self.blend_pipeline.as_ref().unwrap_or(&self.pipeline)
                } else {
                    &self.pipeline
                };
                pass.set_pipeline(pipeline);
                let geometry = self.geometry.get(&object.mesh);
                pass.set_bind_group(0, group, &[]);
                pass.set_vertex_buffer(0, geometry.vertices.slice(..));
                pass.set_vertex_buffer(1, self.slots[start + index].instances.slice(..));
                pass.set_index_buffer(geometry.indices.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..geometry.count, 0, 0..batch.len() as u32);
            }
        }
        drop(pass);
        if let (Some(pipeline), Some(hdr), Some(destination)) =
            (&self.display_pipeline, &hdr_view, destination)
        {
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

    pub(crate) fn copy_linear_color(
        &self,
        destination: &wgpu::Texture,
        encoder: &mut wgpu::CommandEncoder,
    ) {
        let source = self.targets.as_ref().unwrap().hdr.as_ref().unwrap();
        if self.samples == 1 {
            encoder.copy_texture_to_texture(
                source.as_image_copy(),
                destination.as_image_copy(),
                source.size(),
            );
        } else {
            let source = source.create_view(&Default::default());
            let destination = destination.create_view(&Default::default());
            let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("scene3d_linear_resolve"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &source,
                    resolve_target: Some(&destination),
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
        }
    }
}

fn shadow_texture(device: &wgpu::Device, resolution: u32) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("scene3d_shadow_depth"),
        size: wgpu::Extent3d {
            width: resolution,
            height: resolution,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Depth32Float,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    })
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

    const IDENTITY: [[f32; 4]; 4] = [
        [1., 0., 0., 0.],
        [0., 1., 0., 0.],
        [0., 0., 1., 0.],
        [0., 0., 0., 1.],
    ];

    fn frame(objects: &[gpui::MeshDraw3d]) -> gpui::Scene3dFrame {
        gpui::Scene3dFrame {
            ui_texture: None,
            view_projection: IDENTITY,
            world_to_view: IDENTITY,
            camera_position: [0., 0., 3.],
            orthographic_view_direction: None,
            light_direction: [0., 0., 1.],
            light: [1.; 4],
            lights: None,
            directional_shadow: None,
            ambient: 0.3,
            diffuse_environment: None,
            background: None,
            specular_environment: None,
            color_output: Default::default(),
            objects: objects.into(),
        }
    }

    fn object() -> gpui::MeshDraw3d {
        gpui::MeshDraw3d {
            cast_shadows: true,
            receive_shadows: true,
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
            model: IDENTITY,
            normal: IDENTITY,
            color: gpui::rgb(0xffffff),
            texture: gpui::MeshTexture3d::None,
            sampling: Default::default(),
            image_color_space: Default::default(),
            pbr: None,
            metallic_roughness_texture: None,
            emissive_texture: None,
            normal_texture: None,
            normal_scale: 1.,
            occlusion_texture: None,
            occlusion_strength: 1.,
            unlit: true,
            alpha_cutoff: 0.5,
            alpha_mode: gpui::AlphaMode3d::Opaque,
            sort_depth: 0.,
        }
    }

    #[test]
    fn scene3d_batch_plans_separate_camera_and_shadow_visibility_without_renumbering() {
        let visible = object();
        let mut caster = visible.clone();
        caster.model[3][0] = 3.;
        caster.output_id = 7;
        let mut outside = visible.clone();
        outside.model[3][0] = 10.;
        let mut blend = caster.clone();
        blend.alpha_mode = gpui::AlphaMode3d::Blend;
        let mut noncaster = caster.clone();
        noncaster.cast_shadows = false;
        let mut input = frame(&[outside, visible.clone(), caster, blend, noncaster, visible]);
        let mut shadow_matrix = IDENTITY;
        shadow_matrix[3][0] = -3.;
        input.directional_shadow = Some(gpui::DirectionalShadow3d {
            light_index: 0,
            view_projection: shadow_matrix,
            resolution: 256,
            depth_bias: 0.,
            normal_bias: 0.,
            softness: 0.,
        });
        let plan = instances::BatchPlan::new(&input, true, 100);
        assert_eq!(plan.order, vec![1, 2, 5]);
        assert_eq!(plan.batches, vec![0..1, 1..2, 2..3]);
        assert!(plan.passes[0].camera && !plan.passes[0].shadow);
        assert!(!plan.passes[1].camera && plan.passes[1].shadow);
        assert!(plan.passes[2].camera && !plan.passes[2].shadow);
        assert_eq!(input.objects[plan.order[1]].output_id, 7);
        let data = instances::BatchPlan::new(&input, false, 100);
        assert_eq!(data.order, vec![1, 5]);
        assert_eq!(data.batches, vec![0..2]);
        input.view_projection = shadow_matrix;
        let moved = instances::BatchPlan::new(&input, false, 100);
        assert_eq!(moved.order, vec![2, 3, 4]);
        input.directional_shadow = None;
        let no_shadow = instances::BatchPlan::new(&input, true, 100);
        assert!(no_shadow.passes.iter().all(|v| !v.shadow));
    }

    #[test]
    fn scene3d_batches_preserve_instance_values_order_and_capacity_boundaries() {
        let source = object();
        let objects: Vec<_> = (0..7)
            .map(|index| {
                let mut object = source.clone();
                object.model[3][0] = index as f32 * 0.05;
                object.normal[2][1] = index as f32 * 0.25;
                object.color = gpui::rgba(0x224488ff + index * 0x10000);
                object.output_id = index + 7;
                object
            })
            .collect();
        let plan = instances::BatchPlan::new(&frame(&objects), true, 3);
        assert_eq!(plan.order, (0..7).collect::<Vec<_>>());
        assert_eq!(plan.batches, vec![0..3, 3..6, 6..7]);
        let data: Vec<_> = plan
            .order
            .iter()
            .map(|&i| Instance::new(&objects[i]))
            .collect();
        for (index, instance) in data.iter().enumerate() {
            let object = &objects[index];
            assert_eq!(instance.model, object.model);
            assert_eq!(instance.normal, object.normal);
            assert_eq!(
                instance.color,
                [
                    object.color.r,
                    object.color.g,
                    object.color.b,
                    object.color.a
                ]
            );
            assert_eq!(instance.ids[0], object.output_id);
        }
        for required in 1..=37 {
            let capacity = instances::capacity(required, 37);
            assert!((required..=37).contains(&capacity));
            assert!(capacity.is_power_of_two() || capacity == 37);
        }
        assert!(
            instances::BatchPlan::new(&frame(&[]), true, 3)
                .batches
                .is_empty()
        );
    }

    #[test]
    fn scene3d_batches_split_materials_geometry_and_blended_objects() {
        use gpui::AlphaMode3d::{Blend, Mask};
        let source = object();
        let mut variants = Vec::new();
        let mut push = |edit: fn(&mut gpui::MeshDraw3d)| {
            let mut changed = source.clone();
            edit(&mut changed);
            variants.push(changed);
        };
        push(|v| v.mesh = object().mesh);
        push(|v| v.alpha_mode = Mask);
        push(|v| v.alpha_cutoff = 0.9);
        push(|v| v.cast_shadows = false);
        push(|v| v.receive_shadows = false);
        push(|v| v.unlit = false);
        push(|v| v.texture = MeshTexture3d::Subtree);
        push(|v| v.pbr = Some(Default::default()));
        push(|v| v.normal_scale = 0.4);
        push(|v| v.occlusion_strength = 0.4);
        for changed in variants {
            let objects = [source.clone(), changed.clone(), changed, source.clone()];
            let plan = instances::BatchPlan::new(&frame(&objects), false, 100);
            assert_eq!(plan.order, vec![0, 1, 2, 3]);
            assert_eq!(plan.batches, vec![0..1, 1..3, 3..4]);
        }
        let mut near = source.clone();
        near.alpha_mode = Blend;
        near.sort_depth = 2.;
        let mut far = near.clone();
        far.sort_depth = 7.;
        let objects = [near, source.clone(), far.clone(), source, far];
        let plan = instances::BatchPlan::new(&frame(&objects), true, 100);
        assert_eq!(plan.order, vec![1, 3, 2, 4, 0]);
        assert_eq!(plan.batches, vec![0..2, 2..3, 3..4, 4..5]);
        let data_plan = instances::BatchPlan::new(&frame(&objects), false, 100);
        assert_eq!(data_plan.order, vec![0, 1, 2, 3, 4]);
        assert_eq!(data_plan.batches, vec![0..1, 1..2, 2..3, 3..4, 4..5]);
    }

    #[test]
    fn scene3d_batches_keep_image_tiles_and_sampling_independent() {
        let tile = gpui::AtlasTile {
            texture_id: gpui::AtlasTextureId {
                index: 0,
                kind: gpui::AtlasTextureKind::Polychrome,
            },
            tile_id: gpui::TileId(1),
            padding: 0,
            bounds: gpui::Bounds::new(
                gpui::point(gpui::DevicePixels(0), gpui::DevicePixels(0)),
                gpui::size(gpui::DevicePixels(16), gpui::DevicePixels(16)),
            ),
        };
        let map = gpui::MaterialTexture3d {
            tile,
            sampling: Default::default(),
        };
        let mut source = object();
        source.unlit = false;
        source.pbr = Some(Default::default());
        source.texture = MeshTexture3d::Image(tile);
        source.metallic_roughness_texture = Some(map);
        source.emissive_texture = Some(map);
        source.normal_texture = Some(map);
        source.occlusion_texture = Some(map);
        let mut variants = Vec::new();
        for slot in 0..4 {
            for change_sampling in [false, true] {
                let mut changed = source.clone();
                let target = match slot {
                    0 => &mut changed.metallic_roughness_texture,
                    1 => &mut changed.emissive_texture,
                    2 => &mut changed.normal_texture,
                    _ => &mut changed.occlusion_texture,
                }
                .as_mut()
                .unwrap();
                if change_sampling {
                    target.sampling.address_u = gpui::TextureAddressMode3d::Repeat;
                } else {
                    target.tile.bounds.origin.x = gpui::DevicePixels(16);
                    target.tile.tile_id = gpui::TileId(2);
                }
                variants.push(changed);
            }
        }
        let mut changed = source.clone();
        changed.texture = MeshTexture3d::Image(gpui::AtlasTile {
            tile_id: gpui::TileId(3),
            ..tile
        });
        variants.push(changed);
        let mut changed = source.clone();
        changed.sampling.filter = gpui::TextureFilter3d::Nearest;
        variants.push(changed);
        let mut changed = source.clone();
        changed.image_color_space = gpui::TextureColorSpace3d::Linear;
        variants.push(changed);
        for changed in variants {
            let plan = instances::BatchPlan::new(
                &frame(&[source.clone(), changed.clone(), changed]),
                true,
                100,
            );
            assert_eq!(plan.batches, vec![0..1, 1..3]);
        }
    }

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
            alpha_mode,
            sort_depth,
            ..object()
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
        let instance_layout = Instance::layout();
        let (instance_type, members) = module
            .types
            .iter()
            .find_map(|(handle, ty)| {
                if ty.name.as_deref() == Some("InstanceInput")
                    && let naga::TypeInner::Struct { members, .. } = &ty.inner
                {
                    Some((handle, members))
                } else {
                    None
                }
            })
            .unwrap();
        assert_eq!(instance_layout.step_mode, wgpu::VertexStepMode::Instance);
        assert_eq!(
            instance_layout.array_stride as usize,
            std::mem::size_of::<Instance>()
        );
        assert_eq!(members.len(), instance_layout.attributes.len());
        for (member, attribute) in members.iter().zip(instance_layout.attributes) {
            let Some(naga::Binding::Location { location, .. }) = member.binding else {
                panic!("instance input must have a vertex location");
            };
            assert_eq!(location, attribute.shader_location);
            let (offset, kind) = match member.name.as_deref().unwrap() {
                "color" => (
                    std::mem::offset_of!(Instance, color),
                    naga::ScalarKind::Float,
                ),
                "ids" => (std::mem::offset_of!(Instance, ids), naga::ScalarKind::Uint),
                name => {
                    let (field, column) = name.rsplit_once('_').unwrap();
                    let base = match field {
                        "model" => std::mem::offset_of!(Instance, model),
                        "normal" => std::mem::offset_of!(Instance, normal),
                        _ => panic!("unknown instance field"),
                    };
                    (
                        base + column.parse::<usize>().unwrap() * 16,
                        naga::ScalarKind::Float,
                    )
                }
            };
            assert_eq!(attribute.offset as usize, offset);
            assert_eq!(
                attribute.format,
                if kind == naga::ScalarKind::Uint {
                    wgpu::VertexFormat::Uint32x4
                } else {
                    wgpu::VertexFormat::Float32x4
                }
            );
            assert_eq!(
                module.types[member.ty].inner,
                naga::TypeInner::Vector {
                    size: naga::VectorSize::Quad,
                    scalar: naga::Scalar { kind, width: 4 },
                }
            );
        }
        for entry in ["vertex", "shadow_vertex"] {
            let entry = module
                .entry_points
                .iter()
                .find(|v| v.name == entry)
                .unwrap();
            assert!(
                entry
                    .function
                    .arguments
                    .iter()
                    .any(|v| v.ty == instance_type)
            );
        }
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
            (
                "specular_environment",
                std::mem::offset_of!(Params, specular_environment),
            ),
            ("shadow_camera", std::mem::offset_of!(Params, shadow_camera)),
            (
                "shadow_settings",
                std::mem::offset_of!(Params, shadow_settings),
            ),
            ("shadow_flags", std::mem::offset_of!(Params, shadow_flags)),
            ("ambient", std::mem::offset_of!(Params, ambient)),
            ("lights", std::mem::offset_of!(Params, lights)),
            ("light_count", std::mem::offset_of!(Params, light_count)),
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
            ("occlusion_map", std::mem::offset_of!(Params, occlusion_map)),
            (
                "occlusion_settings",
                std::mem::offset_of!(Params, occlusion_settings),
            ),
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
        let (members, span) = module
            .types
            .iter()
            .find_map(|(_, ty)| {
                if ty.name.as_deref() == Some("DirectLight")
                    && let naga::TypeInner::Struct { members, span } = &ty.inner
                {
                    Some((members, *span))
                } else {
                    None
                }
            })
            .unwrap();
        assert_eq!(span as usize, std::mem::size_of::<DirectLight>());
        for (name, offset) in [
            (
                "position_kind",
                std::mem::offset_of!(DirectLight, position_kind),
            ),
            (
                "direction_range",
                std::mem::offset_of!(DirectLight, direction_range),
            ),
            (
                "color_intensity",
                std::mem::offset_of!(DirectLight, color_intensity),
            ),
            ("cone", std::mem::offset_of!(DirectLight, cone)),
        ] {
            let member = members
                .iter()
                .find(|member| member.name.as_deref() == Some(name))
                .unwrap();
            assert_eq!(member.offset as usize, offset, "{name}");
        }
    }
}
