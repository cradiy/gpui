use gpui::{AlphaMode3d, MaterialTexture3d, MeshDraw3d, MeshTexture3d, Scene3dFrame};
use std::{
    collections::HashMap,
    hash::{Hash, Hasher},
    ops::Range,
    sync::Arc,
};

#[derive(Clone)]
struct PlanKey {
    objects: Arc<[MeshDraw3d]>,
    camera: [[u32; 4]; 4],
    shadow: Option<[[u32; 4]; 4]>,
    color: bool,
    limit: usize,
}

impl PlanKey {
    fn new(frame: &Scene3dFrame, color: bool, limit: usize) -> Self {
        Self {
            objects: frame.objects.clone(),
            camera: frame.view_projection.map(|column| column.map(f32::to_bits)),
            shadow: color
                .then_some(frame.directional_shadow)
                .flatten()
                .map(|shadow| {
                    shadow
                        .view_projection
                        .map(|column| column.map(f32::to_bits))
                }),
            color,
            limit,
        }
    }
}

impl PartialEq for PlanKey {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.objects, &other.objects)
            && self.camera == other.camera
            && self.shadow == other.shadow
            && self.color == other.color
            && self.limit == other.limit
    }
}

impl Eq for PlanKey {}

impl Hash for PlanKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        (self.objects.as_ptr() as usize).hash(state);
        self.objects.len().hash(state);
        self.camera.hash(state);
        self.shadow.hash(state);
        self.color.hash(state);
        self.limit.hash(state);
    }
}

#[derive(Default)]
pub(super) struct BatchPlanCache {
    entries: HashMap<PlanKey, CachedPlan>,
    epoch: bool,
}

struct CachedPlan {
    plan: Arc<BatchPlan>,
    epoch: bool,
}

impl BatchPlanCache {
    pub(super) fn prepare<'a>(
        &mut self,
        frames: impl IntoIterator<Item = &'a Scene3dFrame>,
        color: bool,
        limit: usize,
    ) {
        self.epoch = !self.epoch;
        for frame in frames {
            let key = PlanKey::new(frame, color, limit);
            let entry = self.entries.entry(key).or_insert_with(|| CachedPlan {
                plan: Arc::new(BatchPlan::new(frame, color, limit)),
                epoch: self.epoch,
            });
            entry.epoch = self.epoch;
        }
        self.entries.retain(|_, entry| entry.epoch == self.epoch);
    }

    pub(super) fn get(&self, frame: &Scene3dFrame, color: bool, limit: usize) -> &BatchPlan {
        &self.entries[&PlanKey::new(frame, color, limit)].plan
    }

    pub(super) fn reuse_from(&mut self, other: &Self) {
        for (key, entry) in &other.entries {
            self.entries
                .entry(key.clone())
                .or_insert_with(|| CachedPlan {
                    plan: entry.plan.clone(),
                    epoch: self.epoch,
                });
        }
    }
}

pub(super) struct BatchPlan {
    color: bool,
    pub extra_batches: Vec<usize>,
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
    pub fn new(frame: &Scene3dFrame, object: &MeshDraw3d, color: bool) -> Self {
        Self {
            camera: object.intersects_clip_volume(frame.view_projection)
                || (color && object.mesh_passes_intersect_clip_volume(frame.view_projection)),
            shadow: color
                && object.cast_shadows
                && object.alpha_mode != AlphaMode3d::Blend
                && frame
                    .directional_shadow
                    .is_some_and(|s| object.intersects_clip_volume(s.view_projection)),
        }
    }
    pub fn any(self) -> bool {
        self.camera || self.shadow
    }
}

impl BatchPlan {
    pub fn statistics(&self, frame: &Scene3dFrame) -> crate::Scene3dDrawStatistics {
        let mut statistics = crate::Scene3dDrawStatistics {
            batches: self.batches.len() as u64,
            instance_upload_bytes: self.order.len() as u64
                * std::mem::size_of::<super::Instance>() as u64,
            uniform_upload_bytes: self.batches.len() as u64
                * std::mem::size_of::<super::Params>() as u64,
            ..Default::default()
        };
        for batch in &self.batches {
            let visibility = self.passes[batch.start];
            let instances = batch.len() as u64;
            let triangles = (frame.objects[self.order[batch.start]].mesh.indices().len() / 3)
                as u64
                * instances;
            if visibility.camera {
                let draws = 1 + if self.color {
                    frame.objects[self.order[batch.start]].mesh_passes.len() as u64
                } else {
                    0
                };
                statistics.camera_draws += draws;
                statistics.camera_instances += instances * draws;
                statistics.camera_triangles += triangles * draws;
            }
            if visibility.shadow {
                statistics.shadow_draws += 1;
                statistics.shadow_instances += instances;
                statistics.shadow_triangles += triangles;
            }
        }
        statistics
    }

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
        let mut extra_batches: Vec<_> = batches
            .iter()
            .enumerate()
            .filter_map(|(index, batch)| {
                (color
                    && passes[batch.start].camera
                    && !objects[order[batch.start]].mesh_passes.is_empty())
                .then_some(index)
            })
            .collect();
        extra_batches.sort_by_key(|&index| order[batches[index].start]);
        Self {
            color,
            extra_batches,
            order,
            batches,
            passes,
        }
    }
}

