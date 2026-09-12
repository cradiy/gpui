# Spatial queries and picking

[3D viewports](../viewport.md)

For comparisons against rendered samples, see [Frame depth comparisons](depth_queries.md).

`Ray::new(origin, direction)` accepts arbitrary world rays and normalizes their
direction. `Scene::raycast(ray)` ignores the camera and its clipping planes,
while respecting mesh geometry, material/vertex alpha, and picking behavior.
It does not resolve images or sample image alpha. Query distance is measured from
the ray origin. Hit normals are normalized after interpolation and world-space
transformation, then flipped on back faces; a zero interpolated normal remains zero.

`Scene::pick_where` and `Scene::raycast_where` accept a per-query predicate over
`QueryObject`. Its borrowed application ID, graph node handle, scene-local index,
and authored picking behavior let the caller filter by an object set or external
metadata without changing scene visibility or rebuilding the spatial index.

```rust
use gpui::rgb;
use gpui_3d::{Material, Mesh, Object, ObjectId, Ray, Scene};
use std::collections::HashSet;

let scene = Scene::new()
    .object(Object::new(Mesh::plane(), Material::color(rgb(0xffffff))).id("surface"));
let allowed = HashSet::from([ObjectId::from("surface")]);
let ray = Ray::new([0., 0., 6.], [0., 0., -1.])?;
let hit = scene.raycast_where(ray, |object| {
    object.object_id.is_some_and(|id| allowed.contains(id))
});
assert!(hit.is_some());
# Ok::<(), gpui_3d::RayError>(())
```

Rejected objects neither return hits nor occlude this query. Accepted candidates
still honor `PickBehavior` and material/vertex alpha; accepting an `Ignore`
object does not make it selectable. Use graph handles or application IDs for
persistent filters because flattened indices can change between evaluated
snapshots. Hidden graph nodes are absent from the scene query.

The predicate runs at most once per visited BVH candidate, before triangle
traversal, in unspecified order. It is not called for every scene object and does
not report occlusion visibility. Screen queries retain viewport and camera clip
checks; world rays remain independent of the camera. Neither filtered public
query resolves image alpha or resource readiness; prepared viewport callbacks
retain their image-alpha sampling behavior.

Mesh queries use a CPU bounding-volume hierarchy (BVH) built lazily on the first
query. `mesh.prepare_spatial_index()` builds it synchronously in advance, without
a window or GPU; a worker can prepare a mesh clone before interactive use.
Clones and subtree instances share the index. Object transforms and material
changes do not rebuild it, and dropping the last mesh reference releases it.
Meshes used only for rendering do not build a query index.

Traversal tests conservative world-space bounds, then intersects candidate
triangles with the same world-space geometry used by picking. Source vertices
and indices remain unchanged. Exactly equal hit distances prefer the earlier
object in scene order, then the earlier triangle in its index buffer. Alpha
cutouts and clip rejection continue searching for eligible surfaces behind the
rejected hit; occluder-only surfaces still block them.

Queries first traverse a world-space object BVH, then the candidate meshes' BVHs.
`Scene::prepare_spatial_index()` prepares only the object index; triangle indices
remain lazy. Local bounds are computed once per shared geometry during object
index construction. Objects with unrepresentable bounds remain candidates for
the ordinary triangle query rather than being silently culled.

Scene clones share their object index. Changing a scene's camera or lighting
retains it; appending an object creates a fresh index without changing earlier
clones. Each `EvaluatedScene` owns a shared index that all scenes derived through
`scene(camera)` reuse. `EvaluatedScene::prepare_spatial_index()` prepares it
without selecting a camera. After graph edits, call `evaluate()` for a new state
with current transforms and inherited visibility. Earlier evaluated states and
their queries remain unchanged, including after node deletion.

Index preparation is synchronous and CPU-only. Reusing an evaluated state across
camera updates avoids rebuilding the object hierarchy. Before querying a new
evaluation, `prepare_spatial_index_from(&previous)` can reuse a prepared snapshot's
partition and update the bounds of changed leaves and their ancestors:

