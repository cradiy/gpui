use super::{RenderObject, RenderedFrame, WgpuIdRemapper, labels::assign_labels};
use crate::Camera;
use anyhow::{Context as _, Result};
use gpui_wgpu::wgpu;
use std::sync::Arc;

/// Independently owned R32Uint labels with the producing camera and source identities.
/// Does not retain source textures or scene geometry. Encoded outputs require
/// submission before use on the GPU; submission does not imply completion.
pub struct RenderedLabels {
    texture: wgpu::Texture,
    camera: Camera,
    labels: Vec<u32>,
    objects: Arc<[RenderObject]>,
}

impl RenderedFrame {
    /// Submits integer label remapping without CPU pixel readback. `assign` runs
    /// once per frame object, including zero-coverage objects. Repeated labels
    /// merge objects; zero excludes them. Background remains zero.
    /// Input metadata and budgets are checked before callbacks. Callback side
    /// effects are not rolled back if allocation or GPU validation fails.
    pub fn label_texture(
        &self,
        mapper: &WgpuIdRemapper,
        assign: impl FnMut(&RenderObject) -> u32,
    ) -> Result<RenderedLabels> {
        let input = self
            .output
            .object_ids()
            .context("frame has no object-ID channel")?;
        mapper.validate_input(input, self.objects.len())?;
        let labels = assign_labels(&self.objects, assign)?;
        Ok(RenderedLabels {
            texture: mapper.render(input, &labels)?,
            camera: self.camera,
            labels,
            objects: self.objects.clone(),
        })
    }

    /// Records label remapping in a caller-owned encoder without submitting it.
    /// Assignment and admission follow `label_texture`. The mapper and encoder
    /// must use the frame's device. The caller owns queue ordering and submission
    /// validation, and must discard the encoder if recording fails.
    pub fn encode_label_texture(
        &self,
        mapper: &WgpuIdRemapper,
        encoder: &mut wgpu::CommandEncoder,
        assign: impl FnMut(&RenderObject) -> u32,
    ) -> Result<RenderedLabels> {
        let input = self
            .output
            .object_ids()
            .context("frame has no object-ID channel")?;
        mapper.validate_input(input, self.objects.len())?;
        let labels = assign_labels(&self.objects, assign)?;
        Ok(RenderedLabels {
            texture: mapper.encode(encoder, input, &labels)?,
            camera: self.camera,
            labels,
            objects: self.objects.clone(),
        })
    }
}

impl RenderedLabels {
    /// Single-sampled, single-mip R32Uint texture with TEXTURE_BINDING,
    /// RENDER_ATTACHMENT, and COPY_SRC usages. Calls never reuse an older output.
    pub fn texture(&self) -> &wgpu::Texture {
        &self.texture
    }
    pub fn size(&self) -> [u32; 2] {
        [self.texture.width(), self.texture.height()]
    }
    pub fn camera(&self) -> Camera {
        self.camera
    }
    /// Zero and unknown source IDs return None; excluded objects return Some(0).
    pub fn label_for_object(&self, output_id: u32) -> Option<u32> {
        self.labels.get(output_id.checked_sub(1)? as usize).copied()
    }
    /// Source objects assigned this label, including zero-coverage objects.
    /// Label zero lists excluded objects, not background.
    pub fn objects(&self, label: u32) -> impl Iterator<Item = &RenderObject> {
        self.objects
            .iter()
            .zip(&self.labels)
            .filter_map(move |(object, &assigned)| (assigned == label).then_some(object))
    }
}
