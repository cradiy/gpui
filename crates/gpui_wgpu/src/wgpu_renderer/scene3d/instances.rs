use gpui::{AlphaMode3d, MaterialTexture3d, MeshDraw3d, MeshTexture3d};
use std::{ops::Range, sync::Arc};

pub(super) struct BatchPlan {
    pub order: Vec<usize>,
    pub batches: Vec<Range<usize>>,
}

impl BatchPlan {
    pub fn new(objects: &[MeshDraw3d], color: bool, limit: usize) -> Self {
        assert!(limit > 0);
        let order = if color {
            super::color_draw_order(objects)
        } else {
            (0..objects.len()).collect()
        };
        let mut batches: Vec<Range<usize>> = Vec::new();
        for (position, &index) in order.iter().enumerate() {
            if let Some(batch) = batches.last_mut()
                && batch.len() < limit
                && compatible(&objects[order[batch.start]], &objects[index])
            {
                batch.end += 1;
            } else {
                batches.push(position..position + 1);
            }
        }
        Self { order, batches }
    }
}

fn same_map(a: Option<MaterialTexture3d>, b: Option<MaterialTexture3d>) -> bool {
    a.map(|m| (m.tile, m.sampling)) == b.map(|m| (m.tile, m.sampling))
}

fn compatible(a: &MeshDraw3d, b: &MeshDraw3d) -> bool {
    a.alpha_mode != AlphaMode3d::Blend
        && Arc::ptr_eq(&a.mesh, &b.mesh)
        && a.alpha_mode == b.alpha_mode
        && a.alpha_cutoff == b.alpha_cutoff
        && a.cast_shadows == b.cast_shadows
        && a.receive_shadows == b.receive_shadows
        && a.unlit == b.unlit
        && match (a.texture, b.texture) {
            (MeshTexture3d::None, MeshTexture3d::None)
            | (MeshTexture3d::Subtree, MeshTexture3d::Subtree) => true,
            (MeshTexture3d::Image(a), MeshTexture3d::Image(b)) => a == b,
            _ => false,
        }
        && a.sampling == b.sampling
        && a.image_color_space == b.image_color_space
        && a.pbr == b.pbr
        && same_map(a.metallic_roughness_texture, b.metallic_roughness_texture)
        && same_map(a.emissive_texture, b.emissive_texture)
        && same_map(a.normal_texture, b.normal_texture)
        && same_map(a.occlusion_texture, b.occlusion_texture)
        && a.normal_scale == b.normal_scale
        && a.occlusion_strength == b.occlusion_strength
}

pub(super) fn capacity(required: usize, limit: usize) -> usize {
    assert!(required > 0 && required <= limit);
    required
        .checked_next_power_of_two()
        .unwrap_or(limit)
        .min(limit)
}
