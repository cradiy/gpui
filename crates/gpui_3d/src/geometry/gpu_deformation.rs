use crate::Mesh;
use anyhow::{Result, ensure};
use bytemuck::{Pod, Zeroable};
use gpui_wgpu::{WgpuContext, wgpu};
use wgpu::util::DeviceExt as _;

mod bounds;
mod external;
mod flat_normals;
mod preparation;
mod readback;
mod smooth_normals;
pub(super) mod support;
mod tangent_adjacency;
mod tangent_derivatives;
mod tangent_frames;
mod tangent_generation;
mod tangent_groups;
mod tangent_weld;
mod tangents;
pub use bounds::{GpuDeformationBounds, GpuDeformationBoundsReadback};
pub use flat_normals::{GpuFlatNormals, GpuFlatNormalsMemory};
pub use preparation::{GpuGeometryBatchPreparation, GpuGeometryPreparation, PreparedGpuGeometry};
pub use readback::GpuDeformationReadback;
pub use smooth_normals::{GpuSmoothNormals, GpuSmoothNormalsMemory};
pub use tangent_adjacency::{
    GpuTangentAdjacency, GpuTangentAdjacencyMemory, GpuTangentAdjacencyOutput, GpuTangentEdge,
};
pub use tangent_derivatives::{
    GpuTangentDerivative, GpuTangentDerivativeOutput, GpuTangentDerivatives,
    GpuTangentDerivativesMemory,
};
pub use tangent_frames::{
    GpuTangentFrame, GpuTangentFrames, GpuTangentFramesMemory, GpuTangentFramesOutput,
};
pub use tangent_generation::{GpuTangentGeneration, GpuTangentGenerationMemory};
pub use tangent_groups::{
    GpuTangentGroup, GpuTangentGroups, GpuTangentGroupsMemory, GpuTangentGroupsOutput,
};
pub use tangent_weld::{
    GpuTangentWeld, GpuTangentWeldMemory, GpuTangentWeldOutput, GpuTangentWeldRecord,
};
pub use tangents::{GpuTangents, GpuTangentsMemory, GpuTangentsOutput};

/// 64-byte storage/vertex-buffer record. XYZ occupies each attribute's first three lanes.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct GpuDeformationVertex {
    pub position: [f32; 4],
    pub normal: [f32; 4],
    /// W is tangent handedness; zero when the mesh has no tangents.
    pub tangent: [f32; 4],
    /// X: zero for valid output, one for detected nonfinite arithmetic, two for an undefined
    /// tangent, three for a singular or unrepresentable blended Skin transform,
    /// four for a rejected zero-area triangle during normal or tangent generation,
    /// five for tangent input outside the supported numeric range.
    /// Remaining lanes are reserved and zero.
    pub status: [u32; 4],
}

/// Per-source and per-result payload limits, not a quota across all retained objects.
#[derive(Clone, Copy, Debug)]
pub struct GpuDeformationLimits {
    pub max_source_bytes: u64,
    /// Also bounds a readback staging buffer for one output.
    pub max_output_bytes: u64,
}
impl Default for GpuDeformationLimits {
    fn default() -> Self {
        Self {
            max_source_bytes: 256 * 1024 * 1024,
            max_output_bytes: 64 * 1024 * 1024,
        }
    }
}

/// Retained GPU attributes. CPU geometry, bounds, and picking remain unchanged until readback.
/// Clones share the immutable vertex buffer rather than copying its contents.
#[derive(Clone)]
pub struct GpuDeformationOutput {
    pub(super) context: WgpuContext,
    pub(super) base: Mesh,
    pub(super) buffer: gpui_wgpu::WgpuResource<wgpu::Buffer>,
}
impl GpuDeformationOutput {
    /// Device and queue shared by producers and downstream consumers.
    pub fn context(&self) -> &WgpuContext {
        &self.context
    }

    /// Uploads reusable UV/color/index inputs for render vertex packing.
    /// Sets are base, metallic/roughness, emission, normal, and occlusion coordinates.
    /// This does not attach the geometry to a Scene or Viewport.
    pub fn render_source(
        &self,
        uv_sets: [u32; 5],
        byte_limit: Option<u64>,
    ) -> Result<gpui_wgpu::WgpuScene3dGeometry> {
        gpui_wgpu::WgpuScene3dGeometry::new(
            self.context.clone(),
            self.base.0.clone(),
            uv_sets,
            byte_limit,
        )
    }

