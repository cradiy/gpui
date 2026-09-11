use super::{MaterialProgram, Scene3dMaterialResourceKind};
use crate::WgpuContext;
use anyhow::{Result, ensure};
use std::sync::Arc;
use wgpu::util::DeviceExt;

mod plan;
#[cfg(test)]
mod tests;

/// Caller-owned inputs for one reflected group 1 binding.
#[derive(Clone, Debug)]
pub enum Scene3dMaterialValue {
    /// Exact WGSL uniform struct bytes, including padding.
    Uniform(Arc<[u8]>),
    /// Retains the view and its texture, without copying pixels.
    Texture(wgpu::TextureView),
    Sampler(wgpu::Sampler),
}

/// Payload budget for one complete binding snapshot, including shared uniforms.
#[derive(Clone, Copy, Debug)]
pub struct Scene3dMaterialBindingLimits {
    pub max_uniform_bytes: u64,
}
impl Default for Scene3dMaterialBindingLimits {
    fn default() -> Self {
        Self {
            max_uniform_bytes: 64 * 1024,
        }
    }
}

struct Source {
    context: WgpuContext,
    program: MaterialProgram,
    shader: wgpu::ShaderModule,
    layout: wgpu::BindGroupLayout,
}

/// Device-local material shader and reusable group 1 layout.
/// Recreate after device replacement. Draw pipelines are prepared by scene renderers.
#[derive(Clone)]
pub struct Scene3dMaterialSource(Arc<Source>);
impl Scene3dMaterialSource {
    pub(crate) fn identity(&self) -> usize {
        Arc::as_ptr(&self.0) as usize
    }

