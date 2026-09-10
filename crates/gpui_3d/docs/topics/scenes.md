# Scene hierarchy

[3D viewports](../viewport.md)

## Scene hierarchy

`SceneGraph` manages nodes with optional mesh, camera, and light properties
independently of a window or GPU.
Each node has a local `AffineTransform` and inherited visibility. Evaluate the
graph once, then create scenes for different cameras from the same result.

```rust
use gpui::{Styled, rgb};
use gpui_3d::{AffineTransform, Camera, Material, Mesh, Node, ReparentMode, SceneGraph, viewport3d};

# fn main() -> Result<(), Box<dyn std::error::Error>> {
let mut graph = SceneGraph::new();
let group = graph.insert(None, Node::new().id("assembly"))?;
let part = graph.insert(Some(group),
    Node::new().id("part")
        .mesh(Mesh::cube(), Material::color(rgb(0x89c8ee)))
        .transform(AffineTransform::from_translation([1., 0., 0.])?),
)?;

graph.set_transform(group, AffineTransform::from_trs(
    [2., 0., 0.], [0., 0., 0., 1.], [1., 2., 1.],
)?)?;
let evaluated = graph.evaluate()?;
let world = evaluated.node(part).unwrap().world;
assert_eq!(world.transform_point([0., 0., 0.]), [3., 0., 0.]);
let viewport = viewport3d("assembly-view", evaluated.scene(Camera::default())).size_full();

graph.reparent(part, None, ReparentMode::KeepWorld)?;
graph.set_visible(group, false)?;
# Ok(())
# }
```

## Identity and editing

- `NodeHandle` is a graph-scoped generational handle. Removing a node invalidates
  its handle; another graph or a replacement node cannot use it interchangeably.
  Handles are runtime identities, not project serialization IDs.
- `Node::id` is an optional application ID, unique across groups and mesh nodes
  in a graph. `find` maps it to a handle. An application can also maintain its
  own external-ID-to-handle map.
- `node`, `parent`, `children`, and `roots` expose the hierarchy. `replace`
  changes node properties while retaining the handle and parent-child links.
  `set_transform`, `set_visible`, and `set_material` update individual properties.
- `remove_subtree` removes a node and all descendants, returning the invalidated
  handles. Their application IDs become available for reuse.
- Invalid or foreign handles, duplicate application IDs, cycles, and invalid
  keep-world transforms return `SceneError` without partially applying an edit.
  Individually valid local matrices can overflow when composed; evaluation
  returns the affected node and transform error instead of a partial result.

`Mesh` clones share immutable geometry. A node owns its material value and local
transform; editing either does not modify another node using the same mesh.
Groups can organize several mesh nodes under one application-defined instance.
File importers and asset/instance managers belong to extensions built on these
format-independent APIs. The graph does not load files or manage model catalogs.

`Node::surface()` borrows the current `(Mesh, Material)` pair without evaluating
the scene. Hidden mesh nodes retain their resources; groups return `None`.
Clone the material to retain its textures and other parameters while changing
one property, then apply it with `set_material` or `set_materials`.
Shared mesh and decoded-image allocations are not copied by cloning.

### Material batches

`set_materials` accepts `(NodeHandle, Material)` replacements. Every target must
be a live mesh node in this graph; hidden mesh nodes are supported. Duplicate
targets return `DuplicateMaterial`, invalid handles return `InvalidHandle`, and
non-mesh targets return `NoMesh`. Any such error leaves all materials and the
graph revision unchanged.

A nonempty batch applies every replacement and increments the revision once.
An empty batch does not change the revision. Geometry, transforms, hierarchy,
identities, and visibility remain unchanged, and retained evaluated snapshots
keep their prior materials. Material parameters and texture compatibility are
validated during scene preparation, as with `set_material`. Input iterator
side effects are outside the graph operation.

## Reusable subtrees

`snapshot_subtree(root)` captures a local hierarchy as an immutable `SceneSubtree`.
It includes the root and all descendants, including hidden nodes, in parent-first
sibling order. Ancestor transforms and inherited visibility outside that subtree
are excluded. The captured root retains its own local transform and has no parent
inside the snapshot. Later edits or destruction of the source graph do not change
the snapshot.

