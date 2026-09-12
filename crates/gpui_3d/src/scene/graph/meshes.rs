use super::*;

impl EvaluatedScene {
    /// Replaces mesh-local geometry on an existing evaluated pose without a graph
    /// or another transform/constraint evaluation. Omitted geometry, world poses,
    /// cameras, lights, visibility, materials, identities and constraint outcomes
    /// remain unchanged. Bounds and spatial-query inputs follow replacement meshes.
    ///
    /// Handles are checked against this snapshot, not the current graph. Hidden
    /// mesh nodes are valid targets. Duplicate, absent, non-mesh targets or invalid
    /// transformed bounds return errors without changing this or older snapshots.
    /// Empty input returns a clone sharing preparation and spatial-query identity.
    pub fn with_meshes(
        &self,
        meshes: impl IntoIterator<Item = (NodeHandle, Mesh)>,
    ) -> Result<Self, SceneError> {
        let mut replacements = HashMap::new();
        for (handle, mesh) in meshes {
            let node = self.node(handle).ok_or(SceneError::InvalidHandle(handle))?;
            if node.bounds.is_none() {
                return Err(SceneError::NoMesh(handle));
            }
            if replacements.contains_key(&handle) {
                return Err(SceneError::DuplicateMesh(handle));
            }
            let local = mesh.bounds();
            let world =
                local
                    .transformed(node.world)
                    .map_err(|source| SceneError::InvalidTransform {
                        node: handle,
                        source,
                    })?;
            replacements.insert(handle, (mesh, local, world));
        }
        if replacements.is_empty() {
            return Ok(self.clone());
        }
        let mut result = self.clone();
        result.preparation_revision = Arc::new(());
        result.spatial_index = Arc::default();
        result.spatial_source = Arc::new(
            self.spatial_source
                .iter()
                .map(|entry| {
                    let Some(handle) = entry.node_handle() else {
                        return *entry;
                    };
                    match replacements.get(&handle) {
                        Some((_, local, _)) => {
                            entry.with_bounds(*local, self.node(handle).unwrap().world)
                        }
                        None => *entry,
                    }
                })
                .collect(),
        );
        for object in &mut result.objects {
            if let Some(handle) = object.node
                && let Some((mesh, _, _)) = replacements.get(&handle)
            {
                object.mesh = mesh.clone();
            }
        }
        result.bounds = None;
        for node in &mut result.nodes {
            if let Some((_, _, world)) = replacements.get(&node.handle) {
                node.bounds = Some(*world);
            }
            node.subtree_bounds = node.bounds;
            if node.visible {
                result.bounds = union(result.bounds, node.bounds);
            }
        }
        for index in (0..result.nodes.len()).rev() {
            let node = &result.nodes[index];
            if let Some(parent) = node.parent {
                let bounds = node.subtree_bounds;
                let parent = &mut result.nodes[result.indices[&parent]];
                parent.subtree_bounds = union(parent.subtree_bounds, bounds);
            }
        }
        Ok(result)
    }
}
