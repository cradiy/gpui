use gpui::{AlphaMode3d, MaterialTexture3d, MeshDraw3d, MeshTexture3d, Scene3dFrame};
use std::{ops::Range, sync::Arc};

pub(super) struct BatchPlan {
    pub order: Vec<usize>,
    pub batches: Vec<Range<usize>>,
    pub passes: Vec<Visibility>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct Visibility {
    pub camera: bool,
    pub shadow: bool,
}

impl Visibility {
    pub fn new(frame: &Scene3dFrame, object: &MeshDraw3d, shadows: bool) -> Self {
        Self {
            camera: object
                .mesh
                .intersects_clip_volume(object.model, frame.view_projection),
            shadow: shadows
                && object.cast_shadows
                && object.alpha_mode != AlphaMode3d::Blend
                && frame.directional_shadow.is_some_and(|s| {
                    object
                        .mesh
                        .intersects_clip_volume(object.model, s.view_projection)
                }),
        }
    }
    pub fn any(self) -> bool {
        self.camera || self.shadow
    }
}

impl BatchPlan {
    pub fn new(frame: &Scene3dFrame, color: bool, limit: usize) -> Self {
        assert!(limit > 0);
        let objects = &frame.objects;
        let mut order = if color {
            super::color_draw_order(objects)
        } else {
            (0..objects.len()).collect()
        };
        let mut passes = Vec::with_capacity(order.len());
        order.retain(|&index| {
            let visibility = Visibility::new(frame, &objects[index], color);
            if visibility.any() {
                passes.push(visibility);
                true
            } else {
                false
            }
        });
        let mut batches: Vec<Range<usize>> = Vec::new();
        for (position, &index) in order.iter().enumerate() {
            if let Some(batch) = batches.last_mut()
                && batch.len() < limit
                && passes[batch.start] == passes[position]
                && compatible(&objects[order[batch.start]], &objects[index])
            {
                batch.end += 1;
            } else {
                batches.push(position..position + 1);
            }
        }
        Self {
            order,
            batches,
            passes,
        }
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
