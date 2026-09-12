use super::*;

/// A replacement node transform in parent-local or world coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TransformOverride {
    /// Composed with the parent's final world transform.
    Local(AffineTransform),
    /// Used directly, independently of the parent's transform.
    World(AffineTransform),
}

impl SceneGraph {
    /// Resolves world matrices, cameras, lights, visibility, and bounds in parent-first order.
    /// No playback history, window, layout, or GPU work is required.
    pub fn evaluate(&self) -> Result<EvaluatedScene, SceneError> {
        self.evaluate_with_transforms([])
    }

    /// Evaluates replacement local transforms without changing the graph or its revision.
    /// Omitted nodes use their authored transforms. Duplicate, foreign, and expired
    /// handles are rejected. Each result owns independent world bounds and query indices.
    pub fn evaluate_with_transforms(
        &self,
        transforms: impl IntoIterator<Item = (NodeHandle, AffineTransform)>,
    ) -> Result<EvaluatedScene, SceneError> {
        self.evaluate_with_overrides(transforms, [])
    }

    /// Evaluates local transform and mesh replacements without changing the graph.
    /// Mesh targets must already contain geometry. Materials, IDs, visibility,
    /// cameras, lights and shadow settings retain their node properties.
    /// Duplicate entries within either list, invalid handles and missing meshes
    /// return errors. Success and failure both leave the graph revision unchanged.
    pub fn evaluate_with_overrides(
        &self,
        transforms: impl IntoIterator<Item = (NodeHandle, AffineTransform)>,
        meshes: impl IntoIterator<Item = (NodeHandle, Mesh)>,
    ) -> Result<EvaluatedScene, SceneError> {
        self.evaluate_transform_overrides(
            transforms
                .into_iter()
                .map(|(node, transform)| (node, TransformOverride::Local(transform))),
            meshes,
        )
    }

    /// Evaluates mixed local/world transforms without mutating the graph.
    /// Input order is irrelevant. Omitted nodes use authored local transforms
    /// composed with their parent's final world transform. World overrides do not
    /// bypass inherited visibility. Bounds, cameras, lights and queries use the
    /// resulting poses. Duplicate, foreign and expired handles return errors.
    /// This performs no constraint solving or skinning; attach final deformed
    /// meshes with `EvaluatedScene::with_meshes`.
    pub fn evaluate_with_transform_overrides(
        &self,
        transforms: impl IntoIterator<Item = (NodeHandle, TransformOverride)>,
    ) -> Result<EvaluatedScene, SceneError> {
        self.evaluate_transform_overrides(transforms, [])
    }

    fn evaluate_transform_overrides(
        &self,
        transforms: impl IntoIterator<Item = (NodeHandle, TransformOverride)>,
        meshes: impl IntoIterator<Item = (NodeHandle, Mesh)>,
    ) -> Result<EvaluatedScene, SceneError> {
        let mut overrides = HashMap::new();
        for (handle, transform) in transforms {
            let key = self.key(handle)?;
            if overrides.insert(key, transform).is_some() {
                return Err(SceneError::DuplicateTransform(handle));
            }
        }
        self.evaluate_using(meshes, |handle, parent| {
            let local = match overrides.get(&handle.key) {
                Some(TransformOverride::World(world)) => return Ok(*world),
                Some(TransformOverride::Local(local)) => *local,
                None => self.nodes[handle.key].node.local,
            };
            parent
                .compose(local)
                .map_err(|source| SceneError::InvalidTransform {
                    node: handle,
                    source,
                })
        })
    }

