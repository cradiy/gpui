use std::sync::Arc;

use anyhow::{Result, ensure};
use gpui::Mesh3d;
use wgpu::util::DeviceExt as _;

use crate::{WgpuContext, wgpu_renderer::scene3d::Vertex};

mod attributes;
#[cfg(test)]
mod tests;
pub use attributes::Scene3dVertexUpdate;

fn validate_support(
    limits: &wgpu::Limits,
    adapter: &wgpu::Limits,
    flags: wgpu::DownlevelFlags,
) -> Result<()> {
    for flag in [
        wgpu::DownlevelFlags::COMPUTE_SHADERS,
        wgpu::DownlevelFlags::INDIRECT_EXECUTION,
    ] {
        ensure!(flags.contains(flag), "GPU geometry requires {flag:?}");
    }
    macro_rules! require {
        ($field:ident, $minimum:expr) => {
            ensure!(
                limits.$field >= $minimum,
                "GPU geometry requires {} >= {}; device enabled {}, adapter supports {}",
                stringify!($field),
                $minimum,
                limits.$field,
                adapter.$field
            );
        };
    }
    require!(max_storage_buffers_per_shader_stage, 5);
    require!(max_bind_groups, 1);
    require!(max_bindings_per_bind_group, 5);
    require!(max_buffers_and_acceleration_structures_per_shader_stage, 5);
    require!(max_storage_buffer_binding_size, 64);
    require!(max_buffer_size, 64);
    require!(max_compute_invocations_per_workgroup, 64);
    require!(max_compute_workgroup_size_x, 64);
    require!(max_compute_workgroup_size_y, 1);
    require!(max_compute_workgroup_size_z, 1);
    require!(max_compute_workgroups_per_dimension, 1);
    Ok(())
}

/// One source and one packed result, excluding external attribute buffers and driver overhead.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Scene3dGpuGeometryMemory {
    pub source_vertex_bytes: u64,
    pub index_bytes: u64,
    pub vertex_bytes: u64,
    pub draw_bytes: u64,
    pub total_bytes: u64,
}

impl Scene3dGpuGeometryMemory {
    pub fn plan(vertices: usize, indices: usize) -> Result<Self> {
        ensure!(
            vertices > 0 && vertices <= u32::MAX as usize / 24,
            "GPU geometry vertex addressing exceeds u32"
        );
        ensure!(
            indices > 0 && indices <= u32::MAX as usize && indices.is_multiple_of(3),
            "GPU geometry requires a u32-sized triangle index list"
        );
        let vertex_bytes = vertices as u64 * std::mem::size_of::<Vertex>() as u64;
        let index_bytes = indices as u64 * 4;
        Ok(Self {
            source_vertex_bytes: vertex_bytes,
            index_bytes,
            vertex_bytes,
            draw_bytes: 20,
            total_bytes: vertex_bytes * 2 + index_bytes + 20,
        })
    }

    fn validate(
        self,
        limits: &wgpu::Limits,
        vertices: usize,
        indices: usize,
        byte_limit: Option<u64>,
    ) -> Result<()> {
        for bytes in [
            self.source_vertex_bytes,
            self.index_bytes,
            self.vertex_bytes,
            self.draw_bytes,
            vertices as u64 * 64,
        ] {
            ensure!(
                bytes <= limits.max_buffer_size && bytes <= limits.max_storage_buffer_binding_size,
                "GPU geometry buffer exceeds device limits"
            );
        }
        ensure!(
            vertices.max(indices / 3).div_ceil(64) as u64
                <= u64::from(limits.max_compute_workgroups_per_dimension),
            "GPU geometry dispatch exceeds device limits"
        );
        ensure!(
            byte_limit.is_none_or(|limit| self.total_bytes <= limit),
            "GPU geometry exceeds payload budget"
        );
        Ok(())
    }
}

/// Reusable material-coordinate and index inputs for GPU vertex packing on one device.
#[derive(Clone)]
pub struct WgpuScene3dGeometry {
    context: WgpuContext,
    mesh: Arc<Mesh3d>,
    uv_sets: [u32; 5],
    memory: Scene3dGpuGeometryMemory,
    source: wgpu::Buffer,
    indices: wgpu::Buffer,
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::ComputePipeline,
    attribute_kernel: Arc<parking_lot::Mutex<Option<attributes::AttributeKernel>>>,
}

impl WgpuScene3dGeometry {
    /// Checks enabled packing and indirect-draw support without creating resources.
    /// Mesh size, payload admission, device health, and render-target support are separate checks.
    pub fn check_support(capabilities: &super::Scene3dDeviceCapabilities) -> Result<()> {
        validate_support(
            &capabilities.limits,
            &capabilities.adapter_limits,
            capabilities.downlevel.flags,
        )
    }