    /// Rebinds a same-device source to this output's base mesh and coordinate selection.
    /// Reuses packing kernels and shared index storage, with fresh base-mesh UV/colors.
    /// Earlier source updates are not inherited. The budget includes the complete source
    /// and one packed result, not only newly allocated storage.
    pub fn rebind_render_source(
        &self,
        source: &gpui_wgpu::WgpuScene3dGeometry,
        uv_sets: [u32; 5],
        byte_limit: Option<u64>,
    ) -> Result<gpui_wgpu::WgpuScene3dGeometry> {
        ensure!(
            std::sync::Arc::ptr_eq(&self.context.device, &source.context().device),
            "GPU render geometry belongs to a different device"
        );
        source.with_mesh(self.base.0.clone(), uv_sets, byte_limit)
    }

    /// Packs this output into render vertex/index/indirect buffers without CPU readback.
    /// The source must match the original CPU mesh allocation and device.
    pub fn render_geometry(
        &self,
        source: &gpui_wgpu::WgpuScene3dGeometry,
    ) -> Result<gpui_wgpu::Scene3dGpuGeometry> {
        self.validate_render_source(source)?;
        source.evaluate(&self.buffer)
    }

    fn validate_render_source(&self, source: &gpui_wgpu::WgpuScene3dGeometry) -> Result<()> {
        ensure!(
            !self.context.device_lost(),
            "GPU deformation device is lost"
        );
        ensure!(
            std::sync::Arc::ptr_eq(&self.context.device, &source.context().device),
            "GPU render geometry belongs to a different device"
        );
        ensure!(
            std::sync::Arc::ptr_eq(&self.base.0, source.base_mesh()),
            "GPU render geometry source mesh mismatch"
        );
        Ok(())
    }

    /// Uploads an immutable CPU mesh as input for GPU deformation.
    pub fn upload(context: WgpuContext, mesh: Mesh, limits: GpuDeformationLimits) -> Result<Self> {
        ensure!(!context.device_lost(), "GPU deformation device is lost");
        let bytes = mesh.vertex_count() as u64 * 64;
        ensure!(
            bytes <= limits.max_source_bytes && bytes <= limits.max_output_bytes,
            "GPU deformation mesh exceeds payload budget"
        );
        validate_storage(&context.device.limits(), &[bytes], mesh.vertex_count())?;
        let records = pack_mesh(&mesh);
        let scope = context
            .device
            .push_error_scope(wgpu::ErrorFilter::Validation);
        let buffer = context.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("gpui_3d.deformation.upload"),
            contents: bytemuck::cast_slice(&records),
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::VERTEX
                | wgpu::BufferUsages::COPY_SRC,
        });
        if let Some(error) = gpui::block_on(scope.pop()) {
            anyhow::bail!("GPU deformation upload: {error}");
        }
        Ok(Self {
            context,
            base: mesh,
            buffer,
        })
    }

    /// Read-only by contract; records use `GpuDeformationVertex` layout and source vertex order.
    /// External consumers must inspect status before using results. No indices/UVs are stored.
    /// The resource retains its creating device; `raw()` borrows the WGPU handle.
    pub fn buffer(&self) -> &gpui_wgpu::WgpuResource<wgpu::Buffer> {
        &self.buffer
    }
    pub fn base_mesh(&self) -> &Mesh {
        &self.base
    }

    /// Waits up to 30 seconds for device completion plus one second for the map callback,
    /// validates records, and constructs a fresh CPU mesh.
    /// No existing mesh or scene is changed. Do not call on an interactive render loop.
    pub fn readback(&self) -> Result<Mesh> {
        self.request_readback(None)?.wait()
    }
}

pub(super) fn decode(base: &Mesh, bytes: &[u8]) -> Result<Mesh> {
    ensure!(
        bytes.len() == base.vertex_count() * 64,
        "GPU deformation output size mismatch"
    );
    let mut vertices = base.vertices().to_vec();
    let mut tangents = base.tangents().map(<[_]>::to_vec);
    for (index, bytes) in bytes.chunks_exact(64).enumerate() {
        let record: GpuDeformationVertex = bytemuck::pod_read_unaligned(bytes);
        ensure!(
            record.status == [0; 4],
            "GPU deformation vertex {index} failed with status {:?}",
            record.status
        );
        vertices[index]
            .position
            .copy_from_slice(&record.position[..3]);
        vertices[index].normal.copy_from_slice(&record.normal[..3]);
        if let Some(tangents) = &mut tangents {
            tangents[index] = record.tangent;
        }
    }
    Ok(base.with_vertices(vertices, tangents)?)
}

pub(super) fn pad(v: [f32; 3]) -> [f32; 4] {
    [v[0], v[1], v[2], 0.]
}

