use super::gpu_deformation::{ComputeKernel, buffer, validate_storage};
use crate::{AffineTransform, GpuDeformationLimits, GpuDeformationOutput, Skin, SkinPalette};
use anyhow::{Context as _, Result, ensure};
use gpui_wgpu::{WgpuContext, wgpu};
use std::sync::Arc;

#[cfg(test)]
mod tests;

/// Skin payload sizes, excluding the separately owned input mesh and readback staging.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GpuSkinMemory {
    pub binding_bytes: u64,
    pub palette_bytes: u64,
    pub output_bytes: u64,
    pub uniform_bytes: u64,
}

impl GpuSkinMemory {
    pub fn plan(
        vertices: usize,
        joints: usize,
        influences: usize,
        limits: GpuDeformationLimits,
    ) -> Result<Self> {
        ensure!(
            vertices > 0 && vertices < u32::MAX as usize,
            "GPU Skin vertex count exceeds offset indexing"
        );
        ensure!(
            joints > 0 && joints <= u32::MAX as usize,
            "GPU Skin joint count must fit positive u32"
        );
        ensure!(
            influences >= vertices && influences <= u32::MAX as usize,
            "GPU Skin influence count is invalid"
        );
        let words = vertices as u64 + 1 + 2 * influences as u64;
        ensure!(
            words <= u64::from(u32::MAX),
            "GPU Skin binding indexing exceeds u32"
        );
        let memory = Self {
            binding_bytes: words * 4,
            palette_bytes: joints as u64 * 64,
            output_bytes: vertices as u64 * 64,
            uniform_bytes: 32,
        };
        ensure!(
            memory.binding_bytes + memory.palette_bytes + memory.uniform_bytes
                <= limits.max_source_bytes,
            "GPU Skin source exceeds payload budget"
        );
        ensure!(
            memory.output_bytes <= limits.max_output_bytes,
            "GPU Skin output exceeds payload budget"
        );
        Ok(memory)
    }

    fn validate_device(self, limits: &wgpu::Limits) -> Result<()> {
        validate_storage(
            limits,
            &[self.binding_bytes, self.palette_bytes, self.output_bytes],
            (self.output_bytes / 64) as usize,
        )
    }
}

/// Immutable GPU influence binding. Mesh vertex order must match the CPU Skin binding.
pub struct GpuSkin {
    context: WgpuContext,
    source: Skin,
    identity: Arc<()>,
    memory: GpuSkinMemory,
    binding: wgpu::Buffer,
    params: [wgpu::Buffer; 2],
    kernel: ComputeKernel,
}

/// Immutable joint palette reusable across evaluations of its originating GpuSkin.
pub struct GpuSkinPalette {
    identity: Arc<()>,
    buffer: wgpu::Buffer,
}

impl GpuSkin {
    /// Checks enabled compute limits without allocating resources or submitting work.
    /// Binding size, payload admission, device health, and output validity are checked separately.
    pub fn check_support(capabilities: &gpui_wgpu::Scene3dDeviceCapabilities) -> Result<()> {
        super::gpu_deformation::support::validate(capabilities, 4, 1, 0)
    }

    pub fn new(context: WgpuContext, source: Skin, limits: GpuDeformationLimits) -> Result<Self> {
        ensure!(!context.device_lost(), "GPU Skin device is lost");
        Self::check_support(&gpui_wgpu::Scene3dDeviceCapabilities::query(&context))?;
        let influences = (0..source.vertex_count()).try_fold(0_usize, |count, vertex| {
            count
                .checked_add(source.vertex_influences(vertex)?.len())
                .context("GPU Skin influence count overflow")
        })?;
        let memory = GpuSkinMemory::plan(
            source.vertex_count(),
            source.joint_count(),
            influences,
            limits,
        )?;
        memory.validate_device(&context.device.limits())?;
        let binding = pack_binding(&source, memory)?;
        let device = &context.device;
        let kernel =
            ComputeKernel::new(device, include_str!("gpu_skin.wgsl"), "skin", [64, 4, 64])?;
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let binding = buffer(
            device,
            "gpui_3d.skin.binding",
            bytemuck::cast_slice(&binding),
            wgpu::BufferUsages::STORAGE,
        );
        let params = std::array::from_fn(|tangents| {
            buffer(
                device,
                "gpui_3d.skin.params",
                bytemuck::cast_slice(&[
                    source.vertex_count() as u32,
                    tangents as u32,
                    source.vertex_count() as u32 + 1,
                    0,
                ]),
                wgpu::BufferUsages::UNIFORM,
            )
        });
        if let Some(error) = gpui::block_on(scope.pop()) {
            anyhow::bail!("GPU Skin preparation: {error}");
        }
        Ok(Self {
            context,
            source,
            identity: Arc::new(()),
            memory,
            binding,
            params,
            kernel,
        })
    }

