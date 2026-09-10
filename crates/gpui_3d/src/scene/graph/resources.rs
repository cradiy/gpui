use super::*;

impl SceneGraph {
    /// Replaces only this node's material; shared geometry is unchanged.
    pub fn set_material(
        &mut self,
        handle: NodeHandle,
        material: Material,
    ) -> Result<(), SceneError> {
        let key = self.key(handle)?;
        let (_, current) = self.nodes[key]
            .node
            .surface
            .as_mut()
            .ok_or(SceneError::NoMesh(handle))?;
        *current = material;
        self.revision += 1;
        Ok(())
    }

    /// Replaces materials after validating every target, without partial graph edits.
    /// Duplicate, foreign, expired, and non-mesh targets return errors. Hidden mesh
    /// nodes are valid targets. A nonempty batch increments the revision once;
    /// an empty batch leaves it unchanged. Geometry and other node properties remain
    /// unchanged, and existing snapshots retain their materials.
    /// Material parameters and resource compatibility are checked during preparation,
    /// not by this operation. Iterator side effects are not rolled back.
    pub fn set_materials(
        &mut self,
        materials: impl IntoIterator<Item = (NodeHandle, Material)>,
    ) -> Result<(), SceneError> {
        let mut replacements = HashMap::new();
        for (handle, material) in materials {
            let key = self.key(handle)?;
            if self.nodes[key].node.surface.is_none() {
                return Err(SceneError::NoMesh(handle));
            }
            if replacements.insert(key, material).is_some() {
                return Err(SceneError::DuplicateMaterial(handle));
            }
        }
        if replacements.is_empty() {
            return Ok(());
        }
        for (key, material) in replacements {
            self.nodes[key].node.surface.as_mut().unwrap().1 = material;
        }
        self.revision += 1;
        Ok(())
    }

    /// Replaces an existing mesh and its local bounds, retaining material, identity,
    /// hierarchy and transform. Previous evaluated scenes keep their geometry.
    pub fn set_mesh(&mut self, handle: NodeHandle, mesh: Mesh) -> Result<(), SceneError> {
        let key = self.key(handle)?;
        let node = &mut self.nodes[key].node;
        let (current, _) = node.surface.as_mut().ok_or(SceneError::NoMesh(handle))?;
        node.bounds = Some(mesh.bounds());
        *current = mesh;
        self.revision += 1;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AlphaMode, PreparationCache, Ray, ResolvedTexture, TextureState};
    use gpui::{rgb, rgba};