    pub(in crate::scene) fn evaluate_using(
        &self,
        meshes: impl IntoIterator<Item = (NodeHandle, Mesh)>,
        mut world_transform: impl FnMut(
            NodeHandle,
            AffineTransform,
        ) -> Result<AffineTransform, SceneError>,
    ) -> Result<EvaluatedScene, SceneError> {
        let mut replacements = HashMap::new();
        for (handle, mesh) in meshes {
            let key = self.key(handle)?;
            if self.nodes[key].node.surface.is_none() {
                return Err(SceneError::NoMesh(handle));
            }
            if replacements.insert(key, mesh).is_some() {
                return Err(SceneError::DuplicateMesh(handle));
            }
        }
        let mut evaluated = EvaluatedScene {
            preparation_revision: Arc::new(()),
            revision: self.revision,
            nodes: Vec::with_capacity(self.len()),
            indices: HashMap::with_capacity(self.len()),
            objects: Vec::new(),
            lights: None,
            bounds: None,
            spatial_index: Arc::default(),
            spatial_source: Arc::default(),
            constraint_status: HashMap::new(),
        };
        let mut spatial_source = Vec::new();
        let mut pending = self
            .roots
            .iter()
            .rev()
            .map(|key| (*key, None))
            .collect::<Vec<_>>();
        while let Some((key, parent_index)) = pending.pop() {
            let entry = &self.nodes[key];
            let node = &entry.node;
            let handle = self.handle(key);
            let parent: Option<&EvaluatedNode> = parent_index.map(|i| &evaluated.nodes[i]);
            let world = world_transform(
                handle,
                parent.map_or(AffineTransform::IDENTITY, |parent| parent.world),
            )?;
            let visible = !node.hidden && parent.is_none_or(|parent| parent.visible);
            let camera = node
                .camera
                .map(|camera| camera.transformed(world))
                .transpose()
                .map_err(|source| SceneError::InvalidCamera {
                    node: handle,
                    source,
                })?;
            let light = node
                .light
                .map(|light| light.transformed(world))
                .transpose()
                .map_err(|source| SceneError::InvalidLight {
                    node: handle,
                    source,
                })?;
            if let Some(light) = light {
                let lights = evaluated.lights.get_or_insert_with(Vec::new);
                if visible {
                    lights.push((handle, light));
                }
            }
            let mesh_override = replacements.get(&key);
            let local_bounds = mesh_override.map(Mesh::bounds).or(node.bounds);
            let bounds = local_bounds
                .map(|bounds| bounds.transformed(world))
                .transpose()
                .map_err(|source| SceneError::InvalidTransform {
                    node: handle,
                    source,
                })?;
            if let Some(local_bounds) = local_bounds {
                spatial_source.push(crate::spatial::bvh::IndexObject::node(
                    handle,
                    local_bounds,
                    world,
                    visible.then_some(evaluated.objects.len()),
                ));
            }
            if visible && let Some((mesh, material)) = &node.surface {
                let mut object =
                    Object::new(mesh_override.unwrap_or(mesh).clone(), material.clone());
                object.id = node.id.clone();
                object.node = Some(handle);
                object.world = Some(world);
                object.pick_behavior = node.picking;
                object.cast_shadows = !node.no_shadow_cast;
                object.receive_shadows = !node.no_shadow_receive;
                evaluated.objects.push(object);
                evaluated.bounds = union(evaluated.bounds, bounds);
            }
            let index = evaluated.nodes.len();
            evaluated.indices.insert(handle, index);
            evaluated.nodes.push(EvaluatedNode {
                handle,
                id: node.id.clone(),
                parent: entry.parent.map(|key| self.handle(key)),
                world,
                visible,
                camera,
                light,
                bounds,
                subtree_bounds: bounds,
            });
            pending.extend(entry.children.iter().rev().map(|key| (*key, Some(index))));
        }
        for i in (0..evaluated.nodes.len()).rev() {
            let node = &evaluated.nodes[i];
            if let Some(parent) = node.parent {
                let bounds = node.subtree_bounds;
                let parent = &mut evaluated.nodes[evaluated.indices[&parent]];
                parent.subtree_bounds = union(parent.subtree_bounds, bounds);
            }
        }
        evaluated.spatial_source = Arc::new(spatial_source);
        Ok(evaluated)
    }
}