    pub fn source(&self) -> &Skin {
        &self.source
    }
    pub fn memory(&self) -> GpuSkinMemory {
        self.memory
    }

    /// Uploads `inverse(mesh_world) * joint_world * inverse_bind` in binding order.
    /// Joint matrices are composed and validated on the CPU; vertex blending runs on the GPU.
    pub fn palette(
        &self,
        mesh_world: AffineTransform,
        joint_world: &[AffineTransform],
    ) -> Result<GpuSkinPalette> {
        ensure!(!self.context.device_lost(), "GPU Skin device is lost");
        self.upload_palette(&self.source.palette(mesh_world, joint_world)?)
    }

    /// Uploads an already composed CPU palette without recomputing its matrices.
    /// The palette must retain the source's joint bindings. CPU palettes can be
    /// reused across devices; the uploaded result belongs to this `GpuSkin` only.
    pub fn upload_palette(&self, palette: &SkinPalette) -> Result<GpuSkinPalette> {
        palette.validate_binding(&self.source)?;
        ensure!(!self.context.device_lost(), "GPU Skin device is lost");
        let device = &self.context.device;
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let buffer = buffer(
            device,
            "gpui_3d.skin.palette",
            bytemuck::cast_slice(palette.matrices()),
            wgpu::BufferUsages::STORAGE,
        );
        if let Some(error) = gpui::block_on(scope.pop()) {
            anyhow::bail!("GPU Skin palette: {error}");
        }
        Ok(GpuSkinPalette {
            identity: self.identity.clone(),
            buffer,
        })
    }

    /// Accepts an uploaded bind mesh or a Morph output without CPU readback.
    /// Previous skinned results are not bind-space inputs. The source buffer is never modified.
    pub fn evaluate(
        &self,
        input: &GpuDeformationOutput,
        palette: &GpuSkinPalette,
    ) -> Result<GpuDeformationOutput> {
        ensure!(
            Arc::ptr_eq(&self.context.device, &input.context.device),
            "GPU Skin input belongs to a different device"
        );
        ensure!(
            Arc::ptr_eq(&self.identity, &palette.identity),
            "GPU Skin palette belongs to a different binding"
        );
        ensure!(
            input.base_mesh().vertex_count() == self.source.vertex_count(),
            "GPU Skin vertex count mismatch"
        );
        self.kernel.evaluate(
            &self.context,
            input.base_mesh().clone(),
            [input.buffer(), &self.binding, &palette.buffer],
            &self.params[usize::from(input.base_mesh().tangents().is_some())],
        )
    }
}

fn pack_binding(source: &Skin, memory: GpuSkinMemory) -> Result<Vec<u32>> {
    let offset_words = source.vertex_count() + 1;
    let mut words = Vec::with_capacity((memory.binding_bytes / 4) as usize);
    words.resize(offset_words, 0);
    for vertex in 0..source.vertex_count() {
        words[vertex] = ((words.len() - offset_words) / 2) as u32;
        for influence in source.vertex_influences(vertex)? {
            let weight = influence.weight as f32;
            ensure!(
                weight.is_normal() && weight > 0.,
                "GPU Skin vertex {vertex} weight is below normal f32 range"
            );
            words.push(influence.joint as u32);
            words.push(weight.to_bits());
        }
    }
    words[source.vertex_count()] = ((words.len() - offset_words) / 2) as u32;
    Ok(words)
}