```rust
use gpui::rgb;
use gpui_3d::{AffineTransform, Material, Mesh, Node, SceneGraph};

# fn main() -> Result<(), Box<dyn std::error::Error>> {
let mut source = SceneGraph::new();
let root = source.insert(None, Node::new().id("assembly"))?;
let body = source.insert(Some(root), Node::new().id("body")
    .mesh(Mesh::cube(), Material::color(rgb(0x8dd8e8))))?;
let subtree = source.snapshot_subtree(root)?;

let mut scene = SceneGraph::new();
let left = scene.instantiate(None, &subtree)?;
let right = scene.instantiate(None, &subtree)?;
scene.set_transform(right.root(), AffineTransform::from_translation([3., 0., 0.])?)?;
scene.set_material(left.node(body).unwrap(), Material::color(rgb(0xf09e8e)))?;
# Ok(())
# }
```

`instantiate(parent, &subtree)` appends independent graph nodes under a parent,
or creates a new root for `None`. Local transforms, local visibility, picking
behavior, and material values are copied. Mesh allocations and image sources
remain shared. Cloning a `SceneSubtree` also shares its snapshot storage.
Edits do not propagate from the snapshot or one instance to another.

Application IDs are cleared by default so the same subtree can be instantiated
repeatedly in one graph. `instantiate_with_ids(parent, &subtree, map_id)` accepts
a callback `(source_handle, Option<&ObjectId>) -> Option<ObjectId>` to assign,
preserve, rename, or omit each ID. The callback visits every node, including
unnamed nodes. Invalid parents and IDs colliding with the destination graph or
another new node return `SceneError` before any insertion or revision change.
One successful instantiation increments the destination revision once. World
transform representability is checked during ordinary evaluation.

`SubtreeInstance::root()` returns the new root. `node(source_handle)` resolves
a snapshot handle to the corresponding destination node; `mappings()` exposes
all pairs in unspecified order. Picking, evaluated nodes, and render output IDs
use these ordinary destination handles. Extensions can retain the mapping to
associate imported nodes or primitives with their instances.

The returned instance is a non-owning handle map, not a live link or asset
manager. Dropping it does not delete graph nodes. Ordinary graph edits can
reparent or delete those nodes; mapping lookups do not validate their continued
existence. `remove_subtree(instance.root())` removes the root's current subtree,
not nodes that have since been reparented elsewhere. Snapshot handles remain
valid lookup keys after source deletion, but are not persistent file IDs.

Instantiation copies editable scene nodes while sharing resources. Rendering
batches compatible adjacent mesh nodes under the same rules as ordinary objects;
a subtree instance does not define a draw-call boundary.

## Transforms and bounds

`AffineTransform::from_trs` applies scale, quaternion rotation, then translation.
Quaternion order is `[x, y, z, w]`; finite nonzero quaternions are normalized.
Negative scale is supported. `from_matrix` accepts a column-major affine matrix,
including shear, with last row `[0, 0, 0, 1]`. Non-finite, singular, numerically
near-collinear, or unrepresentable transforms return `TransformError`.

World matrices are `parent_world * local`. `KeepLocal` retains the local matrix
when reparenting; `KeepWorld` computes a new local matrix without decomposing away
shear. Floating-point tolerance applies. Visibility is inherited from the new
parent in either mode. Normals use the world inverse-transpose matrix.

`Mesh::bounds` and `Node::local_bounds` describe mesh-local AABBs. Evaluated nodes
provide conservative world-aligned bounds and subtree bounds, including hidden
geometry. Empty groups have no own bounds. `EvaluatedScene::bounds` includes only
geometry with inherited visibility enabled; it does not test camera clipping or
occlusion. Bounds are not exact mesh overlap or collision queries.

## Evaluation and picking

`evaluate` returns an owned, camera-independent `EvaluatedScene`, retaining shared
geometry and image sources. Later edits or deletion do not change previous
results. The revision identifies graph edits within that graph; it is not a
global asset version or image-readiness indicator. Evaluation traverses the
hierarchy without recursive calls and performs no image loading or GPU work.

