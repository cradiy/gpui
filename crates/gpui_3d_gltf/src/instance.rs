use std::collections::HashMap;

use gpui_3d::{
    EvaluatedScene, Mesh, NodeHandle, ObjectId, SceneError, SceneGraph, SubtreeInstance,
};

use crate::{SceneAsset, SceneNode, ScenePrimitive};

/// Shared source asset and the graph-scoped identities of one instantiation.
/// Dropping this value does not remove graph nodes. Lookups retain their original
/// mappings after graph edits; the graph validates whether handles remain live.
pub struct SceneInstance {
    asset: SceneAsset,
    subtree: SubtreeInstance,
    nodes: HashMap<usize, NodeHandle>,
    primitives: HashMap<(usize, usize), NodeHandle>,
    source_nodes: HashMap<NodeHandle, usize>,
    source_primitives: HashMap<NodeHandle, usize>,
}

impl SceneAsset {
    /// Instantiates shared geometry and images with independent node properties.
    /// The synthetic root is inserted beneath `parent`, or as a graph root.
    pub fn instantiate(
        &self,
        graph: &mut SceneGraph,
        parent: Option<NodeHandle>,
    ) -> Result<SceneInstance, SceneError> {
        self.instantiate_with_ids(graph, parent, |_, _| None)
    }

    /// Assigns application IDs using source-subtree handles. Invalid parents or
    /// duplicate IDs leave the graph unchanged. Callback side effects are not undone.
    pub fn instantiate_with_ids(
        &self,
        graph: &mut SceneGraph,
        parent: Option<NodeHandle>,
        map_id: impl FnMut(NodeHandle, Option<&ObjectId>) -> Option<ObjectId>,
    ) -> Result<SceneInstance, SceneError> {
        let subtree = graph.instantiate_with_ids(parent, self.subtree(), map_id)?;
        let mut nodes = HashMap::with_capacity(self.nodes().len());
        let mut source_nodes = HashMap::with_capacity(self.nodes().len());
        for (index, source) in self.nodes().iter().enumerate() {
            let handle = subtree.node(source.handle).unwrap();
            nodes.insert(source.index, handle);
            source_nodes.insert(handle, index);
        }
        let mut primitives = HashMap::with_capacity(self.primitives().len());
        let mut source_primitives = HashMap::with_capacity(self.primitives().len());
        for (index, source) in self.primitives().iter().enumerate() {
            let handle = subtree.node(source.handle).unwrap();
            primitives.insert((source.node_index, source.primitive_index), handle);
            source_primitives.insert(handle, index);
        }
        Ok(SceneInstance {
            asset: self.clone(),
            subtree,
            nodes,
            primitives,
            source_nodes,
            source_primitives,
        })
    }
}

impl SceneInstance {
    pub fn asset(&self) -> &SceneAsset {
        &self.asset
    }

    pub fn root(&self) -> NodeHandle {
        self.subtree.root()
    }

    /// Core mapping for APIs accepting a `SubtreeInstance`.
    pub fn subtree_instance(&self) -> &SubtreeInstance {
        &self.subtree
    }

    /// Looks up an original glTF node index, not a mesh or primitive index.
    pub fn node(&self, node_index: usize) -> Option<NodeHandle> {
        self.nodes.get(&node_index).copied()
    }

    /// Looks up a primitive occurrence by its original node and mesh-local index.
    pub fn primitive(&self, node_index: usize, primitive_index: usize) -> Option<NodeHandle> {
        self.primitives.get(&(node_index, primitive_index)).copied()
    }

    /// Returns source metadata for an original-node group in this instance.
    /// Synthetic roots, primitive children and other instances return `None`.
    pub fn source_node(&self, handle: NodeHandle) -> Option<&SceneNode> {
        self.source_nodes
            .get(&handle)
            .map(|&index| &self.asset.nodes()[index])
    }

    /// Returns source metadata for a renderable primitive in this instance.
    /// Use the primitive node handle reported by picking or frame-output mappings.
    pub fn source_primitive(&self, handle: NodeHandle) -> Option<&ScenePrimitive> {
        self.source_primitives
            .get(&handle)
            .map(|&index| &self.asset.primitives()[index])
    }

    /// Primitive nodes using an authored material index, in scene order.
    /// `None` selects implicit glTF materials. Graph material overrides do not
    /// change this source association; mutations use ordinary `SceneGraph` APIs.
    pub fn material_nodes(
        &self,
        material_index: Option<usize>,
    ) -> impl Iterator<Item = NodeHandle> + '_ {
        self.asset
            .primitives()
            .iter()
            .filter(move |source| source.material_index == material_index)
            .map(|source| self.subtree.node(source.handle).unwrap())
    }

    /// Evaluates Morph before Skin against this instance's final pose snapshot.
    /// Weight targets are destination node handles. Omitted weights use authored
    /// defaults; no graph changes occur, including on failure.
    pub fn deform(
        &self,
        poses: &EvaluatedScene,
        weights: &[(NodeHandle, Vec<f32>)],
    ) -> anyhow::Result<Vec<(NodeHandle, Mesh)>> {
        self.asset.deform(&self.subtree, poses, weights)
    }
}