```rust
use gpui_3d::{AffineTransform, Material, Mesh, Node, SceneGraph};

let mut graph = SceneGraph::new();
let object = graph.insert(None, Node::new().mesh(
    Mesh::cube(), Material::color(gpui::rgb(0x80a0c0)),
))?;
let previous = graph.evaluate()?;
previous.prepare_spatial_index();

graph.set_transform(object, AffineTransform::from_translation([2., 0., 0.])?)?;
let current = graph.evaluate()?;
current.prepare_spatial_index_from(&previous);
# Ok::<(), Box<dyn std::error::Error>>(())
```

Graph indices retain slots for hidden mesh nodes. Motion, mesh replacement,
visibility changes, and reparenting preserve the partition when the mesh-node
set is unchanged; query indices follow the current visible scene order. Adding
or removing mesh nodes, using another graph, or changing whether a bound is
representable rebuilds the index. Group-only edits do not require new slots.
`Scene::prepare_spatial_index_from` offers the same operation for flat scenes,
whose slots are object positions in the list rather than graph identities.

An unprepared previous snapshot causes a fresh build. A current index already
prepared by a query or explicit preparation is unchanged. Each evaluation still
computes its full transforms and bounds; refitting only changes index preparation.
The partition is shared, while changed index bounds use independent storage.
Old snapshots retain their queries without retaining the graph or its edit history.

Refitting does not rebalance the partition. After large movements, preparing a
fresh evaluation without a previous index can improve query pruning. Heavily
overlapping object or triangle bounds can still require broad traversal. These
indices accelerate queries, not render submission or GPU draw batching.

`cargo bench -p gpui_3d --bench scene -- spatial_index` compares full builds and
refits for unchanged, one-percent-motion, and all-motion scenes. Graph evaluation
is outside the timed routine; the routine includes preparation and snapshot release.

Invalid camera, viewport, and point inputs return `CameraError` from the public
matrix/projection/query methods. Invalid rays return `RayError`.

## Bounds overlap and distance

`Aabb::intersects(other)` tests closed axis-aligned boxes. Touching faces, edges,
and corners count as intersections, including boxes with zero extent.
`intersection(other)` returns their shared box or `None`. `distance(other)` is
the minimum Euclidean distance between the closed boxes: zero for overlap or
contact, never a signed penetration depth. It returns f64 to keep distances
finite across the full valid f32 coordinate range. These operations compare
bounds, not the meshes they enclose. Points can be represented as zero-extent
boxes through `Aabb::new(point, point)`.

`Scene::bounds_candidates(region)` returns objects whose conservative world AABBs
overlap the supplied world-space region. It uses the shared object BVH and tests
individual object bounds after branch pruning; it neither builds nor traverses
mesh triangle indices. Bounds enclose all mesh vertices, including unreferenced
vertices, with padding for transform arithmetic. Results can include empty parts
of a mesh's AABB or small gaps within that padding. Objects without computable
finite bounds remain candidates. This query does not validate flat-object
transforms; their normal construction requirements still apply.

```rust
use gpui_3d::{Aabb, Material, Mesh, Object, PickBehavior, Scene};

let scene = Scene::new()
    .object(Object::new(Mesh::cube(), Material::color(gpui::rgb(0x80a0c0)))
        .id("crate").position([2., 0., 0.]));
let region = Aabb::new([1., -1., -1.], [3., 1., 1.]).unwrap();
let candidates = scene.bounds_candidates_where(region, |object| {
    object.pick_behavior != PickBehavior::Ignore
});
for object in candidates {
    let identity = (object.node, object.object_id, object.object_index);
    # let _ = identity;
}
```

Results are borrowed `QueryObject` identities in scene object order, not distance
order. `bounds_candidates_where` invokes its predicate exactly once for each
candidate in that order. Use node handles or application IDs for persistent
identity; scene-local indices can change after evaluation. Filtering does not
change rendering or other queries.

