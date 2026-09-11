use super::{Instance, Vertex, material_bindings};
#[cfg(not(target_family = "wasm"))]
use crate::{Scene3dMaterialSnapshot, Scene3dMaterialSource};
#[cfg(not(target_family = "wasm"))]
use anyhow::{Context as _, Result, ensure};
#[cfg(not(target_family = "wasm"))]
use std::collections::{HashMap, HashSet};

#[cfg(all(test, not(target_family = "wasm")))]
mod tests;

#[cfg(not(target_family = "wasm"))]
pub(super) struct Pipelines {
    _source: Scene3dMaterialSource,
    pub mesh: wgpu::RenderPipeline,
    pub blend: Option<wgpu::RenderPipeline>,
    pub shadow: Option<wgpu::RenderPipeline>,
}

#[derive(Default)]
#[cfg(not(target_family = "wasm"))]
pub(super) struct MaterialCache(HashMap<usize, Pipelines>);

#[cfg(not(target_family = "wasm"))]
pub(super) fn snapshot(object: &gpui::MeshDraw3d) -> Result<Option<&Scene3dMaterialSnapshot>> {
    object
        .custom_material
        .as_ref()
        .map(|material| {
            material
                .downcast_ref::<Scene3dMaterialSnapshot>()
                .context("unsupported 3D material backend")
        })
        .transpose()
}

#[cfg(not(target_family = "wasm"))]
impl MaterialCache {
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        frames: &[&gpui::Scene3dFrame],
        format: wgpu::TextureFormat,
        samples: u32,
    ) -> Result<()> {
        let mut used = HashSet::new();
        for frame in frames {
            for object in frame.objects.iter() {
                let Some(snapshot) = snapshot(object)? else {
                    continue;
                };
                let source = snapshot.source();
                snapshot.validate_vertex_count(object.mesh.vertices().len())?;
                ensure!(
                    !source.context().device_lost()
                        && std::ptr::eq(device, source.context().device.as_ref()),
                    "3D material belongs to a different or lost device"
                );
                let identity = source.identity();
                used.insert(identity);
                if let std::collections::hash_map::Entry::Vacant(entry) = self.0.entry(identity) {
                    entry.insert(Pipelines::new(device, source, format, samples)?);
                }
            }
        }
        self.0.retain(|shader, _| used.contains(shader));
        Ok(())
    }

    pub fn get(&self, snapshot: &Scene3dMaterialSnapshot) -> &Pipelines {
        &self.0[&snapshot.source().identity()]
    }
}

#[derive(Clone, Copy)]
pub(super) enum Pass {
    Opaque,
    Blend,
    Shadow,
}

pub(super) fn create_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    extension: Option<&wgpu::BindGroupLayout>,
    vertex_extension: Option<&wgpu::BindGroupLayout>,
    format: wgpu::TextureFormat,
    samples: u32,
    pass: Pass,
) -> wgpu::RenderPipeline {
    let shadow = matches!(pass, Pass::Shadow);
    let blend = matches!(pass, Pass::Blend);
    let fragment = match format {
        wgpu::TextureFormat::R32Uint => "object_id",
        wgpu::TextureFormat::R32Float => "linear_depth",
        wgpu::TextureFormat::Rgba32Float => "world_normal",
        _ => "fragment",
    };
    let data = fragment != "fragment";
    let standard = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("scene3d_material_standard"),
        entries: &material_bindings(data || shadow),
    });
    let groups = [Some(&standard), extension, vertex_extension];
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("scene3d_material_program"),
        bind_group_layouts: &groups[..if vertex_extension.is_some() {
            3
        } else if extension.is_some() {
            2
        } else {
            1
        }],
        immediate_size: 0,
    });
    let target = Some(wgpu::ColorTargetState {
        format: if data {
            format
        } else {
            wgpu::TextureFormat::Rgba16Float
        },
        blend: blend.then_some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
        write_mask: wgpu::ColorWrites::ALL,
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("scene3d_material_program"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some(if shadow { "shadow_vertex" } else { "vertex" }),
            compilation_options: Default::default(),
            buffers: &[Some(Vertex::layout()), Some(Instance::layout())],
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some(if shadow { "shadow_fragment" } else { fragment }),
            compilation_options: Default::default(),
            targets: if shadow {
                &[]
            } else {
                std::slice::from_ref(&target)
            },
        }),
        primitive: Default::default(),
        depth_stencil: Some(wgpu::DepthStencilState {
            format: wgpu::TextureFormat::Depth32Float,
            depth_write_enabled: Some(!blend),
            depth_compare: Some(wgpu::CompareFunction::Less),
            stencil: Default::default(),
            bias: Default::default(),
        }),
        multisample: wgpu::MultisampleState {
            count: if shadow { 1 } else { samples },
            ..Default::default()
        },
        multiview_mask: None,
        cache: None,
    })
}

#[cfg(not(target_family = "wasm"))]
impl Pipelines {
    fn new(
        device: &wgpu::Device,
        source: &Scene3dMaterialSource,
        format: wgpu::TextureFormat,
        samples: u32,
    ) -> Result<Self> {
        let data = matches!(
            format,
            wgpu::TextureFormat::R32Uint
                | wgpu::TextureFormat::R32Float
                | wgpu::TextureFormat::Rgba32Float
        );
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let create = |pass| {
            create_pipeline(
                device,
                source.shader(),
                Some(source.layout()),
                source.vertex_layout(),
                format,
                samples,
                pass,
            )
        };
        let mesh = create(Pass::Opaque);
        let blend = (!data).then(|| create(Pass::Blend));
        let shadow = (!data).then(|| create(Pass::Shadow));
        if let Some(error) = gpui::block_on(scope.pop()) {
            anyhow::bail!("3D material pipeline: {error}");
        }
        Ok(Self {
            _source: source.clone(),
            mesh,
            blend,
            shadow,
        })
    }
}