`evaluated.scene(camera)` supplies visible objects to the ordinary viewport and
`Scene::pick`, using the same world and normal matrices. `Hit::node` identifies
the source graph node, even without an application ID. `Hit::object_id` carries
its application ID, while `object_index` is only the index in that evaluated
scene's flattened visible-object list. Flat scenes built with `Scene::object`
have no graph handle. An old evaluated scene may return a handle already removed
from the live graph; validate it through `graph.node` before editing.

The viewport resolves image resources during rendering. Constraints and asset
readiness are independent of hierarchy evaluation.

## Camera and light nodes

`Node::camera` and `Node::light` attach local-space properties independently of
geometry. A node can have both, with or without a mesh. `local_camera()` and
`local_light()` inspect authored properties. `SceneGraph::set_camera` and
`set_light` replace or remove them with `Some(value)` or `None`, preserving the
node's transform, geometry, material, and children. Subtree snapshots and instances
copy these properties; subsequent edits remain independent.

```rust
use gpui_3d::{AffineTransform, Camera, Node, PunctualLight, SceneGraph};

let mut graph = SceneGraph::new();
let rig = graph.insert(None, Node::new()
    .transform(AffineTransform::from_translation([0., 1., 5.])?))?;
let camera = graph.insert(Some(rig), Node::new().camera(Camera {
    eye: [0.; 3], target: [0., 0., -1.], ..Default::default()
}))?;
graph.insert(Some(rig), Node::new().light(
    PunctualLight::spot([0.; 3], [0., 0., -1.]).intensity(20.)
))?;
let evaluated = graph.evaluate()?;
let scene = evaluated.scene_from_camera(camera)?;
# let _ = scene;
# Ok::<(), Box<dyn std::error::Error>>(())
```

Evaluation transforms a camera's local `eye`, `target`, and up direction with the
complete node world matrix. `Camera::default()` retains its local eye offset of
`[0, 0, 6]`; use eye zero and target negative Z for a camera at the node origin.
Projection and clipping remain explicit, without inheriting scale. Read the
world camera from `EvaluatedNode::camera` or select it with `scene_from_camera`.
Selection is never automatic, and hidden-node cameras remain explicitly usable.
Missing properties return `SceneError::NoCamera`; foreign or expired handles
return `InvalidHandle`.

Light positions and normalized directions inherit the complete world transform.
Directional lights ignore position; point lights ignore direction. Directional
directions point toward the source, whereas spot directions point outward.
Intensity, color, range, distance clamp, and cone angles do not inherit scale.
`PunctualLight::transformed` exposes the same conversion without a scene graph;
`kind()`, `position()`, and `direction()` inspect the result. `LightKind` identifies
directional, point, and spot sources.

`EvaluatedNode::light` includes hidden sources for inspection. `evaluated.lights()`
yields only visible `(NodeHandle, PunctualLight)` pairs in parent-first order.
`scene(camera)` and `scene_from_camera` use this list as direct lighting if the
graph contains any attached lights. If all are hidden, direct light is disabled;
ambient illumination remains. Graphs without light properties retain the default
scene light. Calling `Scene::light` or `Scene::lights` explicitly replaces the
derived direct-light configuration. These properties do not add geometry, bounds,
or picking/output IDs.

Evaluation permits more than `MAX_PUNCTUAL_LIGHTS` sources; rendering rejects an
excess list before resource resolution. Select a subset with `Scene::lights` when
needed. Directional-shadow `light_index` addresses the final visible/selected
light list, not a node index; recompute it from node identities when visibility
or hierarchy order changes. Cameras and lights use the same pose overrides and
keep-world reparenting as mesh nodes.

Camera and light validation runs during evaluation, including hidden nodes.
`SceneError::InvalidCamera` and `InvalidLight` identify the node and underlying
`CameraError` or `LightError`. Invalid parameters, transformed overflow, or a
camera view lost to coordinate precision fail evaluation without modifying
authored data or earlier snapshots. There is no active-camera state, controller,
light-selection policy, or animation clock inside the graph.

## Related topics

[Animation and deformation](animation.md). See [Scene evaluation](evaluation.md)
for nonmutating local-transform and mesh replacements.
