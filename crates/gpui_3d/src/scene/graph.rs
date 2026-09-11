mod evaluation;
pub use evaluation::TransformOverride;
#[cfg(test)]
mod evaluation_tests;
mod meshes;
mod resources;

use crate::{
    Aabb, AffineTransform, AimError, Camera, CameraError, ConstraintStatus, LightError, Material,
    Mesh, Object, ObjectId, PickBehavior, PunctualLight, Scene, TransformError,
};
use slotmap::{SlotMap, new_key_type};
use std::{
    collections::{HashMap, HashSet},
    fmt,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

new_key_type! { struct NodeKey; }

/// Graph-scoped generational identity. Not a project serialization ID.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NodeHandle {
    graph: u64,
    key: NodeKey,
}

/// A transformed hierarchy node with optional mesh, camera, and light properties.
#[derive(Clone, Default)]
pub struct Node {
    id: Option<ObjectId>,
    local: AffineTransform,
    hidden: bool,
    surface: Option<(Mesh, Material)>,
    camera: Option<Camera>,
    light: Option<PunctualLight>,
    bounds: Option<Aabb>,
    picking: PickBehavior,
    no_shadow_cast: bool,
    no_shadow_receive: bool,
}

impl Node {
    pub fn new() -> Self {
        Self::default()
    }
    /// Sets an application ID, unique within the graph.
    pub fn id(mut self, id: impl Into<ObjectId>) -> Self {
        self.id = Some(id.into());
        self
    }
    pub fn transform(mut self, transform: AffineTransform) -> Self {
        self.local = transform;
        self
    }
    /// Visibility is inherited; a hidden ancestor hides every descendant.
    pub fn visible(mut self, visible: bool) -> Self {
        self.hidden = !visible;
        self
    }
    /// Geometry is shared; this node owns its material value.
    pub fn mesh(mut self, mesh: Mesh, material: Material) -> Self {
        self.bounds = Some(mesh.bounds());
        self.surface = Some((mesh, material));
        self
    }
    /// Borrows this node's geometry and material, including when hidden.
    /// Groups return `None`. Clone the material before editing it through the graph;
    /// this does not copy shared mesh or decoded-image storage.
    pub fn surface(&self) -> Option<(&Mesh, &Material)> {
        self.surface
            .as_ref()
            .map(|(mesh, material)| (mesh, material))
    }
    pub fn pick_behavior(mut self, behavior: PickBehavior) -> Self {
        self.picking = behavior;
        self
    }
    /// Attaches a camera whose eye, target and up are in this node's local space.
    /// Projection and clip distances are not scaled. Validated during evaluation.
    pub fn camera(mut self, camera: Camera) -> Self {
        self.camera = Some(camera);
        self
    }
    /// Attaches a local-space light. Inherited visibility controls its emission.
    /// Physical parameters are not scaled. Validated during evaluation.
    pub fn light(mut self, light: PunctualLight) -> Self {
        self.light = Some(light);
        self
    }
    pub fn local_camera(&self) -> Option<Camera> {
        self.camera
    }
    pub fn local_light(&self) -> Option<PunctualLight> {
        self.light
    }
    pub fn object_id(&self) -> Option<&ObjectId> {
        self.id.as_ref()
    }
    /// Controls this mesh's shadow casting, not its descendants. Defaults to true.
    pub fn cast_shadows(mut self, enabled: bool) -> Self {
        self.no_shadow_cast = !enabled;
        self
    }
    /// Controls this mesh's shadow reception, not its descendants. Defaults to true.
    pub fn receive_shadows(mut self, enabled: bool) -> Self {
        self.no_shadow_receive = !enabled;
        self
    }
    pub fn local_transform(&self) -> AffineTransform {
        self.local
    }
    pub fn is_visible(&self) -> bool {
        !self.hidden
    }
    pub fn local_bounds(&self) -> Option<Aabb> {
        self.bounds
    }
}

/// Transform policy for changing a node's parent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReparentMode {
    KeepLocal,
    /// Retains the full world matrix, including shear, within floating-point precision.
    KeepWorld,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SceneError {
    InvalidHandle(NodeHandle),
    DuplicateId(ObjectId),
    DuplicateTransform(NodeHandle),
    DuplicateMesh(NodeHandle),
    DuplicateMaterial(NodeHandle),
    DuplicateConstraint(NodeHandle),
    InvalidConstraintTarget {
        node: NodeHandle,
        target: NodeHandle,
    },
    ConstraintCycle(Vec<NodeHandle>),
    InvalidAim {
        node: NodeHandle,
        source: AimError,
    },
    Cycle(NodeHandle),
    NoMesh(NodeHandle),
    NoCamera(NodeHandle),
    InvalidCamera {
        node: NodeHandle,
        source: CameraError,
    },
    InvalidLight {
        node: NodeHandle,
        source: LightError,
    },
    InvalidTransform {
        node: NodeHandle,
        source: TransformError,
    },
}
impl fmt::Display for SceneError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidHandle(node) => write!(f, "unknown or expired node {node:?}"),
            Self::DuplicateId(id) => write!(f, "duplicate node ID {id:?}"),
            Self::DuplicateTransform(node) => {
                write!(f, "duplicate local transform for node {node:?}")
            }
            Self::DuplicateMesh(node) => write!(f, "duplicate mesh override for node {node:?}"),
            Self::DuplicateMaterial(node) => {
                write!(f, "duplicate material replacement for node {node:?}")
            }
            Self::DuplicateConstraint(node) => write!(f, "duplicate constraint for node {node:?}"),
            Self::InvalidConstraintTarget { node, target } => {
                write!(
                    f,
                    "constraint on node {node:?} references unknown or expired target {target:?}"
                )
            }
            Self::ConstraintCycle(path) => write!(f, "transform dependency cycle: {path:?}"),
            Self::InvalidAim { node, source } => {
                write!(f, "aim constraint on node {node:?}: {source}")
            }
            Self::Cycle(node) => write!(f, "parent assignment would create a cycle at {node:?}"),
            Self::NoMesh(node) => write!(f, "node {node:?} has no mesh"),
            Self::NoCamera(node) => write!(f, "node {node:?} has no camera"),
            Self::InvalidCamera { node, source } => write!(f, "camera node {node:?}: {source}"),
            Self::InvalidLight { node, source } => write!(f, "light node {node:?}: {source}"),
            Self::InvalidTransform { node, source } => write!(f, "node {node:?}: {source}"),
        }
    }
}
impl std::error::Error for SceneError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InvalidTransform { source, .. } => Some(source),
            Self::InvalidCamera { source, .. } => Some(source),
            Self::InvalidLight { source, .. } => Some(source),
            Self::InvalidAim { source, .. } => Some(source),
            _ => None,
        }
    }
}

