use anyhow::{Result, ensure};
use wgpu::naga::{self, Module, TypeInner};

/// CPU admission limits for material source and declared resource layouts.
#[derive(Clone, Copy, Debug)]
pub struct Scene3dMaterialLimits {
    pub max_source_bytes: usize,
    pub max_resources: usize,
    /// Maximum declared custom vertex streams; enabled device limits are checked separately.
    pub max_vertex_attributes: usize,
    /// Sum of minimum binding sizes across all declared uniform blocks.
    pub max_uniform_bytes: u64,
}
impl Default for Scene3dMaterialLimits {
    fn default() -> Self {
        Self {
            max_source_bytes: 64 * 1024,
            max_resources: 16,
            max_vertex_attributes: 16,
            max_uniform_bytes: 64 * 1024,
        }
    }
}

/// Resource type for a material's group 1 binding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scene3dMaterialResourceKind {
    Uniform {
        min_size: u64,
    },
    /// Non-multisampled float texture; the bound view must support filtering.
    Texture {
        dimension: wgpu::TextureViewDimension,
    },
    /// Non-comparison sampler, with filtering allowed.
    Sampler,
}

/// Reflected resource declaration. Unused declarations still require bindings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scene3dMaterialResource {
    pub name: String,
    pub binding: u32,
    pub kind: Scene3dMaterialResourceKind,
    /// The surface evaluator or one of its helpers accesses this resource.
    pub coverage: bool,
    /// The color-shading evaluator or one of its helpers accesses this resource.
    pub shading: bool,
}
impl Scene3dMaterialResource {
    /// All output passes use the same fragment-visible group 1 layout.
    pub fn layout_entry(&self) -> wgpu::BindGroupLayoutEntry {
        wgpu::BindGroupLayoutEntry {
            binding: self.binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: match self.kind {
                Scene3dMaterialResourceKind::Uniform { min_size } => wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(min_size),
                },
                Scene3dMaterialResourceKind::Texture { dimension } => wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: dimension,
                    multisampled: false,
                },
                Scene3dMaterialResourceKind::Sampler => {
                    wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering)
                }
            },
            count: None,
        }
    }
}

pub(super) fn reflect(
    module: &Module,
    info: &naga::valid::ModuleInfo,
    limits: Scene3dMaterialLimits,
    core_len: usize,
) -> Result<Vec<Scene3dMaterialResource>> {
    let surface = super::named(module, "material_surface")?;
    let shading = super::named(module, "material_shading")?;
    let mut resources = Vec::new();
    let mut uniform_bytes = 0u64;
    for (handle, global) in module.global_variables.iter() {
        if module
            .global_variables
            .get_span(handle)
            .to_range()
            .is_some_and(|s| s.start < core_len)
        {
            continue;
        }
        let binding = global
            .binding
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("material globals require group 1 resource bindings"))?;
        ensure!(
            binding.group == 1,
            "material globals must use resource group 1"
        );
        let kind = match (&module.types[global.ty].inner, global.space) {
            (TypeInner::Struct { span, .. }, naga::AddressSpace::Uniform) => {
                uniform_bytes = uniform_bytes
                    .checked_add(u64::from(*span))
                    .ok_or_else(|| anyhow::anyhow!("material uniform byte overflow"))?;
                ensure!(
                    uniform_bytes <= limits.max_uniform_bytes,
                    "material uniform blocks exceed byte limit"
                );
                Scene3dMaterialResourceKind::Uniform {
                    min_size: u64::from(*span),
                }
            }
            (
                TypeInner::Image {
                    dim,
                    arrayed: false,
                    class:
                        naga::ImageClass::Sampled {
                            kind: naga::ScalarKind::Float,
                            multi: false,
                        },
                },
                naga::AddressSpace::Handle,
            ) => Scene3dMaterialResourceKind::Texture {
                dimension: match dim {
                    naga::ImageDimension::D2 => wgpu::TextureViewDimension::D2,
                    naga::ImageDimension::Cube => wgpu::TextureViewDimension::Cube,
                    _ => anyhow::bail!("material textures must be 2D or cube views"),
                },
            },
            (TypeInner::Sampler { comparison: false }, naga::AddressSpace::Handle) => {
                Scene3dMaterialResourceKind::Sampler
            }
            _ => anyhow::bail!("unsupported material resource {:?}", global.name),
        };
        ensure!(
            resources.len() < limits.max_resources,
            "material resource count exceeds limit"
        );
        resources.push(Scene3dMaterialResource {
            name: global.name.clone().unwrap_or_default(),
            binding: binding.binding,
            kind,
            coverage: !info[surface][handle].is_empty(),
            shading: !info[shading][handle].is_empty(),
        });
    }
    resources.sort_by_key(|resource| resource.binding);
    ensure!(
        !resources
            .windows(2)
            .any(|pair| pair[0].binding == pair[1].binding),
        "duplicate material resource binding"
    );
    Ok(resources)
}

pub(super) fn validate_limits(
    resources: &[Scene3dMaterialResource],
    limits: &wgpu::Limits,
) -> Result<()> {
    crate::wgpu_renderer::scene3d::validate_device_limits(limits)?;
    let mut bindings = crate::wgpu_renderer::scene3d::material_bindings(false);
    bindings.extend(resources.iter().map(Scene3dMaterialResource::layout_entry));
    let count = |predicate: fn(&wgpu::BindGroupLayoutEntry) -> bool| {
        bindings.iter().filter(|entry| predicate(entry)).count() as u64
    };
    for (name, available, required) in [
        (
            "bind groups",
            u64::from(limits.max_bind_groups),
            if resources.is_empty() { 1 } else { 2 },
        ),
        (
            "binding index",
            u64::from(limits.max_bindings_per_bind_group),
            resources
                .iter()
                .map(|r| u64::from(r.binding) + 1)
                .max()
                .unwrap_or(0),
        ),
        (
            "sampled textures",
            u64::from(limits.max_sampled_textures_per_shader_stage),
            count(|e| matches!(e.ty, wgpu::BindingType::Texture { .. })),
        ),
        (
            "samplers",
            u64::from(limits.max_samplers_per_shader_stage),
            count(|e| matches!(e.ty, wgpu::BindingType::Sampler(_))),
        ),
        (
            "uniform blocks",
            u64::from(limits.max_uniform_buffers_per_shader_stage),
            count(|e| matches!(e.ty, wgpu::BindingType::Buffer { .. })),
        ),
    ] {
        ensure!(
            required <= available,
            "material {name} require {required}, device enables {available}"
        );
    }
    for resource in resources {
        if let Scene3dMaterialResourceKind::Uniform { min_size } = resource.kind {
            ensure!(
                min_size <= limits.max_uniform_buffer_binding_size
                    && min_size <= limits.max_buffer_size,
                "material uniform {} exceeds device buffer limits",
                resource.name
            );
        }
    }
    Ok(())
}
