use crate::{
    Aabb, AffineTransform, Camera, Material, Mesh, Object, ObjectId, PickBehavior, Scene,
    TransformError,
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

/// A group or a single mesh surface with a local transform.
#[derive(Clone, Default)]
pub struct Node {
    id: Option<ObjectId>,
    local: AffineTransform,
    hidden: bool,
    surface: Option<(Mesh, Material)>,
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
    pub fn pick_behavior(mut self, behavior: PickBehavior) -> Self {
        self.picking = behavior;
        self
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
    Cycle(NodeHandle),
    NoMesh(NodeHandle),
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
            Self::Cycle(node) => write!(f, "parent assignment would create a cycle at {node:?}"),
            Self::NoMesh(node) => write!(f, "node {node:?} has no mesh"),
            Self::InvalidTransform { node, source } => write!(f, "node {node:?}: {source}"),
        }
    }
}
impl std::error::Error for SceneError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InvalidTransform { source, .. } => Some(source),
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
    /// transforms, visibility, picking, and materials are copied; geometry and
    /// image sources remain shared. Application IDs are cleared.
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

    /// Resolves world matrices, inherited visibility, and bounds in parent-first order.
    /// No playback history, window, layout, or GPU work is required.
    pub fn evaluate(&self) -> Result<EvaluatedScene, SceneError> {
        let mut evaluated = EvaluatedScene {
            revision: self.revision,
            nodes: Vec::with_capacity(self.len()),
            indices: HashMap::with_capacity(self.len()),
            objects: Vec::new(),
            bounds: None,
            spatial_index: Arc::default(),
        };
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
            let world = parent
                .map_or(AffineTransform::IDENTITY, |parent| parent.world)
                .compose(node.local)
                .map_err(|source| SceneError::InvalidTransform {
                    node: handle,
                    source,
                })?;
            let visible = !node.hidden && parent.is_none_or(|parent| parent.visible);
            let bounds = node
                .bounds
                .map(|bounds| bounds.transformed(world))
                .transpose()
                .map_err(|source| SceneError::InvalidTransform {
                    node: handle,
                    source,
                })?;
            if visible && let Some((mesh, material)) = &node.surface {
                let mut object = Object::new(mesh.clone(), material.clone());
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
        Ok(evaluated)
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
    /// World-aligned bounds of this node's mesh, regardless of visibility.
    pub bounds: Option<Aabb>,
    /// Union of this node and all descendants, regardless of visibility.
    pub subtree_bounds: Option<Aabb>,
}

/// Camera-independent evaluated hierarchy. Resources remain shared and alive.
#[derive(Clone)]
pub struct EvaluatedScene {
    revision: u64,
    nodes: Vec<EvaluatedNode>,
    indices: HashMap<NodeHandle, usize>,
    objects: Vec<Object>,
    bounds: Option<Aabb>,
    spatial_index: Arc<std::sync::OnceLock<crate::bvh::ObjectIndex>>,
}
impl EvaluatedScene {
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
    pub fn scene(&self, camera: Camera) -> Scene {
        Scene {
            camera,
            objects: self.objects.clone(),
            spatial_index: self.spatial_index.clone(),
            ..Scene::default()
        }
    }

    /// Prepares the camera-independent object index shared by derived scenes.
    pub fn prepare_spatial_index(&self) {
        self.spatial_index
            .get_or_init(|| crate::bvh::ObjectIndex::build(&self.objects));
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