pub(super) fn pack_mesh(mesh: &Mesh) -> Vec<GpuDeformationVertex> {
    mesh.vertices()
        .iter()
        .enumerate()
        .map(|(index, vertex)| GpuDeformationVertex {
            position: pad(vertex.position),
            normal: pad(vertex.normal),
            tangent: mesh.tangents().map_or([0.; 4], |t| t[index]),
            status: [0; 4],
        })
        .collect()
}

pub(super) fn validate_storage(
    limits: &wgpu::Limits,
    sizes: &[u64],
    vertices: usize,
) -> Result<()> {
    ensure!(
        vertices > 0 && vertices <= u32::MAX as usize,
        "GPU deformation vertex count exceeds u32"
    );
    for &bytes in sizes {
        ensure!(
            bytes <= limits.max_buffer_size && bytes <= limits.max_storage_buffer_binding_size,
            "GPU deformation buffer exceeds device limits"
        );
    }
    ensure!(
        (vertices as u64).div_ceil(64) <= u64::from(limits.max_compute_workgroups_per_dimension),
        "GPU deformation dispatch exceeds device limits"
    );
    Ok(())
}

pub(super) struct ComputeKernel {
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::ComputePipeline,
}

impl ComputeKernel {
    pub(super) fn new(
        device: &wgpu::Device,
        shader: &str,
        entry: &str,
        minimums: [u64; 3],
    ) -> Result<Self> {
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let entries: Vec<_> = minimums
            .into_iter()
            .chain([64, 16])
            .enumerate()
            .map(|(binding, minimum)| wgpu::BindGroupLayoutEntry {
                binding: binding as u32,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: if binding == 4 {
                        wgpu::BufferBindingType::Uniform
                    } else {
                        wgpu::BufferBindingType::Storage {
                            read_only: binding != 3,
                        }
                    },
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(minimum),
                },
                count: None,
            })
            .collect();
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("gpui_3d.deformation.inputs"),
            entries: &entries,
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("gpui_3d.deformation.layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("gpui_3d.deformation.shader"),
            source: wgpu::ShaderSource::Wgsl(shader.into()),
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some(entry),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some(entry),
            compilation_options: Default::default(),
            cache: None,
        });
        if let Some(error) = gpui::block_on(scope.pop()) {
            anyhow::bail!("GPU deformation pipeline: {error}");
        }
        Ok(Self { layout, pipeline })
    }

    pub(super) fn evaluate(
        &self,
        context: &WgpuContext,
        base: Mesh,
        inputs: [&wgpu::Buffer; 3],
        params: &wgpu::Buffer,
    ) -> Result<GpuDeformationOutput> {
        let buffer = self.evaluate_records(context, base.vertex_count(), inputs, params)?;
        Ok(GpuDeformationOutput {
            context: context.clone(),
            base,
            buffer,
        })
    }

    /// Dispatches one invocation per 64-byte output record. Callers admit payloads
    /// and validate dispatch limits before creating their retained source.
    fn evaluate_records(
        &self,
        context: &WgpuContext,
        records: usize,
        inputs: [&wgpu::Buffer; 3],
        params: &wgpu::Buffer,
    ) -> Result<gpui_wgpu::WgpuResource<wgpu::Buffer>> {
        ensure!(!context.device_lost(), "GPU deformation device is lost");
        let device = &context.device;
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let output = context.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gpui_3d.deformation.output"),
            size: records as u64 * 64,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::VERTEX
                | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("gpui_3d.deformation"),
        });
        self.encode(
            device,
            &mut encoder,
            records as u32,
            [inputs[0], inputs[1], inputs[2], &output, params],
        );
        context.queue.submit(Some(encoder.finish()));
        if let Some(error) = gpui::block_on(scope.pop()) {
            anyhow::bail!("GPU deformation submission: {error}");
        }
        Ok(output)
    }

    fn encode(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        records: u32,
        buffers: [&wgpu::Buffer; 5],
    ) {
        let entries: Vec<_> = buffers
            .into_iter()
            .enumerate()
            .map(|(binding, buffer)| wgpu::BindGroupEntry {
                binding: binding as u32,
                resource: buffer.as_entire_binding(),
            })
            .collect();
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("gpui_3d.deformation.bind"),
            layout: &self.layout,
            entries: &entries,
        });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("gpui_3d.deformation"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &bind, &[]);
            pass.dispatch_workgroups(records.div_ceil(64), 1, 1);
        }
    }
}
pub(super) fn buffer(
    device: &wgpu::Device,
    label: &str,
    bytes: &[u8],
    usage: wgpu::BufferUsages,
) -> wgpu::Buffer {
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some(label),
        contents: bytes,
        usage,
    })
}