struct Entry {
    node: Node,
    parent: Option<NodeKey>,
    children: Vec<NodeKey>,
}

/// A node captured in a reusable subtree. Parent handles refer only to other
/// entries in the same snapshot; the captured root has no parent.
#[derive(Clone)]
pub struct SubtreeNode {
    pub source: NodeHandle,
    pub parent: Option<NodeHandle>,
    pub node: Node,
}

/// Immutable local hierarchy, independent of its source graph's lifetime.
/// Cloning the snapshot shares its storage and mesh/image allocations.
#[derive(Clone)]
pub struct SceneSubtree {
    nodes: Arc<[SubtreeNode]>,
}
impl SceneSubtree {
    pub fn root(&self) -> NodeHandle {
        self.nodes[0].source
    }
    /// Parent-first order, preserving the source's sibling order.
    pub fn nodes(&self) -> &[SubtreeNode] {
        &self.nodes
    }
}

/// Non-owning handles created by one instantiation. Dropping this value does
/// not remove nodes. Handles expire through ordinary scene graph mutations.
pub struct SubtreeInstance {
    root: NodeHandle,
    nodes: HashMap<NodeHandle, NodeHandle>,
}
impl SubtreeInstance {
    pub fn root(&self) -> NodeHandle {
        self.root
    }
    /// Looks up an instance node by its source-snapshot handle. This does not
    /// check whether the destination node still exists in the graph.
    pub fn node(&self, source: NodeHandle) -> Option<NodeHandle> {
        self.nodes.get(&source).copied()
    }
    /// Source-to-instance pairs, in unspecified order.
    pub fn mappings(&self) -> impl Iterator<Item = (NodeHandle, NodeHandle)> + '_ {
        self.nodes.iter().map(|(source, node)| (*source, *node))
    }
}

/// Editable hierarchy. Evaluation requires neither a window nor a GPU.
pub struct SceneGraph {
    identity: u64,
    nodes: SlotMap<NodeKey, Entry>,
    ids: HashMap<ObjectId, NodeKey>,
    roots: Vec<NodeKey>,
    revision: u64,
}

impl Default for SceneGraph {
    fn default() -> Self {
        Self::new()
    }
}