fn same_map(a: Option<MaterialTexture3d>, b: Option<MaterialTexture3d>) -> bool {
    a.map(|m| (m.tile, m.sampling, m.uv_set)) == b.map(|m| (m.tile, m.sampling, m.uv_set))
}

fn compatible(a: &MeshDraw3d, b: &MeshDraw3d) -> bool {
    (match (&a.custom_material, &b.custom_material) {
        (None, None) => true,
        (Some(a), Some(b)) => a.same_snapshot(b),
        _ => false,
    }) && a.mesh_passes.is_empty()
        && b.mesh_passes.is_empty()
        && a.render_bounds.is_none()
        && b.render_bounds.is_none()
        && a.gpu_geometry.is_none()
        && b.gpu_geometry.is_none()
        && a.alpha_mode != AlphaMode3d::Blend
        && Arc::ptr_eq(&a.mesh, &b.mesh)
        && a.alpha_mode == b.alpha_mode
        && a.alpha_cutoff == b.alpha_cutoff
        && a.double_sided == b.double_sided
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
        && a.texture_uv_sets() == b.texture_uv_sets()
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

pub(super) fn retained_capacity(current: usize, required: usize, limit: usize) -> usize {
    let desired = capacity(required, limit);
    if current < required || current > limit || required <= current / 4 {
        desired
    } else {
        current
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{IDENTITY, frame, object};
    use super::*;

    #[test]
    fn scene3d_expansion_keeps_camera_candidates_without_expanding_data_or_shadow_bounds() {
        let mut object = object();
        object.render_bounds = Some([[1.1, 0., 0.5], [1.2, 0.1, 0.6]]);
        object.mesh_passes = vec![gpui::MeshPass3d {
            material: gpui::MeshMaterial3d::new(Arc::new(())),
            state: Default::default(),
            expansion: Some(gpui::MeshPassExpansion3d::new(
                gpui::MeshPassSpace3d::World,
                0.2,
            )),
        }]
        .into();
        let mut frame = frame(&[object]);
        frame.directional_shadow = Some(gpui::DirectionalShadow3d {
            light_index: 0,
            view_projection: IDENTITY,
            resolution: 64,
            depth_bias: 0.,
            normal_bias: 0.,
            softness: 0.,
        });
        let plan = BatchPlan::new(&frame, true, 100);
        assert_eq!(plan.order, [0]);
        assert_eq!(plan.extra_batches, [0]);
        assert!(!plan.passes[0].shadow);
        assert!(BatchPlan::new(&frame, false, 100).order.is_empty());
    }

    #[test]
    fn scene3d_mesh_passes_keep_submission_order_and_separate_instance_batches() {
        let mut a = object();
        a.alpha_mode = AlphaMode3d::Blend;
        a.sort_depth = 1.;
        a.mesh_passes = vec![gpui::MeshPass3d {
            expansion: None,
            material: gpui::MeshMaterial3d::new(Arc::new(())),
            state: Default::default(),
        }]
        .into();
        let mut b = a.clone();
        b.alpha_mode = AlphaMode3d::Opaque;
        b.mesh_passes = vec![b.mesh_passes[0].clone(); 2].into();
        let mut c = a.clone();
        c.sort_depth = 3.;
        let frame = frame(&[a, b, c]);
        let plan = BatchPlan::new(&frame, true, 100);
        assert_eq!(plan.order, [1, 2, 0]);
        assert_eq!(plan.batches, [0..1, 1..2, 2..3]);
        assert_eq!(plan.extra_batches, [2, 0, 1]);
        assert_eq!(plan.statistics(&frame).camera_draws, 7);
        let data = BatchPlan::new(&frame, false, 100);
        assert!(data.extra_batches.is_empty());
        assert_eq!(data.statistics(&frame).camera_draws, 3);
        assert!(!compatible(&frame.objects[1], &frame.objects[1]));
    }

    #[test]
    fn explicit_render_bounds_separate_instances_and_control_camera_and_shadow_passes() {
        let first = object();
        let mut second = first.clone();
        second.output_id = 2;
        second.render_bounds = Some([[4., 0., 0.], [5., 1., 1.]]);
        let mut third = first.clone();
        third.output_id = 3;
        third.render_bounds = Some([[0., 0., 0.], [1., 1., 1.]]);
        let mut frame = frame(&[first, second, third]);
        let mut projection = IDENTITY;
        projection[3][0] = -4.;
        frame.directional_shadow = Some(gpui::DirectionalShadow3d {
            light_index: 0,
            view_projection: projection,
            resolution: 64,
            depth_bias: 0.,
            normal_bias: 0.,
            softness: 0.,
        });
        let plan = BatchPlan::new(&frame, true, 100);
        assert_eq!(plan.order, [0, 1, 2]);
        assert_eq!(plan.batches, [0..1, 1..2, 2..3]);
        assert!(plan.passes[0].camera && !plan.passes[0].shadow);
        assert!(!plan.passes[1].camera && plan.passes[1].shadow);
        assert!(plan.passes[2].camera && !plan.passes[2].shadow);
        assert_eq!(BatchPlan::new(&frame, false, 100).order, [0, 2]);
    }

    #[test]
    fn scene3d_instance_capacity_reclaims_peaks_without_threshold_churn() {
        let limit = 1000;
        let mut retained = 0;
        let mut allocations = 0;
        for required in [1, 513, 999, 501, 251, 500, 251] {
            let next = retained_capacity(retained, required, limit);
            allocations += usize::from(next != retained);
            retained = next;
            assert!((required..=limit).contains(&retained));
        }
        assert_eq!(allocations, 2);
        assert_eq!(retained, 1000);
        retained = retained_capacity(retained, 250, limit);
        assert_eq!(retained, 256);
        for required in [249, 250, 251, 256, 249, 251] {
            assert_eq!(retained_capacity(retained, required, limit), retained);
        }
        retained = retained_capacity(retained, 1, limit);
        assert_eq!(retained, 1);

        let mut retained = 0;
        for required in (1..=limit).chain((1..=limit).rev()) {
            retained = retained_capacity(retained, required, limit);
            assert!(retained >= required && retained <= limit);
            assert!(retained / 4 < required);
        }
        assert_eq!(retained, 1);
        let huge = usize::MAX / 2 + 1;
        assert_eq!(retained_capacity(usize::MAX, huge, usize::MAX), usize::MAX);
        assert_eq!(retained_capacity(usize::MAX, 1, usize::MAX), 1);
        assert_eq!(retained_capacity(usize::MAX, 17, 100), 32);
    }

    fn retained(
        cache: &BatchPlanCache,
        frame: &Scene3dFrame,
        color: bool,
        limit: usize,
    ) -> Arc<BatchPlan> {
        cache.entries[&PlanKey::new(frame, color, limit)]
            .plan
            .clone()
    }

    #[test]
    fn scene3d_batches_keep_material_snapshots_separate() {
        let mut first = object();
        let mut second = first.clone();
        assert!(compatible(&first, &second));
        first.custom_material = Some(gpui::MeshMaterial3d::new(Arc::new(())));
        assert!(!compatible(&first, &second));
        second.custom_material = first.custom_material.clone();
        assert!(compatible(&first, &second));
        let retained = second.clone();
        second.custom_material = Some(gpui::MeshMaterial3d::new(Arc::new(())));
        assert!(!compatible(&first, &second));
        assert!(compatible(&first, &retained));
        for color in [false, true] {
            let input = frame(&[first.clone(), retained.clone(), second.clone()]);
            let plan = BatchPlan::new(&input, color, 1024);
            assert_eq!(plan.batches.len(), 2);
            assert_eq!(plan.batches[0].len(), 2);
            assert_eq!(plan.batches[1].len(), 1);
        }
    }

    #[test]
    fn scene3d_batch_cache_reuses_snapshots_and_invalidates_camera_and_object_changes() {
        let object = object();
        let mut outside = object.clone();
        outside.model[3][0] = 5.;
        let mut input = frame(&[object.clone(), object, outside]);
        let mut cache = BatchPlanCache::default();
        cache.prepare([&input], true, 100);
        let saved = retained(&cache, &input, true, 100);
        assert_eq!(saved.order, [0, 1]);
        assert_eq!(saved.batches, [0..2]);
        input.ambient = 0.8;
        input.light[3] = 2.;
        cache.prepare([&input.clone()], true, 100);
        assert!(Arc::ptr_eq(&saved, &retained(&cache, &input, true, 100)));

        input.view_projection[3][0] = -5.;
        cache.prepare([&input], true, 100);
        assert_eq!(cache.get(&input, true, 100).order, [2]);
        assert!(!Arc::ptr_eq(&saved, &retained(&cache, &input, true, 100)));
        input.view_projection = IDENTITY;
        cache.prepare([&input], true, 100);
        let before_edit = input.clone();
        let before_plan = retained(&cache, &input, true, 100);
        Arc::make_mut(&mut input.objects)[0].model[3][0] = 8.;
        cache.prepare([&input, &before_edit], true, 100);
        assert_eq!(cache.get(&input, true, 100).order, [1]);
        assert_eq!(before_plan.order, [0, 1]);
        assert!(Arc::ptr_eq(
            &before_plan,
            &retained(&cache, &before_edit, true, 100)
        ));
        assert!(!Arc::ptr_eq(
            &before_plan,
            &retained(&cache, &input, true, 100)
        ));

        let mut material = before_edit.clone();
        Arc::make_mut(&mut material.objects)[0].unlit = false;
        cache.prepare([&material], true, 100);
        assert_eq!(cache.get(&material, true, 100).batches, [0..1, 1..2]);
    }

    #[test]
    fn scene3d_batch_cache_separates_color_shadows_limits_and_transparency_order() {
        let object = object();
        let mut caster = object.clone();
        caster.model[3][0] = 4.;
        let mut near = object.clone();
        near.alpha_mode = AlphaMode3d::Blend;
        near.sort_depth = 1.;
        let mut far = near.clone();
        far.sort_depth = 3.;
        let mut input = frame(&[object.clone(), object, caster, near, far]);
        let mut shadow_matrix = IDENTITY;
        shadow_matrix[3][0] = -4.;
        input.directional_shadow = Some(gpui::DirectionalShadow3d {
            light_index: 0,
            view_projection: shadow_matrix,
            resolution: 256,
            depth_bias: 0.,
            normal_bias: 0.,
            softness: 0.,
        });
        let mut cache = BatchPlanCache::default();
        cache.prepare([&input], true, 100);
        let color = retained(&cache, &input, true, 100);
        assert_eq!(color.order, [0, 1, 2, 4, 3]);
        assert_eq!(color.statistics(&input).shadow_instances, 1);
        cache.prepare([&input], true, 1);
        assert_eq!(cache.get(&input, true, 1).batches.len(), 5);
        assert!(!Arc::ptr_eq(&color, &retained(&cache, &input, true, 1)));
        cache.prepare([&input], false, 100);
        let data = retained(&cache, &input, false, 100);
        assert_eq!(data.order, [0, 1, 3, 4]);
        assert_eq!(data.statistics(&input).shadow_instances, 0);
        input.directional_shadow.as_mut().unwrap().view_projection = IDENTITY;
        cache.prepare([&input], false, 100);
        assert!(Arc::ptr_eq(&data, &retained(&cache, &input, false, 100)));
        cache.prepare([&input], true, 100);
        assert_eq!(cache.get(&input, true, 100).order, [0, 1, 4, 3]);
        assert_eq!(
            cache
                .get(&input, true, 100)
                .statistics(&input)
                .shadow_instances,
            2
        );
    }

    #[test]
    fn scene3d_batch_cache_shares_active_channels_and_releases_inactive_snapshots() {
        let source = object();
        let first = frame(&[source.clone(), source]);
        let mut second = first.clone();
        second.view_projection[3][0] = 10.;
        let mut ids = BatchPlanCache::default();
        ids.prepare([&first, &second, &first], false, 100);
        assert_eq!(ids.entries.len(), 2);
        let first_plan = retained(&ids, &first, false, 100);
        let second_plan = Arc::downgrade(&retained(&ids, &second, false, 100));
        let mut depth = BatchPlanCache::default();
        depth.reuse_from(&ids);
        depth.prepare([&first], false, 100);
        assert_eq!(depth.entries.len(), 1);
        assert!(Arc::ptr_eq(
            &first_plan,
            &retained(&depth, &first, false, 100)
        ));
        ids.prepare([&first], false, 100);
        assert!(second_plan.upgrade().is_none());

        let objects = Arc::downgrade(&first.objects);
        drop(first);
        drop(second);
        ids.prepare([], false, 100);
        assert!(objects.upgrade().is_some());
        depth.prepare([], false, 100);
        assert!(objects.upgrade().is_none());
        assert!(ids.entries.is_empty() && depth.entries.is_empty());
    }
}
