use super::{
    GpuDeformationLimits, GpuDeformationOutput, GpuTangentFramesOutput, buffer, validate_storage,
};
use crate::{Mesh, TangentGenerationMode};
use anyhow::{Result, ensure};
use bytemuck::{Pod, Zeroable};
use gpui_wgpu::{WgpuContext, wgpu};
use std::sync::Arc;

#[cfg(test)]
mod tests;

const SHADER: &str = concat!(
    include_str!("precision.wgsl"),
    include_str!("tangents.wgsl")
);

/// Payload admission for tangent publication. CPU mesh preparation, input snapshots,
/// pipeline storage and driver overhead are excluded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GpuTangentsMemory {
    pub topology_bytes: u64,
    pub uniform_bytes: u64,
    pub vertex_bytes: u64,
    pub repair_bytes: u64,
}
impl GpuTangentsMemory {
    pub fn plan(vertices: usize, limits: GpuDeformationLimits) -> Result<Self> {
        ensure!(
            vertices > 0 && vertices <= u32::MAX as usize && vertices.is_multiple_of(3),
            "GPU tangent publication requires a positive u32 triangle-corner count"
        );
        let memory = Self {
            topology_bytes: vertices as u64 * 16,
            uniform_bytes: 16,
            vertex_bytes: vertices as u64 * 64,
            repair_bytes: vertices as u64 * 4,
        };
        ensure!(
            memory.topology_bytes + memory.uniform_bytes <= limits.max_source_bytes,
            "GPU tangent topology exceeds source payload budget"
        );
        ensure!(
            memory.vertex_bytes + memory.repair_bytes <= limits.max_output_bytes,
            "GPU tangent vertices and repair tags exceed output payload budget"
        );
        Ok(memory)
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Topology {
    vertex: u32,
    zero_uv: u32,
    uv: [f32; 2],
}

/// Publishes corner frames into fixed-order deformation vertices. Every vertex
/// must occur exactly once in the index buffer. Prepare shared-vertex splits and
/// remap external attributes before constructing this source.
///
/// Construction generates initial CPU tangents using the selected policy and
/// retains a render mesh with their coordinate metadata. Evaluation preserves
/// source vertex order and publishes that mesh identity without CPU vertex readback.
/// Requires device-enabled `SHADER_F64` for projection and geometric classification.
pub struct GpuTangents {
    context: WgpuContext,
    base: Mesh,
    output_mesh: Mesh,
    uv_set: u32,
    memory: GpuTangentsMemory,
    topology: wgpu::Buffer,
    params: wgpu::Buffer,
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::ComputePipeline,
}
impl GpuTangents {
    pub fn check_support(capabilities: &gpui_wgpu::Scene3dDeviceCapabilities) -> Result<()> {
        ensure!(
            capabilities
                .enabled_features
                .contains(wgpu::Features::SHADER_F64),
            "GPU tangent publication requires enabled SHADER_F64"
        );
        super::support::validate(capabilities, 5, 1, 0)
    }
    pub fn new(
        context: WgpuContext,
        base: Mesh,
        uv_set: u32,
        mode: TangentGenerationMode,
        limits: GpuDeformationLimits,
    ) -> Result<Self> {
        ensure!(!context.device_lost(), "GPU deformation device is lost");
        Self::check_support(&gpui_wgpu::Scene3dDeviceCapabilities::query(&context))?;
        let memory = GpuTangentsMemory::plan(base.vertex_count(), limits)?;
        validate_storage(
            &context.device.limits(),
            &[
                memory.topology_bytes,
                memory.vertex_bytes,
                memory.repair_bytes,
            ],
            base.vertex_count() / 3,
        )?;
        let (topology, output_mesh) = prepare(&base, uv_set, mode)?;
        let device = &context.device;
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let entries: Vec<_> = [64, 64, 16, 64, 4, 16]
            .into_iter()
            .enumerate()
            .map(|(binding, minimum)| wgpu::BindGroupLayoutEntry {
                binding: binding as u32,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: if binding == 5 {
                        wgpu::BufferBindingType::Uniform
                    } else {
                        wgpu::BufferBindingType::Storage {
                            read_only: binding < 3,
                        }
                    },
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(minimum),
                },
                count: None,
            })
            .collect();
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("gpui_3d.tangents.inputs"),
            entries: &entries,
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("gpui_3d.tangents.layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("gpui_3d.tangents.shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("gpui_3d.tangents"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("publish"),
            compilation_options: Default::default(),
            cache: None,
        });
        let topology = buffer(
            device,
            "gpui_3d.tangents.topology",
            bytemuck::cast_slice(&topology),
            wgpu::BufferUsages::STORAGE,
        );
        let mode = match mode {
            TangentGenerationMode::Strict => 0u32,
            TangentGenerationMode::Inherit => 1,
            TangentGenerationMode::Repair => 2,
        };
        let params = buffer(
            device,
            "gpui_3d.tangents.params",
            bytemuck::cast_slice(&[base.vertex_count() as u32, mode, 0, 0]),
            wgpu::BufferUsages::UNIFORM,
        );
        if let Some(error) = gpui::block_on(scope.pop()) {
            anyhow::bail!("GPU tangent preparation: {error}");
        }
        Ok(Self {
            context,
            base,
            output_mesh,
            uv_set,
            memory,
            topology,
            params,
            layout,
            pipeline,
        })
    }
    pub fn memory(&self) -> GpuTangentsMemory {
        self.memory
    }
    /// Initial CPU source carrying tangent metadata, not the evaluated GPU geometry.
    pub fn output_mesh(&self) -> &Mesh {
        &self.output_mesh
    }

    /// Uses the frame output's retained input vertices. Input failures and incompatible
    /// triangle signs remain failures in every mode. Repair tags do not affect draw status.
    pub fn evaluate(&self, frames: &GpuTangentFramesOutput) -> Result<GpuTangentsOutput> {
        let faces = frames.groups().adjacency().weld().derivatives();
        ensure!(
            !self.context.device_lost(),
            "GPU deformation device is lost"
        );
        ensure!(
            Arc::ptr_eq(&self.context.device, &faces.context().device),
            "GPU tangent input belongs to a different device"
        );
        ensure!(
            Arc::ptr_eq(&self.base.0, &faces.base_mesh().0),
            "GPU tangent source mesh mismatch"
        );
        ensure!(
            self.uv_set == faces.uv_set(),
            "GPU tangent coordinate set mismatch"
        );
        let device = &self.context.device;
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let allocate = |size, label, usage| {
            self.context.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC | usage,
                mapped_at_creation: false,
            })
        };
        let output = allocate(
            self.memory.vertex_bytes,
            "gpui_3d.tangents.vertices",
            wgpu::BufferUsages::VERTEX,
        );
        let repairs = allocate(
            self.memory.repair_bytes,
            "gpui_3d.tangents.repairs",
            wgpu::BufferUsages::empty(),
        );
        let bindings = [
            faces.input_buffer(),
            frames.buffer(),
            &self.topology,
            &output,
            &repairs,
            &self.params,
        ];
        let entries: Vec<_> = bindings
            .into_iter()
            .enumerate()
            .map(|(binding, buffer)| wgpu::BindGroupEntry {
                binding: binding as u32,
                resource: buffer.as_entire_binding(),
            })
            .collect();
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("gpui_3d.tangents.inputs"),
            layout: &self.layout,
            entries: &entries,
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("gpui_3d.tangents"),
        });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("gpui_3d.tangents"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.dispatch_workgroups((self.base.vertex_count() as u32 / 3).div_ceil(64), 1, 1);
        }
        self.context.queue.submit(Some(encoder.finish()));
        if let Some(error) = gpui::block_on(scope.pop()) {
            anyhow::bail!("GPU tangent publication: {error}");
        }
        Ok(GpuTangentsOutput {
            deformation: GpuDeformationOutput {
                context: self.context.clone(),
                base: self.output_mesh.clone(),
                buffer: output,
            },
            repairs: repairs.raw().clone(),
        })
    }
}

