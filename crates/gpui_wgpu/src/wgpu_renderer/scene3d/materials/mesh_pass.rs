use super::*;
use gpui::{MeshPassBlend3d, MeshPassDepth3d, MeshPassState3d};
mod expansion;
pub(super) use expansion::Expansion;

#[cfg(all(test, not(target_family = "wasm")))]
mod tests;

pub(super) fn constants(state: MeshPassState3d) -> Vec<(&'static str, f64)> {
    vec![
        ("mesh_pass_cull", state.cull as u32 as f64),
        ("mesh_pass_alpha_mode", state.alpha_mode as u32 as f64),
        ("mesh_pass_alpha_cutoff", f64::from(state.alpha_cutoff)),
    ]
}

pub(super) fn blend(mode: MeshPassBlend3d) -> Option<wgpu::BlendState> {
    match mode {
        MeshPassBlend3d::Replace => None,
        MeshPassBlend3d::SourceOver => Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
        MeshPassBlend3d::Additive => Some(wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING.alpha,
        }),
    }
}

pub(super) fn depth(state: MeshPassState3d) -> wgpu::DepthStencilState {
    let compare = match state.depth_compare {
        MeshPassDepth3d::Never => wgpu::CompareFunction::Never,
        MeshPassDepth3d::Less => wgpu::CompareFunction::Less,
        MeshPassDepth3d::Equal => wgpu::CompareFunction::Equal,
        MeshPassDepth3d::LessEqual => wgpu::CompareFunction::LessEqual,
        MeshPassDepth3d::Greater => wgpu::CompareFunction::Greater,
        MeshPassDepth3d::NotEqual => wgpu::CompareFunction::NotEqual,
        MeshPassDepth3d::GreaterEqual => wgpu::CompareFunction::GreaterEqual,
        MeshPassDepth3d::Always => wgpu::CompareFunction::Always,
    };
    wgpu::DepthStencilState {
        format: wgpu::TextureFormat::Depth32Float,
        depth_write_enabled: Some(state.depth_write),
        depth_compare: Some(compare),
        stencil: Default::default(),
        bias: wgpu::DepthBiasState {
            constant: state.depth_bias,
            slope_scale: state.depth_slope_bias,
            clamp: state.depth_bias_clamp,
        },
    }
}

#[cfg(not(target_family = "wasm"))]
pub(in super::super) fn pass_snapshot(pass: &gpui::MeshPass3d) -> Result<&Scene3dMaterialSnapshot> {
    pass.material
        .downcast_ref()
        .context("unsupported 3D mesh pass backend")
}

#[cfg(not(target_family = "wasm"))]
#[derive(Default)]
pub(in super::super) struct MeshPassCache(HashMap<(usize, [u32; 9], [u32; 4]), MeshPassPipeline>);

#[cfg(not(target_family = "wasm"))]
struct MeshPassPipeline {
    _source: Scene3dMaterialSource,
    pipeline: wgpu::RenderPipeline,
}

#[cfg(not(target_family = "wasm"))]
fn key(state: MeshPassState3d) -> [u32; 9] {
    [
        state.cull as u32,
        state.depth_compare as u32,
        u32::from(state.depth_write),
        state.depth_bias as u32,
        state.depth_slope_bias.to_bits(),
        state.depth_bias_clamp.to_bits(),
        state.blend as u32,
        state.alpha_mode as u32,
        state.alpha_cutoff.to_bits(),
    ]
}

#[cfg(not(target_family = "wasm"))]
impl MeshPassCache {
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        frames: &[&gpui::Scene3dFrame],
        format: wgpu::TextureFormat,
        samples: u32,
    ) -> Result<()> {
        let color = !matches!(
            format,
            wgpu::TextureFormat::R32Uint
                | wgpu::TextureFormat::R32Float
                | wgpu::TextureFormat::Rgba32Float
        );
        let mut used = HashSet::new();
        for object in frames.iter().flat_map(|frame| frame.objects.iter()) {
            ensure!(
                object.mesh_passes.len() <= gpui::MAX_MESH_PASSES_3D,
                "too many additional mesh passes"
            );
            for pass in object.mesh_passes.iter() {
                ensure!(pass.state.is_valid(), "invalid additional mesh pass state");
                let snapshot = pass_snapshot(pass)?;
                snapshot.validate_vertex_count(object.mesh.vertices().len())?;
                let source = snapshot.source();
                let expansion = Expansion::new(
                    source.program().vertex_attributes(),
                    pass.expansion.as_ref(),
                )?;
                ensure!(
                    !source.context().device_lost()
                        && std::ptr::eq(device, source.context().device.as_ref()),
                    "3D mesh pass belongs to a different or lost device"
                );
                if !color {
                    continue;
                }
                let identity = (source.identity(), key(pass.state), expansion.key());
                used.insert(identity);
                if let std::collections::hash_map::Entry::Vacant(entry) = self.0.entry(identity) {
                    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
                    let pipeline = create_pipeline(
                        device,
                        source.shader(),
                        Some(source.layout()),
                        source.vertex_layout(),
                        format,
                        samples,
                        Pass::Additional(pass.state, expansion),
                    );
                    if let Some(error) = gpui::block_on(scope.pop()) {
                        anyhow::bail!("3D mesh pass pipeline: {error}");
                    }
                    entry.insert(MeshPassPipeline {
                        _source: source.clone(),
                        pipeline,
                    });
                }
            }
        }
        self.0.retain(|key, _| used.contains(key));
        Ok(())
    }

    pub fn get(&self, pass: &gpui::MeshPass3d) -> &wgpu::RenderPipeline {
        let snapshot = pass_snapshot(pass).expect("validated mesh pass");
        let expansion = Expansion::new(
            snapshot.source().program().vertex_attributes(),
            pass.expansion.as_ref(),
        )
        .expect("validated mesh pass expansion");
        &self.0[&(
            snapshot.source().identity(),
            key(pass.state),
            expansion.key(),
        )]
            .pipeline
    }
}
