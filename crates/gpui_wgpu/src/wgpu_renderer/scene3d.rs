use super::*;
use gpui::{Mesh3d, MeshTexture3d, SubtreeLayer};
use wgpu::util::DeviceExt;

mod background;
mod geometry;
mod images;
mod instances;
mod output_cache;
mod specular;
mod target;
mod viewport;

pub(super) use output_cache::OutputBudget;
pub(crate) use target::RenderRegion;
pub(super) use viewport::ViewportRenderer;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct DisplayParams {
    settings: [f32; 4],
    output_rect: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Vertex {
    position: [f32; 3],
    normal: [f32; 3],
    uv: [f32; 4],
    tangent: [f32; 4],
    detail_uv: [f32; 4],
    occlusion_uv: [f32; 2],
    vertex_color: [f32; 4],
}

impl Vertex {
    fn new(mesh: &Mesh3d, index: usize, sets: [u32; 5]) -> Self {
        let v = mesh.vertices()[index];
        let [base, surface, emission, normal, occlusion] = sets.map(|set| {
            mesh.uv_at(set, index)
                .expect("missing material coordinate set")
        });
        Self {
            position: v.position,
            normal: v.normal,
            uv: [base[0], base[1], surface[0], surface[1]],
            tangent: mesh.tangents().map_or([0.; 4], |t| t[index]),
            detail_uv: [emission[0], emission[1], normal[0], normal[1]],
            occlusion_uv: occlusion,
            vertex_color: mesh.vertex_colors().map_or([1.; 4], |colors| colors[index]),
        }
    }

    fn layout() -> wgpu::VertexBufferLayout<'static> {
        const ATTRIBUTES: [wgpu::VertexAttribute; 7] = wgpu::vertex_attr_array![
            0 => Float32x3, 1 => Float32x3, 2 => Float32x4, 3 => Float32x4,
            14 => Float32x4, 15 => Float32x2, 11 => Float32x4
        ];
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Self>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &ATTRIBUTES,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Instance {
    model: [[f32; 4]; 4],
    normal: [[f32; 4]; 3],
    color: [f32; 4],
    ids: [u32; 4],
}

impl Instance {
    fn new(object: &gpui::MeshDraw3d) -> Self {
        Self {
            model: object.model,
            normal: [object.normal[0], object.normal[1], object.normal[2]],
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
        const ATTRIBUTES: [wgpu::VertexAttribute; 9] = wgpu::vertex_attr_array![
            4 => Float32x4, 5 => Float32x4, 6 => Float32x4, 7 => Float32x4,
            8 => Float32x4, 9 => Float32x4, 10 => Float32x4,
            12 => Float32x4, 13 => Uint32x4
        ];
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Self>() as u64,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &ATTRIBUTES,
        }
    }
}

fn material_bindings(data_output: bool) -> Vec<wgpu::BindGroupLayoutEntry> {
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
        for binding in 12..=15 {
            bindings.push(wgpu::BindGroupLayoutEntry {
                binding,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            });
        }
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
    bindings
}

struct BatchSlot {
    params: wgpu::Buffer,
    instances: wgpu::Buffer,
}

pub(crate) fn instance_limit(device: &wgpu::Device) -> usize {
    ((device.limits().max_buffer_size - std::mem::size_of::<Params>() as u64)
        / std::mem::size_of::<Instance>() as u64)
        .min(u64::from(u32::MAX)) as usize
}

pub(crate) fn validate_device_limits(limits: &wgpu::Limits) -> anyhow::Result<()> {
    let bindings = material_bindings(false);
    let count = |predicate: fn(&wgpu::BindGroupLayoutEntry) -> bool| {
        bindings.iter().filter(|entry| predicate(entry)).count() as u64
    };
    let attributes =
        (Vertex::layout().attributes.len() + Instance::layout().attributes.len()) as u64;
    let uniform_size = std::mem::size_of::<Params>() as u64;
    for (name, actual, required) in [
        ("max_bind_groups", u64::from(limits.max_bind_groups), 1),
        (
            "max_bindings_per_bind_group",
            u64::from(limits.max_bindings_per_bind_group),
            bindings.len() as u64,
        ),
        (
            "max_sampled_textures_per_shader_stage",
            u64::from(limits.max_sampled_textures_per_shader_stage),
            count(|entry| matches!(entry.ty, wgpu::BindingType::Texture { .. })),
        ),
        (
            "max_samplers_per_shader_stage",
            u64::from(limits.max_samplers_per_shader_stage),
            count(|entry| matches!(entry.ty, wgpu::BindingType::Sampler(_))),
        ),
        (
            "max_uniform_buffers_per_shader_stage",
            u64::from(limits.max_uniform_buffers_per_shader_stage),
            1,
        ),
        (
            "max_uniform_buffer_binding_size",
            limits.max_uniform_buffer_binding_size,
            uniform_size,
        ),
        (
            "max_buffer_size",
            limits.max_buffer_size,
            uniform_size + std::mem::size_of::<Instance>() as u64,
        ),
        (
            "max_vertex_buffers",
            u64::from(limits.max_vertex_buffers),
            2,
        ),
        (
            "max_vertex_attributes",
            u64::from(limits.max_vertex_attributes),
            attributes,
        ),
        (
            "max_vertex_buffer_array_stride",
            u64::from(limits.max_vertex_buffer_array_stride),
            std::mem::size_of::<Instance>().max(std::mem::size_of::<Vertex>()) as u64,
        ),
        (
            "max_inter_stage_shader_variables",
            u64::from(limits.max_inter_stage_shader_variables),
            9,
        ),
        (
            "max_color_attachments",
            u64::from(limits.max_color_attachments),
            1,
        ),
        (
            "max_color_attachment_bytes_per_sample",
            u64::from(limits.max_color_attachment_bytes_per_sample),
            8,
        ),
        (
            "max_texture_array_layers",
            u64::from(limits.max_texture_array_layers),
            6,
        ),
        (
            "max_texture_dimension_2d",
            u64::from(limits.max_texture_dimension_2d),
            1024,
        ),
    ] {
        anyhow::ensure!(
            actual >= required,
            "3D rendering requires {name} >= {required}, device enables {actual}"
        );
    }
    Ok(())
}

fn instance_buffer(device: &wgpu::Device, capacity: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("mesh_instances"),
        size: (capacity * std::mem::size_of::<Instance>()) as u64,
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
            uv_u: [
                rows[0][0],
                rows[0][1],
                rows[0][2],
                f32::from(sampling.mip_filter != gpui::TextureMipFilter3d::None),
            ],
            uv_v: [rows[1][0], rows[1][1], rows[1][2], 0.],
            sampling: [
                sampling.address_u as u32,
                sampling.address_v as u32,
                images::filter_flags(sampling),
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
    fn prepare(
        device: &wgpu::Device,
        previous: Option<Self>,
        mesh: &Mesh3d,
        uv_sets: [u32; 5],
    ) -> Self {
        let vertices = (0..mesh.vertices().len())
            .map(|index| Vertex::new(mesh, index, uv_sets))
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
    images: Arc<parking_lot::Mutex<images::ImageCache>>,
    slots: Vec<BatchSlot>,
    offsets: HashMap<usize, usize>,
    plans: instances::BatchPlanCache,
    batch_limit: usize,
    targets: HashMap<[u32; 2], Targets>,
    regions: HashMap<usize, RenderRegion>,
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
        let bindings = material_bindings(data_output);
        let shadow_pipeline = (!data_output).then(|| {
            let material = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("scene3d_shadow_material"),
                entries: &bindings
                    .iter()
                    .filter(|binding| binding.binding <= 2)
                    .cloned()
                    .collect::<Vec<_>>(),
            });
            let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("scene3d_shadow"),
                bind_group_layouts: &[Some(&material)],
                immediate_size: 0,
            });
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("scene3d_shadow"),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("shadow_vertex"),
                    compilation_options: Default::default(),
                    buffers: &[Some(Vertex::layout()), Some(Instance::layout())],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("shadow_fragment"),
                    compilation_options: Default::default(),
                    targets: &[],
                }),
                primitive: wgpu::PrimitiveState {
                    cull_mode: None,
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    depth_write_enabled: Some(true),
                    depth_compare: Some(wgpu::CompareFunction::Less),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: Default::default(),
                multiview_mask: None,
                cache: None,
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
                label: Some("scene3d"),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vertex"),
                    compilation_options: Default::default(),
                    buffers: &[Some(Vertex::layout()), Some(Instance::layout())],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(fragment),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: mesh_format,
                        blend: blend.then_some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState {
                    cull_mode: None,
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    depth_write_enabled: Some(!blend),
                    depth_compare: Some(wgpu::CompareFunction::Less),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: wgpu::MultisampleState {
                    count: samples,
                    ..Default::default()
                },
                multiview_mask: None,
                cache: None,
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
            images: Default::default(),
            slots: Vec::new(),
            offsets: HashMap::new(),
            plans: instances::BatchPlanCache::default(),
            batch_limit: instance_limit(device),
            targets: HashMap::default(),
            regions: HashMap::default(),
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
        capabilities: gpui::Scene3dViewportCapabilities,
    ) {
        self.offsets.clear();
        self.regions.clear();
        let mut frames = Vec::new();
        let mut layers = Vec::new();
        viewport::visit_scenes(scene, |scene| {
            for layer in &scene.subtree_layers {
                let Some(frame) = &layer.scene3d else {
                    continue;
                };
                if capabilities.color_samples_for(frame.viewport_quality) != self.samples {
                    continue;
                }
                let bounds = layer.composite.bounds;
                let Some(region) = RenderRegion::viewport(
                    [
                        bounds.origin.x.0,
                        bounds.origin.y.0,
                        bounds.size.width.0,
                        bounds.size.height.0,
                    ],
                    [width, height],
                    frame.viewport_quality.resolution_scale(),
                    capabilities.max_texture_dimension,
                ) else {
                    continue;
                };
                self.regions.insert(layer as *const _ as usize, region);
                layers.push(layer as *const _ as usize);
                frames.push(frame.clone());
            }
        });
        let sizes: Vec<_> = self.regions.values().map(|region| region.size).collect();
        self.prepare_frames(device, queue, frames.iter().map(AsRef::as_ref), sizes);
        let mut slot_count = 0;
        for (layer, frame) in layers.into_iter().zip(&frames) {
            self.offsets.insert(layer, slot_count);
            slot_count += self.plan(frame).batches.len();
        }
    }

    fn plan(&self, frame: &gpui::Scene3dFrame) -> &instances::BatchPlan {
        self.plans
            .get(frame, self.blend_pipeline.is_some(), self.batch_limit)
    }

    pub(crate) fn draw_statistics(
        &self,
        frame: &gpui::Scene3dFrame,
    ) -> crate::Scene3dDrawStatistics {
        self.plan(frame).statistics(frame)
    }

    pub(crate) fn plan_statistics(
        frame: &gpui::Scene3dFrame,
        color: bool,
        limit: usize,
    ) -> crate::Scene3dDrawStatistics {
        instances::BatchPlan::new(frame, color, limit).statistics(frame)
    }

    pub(crate) fn prepare_frames<'a>(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        frames: impl IntoIterator<Item = &'a gpui::Scene3dFrame>,
        sizes: impl IntoIterator<Item = [u32; 2]>,
    ) {
        let frames: Vec<_> = frames.into_iter().collect();
        self.images
            .lock()
            .retain(frames.iter().flat_map(|frame| frame.objects.iter()));
        self.plans.prepare(
            frames.iter().copied(),
            self.blend_pipeline.is_some(),
            self.batch_limit,
        );
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
                self.plan(frame).order.iter().map(|&index| {
                    let object = &frame.objects[index];
                    (object.mesh.clone(), object.texture_uv_sets())
                })
            })
            .collect();
        self.geometry.prepare(meshes, |previous, mesh, uv_sets| {
            Geometry::prepare(device, previous, mesh, uv_sets)
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
            let current = self.slots.get(index).map_or(0, |slot| {
                (slot.instances.size() / std::mem::size_of::<Instance>() as u64) as usize
            });
            let capacity = instances::retained_capacity(current, count, self.batch_limit);
            if index == self.slots.len() {
                self.slots.push(BatchSlot {
                    params: device.create_buffer(&wgpu::BufferDescriptor {
                        label: Some("mesh_params"),
                        size: std::mem::size_of::<Params>() as u64,
                        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                        mapped_at_creation: false,
                    }),
                    instances: instance_buffer(device, capacity),
                });
            } else if current != capacity {
                self.slots[index].instances = instance_buffer(device, capacity);
            }
        }
        if !has_frame {
            self.targets.clear();
            return;
        }
        let sizes: HashSet<_> = sizes.into_iter().collect();
        self.targets.retain(|size, _| sizes.contains(size));
        for [width, height] in sizes {
            if self.targets.contains_key(&[width, height]) {
                continue;
            }
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
            self.targets.insert(
                [width, height],
                Targets {
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
                },
            );
        }
    }

    pub(crate) fn reuse_resources_from(&mut self, other: &Self) {
        self.geometry.reuse_from(&other.geometry);
        self.images = other.images.clone();
        self.reuse_plans_from(other);
    }

    pub(crate) fn reuse_plans_from(&mut self, other: &Self) {
        self.plans.reuse_from(&other.plans);
    }

    pub(crate) fn prepare_frame_retention(
        &mut self,
        frame: Option<&gpui::Scene3dFrame>,
        retain_geometry: bool,
    ) {
        let color = self.blend_pipeline.is_some();
        self.plans.prepare(frame, color, self.batch_limit);
        if !retain_geometry {
            return;
        }
        let plans = &self.plans;
        let limit = self.batch_limit;
        self.geometry.retain(frame.into_iter().flat_map(|frame| {
            plans.get(frame, color, limit).order.iter().map(|&index| {
                let object = &frame.objects[index];
                (object.mesh.clone(), object.texture_uv_sets())
            })
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
        let Some(&region) = self.regions.get(&(layer as *const _ as usize)) else {
            let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("scene3d_empty"),
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
            return;
        };
        let start = self.offsets[&(layer as *const _ as usize)];
        self.encode_frame(
            device,
            queue,
            atlas,
            frame,
            region,
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
        queue: &wgpu::Queue,
        atlas: &WgpuAtlas,
        frame: &gpui::Scene3dFrame,
        region: RenderRegion,
        start: usize,
        source: Option<&wgpu::TextureView>,
        destination: Option<&wgpu::TextureView>,
        encoder: &mut wgpu::CommandEncoder,
    ) {
        let targets = &self.targets[&region.size];
        let rect = region.rect;
        let plan = self.plan(frame);
        let mut uploaded = HashSet::new();
        for &index in &plan.order {
            let object = &frame.objects[index];
            if uploaded.insert((Arc::as_ptr(&object.mesh), object.texture_uv_sets())) {
                self.geometry
                    .get(&object.mesh, object.texture_uv_sets())
                    .encode_upload(encoder);
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
            let resolve_image = |map: gpui::MaterialTexture3d, color_space| {
                self.images
                    .lock()
                    .get(device, queue, atlas, map.tile, color_space, map.sampling)
            };
            let atlas_texture = match object.texture {
                MeshTexture3d::Image(tile) => Some(resolve_image(
                    gpui::MaterialTexture3d {
                        tile,
                        sampling: object.sampling,
                        uv_set: object.uv_set,
                    },
                    object.image_color_space,
                )),
                _ => None,
            };
            let (texture, texture_rect, premultiplied) = match object.texture {
                MeshTexture3d::None => (&self.white, [0., 0., 1., 1.], 0.),
                MeshTexture3d::Subtree => {
                    let texture_rect = frame.ui_texture.map_or(region.source_rect, |texture| {
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
            let maps_enabled =
                object.pbr.is_some() && !object.unlit && self.display_pipeline.is_some();
            let metallic_roughness_map = object.metallic_roughness_texture.filter(|_| maps_enabled);
            let emissive_map = object.emissive_texture.filter(|_| maps_enabled);
            let normal_map = object
                .normal_texture
                .filter(|_| maps_enabled && object.normal_scale > 0.);
            let normal_image =
                normal_map.map(|map| resolve_image(map, gpui::TextureColorSpace3d::Linear));
            let occlusion_map = object.occlusion_texture.filter(|_| {
                !object.unlit && object.occlusion_strength > 0. && self.display_pipeline.is_some()
            });
            let occlusion_image =
                occlusion_map.map(|map| resolve_image(map, gpui::TextureColorSpace3d::Linear));
            let metallic_roughness_image = metallic_roughness_map
                .map(|map| resolve_image(map, gpui::TextureColorSpace3d::Linear));
            let emissive_image =
                emissive_map.map(|map| resolve_image(map, gpui::TextureColorSpace3d::Srgb));
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
                    object.alpha_cutoff,
                    f32::from(object.unlit),
                    premultiplied,
                    f32::from(matches!(object.texture, MeshTexture3d::Image(_))),
                ],
                ids: [
                    0,
                    object.alpha_mode as u32,
                    u32::from(object.double_sided),
                    0,
                ],
                uv_u: [
                    rows[0][0],
                    rows[0][1],
                    rows[0][2],
                    f32::from(object.sampling.mip_filter != gpui::TextureMipFilter3d::None),
                ],
                uv_v: [rows[1][0], rows[1][1], rows[1][2], 0.],
                sampling: [
                    object.sampling.address_u as u32,
                    object.sampling.address_v as u32,
                    images::filter_flags(object.sampling),
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
                    resource: wgpu::BindingResource::Sampler(
                        atlas_texture
                            .as_ref()
                            .map_or(&self.sampler, |image| &image.sampler),
                    ),
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
                for (binding, image) in [
                    (12, &metallic_roughness_image),
                    (13, &emissive_image),
                    (14, &normal_image),
                    (15, &occlusion_image),
                ] {
                    entries.push(wgpu::BindGroupEntry {
                        binding,
                        resource: wgpu::BindingResource::Sampler(
                            image.as_ref().map_or(&self.sampler, |image| &image.sampler),
                        ),
                    });
                }
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
                let geometry = self.geometry.get(&object.mesh, object.texture_uv_sets());
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
                let geometry = self.geometry.get(&object.mesh, object.texture_uv_sets());
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
            let params = DisplayParams {
                settings: [
                    2.0_f32.powf(frame.color_output.exposure),
                    frame.color_output.tone_mapping as u32 as f32,
                    f32::from(self.format.is_srgb()),
                    0.,
                ],
                output_rect: [
                    region.origin[0] as f32,
                    region.origin[1] as f32,
                    region.output_size[0] as f32,
                    region.output_size[1] as f32,
                ],
            };
            let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("scene3d_display_params"),
                contents: bytemuck::bytes_of(&params),
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
            pass.set_scissor_rect(
                region.origin[0],
                region.origin[1],
                region.output_size[0],
                region.output_size[1],
            );
            pass.set_bind_group(0, &group, &[]);
            pass.draw(0..3, 0..1);
        }
    }

    pub(crate) fn copy_linear_color(
        &self,
        destination: &wgpu::Texture,
        encoder: &mut wgpu::CommandEncoder,
    ) {
        let source = self.targets[&[destination.width(), destination.height()]]
            .hdr
            .as_ref()
            .unwrap();
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

    fn output_layer(texture: MeshTexture3d) -> SubtreeLayer {
        let mut object = object();
        object.texture = texture;
        let bounds = gpui::Bounds::new(
            gpui::point(gpui::ScaledPixels(8.), gpui::ScaledPixels(4.)),
            gpui::size(gpui::ScaledPixels(32.), gpui::ScaledPixels(24.)),
        );
        SubtreeLayer {
            scene3d: Some(Arc::new(frame(&[object]))),
            scene: std::rc::Rc::new(Scene::default()),
            second_scene: None,
            intermediate_effects: Arc::default(),
            composite: gpui::EffectQuad {
                order: 0,
                bounds,
                effect_bounds: bounds,
                transformation: Default::default(),
                content_mask: gpui::ContentMask { bounds },
                shader: gpui::EffectShader::wgsl_image(
                    "fn effect(input: EffectInput, params: EffectParams) -> vec4<f32> { return sample_effect_image(input, input.uv); }",
                ),
                uniforms: Default::default(),
                time: 0.,
                corner_radii: Default::default(),
                opacity: 1.,
                image_tile: None,
                second_image_tile: None,
                third_image_tile: None,
                fourth_image_tile: None,
            },
        }
    }

    #[test]
    fn output_reuse_requires_submission_and_failed_encoding_does_not_commit_pixels() {
        let state = output_cache::OutputValidity::default();
        state.commit(true);
        assert!(!state.reusable());
        state.encoded();
        assert!(!state.reusable());
        state.commit(false);
        assert!(!state.reusable());
        state.encoded();
        state.commit(true);
        assert!(state.reusable());
        state.commit(false);
        assert!(state.reusable());
    }

    #[test]
    fn output_keys_track_owned_frames_regions_ui_snapshots_and_atlas_versions() {
        use output_cache::OutputKey;
        let tile = gpui::AtlasTile {
            texture_id: gpui::AtlasTextureId {
                index: 0,
                kind: gpui::AtlasTextureKind::Polychrome,
            },
            tile_id: gpui::TileId(1),
            padding: 0,
            bounds: gpui::Bounds::new(
                gpui::point(gpui::DevicePixels(0), gpui::DevicePixels(0)),
                gpui::size(gpui::DevicePixels(8), gpui::DevicePixels(8)),
            ),
        };
        let mut layer = output_layer(MeshTexture3d::Image(tile));
        let region = RenderRegion::viewport([8., 4., 32., 24.], [64, 64], 1., 4096).unwrap();
        let key = OutputKey::new(&layer, region, |_| Some(10)).unwrap();
        assert!(key.matches(&OutputKey::new(&layer.clone(), region, |_| Some(10)).unwrap()));
        assert!(!key.matches(&OutputKey::new(&layer, region, |_| Some(11)).unwrap()));
        assert!(OutputKey::new(&layer, region, |_| None).is_none());
        let scaled = RenderRegion::viewport([8., 4., 32., 24.], [64, 64], 0.5, 4096).unwrap();
        assert!(!key.matches(&OutputKey::new(&layer, scaled, |_| Some(10)).unwrap()));
        layer.scene = std::rc::Rc::new(Scene::default());
        assert!(key.matches(&OutputKey::new(&layer, region, |_| Some(10)).unwrap()));
        Arc::make_mut(layer.scene3d.as_mut().unwrap()).ambient = 0.7;
        assert!(!key.matches(&OutputKey::new(&layer, region, |_| Some(10)).unwrap()));

        let mut ui = output_layer(MeshTexture3d::Subtree);
        let mut image = ui.composite.clone();
        image.image_tile = Some(tile);
        std::rc::Rc::get_mut(&mut ui.scene)
            .unwrap()
            .insert_primitive(image);
        let ui_key = OutputKey::new(&ui, region, |input| {
            assert_eq!(input, tile);
            Some(20)
        })
        .unwrap();
        assert!(ui_key.matches(&OutputKey::new(&ui.clone(), region, |_| Some(20)).unwrap()));
        assert!(!ui_key.matches(&OutputKey::new(&ui, region, |_| Some(21)).unwrap()));
        ui.scene = std::rc::Rc::new(Scene::default());
        assert!(!ui_key.matches(&OutputKey::new(&ui, region, |_| Some(20)).unwrap()));
        ui.intermediate_effects = vec![gpui::SubtreeEffectPass {
            shader: ui.composite.shader.clone(),
            uniforms: Default::default(),
            time: 0.,
            images: Default::default(),
            bloom: None,
            feedback: None,
            distance_field: None,
            particles: None,
            particle_transition: None,
        }]
        .into();
        let processed = OutputKey::new(&ui, region, |_| Some(20)).unwrap();
        ui.composite.effect_bounds.size.width.0 += 4.;
        assert!(!processed.matches(&OutputKey::new(&ui, region, |_| Some(20)).unwrap()));

        let mesh_frame = Arc::make_mut(layer.scene3d.as_mut().unwrap());
        let object = &mut Arc::make_mut(&mut mesh_frame.objects)[0];
        for (index, slot) in [
            &mut object.metallic_roughness_texture,
            &mut object.emissive_texture,
            &mut object.normal_texture,
            &mut object.occlusion_texture,
        ]
        .into_iter()
        .enumerate()
        {
            *slot = Some(gpui::MaterialTexture3d {
                tile: gpui::AtlasTile {
                    tile_id: gpui::TileId(index as u32 + 2),
                    ..tile
                },
                sampling: Default::default(),
                uv_set: 0,
            });
        }
        let mut referenced = Vec::new();
        let mapped = OutputKey::new(&layer, region, |tile| {
            referenced.push(tile.tile_id.0);
            Some(u64::from(tile.tile_id.0))
        })
        .unwrap();
        assert_eq!(referenced, vec![1, 2, 3, 4, 5]);
        for changed in referenced {
            assert!(
                !mapped.matches(
                    &OutputKey::new(&layer, region, |tile| {
                        Some(
                            u64::from(tile.tile_id.0)
                                + if tile.tile_id.0 == changed { 100 } else { 0 },
                        )
                    })
                    .unwrap()
                )
            );
        }
    }

    #[test]
    fn dynamic_ui_is_not_reused_and_capture_boundaries_have_independent_ownership() {
        let mut layer = output_layer(MeshTexture3d::Subtree);
        let region = RenderRegion::full([64, 64]);
        let draw = gpui::ParticleDraw {
            order: 0,
            bounds: layer.composite.bounds,
            content_mask: layer.composite.content_mask,
            scale_factor: 1.,
            opacity: 1.,
            frame: Arc::new(gpui::ParticleFrame {
                id: gpui::EffectHistoryId::new(),
                generation: 0,
                frame: 0,
                time: Default::default(),
                capacity: 16,
                physics: Default::default(),
                spawns: Arc::default(),
                needs_animation: true,
            }),
        };
        std::rc::Rc::get_mut(&mut layer.scene)
            .unwrap()
            .insert_primitive(draw);
        assert!(output_cache::OutputKey::new(&layer, region, |_| Some(0)).is_none());
        let nested = output_layer(MeshTexture3d::None);
        std::rc::Rc::get_mut(&mut layer.scene)
            .unwrap()
            .insert_primitive(gpui::Primitive::SubtreeLayer(nested));
        Arc::get_mut(layer.scene3d.as_mut().unwrap())
            .unwrap()
            .ui_texture = Some(gpui::UiTexture3d::new(
            gpui::size(gpui::px(64.), gpui::px(64.)),
            1.,
        ));
        let mut scene = Scene::default();
        scene.insert_primitive(gpui::Primitive::SubtreeLayer(layer));
        let mut count = 0;
        viewport::visit_scenes(&scene, |scene| count += scene.subtree_layers.len());
        assert_eq!(count, 1);
        let child = &scene.subtree_layers[0].scene;
        let mut child_count = 0;
        viewport::visit_scenes(child, |scene| child_count += scene.subtree_layers.len());
        assert_eq!(child_count, 1);
    }

    pub(super) const IDENTITY: [[f32; 4]; 4] = [
        [1., 0., 0., 0.],
        [0., 1., 0., 0.],
        [0., 0., 1., 0.],
        [0., 0., 0., 1.],
    ];

    pub(super) fn frame(objects: &[gpui::MeshDraw3d]) -> gpui::Scene3dFrame {
        gpui::Scene3dFrame {
            viewport_quality: Default::default(),
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

    pub(super) fn object() -> gpui::MeshDraw3d {
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
            uv_set: 0,
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
            double_sided: true,
            alpha_mode: gpui::AlphaMode3d::Opaque,
            sort_depth: 0.,
        }
    }

    #[test]
    #[ignore = "requires a GPU adapter"]
    fn scene3d_target_memory_matches_allocated_attachments_and_shadow_maps() -> anyhow::Result<()> {
        let context = crate::WgpuContext::new_headless()?;
        let capabilities = crate::Scene3dDeviceCapabilities::query(&context).rendering()?;
        let channels = capabilities.channels();
        let mut input = frame(&[]);
        input.directional_shadow = Some(gpui::DirectionalShadow3d {
            light_index: 0,
            view_projection: IDENTITY,
            resolution: 256,
            depth_bias: 0.,
            normal_bias: 0.,
            softness: 0.,
        });
        let bytes = |texture: &wgpu::Texture| {
            u64::from(texture.width())
                * u64::from(texture.height())
                * u64::from(texture.sample_count())
                * u64::from(texture.format().block_copy_size(None).unwrap())
        };
        for &samples in capabilities.color_sample_counts(channels) {
            let config = crate::Scene3dOutputConfig {
                size: [13, 7],
                channels,
                color_samples: samples,
            };
            let predicted = config.target_memory(Some(256))?;
            let mut attachments = 0;
            let mut shadows = 0;
            for (channel, format, samples) in [
                (
                    crate::Scene3dChannels::COLOR,
                    wgpu::TextureFormat::Rgba8Unorm,
                    samples,
                ),
                (
                    crate::Scene3dChannels::OBJECT_ID,
                    wgpu::TextureFormat::R32Uint,
                    1,
                ),
                (
                    crate::Scene3dChannels::LINEAR_DEPTH,
                    wgpu::TextureFormat::R32Float,
                    1,
                ),
                (
                    crate::Scene3dChannels::WORLD_NORMAL,
                    wgpu::TextureFormat::Rgba32Float,
                    1,
                ),
            ] {
                if !channels.contains(channel) {
                    continue;
                }
                let mut renderer =
                    Scene3dRenderer::new(&context.device, &context.queue, format, samples);
                renderer.prepare_frames(&context.device, &context.queue, [&input], [config.size]);
                let targets = &renderer.targets[&config.size];
                attachments += bytes(&targets.depth) + targets.hdr.as_ref().map_or(0, bytes);
                shadows += renderer.shadow_maps.values().map(bytes).sum::<u64>();
            }
            assert_eq!(predicted.attachment_bytes, attachments);
            assert_eq!(predicted.shadow_bytes, shadows);
        }
        Ok(())
    }

    #[test]
    #[ignore = "requires a GPU adapter"]
    fn scene3d_instance_buffers_shrink_reuse_and_release_with_active_batches() -> anyhow::Result<()>
    {
        let context = crate::WgpuContext::new_headless()?;
        let source = object();
        let dense = frame(&vec![source.clone(); 512]);
        let medium = frame(&vec![source.clone(); 257]);
        let sparse = frame(&vec![source.clone(); 8]);
        let nearby = frame(&vec![source.clone(); 7]);
        let mut different = source;
        different.alpha_mode = gpui::AlphaMode3d::Mask;
        let split = frame(&[dense.objects[0].clone(), different]);
        for format in [
            wgpu::TextureFormat::Rgba8Unorm,
            wgpu::TextureFormat::R32Uint,
        ] {
            let mut renderer = Scene3dRenderer::new(&context.device, &context.queue, format, 1);
            renderer.prepare_frames(&context.device, &context.queue, [&dense], [[16, 16]]);
            assert_eq!(renderer.slots.len(), 1);
            let large = renderer.slots[0].instances.clone();
            let params = renderer.slots[0].params.clone();
            assert_eq!(large.size(), 512 * std::mem::size_of::<Instance>() as u64);
            renderer.prepare_frames(&context.device, &context.queue, [&medium], [[16, 16]]);
            assert_eq!(renderer.slots[0].instances, large);

            renderer.prepare_frames(&context.device, &context.queue, [&sparse], [[16, 16]]);
            let small = renderer.slots[0].instances.clone();
            assert_eq!(small.size(), 8 * std::mem::size_of::<Instance>() as u64);
            assert_ne!(small, large);
            assert_eq!(renderer.slots[0].params, params);
            assert_eq!(large.size(), 512 * std::mem::size_of::<Instance>() as u64);
            renderer.prepare_frames(&context.device, &context.queue, [&nearby], [[16, 16]]);
            assert_eq!(renderer.slots[0].instances, small);

            renderer.prepare_frames(&context.device, &context.queue, [&split], [[16, 16]]);
            assert_eq!(renderer.slots.len(), 2);
            assert!(
                renderer
                    .slots
                    .iter()
                    .all(|slot| slot.instances.size() == std::mem::size_of::<Instance>() as u64)
            );
            renderer.prepare_frames(&context.device, &context.queue, [&dense], [[16, 16]]);
            assert_eq!(renderer.slots.len(), 1);
            assert_eq!(renderer.slots[0].instances.size(), large.size());
            assert_ne!(renderer.slots[0].instances, large);
            renderer.prepare_frames(&context.device, &context.queue, [], []);
            assert!(renderer.slots.is_empty());
        }
        Ok(())
    }

    #[test]
    #[ignore = "requires a GPU adapter"]
    fn scene3d_target_cache_tracks_viewport_sizes_reuse_and_eviction() -> anyhow::Result<()> {
        let context = crate::WgpuContext::new_headless()?;
        let capabilities = gpui::Scene3dViewportCapabilities {
            max_texture_dimension: context.device.limits().max_texture_dimension_2d,
            color_samples: 1,
            max_ui_texture_dimension: 2048,
        };
        let mut renderer = Scene3dRenderer::new(
            &context.device,
            &context.queue,
            wgpu::TextureFormat::Rgba8Unorm,
            1,
        );
        let make_scene = |rects: &[[f32; 4]]| {
            let mut scene = Scene::default();
            for &[x, y, width, height] in rects {
                let bounds = gpui::Bounds::new(
                    gpui::point(gpui::ScaledPixels(x), gpui::ScaledPixels(y)),
                    gpui::size(gpui::ScaledPixels(width), gpui::ScaledPixels(height)),
                );
                scene.insert_primitive(gpui::Primitive::SubtreeLayer(SubtreeLayer {
                    scene3d: Some(Arc::new(frame(&[]))),
                    scene: std::rc::Rc::new(Scene::default()),
                    second_scene: None,
                    intermediate_effects: Arc::default(),
                    composite: gpui::EffectQuad {
                        order: 0,
                        bounds,
                        effect_bounds: bounds,
                        transformation: Default::default(),
                        content_mask: gpui::ContentMask { bounds },
                        shader: gpui::EffectShader::wgsl_image(
                            "fn effect(input: EffectInput, params: EffectParams) -> vec4<f32> { return sample_effect_image(input, input.uv); }",
                        ),
                        uniforms: Default::default(),
                        time: 0.,
                        corner_radii: Default::default(),
                        opacity: 1.,
                        image_tile: None,
                        second_image_tile: None,
                        third_image_tile: None,
                        fourth_image_tile: None,
                    },
                }));
            }
            scene.finish();
            scene
        };
        let scene = make_scene(&[
            [12., 20., 64., 48.],
            [100., 20., 64., 48.],
            [12.25, 100.75, 32.5, 24.5],
            [3000., 0., 64., 48.],
        ]);
        renderer.prepare(
            &context.device,
            &context.queue,
            &scene,
            1024,
            768,
            capabilities,
        );
        assert_eq!(renderer.targets.len(), 2);
        assert_eq!(renderer.regions.len(), 3);
        let depth = renderer.targets[&[64, 48]].depth.clone();
        let hdr = renderer.targets[&[64, 48]].hdr.as_ref().unwrap().clone();
        assert_eq!([depth.width(), depth.height()], [64, 48]);
        assert_eq!([hdr.width(), hdr.height()], [64, 48]);
        assert_eq!(renderer.targets[&[33, 26]].depth.height(), 26);

        renderer.prepare(
            &context.device,
            &context.queue,
            &scene,
            2048,
            1536,
            capabilities,
        );
        assert_eq!(renderer.targets.len(), 2);
        assert_eq!(renderer.targets[&[64, 48]].depth, depth);
        assert_eq!(renderer.targets[&[64, 48]].hdr.as_ref().unwrap(), &hdr);
        let smaller = make_scene(&[[12., 20., 64., 48.]]);
        renderer.prepare(
            &context.device,
            &context.queue,
            &smaller,
            40,
            40,
            capabilities,
        );
        assert_eq!(renderer.targets.len(), 1);
        assert!(renderer.targets.contains_key(&[28, 20]));
        renderer.prepare(
            &context.device,
            &context.queue,
            &Scene::default(),
            40,
            40,
            capabilities,
        );
        assert!(renderer.targets.is_empty());
        assert!(renderer.regions.is_empty());
        Ok(())
    }

    #[test]
    fn scene3d_draw_statistics_count_shared_work_and_selected_channels() {
        use crate::{Scene3dChannels as C, Scene3dDrawStatistics as S};
        let input = frame(&vec![object(); 7]);
        let color = S::plan(&input, C::COLOR, 3).unwrap();
        assert_eq!(
            (
                color.camera_draws,
                color.camera_instances,
                color.camera_triangles
            ),
            (3, 7, 7)
        );
        assert_eq!(color.shadow_draws, 0);
        assert_eq!(color.batches, 3);
        assert_eq!(
            color.instance_upload_bytes,
            7 * std::mem::size_of::<Instance>() as u64
        );
        assert_eq!(
            color.uniform_upload_bytes,
            3 * std::mem::size_of::<Params>() as u64
        );
        assert_eq!(
            S::plan(&input, C::COLOR | C::LINEAR_COLOR, 3).unwrap(),
            color
        );
        let all = S::plan(&input, C::all(), 3).unwrap();
        let mut expected = color;
        for _ in 0..3 {
            expected += color;
        }
        assert_eq!(all, expected);
        assert_eq!(S::plan(&input, C::OBJECT_ID, 100).unwrap().camera_draws, 1);
        assert_eq!(S::plan(&input, C::WORLD_NORMAL, 1).unwrap().camera_draws, 7);
        assert_eq!(S::plan(&frame(&[]), C::all(), 1).unwrap(), S::default());
        assert!(S::plan(&input, C::empty(), 1).is_err());
        assert!(S::plan(&input, C::from_bits_retain(128), 1).is_err());
        assert!(S::plan(&input, C::COLOR, 0).is_err());
    }

    #[test]
    fn scene3d_draw_statistics_distinguish_shadow_only_meshes_and_batch_breaks() {
        use crate::{Scene3dChannels as C, Scene3dDrawStatistics as S};
        let source = object();
        let mut caster = source.clone();
        caster.model[3][0] = 3.;
        let mut outside = source.clone();
        outside.model[3][0] = 20.;
        let mut blended = source.clone();
        blended.alpha_mode = gpui::AlphaMode3d::Blend;
        let mut different = source.clone();
        different.pbr = Some(Default::default());
        let mut vertices = source.mesh.vertices().to_vec();
        vertices.push(gpui::MeshVertex3d {
            position: [1., 1., 0.],
            normal: [0., 0., 1.],
            uv: [1., 1.],
        });
        different.mesh = gpui::Mesh3d::new(vertices, vec![0, 1, 2, 2, 1, 3]);
        let mut input = frame(&[
            source.clone(),
            source,
            different,
            caster,
            outside,
            blended.clone(),
            blended,
        ]);
        let mut shadow_matrix = IDENTITY;
        shadow_matrix[0][0] = 0.2;
        input.directional_shadow = Some(gpui::DirectionalShadow3d {
            light_index: 0,
            view_projection: shadow_matrix,
            resolution: 256,
            depth_bias: 0.,
            normal_bias: 0.,
            softness: 0.,
        });
        let color = S::plan(&input, C::COLOR, 100).unwrap();
        assert_eq!(
            (
                color.camera_draws,
                color.camera_instances,
                color.camera_triangles
            ),
            (4, 5, 6)
        );
        assert_eq!(
            (
                color.shadow_draws,
                color.shadow_instances,
                color.shadow_triangles
            ),
            (3, 4, 5)
        );
        // Camera/shadow-shared batches upload once; shadow-only batches still upload.
        assert_eq!(color.batches, 5);
        assert_eq!(
            color.instance_upload_bytes,
            6 * std::mem::size_of::<Instance>() as u64
        );
        let data = S::plan(&input, C::OBJECT_ID, 100).unwrap();
        assert_eq!(
            (data.camera_draws, data.camera_instances, data.batches),
            (4, 5, 4)
        );
        assert_eq!(data.shadow_draws, 0);
        let mut combined = color;
        combined += data;
        assert_eq!(
            S::plan(&input, C::COLOR | C::OBJECT_ID, 100).unwrap(),
            combined
        );
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
            assert_eq!(instance.normal, object.normal[..3]);
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
        push(|v| v.double_sided = false);
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
            uv_set: 0,
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
        let mut changed = source.clone();
        changed.uv_set = 7;
        variants.push(changed);
        for slot in 0..4 {
            let mut changed = source.clone();
            [
                &mut changed.metallic_roughness_texture,
                &mut changed.emissive_texture,
                &mut changed.normal_texture,
                &mut changed.occlusion_texture,
            ][slot]
                .as_mut()
                .unwrap()
                .uv_set = 7;
            variants.push(changed);
        }
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
        changed.sampling.mag_filter = Some(gpui::TextureFilter3d::Nearest);
        variants.push(changed);
        let mut changed = source.clone();
        changed.sampling.mip_filter = gpui::TextureMipFilter3d::Linear;
        variants.push(changed.clone());
        changed.sampling.max_anisotropy = 8;
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
        let (_, params) = module
            .types
            .iter()
            .find(|(_, ty)| ty.name.as_deref() == Some("Params"))
            .unwrap();
        let naga::TypeInner::Struct { members, span } = &params.inner else {
            panic!("display parameters must be a struct");
        };
        assert_eq!(*span as usize, std::mem::size_of::<DisplayParams>());
        for (name, offset) in [
            ("settings", std::mem::offset_of!(DisplayParams, settings)),
            (
                "output_rect",
                std::mem::offset_of!(DisplayParams, output_rect),
            ),
        ] {
            let member = members
                .iter()
                .find(|member| member.name.as_deref() == Some(name))
                .unwrap();
            assert_eq!(member.offset as usize, offset, "{name}");
        }
    }

    #[test]
    fn scene3d_pipeline_bindings_cover_each_shader_entry_point() {
        let module = naga::front::wgsl::parse_str(include_str!("../scene3d.wgsl")).unwrap();
        let info = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap();
        for (index, entry) in module.entry_points.iter().enumerate() {
            let data_output = matches!(
                entry.name.as_str(),
                "object_id" | "linear_depth" | "world_normal"
            );
            let mut bindings = material_bindings(data_output);
            if entry.name.starts_with("shadow_") {
                bindings.retain(|binding| binding.binding <= 2);
            }
            let stage = match entry.stage {
                naga::ShaderStage::Vertex => wgpu::ShaderStages::VERTEX,
                naga::ShaderStage::Fragment => wgpu::ShaderStages::FRAGMENT,
                _ => panic!("unexpected mesh stage"),
            };
            for (handle, global) in module.global_variables.iter() {
                if info.get_entry_point(index)[handle].is_empty() {
                    continue;
                }
                let Some(resource) = &global.binding else {
                    continue;
                };
                assert_eq!(resource.group, 0);
                let binding = bindings
                    .iter()
                    .find(|binding| binding.binding == resource.binding)
                    .unwrap_or_else(|| {
                        panic!(
                            "{} has no layout for {} at binding {}",
                            entry.name,
                            global.name.as_deref().unwrap_or("unnamed"),
                            resource.binding
                        )
                    });
                assert!(binding.visibility.contains(stage));
                assert_eq!(binding.count, None);
                match (&module.types[global.ty].inner, binding.ty) {
                    (naga::TypeInner::Sampler { comparison }, wgpu::BindingType::Sampler(kind)) => {
                        assert_eq!(
                            kind,
                            if *comparison {
                                wgpu::SamplerBindingType::Comparison
                            } else {
                                wgpu::SamplerBindingType::Filtering
                            }
                        );
                    }
                    (
                        naga::TypeInner::Image {
                            dim,
                            arrayed,
                            class,
                        },
                        wgpu::BindingType::Texture {
                            sample_type,
                            view_dimension,
                            multisampled,
                        },
                    ) => {
                        assert!(!arrayed);
                        assert_eq!(
                            view_dimension,
                            match dim {
                                naga::ImageDimension::D2 => wgpu::TextureViewDimension::D2,
                                naga::ImageDimension::Cube => wgpu::TextureViewDimension::Cube,
                                _ => panic!("unexpected image dimension"),
                            }
                        );
                        match class {
                            naga::ImageClass::Sampled {
                                kind: naga::ScalarKind::Float,
                                multi,
                            } => {
                                assert_eq!(
                                    sample_type,
                                    wgpu::TextureSampleType::Float { filterable: true }
                                );
                                assert_eq!(multisampled, *multi);
                            }
                            naga::ImageClass::Depth { multi } => {
                                assert_eq!(sample_type, wgpu::TextureSampleType::Depth);
                                assert_eq!(multisampled, *multi);
                            }
                            _ => panic!("unexpected texture class"),
                        }
                    }
                    (
                        naga::TypeInner::Struct { span, .. },
                        wgpu::BindingType::Buffer {
                            ty,
                            has_dynamic_offset,
                            min_binding_size,
                        },
                    ) => {
                        assert_eq!(global.space, naga::AddressSpace::Uniform);
                        assert_eq!(ty, wgpu::BufferBindingType::Uniform);
                        assert!(!has_dynamic_offset);
                        assert_eq!(min_binding_size.unwrap().get(), u64::from(*span));
                    }
                    _ => panic!(
                        "{} has an incompatible binding {}",
                        entry.name, resource.binding
                    ),
                }
            }
        }
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
        let vertex_layout = Vertex::layout();
        for entry in module
            .entry_points
            .iter()
            .filter(|e| e.stage == naga::ShaderStage::Vertex)
        {
            for argument in &entry.function.arguments {
                let Some(naga::Binding::Location { location, .. }) = argument.binding else {
                    continue;
                };
                let attribute = vertex_layout
                    .attributes
                    .iter()
                    .find(|a| a.shader_location == location)
                    .unwrap();
                let offset = match argument.name.as_deref().unwrap() {
                    "position" => std::mem::offset_of!(Vertex, position),
                    "normal" => std::mem::offset_of!(Vertex, normal),
                    "uv" => std::mem::offset_of!(Vertex, uv),
                    "tangent" => std::mem::offset_of!(Vertex, tangent),
                    "detail_uv" => std::mem::offset_of!(Vertex, detail_uv),
                    "occlusion_uv" => std::mem::offset_of!(Vertex, occlusion_uv),
                    "vertex_color" => std::mem::offset_of!(Vertex, vertex_color),
                    _ => panic!("unknown vertex attribute"),
                };
                assert_eq!(attribute.offset as usize, offset);
                let naga::TypeInner::Vector { size, scalar } = module.types[argument.ty].inner
                else {
                    panic!("vector attribute");
                };
                assert_eq!(scalar.kind, naga::ScalarKind::Float);
                assert_eq!(attribute.format.size(), u64::from(size as u8) * 4);
                assert!(attribute.offset + attribute.format.size() <= vertex_layout.array_stride);
            }
        }
        let instance_layout = Instance::layout();
        let mut locations = std::collections::BTreeSet::new();
        for attribute in vertex_layout
            .attributes
            .iter()
            .chain(instance_layout.attributes)
        {
            assert!(
                locations.insert(attribute.shader_location),
                "duplicate vertex location"
            );
            assert!(attribute.shader_location < wgpu::Limits::default().max_vertex_attributes);
        }
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

    #[test]
    fn scene3d_vertex_upload_packs_selected_material_coordinates() {
        let mut mesh = object().mesh;
        assert_eq!(Vertex::new(&mesh, 0, [0; 5]).vertex_color, [1.; 4]);
        let colors: Vec<_> = (0..mesh.vertices().len())
            .map(|i| [0.25, 0.5, 0.75, i as f32 / mesh.vertices().len() as f32])
            .collect();
        mesh = mesh.with_vertex_colors(colors.clone()).unwrap();
        let sets = [3, 7, 11, 19, u32::MAX];
        for (slot, set) in sets.into_iter().enumerate() {
            mesh = mesh
                .with_uv_set(
                    set,
                    (0..mesh.vertices().len())
                        .map(|vertex| [slot as f32, vertex as f32 + 0.5])
                        .collect(),
                )
                .unwrap();
        }
        for index in 0..mesh.vertices().len() {
            let vertex = Vertex::new(&mesh, index, sets);
            assert_eq!(vertex.position, mesh.vertices()[index].position);
            assert_eq!(vertex.uv, [0., index as f32 + 0.5, 1., index as f32 + 0.5]);
            assert_eq!(
                vertex.detail_uv,
                [2., index as f32 + 0.5, 3., index as f32 + 0.5]
            );
            assert_eq!(vertex.occlusion_uv, [4., index as f32 + 0.5]);
            assert_eq!(vertex.vertex_color, colors[index]);
        }
    }
}
