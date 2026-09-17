use super::{
    Scene3dCapabilities, Scene3dChannels, Scene3dFrameId, Scene3dGpuOutput, Scene3dOutputConfig,
    Scene3dTargetMemory,
};
use crate::wgpu_renderer::scene3d::{RenderRegion, Scene3dRenderer};
use crate::{WgpuAtlas, WgpuContext, WgpuScene3dPickFrame};
use anyhow::{Result, ensure};
use gpui::{Scene3dFrame, Scene3dPickCapture};
use std::{
    collections::HashSet,
    sync::{Arc, atomic::AtomicBool},
};

/// A retained group's independent ID/depth output. Primary object IDs are preserved;
/// auxiliary IDs are local to this group and never enter the primary picking output.
#[derive(Clone)]
pub struct Scene3dOcclusionOutput {
    group_id: u64,
    parent_frame: Scene3dFrameId,
    auxiliary_ids: Arc<[u32]>,
    frame: Arc<WgpuScene3dPickFrame>,
    elements: Option<Arc<WgpuScene3dPickFrame>>,
}

impl Scene3dOcclusionOutput {
    pub fn group_id(&self) -> u64 {
        self.group_id
    }
    /// Exact primary output allocation submitted with this occlusion output.
    pub fn parent_frame_id(&self) -> &Scene3dFrameId {
        &self.parent_frame
    }
    pub fn gpu(&self) -> &Scene3dGpuOutput {
        self.frame.gpu()
    }
    /// Separate point/line IDs and view depth. Zero ID denotes no selectable element.
    pub fn elements(&self) -> Option<&Scene3dGpuOutput> {
        self.elements.as_ref().map(|frame| frame.gpu())
    }
    pub fn auxiliary_index(&self, output_id: u32) -> Option<usize> {
        self.auxiliary_ids.iter().position(|id| *id == output_id)
    }
    /// Raster-space projection rectangle shared with the primary capture.
    pub fn projection_rect(&self) -> [f32; 4] {
        self.frame.projection_rect()
    }
}

struct Group {
    overlay: Option<super::edit_overlay::EditOverlay>,
    source: Arc<Scene3dFrame>,
    renderers: [Scene3dRenderer; 2],
    output: Scene3dOcclusionOutput,
}

pub(crate) struct OcclusionPasses {
    groups: Vec<Group>,
}

pub(crate) fn element_output_count(frame: &Scene3dFrame) -> usize {
    frame
        .occlusion_groups
        .iter()
        .filter(|g| !g.points.is_empty() || !g.lines.is_empty())
        .count()
}

pub(crate) fn target_memory(
    mut base: Scene3dTargetMemory,
    size: [u32; 2],
    count: usize,
    element_count: usize,
    limit: Option<u64>,
) -> Result<Scene3dTargetMemory> {
    let per_group = Scene3dOutputConfig {
        size,
        channels: Scene3dChannels::OBJECT_ID | Scene3dChannels::LINEAR_DEPTH,
        color_samples: 1,
    }
    .target_memory(None)?;
    for (target, extra) in [
        (&mut base.output_bytes, per_group.output_bytes),
        (&mut base.attachment_bytes, per_group.attachment_bytes),
        (&mut base.total_bytes, per_group.total_bytes),
    ] {
        *target = extra
            .checked_mul(count as u64)
            .and_then(|v| target.checked_add(v))
            .ok_or_else(|| anyhow::anyhow!("occlusion target size overflow"))?;
    }
    let element_bytes = u64::from(size[0])
        .checked_mul(u64::from(size[1]))
        .and_then(|n| n.checked_mul(8))
        .and_then(|n| n.checked_mul(element_count as u64))
        .ok_or_else(|| anyhow::anyhow!("edit overlay target size overflow"))?;
    base.output_bytes = base
        .output_bytes
        .checked_add(element_bytes)
        .ok_or_else(|| anyhow::anyhow!("edit overlay target size overflow"))?;
    base.total_bytes = base
        .total_bytes
        .checked_add(element_bytes)
        .ok_or_else(|| anyhow::anyhow!("edit overlay target size overflow"))?;
    ensure!(
        limit.is_none_or(|limit| base.total_bytes <= limit),
        "grouped occlusion exceeds target payload budget"
    );
    Ok(base)
}