impl SceneGraph {
    pub fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let identity = NEXT
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
            .expect("scene graph identity exhausted");
        Self {
            identity,
            nodes: SlotMap::with_key(),
            ids: HashMap::new(),
            roots: Vec::new(),
            revision: 0,
        }
    }
    fn handle(&self, key: NodeKey) -> NodeHandle {
        NodeHandle {
            graph: self.identity,
            key,
        }
    }
    fn key(&self, node: NodeHandle) -> Result<NodeKey, SceneError> {
        if node.graph != self.identity || !self.nodes.contains_key(node.key) {
            return Err(SceneError::InvalidHandle(node));
        }
        Ok(node.key)
    }
    fn check_id(
        &self,
        id: &Option<ObjectId>,
        replacing: Option<NodeKey>,
    ) -> Result<(), SceneError> {
        if let Some(id) = id
            && let Some(key) = self.ids.get(id)
            && Some(*key) != replacing
        {
            return Err(SceneError::DuplicateId(id.clone()));
        }
        Ok(())
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn len(&self) -> usize {
        self.nodes.len()
    }
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
    pub fn node(&self, handle: NodeHandle) -> Result<&Node, SceneError> {
        Ok(&self.nodes[self.key(handle)?].node)
    }
    pub fn find(&self, id: &ObjectId) -> Option<NodeHandle> {
        self.ids.get(id).map(|key| self.handle(*key))
    }
    pub fn roots(&self) -> impl Iterator<Item = NodeHandle> + '_ {
        self.roots.iter().map(|key| self.handle(*key))
    }
    pub fn parent(&self, node: NodeHandle) -> Result<Option<NodeHandle>, SceneError> {
        Ok(self.nodes[self.key(node)?]
            .parent
            .map(|key| self.handle(key)))
    }
    pub fn children(
        &self,
        node: NodeHandle,
    ) -> Result<impl Iterator<Item = NodeHandle> + '_, SceneError> {
        Ok(self.nodes[self.key(node)?]
            .children
            .iter()
            .map(|key| self.handle(*key)))
    }

    /// Captures a nonempty subtree with local transforms and local visibility.
    /// The root's ancestors and their transforms/visibility are not captured.
    /// Later source edits do not modify the snapshot.
    pub fn snapshot_subtree(&self, root: NodeHandle) -> Result<SceneSubtree, SceneError> {
        let root_key = self.key(root)?;
        let mut nodes = Vec::new();
        let mut pending = vec![root_key];
        while let Some(key) = pending.pop() {
            let entry = &self.nodes[key];
            nodes.push(SubtreeNode {
                source: self.handle(key),
                parent: if key == root_key {
                    None
                } else {
                    entry.parent.map(|key| self.handle(key))
                },
                node: entry.node.clone(),
            });
            pending.extend(entry.children.iter().rev().copied());
        }
        Ok(SceneSubtree {
            nodes: nodes.into(),
        })
    }

    /// Appends an editable copy beneath `parent`, or as a new root. Local
    /// transforms, visibility, picking, materials, cameras, and lights are copied;
    /// geometry and image sources remain shared. Application IDs are cleared.
    pub fn instantiate(
        &mut self,
        parent: Option<NodeHandle>,
        subtree: &SceneSubtree,
    ) -> Result<SubtreeInstance, SceneError> {
        self.instantiate_with_ids(parent, subtree, |_, _| None)
    }

    /// Remaps application IDs before insertion, including unnamed nodes.
    /// Duplicate IDs and invalid parents leave the graph and revision unchanged.
    pub fn instantiate_with_ids(
        &mut self,
        parent: Option<NodeHandle>,
        subtree: &SceneSubtree,
        mut map_id: impl FnMut(NodeHandle, Option<&ObjectId>) -> Option<ObjectId>,
    ) -> Result<SubtreeInstance, SceneError> {
        let parent = parent.map(|handle| self.key(handle)).transpose()?;
        let mut prepared = Vec::with_capacity(subtree.nodes.len());
        let mut used = HashSet::new();
        for entry in subtree.nodes.iter() {
            let mut node = entry.node.clone();
            node.id = map_id(entry.source, node.id.as_ref());
            self.check_id(&node.id, None)?;
            if let Some(id) = &node.id
                && !used.insert(id.clone())
            {
                return Err(SceneError::DuplicateId(id.clone()));
            }
            prepared.push(node);
        }
        let mut nodes: HashMap<NodeHandle, NodeHandle> = HashMap::with_capacity(prepared.len());
        for (entry, node) in subtree.nodes.iter().zip(prepared) {
            let parent = entry.parent.map(|source| nodes[&source].key).or(parent);
            let id = node.id.clone();
            let key = self.nodes.insert(Entry {
                node,
                parent,
                children: Vec::new(),
            });
            if let Some(id) = id {
                self.ids.insert(id, key);
            }
            self.siblings_mut(parent).push(key);
            nodes.insert(entry.source, self.handle(key));
        }
        self.revision += 1;
        Ok(SubtreeInstance {
            root: nodes[&subtree.root()],
            nodes,
        })
    }

    /// Adds a root when `parent` is `None`. Failed mutations leave the graph unchanged.
    pub fn insert(
        &mut self,
        parent: Option<NodeHandle>,
        node: Node,
    ) -> Result<NodeHandle, SceneError> {
        let parent = parent.map(|handle| self.key(handle)).transpose()?;
        self.check_id(&node.id, None)?;
        let id = node.id.clone();
        let key = self.nodes.insert(Entry {
            node,
            parent,
            children: Vec::new(),
        });
        if let Some(id) = id {
            self.ids.insert(id, key);
        }
        self.siblings_mut(parent).push(key);
        self.revision += 1;
        Ok(self.handle(key))
    }

    /// Replaces node properties without changing its handle, parent, or children.
    pub fn replace(&mut self, handle: NodeHandle, node: Node) -> Result<(), SceneError> {
        let key = self.key(handle)?;
        self.check_id(&node.id, Some(key))?;
        if let Some(id) = &self.nodes[key].node.id {
            self.ids.remove(id);
        }
        if let Some(id) = &node.id {
            self.ids.insert(id.clone(), key);
        }
        self.nodes[key].node = node;
        self.revision += 1;
        Ok(())
    }
    pub fn set_transform(
        &mut self,
        handle: NodeHandle,
        local: AffineTransform,
    ) -> Result<(), SceneError> {
        let key = self.key(handle)?;
        self.nodes[key].node.local = local;
        self.revision += 1;
        Ok(())
    }
    pub fn set_visible(&mut self, handle: NodeHandle, visible: bool) -> Result<(), SceneError> {
        let key = self.key(handle)?;
        self.nodes[key].node.hidden = !visible;
        self.revision += 1;
        Ok(())
    }
    /// Sets or removes only the camera property. Validated on the next evaluation.
    pub fn set_camera(
        &mut self,
        handle: NodeHandle,
        camera: Option<Camera>,
    ) -> Result<(), SceneError> {
        let key = self.key(handle)?;
        self.nodes[key].node.camera = camera;
        self.revision += 1;
        Ok(())
    }
    /// Sets or removes only the light property. Validated on the next evaluation.
    pub fn set_light(
        &mut self,
        handle: NodeHandle,
        light: Option<PunctualLight>,
    ) -> Result<(), SceneError> {
        let key = self.key(handle)?;
        self.nodes[key].node.light = light;
        self.revision += 1;
        Ok(())
    }
    fn siblings_mut(&mut self, parent: Option<NodeKey>) -> &mut Vec<NodeKey> {
        match parent {
            Some(parent) => &mut self.nodes[parent].children,
            None => &mut self.roots,
        }
    }
    pub fn world_transform(&self, handle: NodeHandle) -> Result<AffineTransform, SceneError> {
        let mut chain = Vec::new();
        let mut cursor = Some(self.key(handle)?);
        while let Some(key) = cursor {
            chain.push(key);
            cursor = self.nodes[key].parent;
        }
        chain
            .into_iter()
            .rev()
            .try_fold(AffineTransform::IDENTITY, |world, key| {
                world.compose(self.nodes[key].node.local).map_err(|source| {
                    SceneError::InvalidTransform {
                        node: self.handle(key),
                        source,
                    }
                })
            })
    }

    /// Appends to the new parent's children. `None` makes a root.
    /// Cycles and unrepresentable keep-world transforms are rejected atomically.
    pub fn reparent(
        &mut self,
        handle: NodeHandle,
        parent: Option<NodeHandle>,
        mode: ReparentMode,
    ) -> Result<(), SceneError> {
        let key = self.key(handle)?;
        let parent = parent.map(|handle| self.key(handle)).transpose()?;
        let mut cursor = parent;
        while let Some(ancestor) = cursor {
            if ancestor == key {
                return Err(SceneError::Cycle(handle));
            }
            cursor = self.nodes[ancestor].parent;
        }
        let previous = self.nodes[key].parent;
        if previous == parent {
            return Ok(());
        }
        let local = match mode {
            ReparentMode::KeepLocal => self.nodes[key].node.local,
            ReparentMode::KeepWorld => {
                let world = self.world_transform(handle)?;
                let parent_world = parent
                    .map(|key| self.world_transform(self.handle(key)))
                    .transpose()?
                    .unwrap_or_default();
                parent_world.inverse().compose(world).map_err(|source| {
                    SceneError::InvalidTransform {
                        node: handle,
                        source,
                    }
                })?
            }
        };
        self.siblings_mut(previous).retain(|child| *child != key);
        self.siblings_mut(parent).push(key);
        self.nodes[key].parent = parent;
        self.nodes[key].node.local = local;
        self.revision += 1;
        Ok(())
    }

    /// Removes the node and every descendant, invalidating all returned handles.
    /// Application IDs become available for reuse. Existing evaluated states remain valid.
    pub fn remove_subtree(&mut self, handle: NodeHandle) -> Result<Vec<NodeHandle>, SceneError> {
        let key = self.key(handle)?;
        let parent = self.nodes[key].parent;
        self.siblings_mut(parent).retain(|child| *child != key);
        let mut pending = vec![key];
        let mut removed = Vec::new();
        while let Some(key) = pending.pop() {
            let entry = self.nodes.remove(key).unwrap();
            pending.extend(entry.children.into_iter().rev());
            if let Some(id) = entry.node.id {
                self.ids.remove(&id);
            }
            removed.push(self.handle(key));
        }
        self.revision += 1;
        Ok(removed)
    }
}