    #[test]
    fn material_batches_update_visible_and_hidden_nodes_without_changing_resources() {
        let mesh = Mesh::plane();
        let mut graph = SceneGraph::new();
        let a = graph
            .insert(
                None,
                Node::new()
                    .id("a")
                    .mesh(mesh.clone(), Material::color(rgb(0xffffff))),
            )
            .unwrap();
        let b = graph
            .insert(
                Some(a),
                Node::new()
                    .id("b")
                    .visible(false)
                    .mesh(mesh.clone(), Material::color(rgb(0xffffff))),
            )
            .unwrap();
        let c = graph
            .insert(
                None,
                Node::new()
                    .id("c")
                    .mesh(mesh.clone(), Material::color(rgb(0xffffff)))
                    .transform(AffineTransform::from_translation([1., 0., 0.]).unwrap()),
            )
            .unwrap();
        let old = graph.evaluate().unwrap();
        let old_scene = old.scene(Camera::default());
        let ray = Ray::new([0., 0., 3.], [0., 0., -1.]).unwrap();
        assert_eq!(old_scene.raycast(ray).unwrap().node, Some(a));
        let mut cache = PreparationCache::new();
        let resolve = |_: crate::TextureRequest<'_>| Ok(TextureState::Ready(ResolvedTexture::None));
        let old_frame = cache.prepare(&old_scene, 1., None, resolve).unwrap();
        let revision = graph.revision();
        graph
            .set_materials([
                (b, Material::color(rgb(0xff0000))),
                (
                    a,
                    Material::color(rgba(0x00ff0000)).alpha_mode(AlphaMode::Mask),
                ),
            ])
            .unwrap();
        assert_eq!(graph.revision(), revision + 1);
        let changed = graph.evaluate().unwrap();
        let scene = changed.scene(Camera::default());
        assert!(scene.raycast(ray).is_none());
        assert_eq!(old_scene.raycast(ray).unwrap().node, Some(a));
        assert_eq!(changed.bounds(), old.bounds());
        assert_eq!(changed.node(b).unwrap().parent, Some(a));
        assert!(!changed.node(b).unwrap().visible);
        assert_eq!(changed.node(c).unwrap().world, old.node(c).unwrap().world);
        let frame = cache.prepare(&scene, 1., None, resolve).unwrap();
        assert!(!Arc::ptr_eq(&old_frame, &frame));
        assert_eq!(frame.frame().objects[0].color, rgba(0x00ff0000));
        assert_eq!(old_frame.frame().objects[0].color, rgb(0xffffff));
        assert_eq!(frame.frame().objects[1].color, rgb(0xffffff));
        for handle in [a, b, c] {
            let (resource, _) = graph.node(handle).unwrap().surface.as_ref().unwrap();
            assert!(Arc::ptr_eq(&resource.0, &mesh.0));
        }
        graph.set_visible(b, true).unwrap();
        let shown = graph.evaluate().unwrap().scene(Camera::default());
        assert_eq!(shown.objects[1].material.color, rgb(0xff0000));
        assert_eq!(shown.objects[1].id, Some("b".into()));
        assert_eq!(graph.find(&"a".into()), Some(a));
    }

    #[test]
    fn invalid_material_targets_leave_all_nodes_and_revision_unchanged() {
        let mut graph = SceneGraph::new();
        let group = graph.insert(None, Node::new()).unwrap();
        let a = graph
            .insert(
                Some(group),
                Node::new().mesh(Mesh::cube(), Material::color(rgb(0xffffff))),
            )
            .unwrap();
        let b = graph
            .insert(
                Some(group),
                Node::new().mesh(Mesh::cube(), Material::color(rgb(0x808080))),
            )
            .unwrap();
        let expired = graph.insert(None, Node::new()).unwrap();
        graph.remove_subtree(expired).unwrap();
        let foreign = SceneGraph::new().insert(None, Node::new()).unwrap();
        let revision = graph.revision();
        for (invalid, expected) in [
            (expired, SceneError::InvalidHandle(expired)),
            (foreign, SceneError::InvalidHandle(foreign)),
            (group, SceneError::NoMesh(group)),
            (a, SceneError::DuplicateMaterial(a)),
        ] {
            let error = graph
                .set_materials([a, b, invalid].map(|node| (node, Material::color(rgb(0xff0000)))))
                .unwrap_err();
            assert_eq!(error, expected);
            assert_eq!(graph.revision(), revision);
            assert_eq!(
                graph.node(a).unwrap().surface.as_ref().unwrap().1.color,
                rgb(0xffffff)
            );
            assert_eq!(
                graph.node(b).unwrap().surface.as_ref().unwrap().1.color,
                rgb(0x808080)
            );
        }
        graph.set_materials([]).unwrap();
        assert_eq!(graph.revision(), revision);
        graph
            .set_materials([
                (b, Material::color(rgb(0x0000ff))),
                (a, Material::color(rgb(0xff0000))),
            ])
            .unwrap();
        assert_eq!(graph.revision(), revision + 1);
        assert_eq!(
            graph.node(a).unwrap().surface.as_ref().unwrap().1.color,
            rgb(0xff0000)
        );
        assert_eq!(
            graph.node(b).unwrap().surface.as_ref().unwrap().1.color,
            rgb(0x0000ff)
        );
    }
}