fn sources(frame: &Scene3dFrame) -> Result<Vec<Arc<Scene3dFrame>>> {
    let ids: HashSet<_> = frame
        .objects
        .iter()
        .map(|object| object.output_id)
        .collect();
    ensure!(
        ids.len() == frame.objects.len() && !ids.contains(&0),
        "occlusion requires unique nonzero primary IDs"
    );
    let mut groups = HashSet::new();
    let mut assigned = HashSet::new();
    frame
        .occlusion_groups
        .iter()
        .map(|group| {
            ensure!(group.elements_are_valid(), "invalid edit overlay elements");
            ensure!(groups.insert(group.id), "duplicate occlusion group ID");
            ensure!(
                !group.members.is_empty(),
                "occlusion group requires members"
            );
            for id in group.members.iter() {
                ensure!(
                    ids.contains(id) && assigned.insert(*id),
                    "unknown or repeated occlusion member"
                );
            }
            let mut output_ids = ids.clone();
            for object in group.occluders.iter() {
                ensure!(
                    object.output_id != 0 && output_ids.insert(object.output_id),
                    "occlusion mesh ID collides with primary geometry"
                );
                ensure!(
                    matches!(object.texture, gpui::MeshTexture3d::None)
                        && object.custom_material.is_none()
                        && object.alpha_mode == gpui::AlphaMode3d::Opaque
                        && object.gpu_geometry.is_none(),
                    "auxiliary occluders require opaque CPU geometry"
                );
            }
            let mut source = frame.clone();
            source.occlusion_groups = Arc::default();
            source.directional_shadow = None;
            source.background = None;
            source.objects = frame
                .objects
                .iter()
                .filter(|object| !group.members.contains(&object.output_id))
                .chain(group.occluders.iter())
                .cloned()
                .collect();
            Ok(Arc::new(source))
        })
        .collect()
}

impl OcclusionPasses {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn prepare(
        context: &WgpuContext,
        capabilities: Scene3dCapabilities,
        frame: &Scene3dFrame,
        region: RenderRegion,
        color_samples: u32,
        parent: &Scene3dFrameId,
        limit: Option<u64>,
        geometry_limit: Option<u64>,
        busy: Arc<AtomicBool>,
    ) -> Result<Self> {
        if frame.occlusion_groups.is_empty() {
            return Ok(Self { groups: Vec::new() });
        }
        let mut sources = sources(frame)?;
        let mut geometry_bytes = 0u64;
        for source in &sources {
            let geometry = super::gpu_draws::validate_frame(&context.device, source)?;
            let memory = super::gpu_draws::memory(
                source,
                Scene3dChannels::OBJECT_ID | Scene3dChannels::LINEAR_DEPTH,
                &geometry,
            )?;
            geometry_bytes = geometry_bytes
                .checked_add(memory.total_bytes)
                .ok_or_else(|| anyhow::anyhow!("occlusion geometry size overflow"))?;
            memory.validate(context.device.limits().max_buffer_size, None)?;
        }
        for definition in frame.occlusion_groups.iter() {
            let payload = super::edit_overlay::payload_bytes(definition)?;
            ensure!(
                payload <= context.device.limits().max_buffer_size,
                "edit overlay exceeds device vertex buffer limits"
            );
            geometry_bytes = geometry_bytes
                .checked_add(payload)
                .ok_or_else(|| anyhow::anyhow!("edit geometry size overflow"))?;
        }
        ensure!(
            geometry_limit.is_none_or(|limit| geometry_bytes <= limit),
            "grouped occlusion exceeds geometry payload budget"
        );
        let mut groups = Vec::new();
        for (definition, mut source) in frame.occlusion_groups.iter().zip(sources.drain(..)) {
            Arc::make_mut(&mut source).pick_capture =
                Some(Scene3dPickCapture::new(limit.unwrap_or(u64::MAX)));
            let mut output = WgpuScene3dPickFrame::allocate(
                context.clone(),
                capabilities,
                &source,
                region.size,
                region.rect,
                region.source_rect,
                busy.clone(),
            )?;
            let mut renderers = [wgpu::TextureFormat::R32Uint, wgpu::TextureFormat::R32Float]
                .map(|format| Scene3dRenderer::new(&context.device, &context.queue, format, 1));
            let mut statistics = super::Scene3dDrawStatistics::default();
            for renderer in &mut renderers {
                renderer.prepare_frames(
                    &context.device,
                    &context.queue,
                    [source.as_ref()],
                    [region.size],
                )?;
                statistics += renderer.draw_statistics(&source);
            }
            output.set_statistics(statistics);
            let overlay = if definition.points.is_empty() && definition.lines.is_empty() {
                None
            } else {
                Some(super::edit_overlay::EditOverlay::new(
                    context,
                    capabilities,
                    &source,
                    definition,
                    region,
                    output.gpu(),
                    color_samples,
                    busy.clone(),
                )?)
            };
            let elements = overlay.as_ref().map(|overlay| overlay.output());
            groups.push(Group {
                overlay,
                source,
                renderers,
                output: Scene3dOcclusionOutput {
                    group_id: definition.id,
                    parent_frame: parent.clone(),
                    auxiliary_ids: definition.occluders.iter().map(|o| o.output_id).collect(),
                    frame: Arc::new(output),
                    elements,
                },
            });
        }
        Ok(Self { groups })
    }