fn union(a: Option<Aabb>, b: Option<Aabb>) -> Option<Aabb> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.union(b)),
        (a, b) => a.or(b),
    }
}

/// Immutable state of a node at evaluation time, including hidden nodes.
#[derive(Clone, Debug)]
pub struct EvaluatedNode {
    pub handle: NodeHandle,
    pub id: Option<ObjectId>,
    pub parent: Option<NodeHandle>,
    pub world: AffineTransform,
    pub visible: bool,
    /// World-space camera, even when this node is hidden. Selection is explicit.
    pub camera: Option<Camera>,
    /// World-space source, even when hidden. Only visible sources illuminate scenes.
    pub light: Option<PunctualLight>,
    /// World-aligned bounds of this node's mesh, regardless of visibility.
    pub bounds: Option<Aabb>,
    /// Union of this node and all descendants, regardless of visibility.
    pub subtree_bounds: Option<Aabb>,
}

/// Camera-independent evaluated hierarchy. Resources remain shared and alive.
#[derive(Clone)]
pub struct EvaluatedScene {
    pub(super) constraint_status: HashMap<NodeHandle, ConstraintStatus>,
    preparation_revision: Arc<()>,
    revision: u64,
    nodes: Vec<EvaluatedNode>,
    indices: HashMap<NodeHandle, usize>,
    objects: Vec<Object>,
    lights: Option<Vec<(NodeHandle, PunctualLight)>>,
    bounds: Option<Aabb>,
    spatial_index: Arc<std::sync::OnceLock<crate::spatial::bvh::ObjectIndex>>,
    spatial_source: Arc<Vec<crate::spatial::bvh::IndexObject>>,
}
impl EvaluatedScene {
    /// Outcome of a node's constraint, or `None` for an unconstrained or absent node.
    pub fn constraint_status(&self, node: NodeHandle) -> Option<ConstraintStatus> {
        self.constraint_status.get(&node).copied()
    }

    /// Source graph revision. Transform overrides and constraints are not part of this value;
    /// equal revisions do not imply equal evaluated poses.
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn node(&self, handle: NodeHandle) -> Option<&EvaluatedNode> {
        self.indices.get(&handle).map(|index| &self.nodes[*index])
    }
    /// Parent-first traversal, with siblings in insertion/reparent order.
    pub fn nodes(&self) -> &[EvaluatedNode] {
        &self.nodes
    }
    /// Bounds of geometry with inherited visibility enabled; not an occlusion query.
    pub fn bounds(&self) -> Option<Aabb> {
        self.bounds
    }
    /// Creates a renderable/pickable scene for a camera without reevaluating the hierarchy.
    /// Attached lights replace default direct lighting; hidden sources are omitted.
    /// Graphs without light properties retain the default source; all-hidden lights disable it.
    pub fn scene(&self, camera: Camera) -> Scene {
        Scene {
            preparation_revision: self.preparation_revision.clone(),
            camera,
            lights: self
                .lights
                .as_ref()
                .map(|lights| lights.iter().map(|(_, light)| light.0).collect()),
            objects: self.objects.clone(),
            spatial_index: self.spatial_index.clone(),
            spatial_source: Some(self.spatial_source.clone()),
            ..Scene::default()
        }
    }