The camera, occlusion, material alpha, image availability, and `PickBehavior` do
not restrict bounds candidates. Apply picking policy explicitly in the predicate
if appropriate. Scenes from hierarchy evaluation contain only mesh nodes with
inherited visibility enabled; group bounds are not additional results. Updated
transforms or deformed meshes require a new evaluated scene. Spatial refitting
and retained snapshots follow the same rules as ray queries. Candidates establish
neither precise mesh contact nor confirmed screen visibility.

## Object picking

Assign stable IDs to objects and attach handlers to the viewport:

```rust
use gpui::{Styled, rgb};
use gpui_3d::{Material, Mesh, Object, Scene, viewport3d};

let world = Scene::new().object(
    Object::new(Mesh::cube(), Material::color(rgb(0x89c8ee))).id("cube"),
);
let viewport = viewport3d("world", world)
    .size_full()
    .on_object_hover(|hit, _window, _cx| {
        // Update application hover state from hit.as_ref().and_then(|h| h.object_id.as_ref()).
    })
    .on_object_click(|hit, _window, _cx| {
        // Use hit.object_id and hit.uv to select an object or inspect its surface.
    });
```

`ObjectId` uses GPUI's `ElementId` representation. Keep IDs unique within a
viewport and stable across scene rebuilds. Unnamed objects remain pickable and
occlude objects behind them; their hit carries `object_id: None`.

`Object::pick_behavior` controls interaction independently of rendering:

- `PickBehavior::Target` returns hits and blocks objects behind the surface.
- `PickBehavior::Occlude` blocks objects behind the surface without returning a hit.
- `PickBehavior::Ignore` lets picking pass through the object, including opaque regions.

```rust
use gpui::{Styled, rgb};
use gpui_3d::{Material, Mesh, Object, PickBehavior, Scene, viewport3d};

let cover = Object::new(Mesh::plane(), Material::color(rgb(0x20314b)))
    .pick_behavior(PickBehavior::Occlude);
let viewport = viewport3d("world", Scene::new().object(cover)).size_full();
```

`Hit` provides the object and triangle indices, world position, interpolated
world-space shading normal, UV, barycentric weights, and distance from the ray
origin. With a perspective camera this is distance from the eye; with an
orthographic camera it is forward distance from the eye plane.
Material face visibility controls picking; backface normals follow the renderer's flipped shading
normal convention. Hits respect the camera's near and far clip planes. Equal-depth
ties use scene insertion order.

Hover callbacks run on pointer movement, with `None` on a miss or viewport exit.
They do not schedule frames or recompute hover for a stationary pointer when a
scene changes. Click callbacks handle left clicks with endpoints on the same
mesh and at most four logical pixels apart. When sharing the button with camera
gestures, ignore clicks after a drag, including drags returning to their starting
position. Application state changes should notify the view as usual.

Viewport callbacks sample the first image frame's alpha using its selected
coordinate set, UV transform, addressing and filter, matching the material shader. Sampled
alpha is multiplied by material alpha and evaluated using the material's alpha
mode; discarded regions allow hits on surfaces behind them. This also applies to occluders.
Images that are loading, failed, empty, or unavailable to the renderer do not
receive hits or block picking. The query uses the image data prepared for the
painted scene, without decoding images or reading GPU pixels during pointer events.

Captured UI textures still use geometric picking, without sampling their alpha;
UI materials with no attached capture are skipped. Viewport callbacks use GPUI's
normal hitbox routing for ancestor clipping and overlapping UI. They do not map
events into captured UI controls or account for visual effect deformation.

`Scene::pick(bounds, position)` provides a geometric query for custom input
handling. It respects picking behavior and material/vertex alpha but does not
resolve images or sample texture alpha. Both arguments use logical window
coordinates; the caller supplies the viewport bounds and handles UI clipping
and input routing. Queries traverse the scene and mesh BVHs on the CPU; their
preparation and snapshot-sharing rules are described above.

## Related topics

[Camera projection](camera.md).