    pub(crate) fn outputs(&self) -> Vec<Scene3dOcclusionOutput> {
        self.groups
            .iter()
            .map(|group| group.output.clone())
            .collect()
    }
    pub(crate) fn statistics(&self, color: bool) -> super::Scene3dDrawStatistics {
        let mut statistics = super::Scene3dDrawStatistics::default();
        for group in &self.groups {
            statistics += group.output.gpu().draw_statistics();
            if let Some(elements) = group.output.elements() {
                let element_stats = elements.draw_statistics();
                statistics += element_stats;
                if color {
                    statistics.camera_draws += element_stats.camera_draws;
                    statistics.camera_instances += element_stats.camera_instances;
                    statistics.camera_triangles += element_stats.camera_triangles;
                }
            }
        }
        statistics
    }

    pub(crate) fn encode(
        &self,
        context: &WgpuContext,
        atlas: &WgpuAtlas,
        region: RenderRegion,
        source: Option<&wgpu::TextureView>,
        encoder: &mut wgpu::CommandEncoder,
    ) {
        for group in &self.groups {
            let output = group.output.gpu();
            for (renderer, texture) in group
                .renderers
                .iter()
                .zip([output.object_ids().unwrap(), output.linear_depth().unwrap()])
            {
                renderer.encode_frame(
                    &context.device,
                    &context.queue,
                    atlas,
                    &group.source,
                    region,
                    0,
                    source,
                    Some(&texture.create_view(&Default::default())),
                    encoder,
                );
            }
            if let Some(overlay) = &group.overlay {
                overlay.encode_data(encoder);
            }
        }
    }

    pub(crate) fn encode_color(
        &self,
        target: &wgpu::TextureView,
        encoder: &mut wgpu::CommandEncoder,
    ) {
        for group in &self.groups {
            if let Some(overlay) = &group.overlay {
                overlay.encode_color(target, encoder);
            }
        }
    }

    pub(crate) fn commit(&self, submitted: bool) {
        for group in &self.groups {
            for renderer in &group.renderers {
                renderer.commit_uploads(submitted);
            }
        }
    }

    pub(crate) fn retain_external_uploads(&self) {
        for group in &self.groups {
            for renderer in &group.renderers {
                renderer.retain_external_uploads();
            }
        }
    }
}