    pub fn new(context: WgpuContext, program: MaterialProgram) -> Result<Self> {
        ensure!(!context.device_lost(), "material device is lost");
        program.validate_limits(&context.device.limits())?;
        let scope = context
            .device
            .push_error_scope(wgpu::ErrorFilter::Validation);
        let shader = context
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("gpui_3d.material.program"),
                source: wgpu::ShaderSource::Wgsl(program.source().into()),
            });
        let entries: Vec<_> = program
            .resources()
            .iter()
            .map(|r| r.layout_entry())
            .collect();
        let layout = context
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("gpui_3d.material.layout"),
                entries: &entries,
            });
        if let Some(error) = gpui::block_on(scope.pop()) {
            anyhow::bail!("material preparation: {error}");
        }
        ensure!(!context.device_lost(), "material device is lost");
        Ok(Self(Arc::new(Source {
            context,
            program,
            shader,
            layout,
        })))
    }
    pub fn context(&self) -> &WgpuContext {
        &self.0.context
    }
    pub fn program(&self) -> &MaterialProgram {
        &self.0.program
    }
    pub fn shader(&self) -> &wgpu::ShaderModule {
        &self.0.shader
    }
    pub fn layout(&self) -> &wgpu::BindGroupLayout {
        &self.0.layout
    }

    /// Binds every declared resource exactly once. Input order is arbitrary.
    /// WGPU validates view dimensions/formats, sampler kinds, usage and device ownership.
    pub fn bind(
        &self,
        values: impl IntoIterator<Item = (u32, Scene3dMaterialValue)>,
        limits: Scene3dMaterialBindingLimits,
    ) -> Result<Scene3dMaterialSnapshot> {
        self.build(values, limits, None)
    }

    fn build(
        &self,
        values: impl IntoIterator<Item = (u32, Scene3dMaterialValue)>,
        limits: Scene3dMaterialBindingLimits,
        previous: Option<&Scene3dMaterialSnapshot>,
    ) -> Result<Scene3dMaterialSnapshot> {
        ensure!(!self.context().device_lost(), "material device is lost");
        let resources = self.program().resources();
        let mut supplied = Vec::new();
        for value in values {
            ensure!(
                supplied.len() < resources.len(),
                "too many material bindings"
            );
            supplied.push(value);
        }
        let (mapping, uniform_bytes) = plan::resolve(
            resources,
            supplied.iter().map(|(binding, value)| {
                (
                    *binding,
                    match value {
                        Scene3dMaterialValue::Uniform(bytes) => {
                            plan::Input::Uniform(bytes.len() as u64)
                        }
                        Scene3dMaterialValue::Texture(_) => plan::Input::Texture,
                        Scene3dMaterialValue::Sampler(_) => plan::Input::Sampler,
                    },
                )
            }),
            previous.is_some(),
            limits,
        )?;
        if supplied.is_empty() {
            if let Some(previous) = previous {
                return Ok(previous.clone());
            }
        }
        let device = &self.context().device;
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let slots: Vec<_> = mapping
            .into_iter()
            .enumerate()
            .map(|(index, value)| match value {
                Some(value) => match &supplied[value].1 {
                    Scene3dMaterialValue::Uniform(bytes) => BoundValue::Uniform {
                        buffer: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                            label: Some("gpui_3d.material.uniform"),
                            contents: bytes,
                            usage: wgpu::BufferUsages::UNIFORM,
                        }),
                    },
                    Scene3dMaterialValue::Texture(view) => BoundValue::Texture(view.clone()),
                    Scene3dMaterialValue::Sampler(sampler) => BoundValue::Sampler(sampler.clone()),
                },
                None => previous.expect("validated partial binding").0.slots[index].clone(),
            })
            .collect();
        let entries: Vec<_> = resources
            .iter()
            .zip(&slots)
            .map(|(resource, value)| wgpu::BindGroupEntry {
                binding: resource.binding,
                resource: match value {
                    BoundValue::Uniform { buffer } => buffer.as_entire_binding(),
                    BoundValue::Texture(view) => wgpu::BindingResource::TextureView(view),
                    BoundValue::Sampler(sampler) => wgpu::BindingResource::Sampler(sampler),
                },
            })
            .collect();
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("gpui_3d.material.bindings"),
            layout: self.layout(),
            entries: &entries,
        });
        if let Some(error) = gpui::block_on(scope.pop()) {
            anyhow::bail!("material binding: {error}");
        }
        ensure!(!self.context().device_lost(), "material device is lost");
        Ok(Scene3dMaterialSnapshot(Arc::new(Snapshot {
            source: self.clone(),
            slots,
            bind_group,
            uniform_bytes,
        })))
    }
}

#[derive(Clone)]
enum BoundValue {
    Uniform { buffer: wgpu::Buffer },
    Texture(wgpu::TextureView),
    Sampler(wgpu::Sampler),
}
struct Snapshot {
    source: Scene3dMaterialSource,
    slots: Vec<BoundValue>,
    bind_group: wgpu::BindGroup,
    uniform_bytes: u64,
}

/// Retained bindings with private immutable uniform buffers. External texture
/// content is shared: callers must not mutate or destroy textures used by retained frames.
#[derive(Clone)]
pub struct Scene3dMaterialSnapshot(Arc<Snapshot>);
impl Scene3dMaterialSnapshot {
    pub fn source(&self) -> &Scene3dMaterialSource {
        &self.0.source
    }
    pub fn bind_group(&self) -> &wgpu::BindGroup {
        &self.0.bind_group
    }
    /// Full uniform payload represented by this snapshot; shared buffers count in each snapshot.
    pub fn uniform_bytes(&self) -> u64 {
        self.0.uniform_bytes
    }
    /// Replaces only named bindings, retaining all others without reuploading them.
    /// Failures leave this snapshot unchanged. The source shader and layout are reused.
    pub fn with_values(
        &self,
        values: impl IntoIterator<Item = (u32, Scene3dMaterialValue)>,
        limits: Scene3dMaterialBindingLimits,
    ) -> Result<Self> {
        self.source().build(values, limits, Some(self))
    }
}