    /// Coordinate sets select base, metallic/roughness, emission, normal, and occlusion UVs.
    /// The byte limit admits this source plus one result; retained results are additional.
    pub fn new(
        context: WgpuContext,
        mesh: Arc<Mesh3d>,
        uv_sets: [u32; 5],
        byte_limit: Option<u64>,
    ) -> Result<Self> {
        ensure!(!context.device_lost(), "GPU geometry device is lost");
        Self::check_support(&super::Scene3dDeviceCapabilities::query(&context))?;
        for set in uv_sets {
            ensure!(
                mesh.uv_at(set, 0).is_some(),
                "GPU geometry missing UV set {set}"
            );
        }
        let memory = Scene3dGpuGeometryMemory::plan(mesh.vertices().len(), mesh.indices().len())?;
        let device = &context.device;
        let limits = device.limits();
        memory.validate(
            &limits,
            mesh.vertices().len(),
            mesh.indices().len(),
            byte_limit,
        )?;
        let vertices: Vec<_> = (0..mesh.vertices().len())
            .map(|index| Vertex::new(&mesh, index, uv_sets))
            .collect();
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let source = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("scene3d.geometry.source"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        });
        let indices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("scene3d.geometry.indices"),
            contents: bytemuck::cast_slice(mesh.indices()),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::INDEX,
        });
        let entries: Vec<_> = [4, 64, 4, 4, 20]
            .into_iter()
            .enumerate()
            .map(|(binding, minimum)| wgpu::BindGroupLayoutEntry {
                binding: binding as u32,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage {
                        read_only: binding < 3,
                    },
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(minimum),
                },
                count: None,
            })
            .collect();
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("scene3d.geometry.inputs"),
            entries: &entries,
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("scene3d.geometry.layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("scene3d.geometry.shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("gpu_geometry.wgsl").into()),
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("scene3d.geometry.pack"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("pack"),
            compilation_options: Default::default(),
            cache: None,
        });
        if let Some(error) = gpui::block_on(scope.pop()) {
            anyhow::bail!("GPU geometry preparation: {error}");
        }
        Ok(Self {
            context,
            mesh,
            uv_sets,
            memory,
            source,
            indices,
            layout,
            pipeline,
            attribute_kernel: Arc::new(parking_lot::Mutex::new(None)),
        })
    }

    pub fn context(&self) -> &WgpuContext {
        &self.context
    }
    pub fn base_mesh(&self) -> &Arc<Mesh3d> {
        &self.mesh
    }
    pub fn uv_sets(&self) -> [u32; 5] {
        self.uv_sets
    }
    pub fn memory(&self) -> Scene3dGpuGeometryMemory {
        self.memory
    }

    /// Consumes 64-byte records: position/normal/tangent `vec4<f32>`, then status `vec4<u32>`.
    /// The buffer must belong to this device and remain immutable. Nonzero status,
    /// nonfinite attributes, or inconsistent triangle tangent signs disable the entire draw.
    /// No CPU geometry or bounds are updated and no GPU readback is performed.
    pub fn evaluate(&self, attributes: &wgpu::Buffer) -> Result<Scene3dGpuGeometry> {
        ensure!(!self.context.device_lost(), "GPU geometry device is lost");
        ensure!(
            attributes.size() == self.mesh.vertices().len() as u64 * 64,
            "GPU geometry attribute count mismatch"
        );
        ensure!(
            attributes.usage().contains(wgpu::BufferUsages::STORAGE),
            "GPU geometry attributes require STORAGE usage"
        );
        let device = &self.context.device;
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let vertices = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("scene3d.geometry.vertices"),
            size: self.memory.vertex_bytes,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::VERTEX
                | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let draw = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("scene3d.geometry.draw"),
            contents: bytemuck::cast_slice(&[self.mesh.indices().len() as u32, 1, 0, 0, 0]),
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::INDIRECT
                | wgpu::BufferUsages::COPY_SRC,
        });
        let entries: Vec<_> = [&self.source, attributes, &self.indices, &vertices, &draw]
            .into_iter()
            .enumerate()
            .map(|(binding, buffer)| wgpu::BindGroupEntry {
                binding: binding as u32,
                resource: buffer.as_entire_binding(),
            })
            .collect();
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("scene3d.geometry.bind"),
            layout: &self.layout,
            entries: &entries,
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("scene3d.geometry"),
        });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("scene3d.geometry"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &bind, &[]);
            pass.dispatch_workgroups(
                self.mesh
                    .vertices()
                    .len()
                    .max(self.mesh.indices().len() / 3)
                    .div_ceil(64) as u32,
                1,
                1,
            );
        }
        self.context.queue.submit([encoder.finish()]);
        if let Some(error) = gpui::block_on(scope.pop()) {
            anyhow::bail!("GPU geometry submission: {error}");
        }
        Ok(Scene3dGpuGeometry {
            context: self.context.clone(),
            mesh: self.mesh.clone(),
            uv_sets: self.uv_sets,
            vertices,
            indices: self.indices.clone(),
            draw,
            memory: self.memory,
        })
    }
}

/// Owned render-ready buffers. Bind vertices and indices, then draw_indexed_indirect at offset zero.
/// CPU metadata retains bind-space geometry, not the GPU-deformed positions or bounds.
pub struct Scene3dGpuGeometry {
    context: WgpuContext,
    mesh: Arc<Mesh3d>,
    uv_sets: [u32; 5],
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    draw: wgpu::Buffer,
    memory: Scene3dGpuGeometryMemory,
}
impl Scene3dGpuGeometry {
    pub fn context(&self) -> &WgpuContext {
        &self.context
    }
    pub fn base_mesh(&self) -> &Arc<Mesh3d> {
        &self.mesh
    }
    pub fn uv_sets(&self) -> [u32; 5] {
        self.uv_sets
    }
    pub fn memory(&self) -> Scene3dGpuGeometryMemory {
        self.memory
    }
    pub fn vertices(&self) -> &wgpu::Buffer {
        &self.vertices
    }
    pub fn indices(&self) -> &wgpu::Buffer {
        &self.indices
    }
    pub fn draw(&self) -> &wgpu::Buffer {
        &self.draw
    }
    /// Uses the same attribute locations and stride as the mesh renderer.
    pub fn vertex_layout() -> wgpu::VertexBufferLayout<'static> {
        Vertex::layout()
    }
}
