use crate::{AffineTransform, Material, Mesh, Object, ObjectId, Scene};
use anyhow::{Result, ensure};
pub use gpui::{
    EditHiddenStyle3d as EditHiddenStyle, EditLine3d as EditLine, EditPoint3d as EditPoint,
    EditStyle3d as EditStyle,
};
use std::{collections::HashSet, sync::Arc};

/// Independent self-occlusion geometry for an application-defined object group.
/// Members identify final-surface objects excluded only from this group's output.
#[derive(Clone)]
pub struct EditOcclusionGroup {
    id: u64,
    members: Vec<ObjectId>,
    occluders: Vec<Object>,
    points: Vec<EditPoint>,
    lines: Vec<EditLine>,
    pixel_scale: f32,
}

impl EditOcclusionGroup {
    pub fn new(id: u64, members: impl IntoIterator<Item = ObjectId>) -> Self {
        Self {
            id,
            members: members.into_iter().collect(),
            occluders: Vec::new(),
            points: Vec::new(),
            lines: Vec::new(),
            pixel_scale: 1.,
        }
    }

    /// Appends points in draw order, after the group's lines.
    pub fn points(mut self, points: impl IntoIterator<Item = EditPoint>) -> Self {
        self.points.extend(points);
        self
    }

    /// Appends segments in draw order. Element IDs must be unique within the group.
    pub fn lines(mut self, lines: impl IntoIterator<Item = EditLine>) -> Self {
        self.lines.extend(lines);
        self
    }

    /// Logical-to-physical scale for direct output. Viewports also apply window DPI.
    pub fn pixel_scale(mut self, scale: f32) -> Self {
        self.pixel_scale = scale;
        self
    }

    /// Adds opaque, double-sided auxiliary geometry in a validated world transform.
    /// An empty set intentionally removes self-occlusion; other groups still occlude.
    pub fn mesh(mut self, mesh: Mesh, world: AffineTransform) -> Self {
        let mut object = Object::new(
            mesh,
            Material::color(gpui::rgb(0xffffff))
                .double_sided(true)
                .alpha_mode(crate::AlphaMode::Opaque),
        );
        object.world = Some(world);
        self.occluders.push(object);
        self
    }
}

impl Scene {
    #[cfg(all(feature = "wgpu", not(target_family = "wasm")))]
    pub(crate) fn occlusion_element_group_count(&self) -> usize {
        self.occlusion_groups
            .iter()
            .filter(|g| !g.points.is_empty() || !g.lines.is_empty())
            .count()
    }
    /// Requests separate grouped ID/depth outputs on headless renders and viewport
    /// pick captures. Does not change the displayed geometry or primary picking.
    pub fn edit_occlusion(mut self, groups: impl IntoIterator<Item = EditOcclusionGroup>) -> Self {
        Arc::make_mut(&mut self.preparation_revision);
        self.occlusion_groups = groups.into_iter().collect();
        self
    }

    pub(crate) fn plan_occlusion(&self, aspect: f32) -> Result<Arc<[gpui::OcclusionGroup3d]>> {
        let mut group_ids = HashSet::new();
        let mut assigned = HashSet::new();
        self.occlusion_groups
            .iter()
            .map(|group| {
                ensure!(
                    group_ids.insert(group.id),
                    "duplicate edit occlusion group ID"
                );
                ensure!(
                    !group.members.is_empty(),
                    "edit occlusion group requires members"
                );
                let members = group
                    .members
                    .iter()
                    .map(|id| {
                        let matches: Vec<_> = self
                            .objects
                            .iter()
                            .enumerate()
                            .filter(|(_, object)| object.id.as_ref() == Some(id))
                            .collect();
                        ensure!(
                            matches.len() == 1,
                            "edit occlusion member must identify exactly one object"
                        );
                        let output_id = u32::try_from(matches[0].0 + 1)?;
                        ensure!(
                            assigned.insert(output_id),
                            "object belongs to multiple edit occlusion groups or is repeated"
                        );
                        Ok(output_id)
                    })
                    .collect::<Result<Vec<_>>>()?;
                let auxiliary = group
                    .occluders
                    .iter()
                    .cloned()
                    .fold(Scene::new().camera(self.camera), Scene::object);
                let mut draws = auxiliary.plan_frame(aspect, None)?.objects.to_vec();
                for (index, draw) in draws.iter_mut().enumerate() {
                    draw.output_id = u32::try_from(
                        self.objects
                            .len()
                            .checked_add(index + 1)
                            .ok_or_else(|| anyhow::anyhow!("too many occlusion meshes"))?,
                    )?;
                }
                let definition = gpui::OcclusionGroup3d {
                    id: group.id,
                    members: members.into(),
                    occluders: draws.into(),
                    points: group.points.clone().into(),
                    lines: group.lines.clone().into(),
                    pixel_scale: group.pixel_scale,
                };
                ensure!(
                    definition.elements_are_valid(),
                    "invalid edit overlay elements"
                );
                Ok(definition)
            })
            .collect::<Result<Vec<_>>>()
            .map(Into::into)
    }
}