    /// Visible lights in parent-first order, with stable node identities.
    /// This is also the direct-light order used by `scene` and shadow indices.
    pub fn lights(&self) -> impl Iterator<Item = (NodeHandle, PunctualLight)> + '_ {
        self.lights.iter().flatten().copied()
    }

    /// Selects a camera explicitly, including cameras on hidden nodes.
    /// Foreign/expired handles and nodes without a camera return errors.
    pub fn scene_from_camera(&self, handle: NodeHandle) -> Result<Scene, SceneError> {
        let node = self.node(handle).ok_or(SceneError::InvalidHandle(handle))?;
        let camera = node.camera.ok_or(SceneError::NoCamera(handle))?;
        Ok(self.scene(camera))
    }

    /// Prepares the camera-independent object index shared by derived scenes.
    pub fn prepare_spatial_index(&self) {
        self.spatial_index
            .get_or_init(|| crate::spatial::bvh::ObjectIndex::build_from(&self.spatial_source));
    }

    /// Prepares the shared object index from an earlier evaluated snapshot when available.
    /// Mesh-node slots include hidden nodes; motion, reparenting, and visibility updates
    /// retain the partition. Adding/removing mesh nodes or using another graph rebuilds it.
    /// Existing prepared indices are unchanged, and previous snapshots remain queryable.
    /// This synchronous CPU operation does not evaluate transforms or access the GPU.
    pub fn prepare_spatial_index_from(&self, previous: &EvaluatedScene) {
        self.spatial_index.get_or_init(|| {
            previous.spatial_index.get().map_or_else(
                || crate::spatial::bvh::ObjectIndex::build_from(&self.spatial_source),
                |index| index.refit(&self.spatial_source),
            )
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::{dot, transform};
    use gpui::{Bounds, point, px, rgb, size};
    use std::sync::Arc;

    fn at(position: [f32; 3]) -> AffineTransform {
        AffineTransform::from_translation(position).unwrap()
    }
    fn mesh(id: &'static str) -> Node {
        Node::new()
            .id(id)
            .mesh(Mesh::cube(), Material::color(rgb(0xffffff)))
    }
    fn close(a: [[f32; 4]; 4], b: [[f32; 4]; 4]) {
        for (a, b) in a.iter().flatten().zip(b.iter().flatten()) {
            assert!((a - b).abs() < 3e-5, "{a} != {b}");
        }
    }

    #[test]
    fn camera_and_light_nodes_follow_pose_overrides_and_inherited_visibility() {
        let mut graph = SceneGraph::new();
        let root = graph.insert(None, Node::new()).unwrap();
        let local = Camera {
            eye: [0.; 3],
            target: [0., 0., -2.],
            ..Default::default()
        };
        let rig = graph
            .insert(
                Some(root),
                Node::new()
                    .camera(local)
                    .light(PunctualLight::spot([0.; 3], [0., 0., -1.]))
                    .transform(at([0., 1., 0.])),
            )
            .unwrap();
        let fill = graph
            .insert(
                None,
                Node::new().light(PunctualLight::directional([0., 1., 0.])),
            )
            .unwrap();
        let pose = AffineTransform::from_trs([3., 2., 1.], [0., 1., 0., 1.], [2., 3., 4.]).unwrap();
        let revision = graph.revision();
        let snapshot = graph.evaluate_with_transforms([(root, pose)]).unwrap();
        assert_eq!(revision, graph.revision());
        let camera = snapshot.node(rig).unwrap().camera.unwrap();
        for (a, b) in camera.eye.into_iter().zip([3., 5., 1.]) {
            assert!((a - b).abs() < 1e-5);
        }
        for (a, b) in camera.target.into_iter().zip([-5., 5., 1.]) {
            assert!((a - b).abs() < 1e-5);
        }
        assert_eq!(camera.projection, local.projection);
        assert_eq!((camera.near, camera.far), (local.near, local.far));
        let light = snapshot.node(rig).unwrap().light.unwrap();
        assert_eq!(light.position(), camera.eye);
        for (a, b) in light.direction().into_iter().zip([-1., 0., 0.]) {
            assert!((a - b).abs() < 1e-5);
        }
        assert_eq!(
            snapshot.lights().map(|(node, _)| node).collect::<Vec<_>>(),
            [rig, fill]
        );
        assert_eq!(snapshot.scene_from_camera(rig).unwrap().camera, camera);
        assert!(snapshot.bounds().is_none());
        assert!(snapshot.scene_from_camera(rig).unwrap().objects.is_empty());
        graph.set_visible(root, false).unwrap();
        graph.set_visible(fill, false).unwrap();
        let hidden = graph.evaluate().unwrap();
        assert_eq!(hidden.lights().count(), 0);
        assert!(hidden.scene(Camera::default()).lights.unwrap().is_empty());
        assert!(hidden.node(rig).unwrap().light.is_some());
        assert!(hidden.scene_from_camera(rig).is_ok());
        assert_eq!(snapshot.lights().count(), 2);
        graph.set_light(rig, None).unwrap();
        graph.set_light(fill, None).unwrap();
        assert!(
            graph
                .evaluate()
                .unwrap()
                .scene(Camera::default())
                .lights
                .is_none()
        );
        graph.set_camera(rig, None).unwrap();
        assert!(
            matches!(graph.evaluate().unwrap().scene_from_camera(rig), Err(SceneError::NoCamera(node)) if node == rig)
        );
        assert_eq!(snapshot.scene_from_camera(rig).unwrap().camera, camera);
    }

    #[test]
    fn camera_light_subtrees_keep_world_on_reparent_and_copy_bindings_on_instance() {
        let mut source = SceneGraph::new();
        let root = source
            .insert(
                None,
                Node::new()
                    .camera(Camera::default())
                    .light(PunctualLight::point([1., 0., 0.])),
            )
            .unwrap();
        let snapshot = source.snapshot_subtree(root).unwrap();
        let mut graph = SceneGraph::new();
        let parent = graph
            .insert(None, Node::new().transform(at([5., 2., 0.])))
            .unwrap();
        let a = graph.instantiate(Some(parent), &snapshot).unwrap().root();
        let b = graph.instantiate(None, &snapshot).unwrap().root();
        let before = graph.evaluate().unwrap();
        assert_eq!(
            before.node(a).unwrap().light.unwrap().position(),
            [6., 2., 0.]
        );
        assert_eq!(
            before.node(b).unwrap().light.unwrap().position(),
            [1., 0., 0.]
        );
        let old_camera = before.scene_from_camera(a).unwrap().camera;
        graph.reparent(a, None, ReparentMode::KeepWorld).unwrap();
        let after = graph.evaluate().unwrap();
        assert_eq!(after.scene_from_camera(a).unwrap().camera, old_camera);
        assert_eq!(
            after.node(a).unwrap().light.unwrap().position(),
            [6., 2., 0.]
        );
        source.remove_subtree(root).unwrap();
        graph.set_camera(a, None).unwrap();
        graph.set_light(a, None).unwrap();
        assert!(graph.node(b).unwrap().local_camera().is_some());
        assert!(graph.node(b).unwrap().local_light().is_some());
        graph.remove_subtree(a).unwrap();
        assert!(
            matches!(graph.evaluate().unwrap().scene_from_camera(a), Err(SceneError::InvalidHandle(node)) if node == a)
        );
        assert_eq!(before.scene_from_camera(a).unwrap().camera, old_camera);
    }

    #[test]
    fn attachment_errors_identify_nodes_without_invalidating_previous_evaluations() {
        let mut graph = SceneGraph::new();
        let rig = graph
            .insert(
                None,
                Node::new()
                    .camera(Camera::default())
                    .light(PunctualLight::point([0.; 3])),
            )
            .unwrap();
        let old = graph.evaluate().unwrap();
        graph
            .set_camera(
                rig,
                Some(Camera {
                    near: 0.,
                    ..Default::default()
                }),
            )
            .unwrap();
        assert!(
            matches!(graph.evaluate(), Err(SceneError::InvalidCamera { node, source: CameraError::InvalidProjection }) if node == rig)
        );
        graph.set_camera(rig, Some(Camera::default())).unwrap();
        graph
            .set_light(rig, Some(PunctualLight::point([0.; 3]).intensity(-1.)))
            .unwrap();
        graph.set_visible(rig, false).unwrap();
        assert!(
            matches!(graph.evaluate(), Err(SceneError::InvalidLight { node, source: LightError::InvalidParameters }) if node == rig)
        );
        let mut foreign = SceneGraph::new();
        let other = foreign.insert(None, Node::new()).unwrap();
        let revision = graph.revision();
        assert!(graph.set_camera(other, None).is_err());
        assert!(graph.set_light(other, None).is_err());
        assert_eq!(graph.revision(), revision);
        assert!(
            matches!(old.scene_from_camera(other), Err(SceneError::InvalidHandle(node)) if node == other)
        );
        assert_eq!(old.scene_from_camera(rig).unwrap().lights.unwrap().len(), 1);
    }

    #[test]
    fn subtree_instances_share_resources_and_keep_edits_independent() {
        let image = Arc::new(gpui::RenderImage::new(vec![image::Frame::new(
            image::RgbaImage::from_pixel(1, 1, image::Rgba([255; 4])),
        )]));
        let mut source = SceneGraph::new();
        let outside = source
            .insert(
                None,
                Node::new().visible(false).transform(at([100., 0., 0.])),
            )
            .unwrap();
        let root = source
            .insert(
                Some(outside),
                Node::new().id("root").transform(at([1., 0., 0.])),
            )
            .unwrap();
        let leaf = source
            .insert(
                Some(root),
                Node::new()
                    .id("leaf")
                    .transform(at([0., 2., 0.]))
                    .mesh(Mesh::cube(), Material::image(image.clone()))
                    .pick_behavior(PickBehavior::Occlude),
            )
            .unwrap();
        let snapshot = source.snapshot_subtree(root).unwrap();
        source.set_transform(root, at([50., 0., 0.])).unwrap();
        source.set_visible(root, false).unwrap();
        source.remove_subtree(outside).unwrap();
        drop(source);

        let mut destination = SceneGraph::new();
        let parent = destination
            .insert(None, Node::new().transform(at([10., 0., 0.])))
            .unwrap();
        let a = destination.instantiate(Some(parent), &snapshot).unwrap();
        let b = destination
            .instantiate(Some(parent), &snapshot.clone())
            .unwrap();
        let a_leaf = a.node(leaf).unwrap();
        let b_leaf = b.node(leaf).unwrap();
        assert_ne!(a_leaf, b_leaf);
        assert_eq!(destination.parent(a_leaf).unwrap(), Some(a.root()));
        assert!(destination.node(a.root()).unwrap().object_id().is_none());
        assert!(destination.node(a_leaf).unwrap().object_id().is_none());
        assert_eq!(
            destination.node(a_leaf).unwrap().picking,
            PickBehavior::Occlude
        );
        assert_eq!(
            destination
                .world_transform(a_leaf)
                .unwrap()
                .transform_point([0.; 3]),
            [11., 2., 0.]
        );
        assert!(
            destination
                .evaluate()
                .unwrap()
                .node(a_leaf)
                .unwrap()
                .visible
        );
        let (mesh_a, material_a) = destination.node(a_leaf).unwrap().surface.as_ref().unwrap();
        let (mesh_b, material_b) = destination.node(b_leaf).unwrap().surface.as_ref().unwrap();
        assert!(Arc::ptr_eq(&mesh_a.0, &mesh_b.0));
        for material in [material_a, material_b] {
            let crate::Texture::Image(gpui::ImageSource::Render(shared)) = &material.texture else {
                panic!("decoded image expected")
            };
            assert!(Arc::ptr_eq(shared, &image));
        }
        let before = destination.evaluate().unwrap();
        destination
            .set_transform(a.root(), at([-3., 0., 0.]))
            .unwrap();
        destination
            .set_material(a_leaf, Material::color(rgb(0xff0000)))
            .unwrap();
        destination.set_visible(a.root(), false).unwrap();
        let after = destination.evaluate().unwrap();
        assert!(!after.node(a_leaf).unwrap().visible);
        assert!(after.node(b_leaf).unwrap().visible);
        assert_eq!(
            after.node(b_leaf).unwrap().world.transform_point([0.; 3]),
            [11., 2., 0.]
        );
        assert_eq!(
            before.node(a_leaf).unwrap().world.transform_point([0.; 3]),
            [11., 2., 0.]
        );
        assert_eq!(after.scene(Camera::default()).objects.len(), 1);
        assert!(matches!(
            destination
                .node(b_leaf)
                .unwrap()
                .surface
                .as_ref()
                .unwrap()
                .1
                .texture,
            crate::Texture::Image(_)
        ));
        let b_root = b.root();
        drop(b);
        drop(snapshot);
        assert!(destination.node(b_root).is_ok());
        destination.remove_subtree(a.root()).unwrap();
        assert_eq!(a.node(leaf), Some(a_leaf));
        assert!(destination.node(a_leaf).is_err());
        assert!(destination.node(b_leaf).is_ok());
    }

    #[test]
    fn instantiation_remaps_ids_atomically_and_preserves_sibling_order() {
        let mut graph = SceneGraph::new();
        let root = graph.insert(None, Node::new().id("source")).unwrap();
        let first = graph.insert(Some(root), Node::new().id("first")).unwrap();
        let second = graph.insert(Some(root), Node::new()).unwrap();
        let snapshot = graph.snapshot_subtree(root).unwrap();
        let revision = graph.revision();
        let roots = graph.roots().collect::<Vec<_>>();
        assert!(matches!(
            graph.instantiate_with_ids(None, &snapshot, |_, id| id.cloned()),
            Err(SceneError::DuplicateId(_))
        ));
        assert!(matches!(
            graph.instantiate_with_ids(None, &snapshot, |_, _| Some("duplicate".into())),
            Err(SceneError::DuplicateId(_))
        ));
        let mut foreign = SceneGraph::new();
        let parent = foreign.insert(None, Node::new()).unwrap();
        assert!(matches!(
            graph.instantiate(Some(parent), &snapshot),
            Err(SceneError::InvalidHandle(_))
        ));
        assert_eq!(graph.revision(), revision);
        assert_eq!(graph.len(), 3);
        assert_eq!(graph.roots().collect::<Vec<_>>(), roots);
        assert_eq!(graph.find(&"first".into()), Some(first));
        let instance = graph
            .instantiate_with_ids(None, &snapshot, |node, _| {
                Some(
                    if node == root {
                        "copy"
                    } else if node == first {
                        "copy-first"
                    } else {
                        "copy-second"
                    }
                    .into(),
                )
            })
            .unwrap();
        assert_eq!(graph.revision(), revision + 1);
        assert_eq!(graph.find(&"copy-first".into()), instance.node(first));
        assert_eq!(graph.find(&"copy-second".into()), instance.node(second));
        assert_eq!(
            graph.children(instance.root()).unwrap().collect::<Vec<_>>(),
            vec![
                instance.node(first).unwrap(),
                instance.node(second).unwrap()
            ]
        );
        let child_copy = graph.instantiate(Some(first), &snapshot).unwrap();
        assert_eq!(graph.parent(child_copy.root()).unwrap(), Some(first));
        assert_eq!(graph.len(), 9);
        assert!(instance.node(parent).is_none());
        assert!(graph.snapshot_subtree(parent).is_err());
        graph.remove_subtree(root).unwrap();
        assert!(graph.node(child_copy.root()).is_err());
        assert!(graph.node(instance.root()).is_ok());
        let later = graph.instantiate(None, &snapshot).unwrap();
        assert_ne!(later.root(), root);
        assert!(later.node(first).is_some());
    }

    #[test]
    fn deep_subtrees_capture_and_instantiate_without_recursion() {
        let mut source = SceneGraph::new();
        let root = source.insert(None, Node::new()).unwrap();
        let mut leaf = root;
        for _ in 1..10_000 {
            leaf = source.insert(Some(leaf), Node::new()).unwrap();
        }
        let snapshot = source.snapshot_subtree(root).unwrap();
        let mut destination = SceneGraph::new();
        let instance = destination.instantiate(None, &snapshot).unwrap();
        let copy_leaf = instance.node(leaf).unwrap();
        assert_eq!(destination.len(), 10_000);
        assert!(destination.evaluate().unwrap().node(copy_leaf).is_some());
        destination.remove_subtree(instance.root()).unwrap();
        assert!(destination.is_empty());
    }

    #[test]
    fn hierarchy_visibility_bounds_and_snapshots_are_consistent() {
        let mut graph = SceneGraph::new();
        let root = graph
            .insert(None, Node::new().id("group").transform(at([3., 0., 0.])))
            .unwrap();
        let a = graph
            .insert(Some(root), mesh("a").transform(at([-1., 0., 0.])))
            .unwrap();
        let b = graph
            .insert(Some(root), mesh("b").transform(at([1., 0., 0.])))
            .unwrap();
        let first = graph.evaluate().unwrap();
        assert_eq!(
            first.node(a).unwrap().world.transform_point([0.; 3]),
            [2., 0., 0.]
        );
        assert_eq!(
            first.bounds().unwrap(),
            Aabb::new([1.5, -0.5, -0.5], [4.5, 0.5, 0.5]).unwrap()
        );
        assert_eq!(first.node(root).unwrap().subtree_bounds, first.bounds());
        graph.set_visible(a, false).unwrap();
        graph.set_visible(root, false).unwrap();
        let hidden = graph.evaluate().unwrap();
        assert!(hidden.bounds().is_none());
        assert!(hidden.scene(Camera::default()).objects.is_empty());
        assert!(!hidden.node(b).unwrap().visible);
        assert_eq!(hidden.node(root).unwrap().subtree_bounds, first.bounds());
        graph.set_visible(root, true).unwrap();
        let shown = graph.evaluate().unwrap();
        assert!(!shown.node(a).unwrap().visible);
        assert_eq!(shown.scene(Camera::default()).objects.len(), 1);
        assert_eq!(first.scene(Camera::default()).objects.len(), 2);
        assert!(first.revision() < shown.revision());
        graph.set_transform(root, at([5., 0., 0.])).unwrap();
        let moved = graph.evaluate().unwrap();
        assert_eq!(
            moved.node(b).unwrap().world.transform_point([0.; 3]),
            [6., 0., 0.]
        );
        assert_eq!(
            first.node(b).unwrap().world.transform_point([0.; 3]),
            [4., 0., 0.]
        );
    }

    #[test]
    fn keep_world_preserves_shear_normals_and_descendants() {
        let mut graph = SceneGraph::new();
        let a = graph
            .insert(
                None,
                Node::new().transform(
                    AffineTransform::from_trs([1., 2., 0.], [0., 0., 0.3, 0.9], [2., -0.5, 3.])
                        .unwrap(),
                ),
            )
            .unwrap();
        let b = graph
            .insert(
                None,
                Node::new().transform(
                    AffineTransform::from_trs([-3., 1., 2.], [0.2, 0.5, 0., 0.8], [0.5, 3., 2.])
                        .unwrap(),
                ),
            )
            .unwrap();
        let child = graph
            .insert(
                Some(a),
                mesh("child").transform(
                    AffineTransform::from_trs([0., 1., 0.], [0.3, 0.1, 0.2, 0.8], [1., 2., 1.])
                        .unwrap(),
                ),
            )
            .unwrap();
        let leaf = graph
            .insert(Some(child), mesh("leaf").transform(at([0., 0., 2.])))
            .unwrap();
        let before = graph.evaluate().unwrap();
        let original_local = graph.node(child).unwrap().local_transform();
        graph
            .reparent(child, Some(b), ReparentMode::KeepWorld)
            .unwrap();
        let after = graph.evaluate().unwrap();
        for handle in [child, leaf] {
            let old = before.node(handle).unwrap().world;
            let new = after.node(handle).unwrap().world;
            close(old.matrix(), new.matrix());
            close(old.normal_matrix(), new.normal_matrix());
        }
        assert_ne!(graph.node(child).unwrap().local_transform(), original_local);
        let local = graph.node(child).unwrap().local_transform();
        graph
            .reparent(child, None, ReparentMode::KeepLocal)
            .unwrap();
        close(
            graph.world_transform(child).unwrap().matrix(),
            local.matrix(),
        );
        assert!(graph.children(a).unwrap().next().is_none());
        assert!(graph.children(b).unwrap().next().is_none());
    }

    #[test]
    fn graph_identity_deletion_and_failed_mutations_do_not_alias_nodes() {
        let mut graph = SceneGraph::new();
        let root = graph.insert(None, Node::new().id("root")).unwrap();
        let child = graph.insert(Some(root), mesh("child")).unwrap();
        let sibling = graph.insert(None, mesh("sibling")).unwrap();
        let old = graph.evaluate().unwrap();
        let mut other = SceneGraph::new();
        let foreign = other.insert(None, Node::new()).unwrap();
        let revision = graph.revision();
        assert_eq!(
            graph.insert(None, mesh("child")).unwrap_err(),
            SceneError::DuplicateId("child".into())
        );
        assert_eq!(
            graph.replace(child, mesh("root")).unwrap_err(),
            SceneError::DuplicateId("root".into())
        );
        assert_eq!(
            graph.reparent(root, Some(child), ReparentMode::KeepLocal),
            Err(SceneError::Cycle(root))
        );
        assert_eq!(
            graph.reparent(child, Some(foreign), ReparentMode::KeepWorld),
            Err(SceneError::InvalidHandle(foreign))
        );
        assert!(graph.node(foreign).is_err());
        assert_eq!(graph.revision(), revision);
        assert_eq!(graph.find(&"child".into()), Some(child));
        assert_eq!(graph.parent(child).unwrap(), Some(root));
        assert_eq!(graph.remove_subtree(root).unwrap(), vec![root, child]);
        let replacement = graph.insert(None, mesh("child")).unwrap();
        assert_ne!(replacement, child);
        assert!(graph.node(child).is_err());
        assert!(graph.set_visible(child, false).is_err());
        assert_eq!(graph.find(&"child".into()), Some(replacement));
        assert!(graph.node(sibling).is_ok());
        assert_eq!(old.node(child).unwrap().id, Some("child".into()));
        assert_eq!(old.scene(Camera::default()).objects.len(), 2);
    }

    #[test]
    fn evaluated_geometry_is_shared_but_materials_and_poses_are_independent() {
        let mut graph = SceneGraph::new();
        let shared = Mesh::cube();
        let a = graph
            .insert(
                None,
                Node::new().mesh(shared.clone(), Material::color(rgb(0xff0000))),
            )
            .unwrap();
        let b = graph
            .insert(
                None,
                Node::new()
                    .mesh(shared, Material::color(rgb(0xff0000)))
                    .transform(at([2., 0., 0.])),
            )
            .unwrap();
        let before = graph.evaluate().unwrap();
        graph
            .set_material(a, Material::color(rgb(0x00ff00)))
            .unwrap();
        graph.set_transform(a, at([-2., 0., 0.])).unwrap();
        let after = graph.evaluate().unwrap();
        let rendered = after.scene(Camera::default());
        assert!(Arc::ptr_eq(
            &rendered.objects[0].mesh.0,
            &rendered.objects[1].mesh.0
        ));
        assert_eq!(
            rendered.objects[1].material.color,
            before.objects[1].material.color
        );
        assert_ne!(
            rendered.objects[0].material.color,
            before.objects[0].material.color
        );
        assert_eq!(before.node(b).unwrap().world, after.node(b).unwrap().world);
    }

    #[test]
    fn picking_and_render_matrices_agree_after_parent_transform() {
        let mut graph = SceneGraph::new();
        let root = graph
            .insert(
                None,
                Node::new().transform(
                    AffineTransform::from_trs([0.3, -0.2, 0.], [0., 0.15, 0., 0.9], [2., 0.7, 1.])
                        .unwrap(),
                ),
            )
            .unwrap();
        let child = graph
            .insert(
                Some(root),
                Node::new()
                    .id("surface")
                    .mesh(Mesh::plane(), Material::color(rgb(0xffffff)))
                    .transform(
                        AffineTransform::from_trs([0.1, 0., 0.], [0., 0., 0.3, 0.9], [1.; 3])
                            .unwrap(),
                    ),
            )
            .unwrap();
        let evaluated = graph.evaluate().unwrap();
        let camera = Camera::default();
        let scene = evaluated.scene(camera);
        let (model, normal) = scene.objects[0].matrices();
        let p = transform(model, [0., 0., 0., 1.]);
        let bounds = Bounds::new(point(px(80.), px(40.)), size(px(800.), px(600.)));
        let clip = transform(camera.matrix(800. / 600.), p);
        let screen = bounds.origin
            + point(
                px((clip[0] / clip[3] + 1.) * 400.),
                px((1. - clip[1] / clip[3]) * 300.),
            );
        let hit = scene.pick(bounds, screen).unwrap();
        assert_eq!(hit.node, Some(child));
        assert_eq!(hit.object_id, Some("surface".into()));
        for (actual, expected) in hit.position.iter().zip(p) {
            assert!((actual - expected).abs() < 1e-4);
        }
        for uv in hit.uv {
            assert!((uv - 0.5).abs() < 1e-4);
        }
        let n = transform(normal, [0., 0., 1., 0.]);
        assert!(dot(hit.normal, [n[0], n[1], n[2]]) > 0.99);
        graph.set_visible(root, false).unwrap();
        assert!(
            graph
                .evaluate()
                .unwrap()
                .scene(camera)
                .pick(bounds, screen)
                .is_none()
        );
        assert!(scene.pick(bounds, screen).is_some());
    }

    #[test]
    fn deep_hierarchies_evaluate_and_delete_without_recursion() {
        let mut graph = SceneGraph::new();
        let root = graph.insert(None, Node::new()).unwrap();
        let mut parent = root;
        for _ in 0..2048 {
            parent = graph
                .insert(Some(parent), Node::new().transform(at([1., 0., 0.])))
                .unwrap();
        }
        graph
            .replace(parent, mesh("tip").transform(at([1., 0., 0.])))
            .unwrap();
        let state = graph.evaluate().unwrap();
        assert_eq!(
            state.node(parent).unwrap().world.transform_point([0.; 3]),
            [2048., 0., 0.]
        );
        assert_eq!(
            state.node(root).unwrap().subtree_bounds,
            state.node(parent).unwrap().bounds
        );
        assert_eq!(graph.remove_subtree(root).unwrap().len(), 2049);
        assert!(graph.is_empty());
    }

    #[test]
    fn evaluation_reports_the_node_with_unrepresentable_world_transform() {
        let mut graph = SceneGraph::new();
        let large = AffineTransform::from_trs([0.; 3], [0., 0., 0., 1.], [1e30; 3]).unwrap();
        let a = graph.insert(None, Node::new().transform(large)).unwrap();
        let b = graph.insert(Some(a), Node::new().transform(large)).unwrap();
        assert!(
            matches!(graph.evaluate(), Err(SceneError::InvalidTransform { node, .. }) if node == b)
        );
        let revision = graph.revision();
        assert!(graph.reparent(b, None, ReparentMode::KeepWorld).is_err());
        assert_eq!(graph.parent(b).unwrap(), Some(a));
        assert_eq!(graph.revision(), revision);
        graph.reparent(b, None, ReparentMode::KeepLocal).unwrap();
        assert!(graph.evaluate().is_ok());
    }
}