/// Published tangent vertices and matching per-vertex repair tags.
pub struct GpuTangentsOutput {
    deformation: GpuDeformationOutput,
    repairs: wgpu::Buffer,
}
impl GpuTangentsOutput {
    pub fn deformation(&self) -> &GpuDeformationOutput {
        &self.deformation
    }
    pub fn into_deformation(self) -> GpuDeformationOutput {
        self.deformation
    }
    /// Read-only u32 per source vertex: 0 unchanged/generated, 1 triangle derivative,
    /// 2 normal-orthogonal basis. Failed vertices have tag 0 and nonzero vertex status.
    pub fn repair_buffer(&self) -> &wgpu::Buffer {
        &self.repairs
    }
}

fn prepare(base: &Mesh, set: u32, mode: TangentGenerationMode) -> Result<(Vec<Topology>, Mesh)> {
    ensure!(
        base.vertex_count() == base.index_count(),
        "GPU tangents require one index per source vertex"
    );
    let mut topology = base
        .indices()
        .iter()
        .map(|&vertex| {
            Ok(Topology {
                vertex,
                zero_uv: 0,
                uv: base
                    .uv_at(set, vertex as usize)
                    .ok_or_else(|| anyhow::anyhow!("missing UV set {set}"))?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    for triangle in topology.chunks_exact_mut(3) {
        let [a, b, c] = std::array::from_fn::<_, 3, _>(|i| triangle[i].uv.map(f64::from));
        let u = [b[0] - a[0], b[1] - a[1]];
        let v = [c[0] - a[0], c[1] - a[1]];
        let zero_uv = u32::from(u[0] * v[1] - u[1] * v[0] == 0.);
        for corner in triangle {
            corner.zero_uv = zero_uv;
        }
    }
    let mut used = vec![false; base.vertex_count()];
    for &vertex in base.indices() {
        ensure!(
            !used[vertex as usize],
            "GPU tangents require unshared triangle corners; vertex {vertex} is repeated"
        );
        used[vertex as usize] = true;
    }
    let initial = base.generate_tangents_for_uv_set(set, mode)?;
    let mut tangents = vec![[0.; 4]; base.vertex_count()];
    for (&vertex, &tangent) in initial
        .source_vertices()
        .iter()
        .zip(initial.mesh().tangents().unwrap())
    {
        tangents[vertex as usize] = tangent;
    }
    Ok((topology, base.with_tangents_for_uv_set(set, tangents)?))
}
