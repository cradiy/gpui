# 3D viewports

`gpui_3d` embeds depth-tested mesh scenes in ordinary GPUI layouts. A viewport
supports perspective and orthographic cameras, indexed triangle geometry, direct lights,
and solid, image or captured-UI materials.

```rust
use gpui::{Styled, rgb};
use gpui_3d::{Camera, Material, Mesh, Object, Scene, viewport3d};

let world = Scene::new()
    .camera(Camera::orbit(0.4, 0.2, 5.))
    .object(
        Object::new(Mesh::cube(), Material::color(rgb(0x89c8ee)))
            .position([0., 0., 0.])
            .rotation([0., 0.5, 0.])
            .scale([1.5, 1., 1.]),
    );

let viewport = viewport3d("world", world).size_full();
```

Give the viewport an explicit size or a bounded parent. Standard `Styled`
methods control its layout and outer appearance. Use an enclosing interactive
`div` for pointer handlers; update the camera and notify the view after input.
The viewport does not schedule animation frames itself.

## Backend support

`Window::scene3d_support()` reports the current window's mesh capabilities or a
`Scene3dUnsupportedReason`. `Window::supports_scene3d()` is the boolean check.
Query inside the view's render/update path when choosing between a mesh viewport
and ordinary UI; querying does not request a frame.

```rust,no_run
use gpui::{Scene3dSupport, Window};

fn viewport_status(window: &Window) -> String {
    match window.scene3d_support() {
        Scene3dSupport::Supported(caps) => format!("3D · up to {} samples", caps.color_samples),
        Scene3dSupport::Unsupported(reason) => reason.to_string(),
    }
}
```

The capabilities include the renderer-selected color sample count, maximum
physical texture dimension, and captured-UI texture limit. The Linux WGPU path
uses four color samples when its linear-color and depth formats support them,
otherwise one. UI raster density is uniformly reduced to fit both the 2048-pixel
capture limit and the device limit without changing logical layout.

Unavailable states distinguish an unimplemented backend, absent renderer
resources, observed device loss, and missing device limits or format features.
Unsupported viewports retain layout and outer styling but do not paint a mesh or
capture UI; object callbacks do not report hits and pending UI routing is cleared
on the next prepaint. The application chooses its fallback content.

Linux X11 and Wayland query their current WGPU renderer. Other platform windows
report `BackendUnsupported` unless their renderer implements this capability.
Support is refreshed when the WGPU renderer is recreated; do not retain a
support result across device replacement. Viewport support is independent of
headless output-channel support and does not certify allocation success or
runtime rendering on an unvalidated platform.

## Coordinates and geometry

World coordinates are right-handed, with positive Y up. The default camera is
at `[0, 0, 6]`, looking toward the origin. Angles are radians. `Camera::orbit`
orbits the origin; `Camera` exposes `eye`, `target`, `up`, `projection`,
`lens_shift`, and clip distances for explicit positioning. Projection matrices are column-major, with
camera forward along local -Z and hardware depth from zero to one. Scene units
are application-defined; camera distances and geometry must use the same units.

`Mesh::plane()` is a unit XY square facing positive Z. `Mesh::cube()` is a unit
cube centered at the origin. Both reuse shared geometry. `Mesh::new` accepts
vertices with position, normal and UV, plus counterclockwise triangle indices.
UV `(0, 0)` is at the top left. Faces render from both sides.

`Mesh::try_new(vertices, indices)` returns `Result<Mesh, MeshError>`. Vertices and
indices must be nonempty, index counts must be multiples of three, and each index
must reference an existing vertex. Position, normal and UV components must be
finite, including those of unused vertices. Errors identify an invalid index's
buffer offset or a non-finite vertex's attribute and component, all zero-based.
`Mesh::new` uses the same validation and panics on invalid input.

```rust
use gpui_3d::{Mesh, Vertex};

# fn main() -> Result<(), gpui_3d::MeshError> {
let vertices = [[-1., -1., 0.], [1., -1., 0.], [0., 1., 0.]]
    .map(|position| Vertex { position, normal: [0., 0., 1.], uv: [0.; 2] })
    .to_vec();
let mesh = Mesh::try_new(vertices, vec![0, 1, 2])?;
let triangle = &mesh.indices()[..3];
let first_position = mesh.vertices()[triangle[0] as usize].position;
# Ok(())
# }
```

`vertices()` and `indices()` borrow immutable storage; mesh clones share that
storage. `vertex_count()`, `index_count()` and `triangle_count()` report the stored
data, without filtering. `bounds()` computes mesh-local bounds of all vertices,
including unused ones, and permits zero extent.

Construction preserves ordering, repeated indices, zero normals and finite UVs
outside `[0, 1]`. It does not generate normals or remove degenerate triangles.
Zero-area triangles have no filled surface and are skipped by ray picking;
retaining their indices preserves subsequent `Hit::triangle_index` values.
Nonzero normals are normalized during shading. Geometry validation does not
guarantee that every camera or transform will yield numerically representable
rendering or intersection results.

Object transforms apply scale, X/Y/Z Euler rotation, then translation. Normals
use inverse-transpose transforms for nonuniform scale. Scale components must be
finite and nonzero. Camera clip distances must satisfy `0 < near < far`.
Perspective cameras allow positive infinity for `far`; orthographic cameras
require a finite far distance.

### Primitive meshes

`Mesh::sphere`, `Mesh::cylinder`, `Mesh::cone`, and `Mesh::subdivided_plane`
accept configuration structs and return `Result<Mesh, PrimitiveError>`.
Dimensions are finite positive scene units. Generation is synchronous; build a
mesh during resource preparation and clone it across objects to share storage.

```rust
use gpui_3d::{ConeOptions, CylinderOptions, Mesh, PlaneOptions, SphereOptions};

let sphere = Mesh::sphere(SphereOptions {
    radius: 0.7,
    segments: [64, 32],
})?;
let cylinder = Mesh::cylinder(CylinderOptions {
    height: 2.,
    segments: [48, 4],
    ..Default::default()
})?;
let cone = Mesh::cone(ConeOptions { capped: false, ..Default::default() })?;
let grid = Mesh::subdivided_plane(PlaneOptions {
    size: [4., 3.],
    segments: [16, 12],
})?;
# Ok::<(), gpui_3d::PrimitiveError>(())
```

Planes face +Z and default to a unit square with one interval per axis. Their
segments are `[horizontal, vertical]`, each at least one. Spheres are centered at
the origin with default radius 0.5 and `[longitude, latitude]` segments `[32, 16]`;
longitude requires at least three sectors and latitude at least two intervals.

Cylinders and cones use the Y axis, default radius 0.5, height 1, and
`[radial, height]` segments `[32, 1]`. Radial sectors require at least three and
height intervals at least one. `capped` defaults to true: both cylinder ends or
the cone base are closed. A cone's tip is at `+height/2` and its base at
`-height/2`.

All primitives provide outward counterclockwise triangles, unit normals, and
analytic tangent frames. Round side surfaces use U from +X toward +Z and V from
top to bottom. UV seams have coincident positions but distinct U values. Sphere
poles and cone tips use a separate vertex per sector with midpoint U; the sphere
normal remains axial at each pole. Flat cap normals and disk UVs use separate
vertices from the sides. Plane UV `(0, 0)` is upper-left. Bounds describe the
generated vertices, so coarse round meshes need not reach every ideal radial
extremum.

Generation rejects invalid dimensions or segment counts before allocation.
Each mesh is limited to 1,048,576 vertices and 6,291,456 indices. Dimensions that
collapse a generated triangle in f32 coordinates return `PrimitiveError::Degenerate`
with its triangle index. Primitive generation does not emit zero-area pole or
tip triangles.

### Normal generation

`Mesh::generate_normals(mode)` returns a `GeneratedNormals` mesh and an
output-to-source vertex map. Normals come from indexed positions and
counterclockwise winding, independently of existing normal values or UVs.

- `NormalMode::Flat` assigns each face's unit normal, splitting shared source
  vertices where the generated face normals differ. Coplanar faces can reuse
  vertices.
- `NormalMode::Smooth` sums face cross products at each shared source index and
  normalizes the sum, giving area-weighted smoothing. It does not weld coincident
  positions or smooth across separate source indices. There is no implicit
  crease-angle threshold.

```rust
use gpui_3d::{Mesh, NormalMode, Vertex};

let vertices = [[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]]
    .map(|position| Vertex { position, normal: [0.; 3], uv: [0.; 2] });
let source = Mesh::try_new(vertices.to_vec(), vec![0, 1, 2])?;
let generated = source.generate_normals(NormalMode::Flat)?;
let mesh = generated.mesh();
let source_index = generated.source_vertices()[0];
# let _ = (mesh, source_index);
# Ok::<(), Box<dyn std::error::Error>>(())
```

Triangle order, winding, positions, and UVs remain unchanged. Output vertices
follow first use, omit unreferenced vertices, and do not merge distinct source
indices. Output bounds exclude omitted positions. Existing normals are replaced
and tangents are removed. Generate tangents from the resulting normals and UVs
when required by the material.

`NormalGenerationError` identifies zero-area triangles and undefined smooth
normals caused by cancelling incident faces. Generation returns an error instead
of removing triangles or choosing an arbitrary normal. Cross products and smooth
sums use f64; output normals are normalized f32 vectors. The source mesh remains
unchanged on success and failure.

`source_vertices()[output_index]` maps generated vertices to source attributes.
Use it to remap skin influences, position deltas, and external data. If tangent
generation also splits vertices, compose its map with the normal-generation map.
Vertex correspondence does not convert normal or tangent deltas to a new basis;
recompute those deltas when replacing their authored basis. To derive normals
from deformed positions, generate after evaluating the position deformation.
Generation is explicit CPU work, not an automatic rendering step.

### Fixed-topology updates

`Mesh::with_vertices(vertices, tangents)` returns an immutable snapshot with new
positions, normals, and UVs. Vertex count and triangle indices remain unchanged,
including unused vertices and degenerate triangles. Snapshots share index storage
but have independent vertex data, bounds, and lazy query indices. Earlier meshes,
evaluated scenes, and submitted headless outputs remain valid.

```rust
use gpui_3d::{Material, Mesh, Node, SceneGraph};

let source = Mesh::plane();
let mut vertices = source.vertices().to_vec();
for vertex in &mut vertices {
    vertex.position[2] += 0.25;
}
let updated = source.with_vertices(vertices, source.tangents().map(<[_]>::to_vec))?;
let mut graph = SceneGraph::new();
let node = graph.insert(None, Node::new().mesh(source, Material::color(gpui::rgb(0xffffff))))?;
graph.set_mesh(node, updated)?;
let evaluated = graph.evaluate()?;
# let _ = evaluated;
# Ok::<(), Box<dyn std::error::Error>>(())
```

All replacement attributes must be finite. Supply tangents appropriate for the
new normals; `None` explicitly omits them. Tangent validation orthogonalizes XYZ
and retains triangle handedness requirements. `MeshUpdateError` distinguishes
vertex count, geometry, and tangent failures. Replacements do not generate normals
or tangents automatically.

`SceneGraph::set_mesh` updates an existing mesh node and its local bounds without
changing its material, identity, hierarchy, or transform. Groups return
`SceneError::NoMesh`; failed updates leave the graph unchanged. Evaluate the graph
again for current world bounds and picking.

The WGPU renderer reuses vertex and index buffers from retired snapshots with
shared topology and equal vertex counts. Simultaneously visible snapshots keep
separate vertex contents. Replacement uploads are ordered before their draws,
including shadow and geometry-output passes. Upload staging storage is allocated
per replacement snapshot; cached replacements replay that copy when drawn.
Meshes kept in their original allocations require no vertex uploads.
Cache entries absent from the prepared scenes are released. Vertex/index count
changes require a new `Mesh`; batched instance streams are separate from vertex
replacement.

### Instanced draws

Reuse a `Mesh` across objects to share GPU geometry and allow automatic WGPU
batching. Each object retains its own transform, base tint, visibility, and
picking identity.

```rust
use gpui_3d::{Material, Mesh, Object, Scene};

let mesh = Mesh::cube();
let mut scene = Scene::new();
for (index, color) in [0x8dd8e8, 0xf4cf89, 0x526a87].into_iter().enumerate() {
    scene = scene.object(
        Object::new(mesh.clone(), Material::color(gpui::rgb(color)))
            .id(format!("cube-{index}"))
            .position([index as f32 * 1.5, 0., 0.]),
    );
}
```

Adjacent opaque or masked objects with the same mesh storage, textures, sampling,
shading factors, and shadow settings share one instanced draw. Transforms, normal
matrices, base tints, and output IDs are per-instance inputs. Separately constructed
meshes are not deduplicated, even when their vertices match. Material changes
split batches. Blended objects retain their back-to-front order and individual
draws; batching does not reorder opaque or masked objects.

Use ordinary object values or `SceneGraph` edits followed by evaluation to supply
new instance data. Instance buffers retain their allocation while capacity fits
and grow within device limits. Unused batch slots are released. Uniform and
instance uploads are encoded before their draws, including shadows and headless
geometry channels, so previously encoded frames retain their inputs. CPU picking
and bounds remain per object.

### Morph targets

`MorphTargets` binds immutable target deltas to a base `Mesh`.
`MorphTarget::positions`, `normals`, and `tangents` contain optional dense XYZ
arrays in mesh-local space. Each supplied array must contain one finite delta
per base vertex, including unused vertices. A target needs at least one attribute;
the target list itself may be empty. Tangent deltas require base mesh tangents
and never modify handedness W. Clones share the base mesh and delta arrays.

```rust
use gpui_3d::{Mesh, MorphTarget, MorphTargets};

let base = Mesh::plane();
let targets = MorphTargets::new(base.clone(), [
    MorphTarget {
        positions: Some(vec![[0., 0., 0.5]; base.vertex_count()].into()),
        ..Default::default()
    },
    MorphTarget {
        positions: Some(vec![[0.25, 0., 0.]; base.vertex_count()].into()),
        ..Default::default()
    },
])?;
let mesh = targets.evaluate(&[0.7, -0.2])?;
# let _ = mesh;
# Ok::<(), gpui_3d::MorphError>(())
```

Evaluation computes `base + sum(weight * delta)` before node transforms. Weights
must be finite and match the target count; negative weights and values above one
are supported without clamping or normalization. Omitted attributes contribute
zero, and UVs and triangle identities remain unchanged. Normals with active
deltas are normalized after the complete blend. Tangents are orthogonalized
against the resulting normals with base handedness. Position-only targets do
not regenerate normals. Zero normals are supported without tangents; undefined
tangent bases return an error.

`evaluate` is synchronous and CPU-only, returning an ordinary immutable `Mesh`
with shared index storage and an independent lazy query index. Use the result in
`Object::new` or `SceneGraph::set_mesh`, then evaluate the graph for current world
bounds. The same vertex snapshot is used by viewport/headless rendering and ray
queries. Zero weights return the shared base mesh. Keep a sampled mesh while its
weights are unchanged; the evaluator has no history, internal cache, or clock.
Work scales with vertex count and the number of nonzero targets.

`MorphError` reports invalid target/attribute/vertex offsets, mismatched weights,
nonfinite inputs, unrepresentable positions, and invalid resulting tangent data.
Failed evaluation does not modify the base or previous results. File-format
decoding, sparse-array expansion, weight animation, and playback policy belong
to the caller.

### Skeletal skinning

`Skin` stores shared inverse bind matrices and per-vertex `SkinInfluence` lists.
An inverse bind matrix maps mesh bind-space coordinates into one joint's
bind-local space. Influence joint indices address this array. The vertex lists
must include unused vertices and match the vertex order of every sampled mesh.
The binding does not own a mesh, so a morph result can be sampled directly.

```rust
use gpui_3d::{AffineTransform, Mesh, Skin, SkinInfluence};

let mesh = Mesh::plane();
let bind = AffineTransform::from_translation([0., 0.25, 0.])?;
let skin = Skin::new(
    [AffineTransform::IDENTITY, bind.inverse()],
    mesh.vertices().iter().map(|v| {
        let weight = v.position[1] + 0.5;
        [
            SkinInfluence { joint: 0, weight: 1. - weight },
            SkinInfluence { joint: 1, weight },
        ]
    }),
)?;
let bent_joint = AffineTransform::from_trs(
    [0., 0.25, 0.], [0., 0., 0.3_f32.sin(), 0.3_f32.cos()], [1.; 3],
)?;
let posed_mesh = skin.evaluate(&mesh, &[AffineTransform::IDENTITY, bent_joint])?;
# let _ = posed_mesh;
# Ok::<(), Box<dyn std::error::Error>>(())
```

Weights must be finite and nonnegative, with a positive total for each vertex.
They are normalized per vertex during construction using f64 accumulation.
Zero entries are discarded after their joint indices are validated; repeated
joint entries contribute additively. There is no fixed joint or influence limit.
Rigid vertices can reference one joint with a positive weight. Empty joint arrays,
empty vertex lists, missing influences, and out-of-range joints return `SkinError`.

`evaluate` accepts current joint-local-to-mesh-local transforms. Each is multiplied
by its inverse bind matrix before per-vertex linear blending. `evaluate_world`
instead accepts `mesh_world` and current world-space joint transforms, computing
`inverse(mesh_world) * joint_world * inverse_bind`. Supply the complete joint
array in binding order. With `SceneGraph`, first evaluate the joint hierarchy,
collect each joint's `EvaluatedNode::world`, sample the skin, then assign the
result with `set_mesh` and evaluate the scene again. Render under the same
`mesh_world` used for sampling.

Positions use the blended affine transform. Normals use its inverse transpose
and are normalized; zero source normals remain zero without tangents. Tangent XYZ
uses the blended linear transform and is orthogonalized against the output normal.
Reflections flip tangent handedness. Triangle order is preserved, so skinning does
not repair folded geometry or reversed winding. Mixed tangent handedness within
one triangle returns a mesh validation error. Normals are not regenerated from
deformed triangles or spatial weight gradients.

All input and blended transforms must satisfy `AffineTransform`'s invertibility
and finite f32 constraints. Even invertible joint transforms can blend to a
singular matrix; this returns `SkinError::InvalidVertexTransform` with the vertex
offset. Matrix composition overflow and out-of-range positions are also errors.
Failed samples leave the source and previous results unchanged.

Sampling is synchronous CPU linear-blend skinning, not dual-quaternion skinning
or a GPU deformation pass. Work scales with joints, vertices, and retained
influences. Results are immutable `Mesh` snapshots with shared indices, fresh
bounds and query indices, and the existing topology-compatible GPU buffer reuse.
Rendering and picking consume the same geometry. Apply morph targets first, then
skin the morph result; do not feed a previous skinned result into the next sample.
Keep the sampled mesh while the pose is unchanged. File loading, joint selection,
animation playback, and pose caching remain caller-owned.

## Camera projection and queries

`Camera::projection` selects `Projection::Perspective { vertical_fov }` in radians
or `Projection::Orthographic { vertical_size }` in scene units. Orthographic size
is the full vertical span; horizontal span is `vertical_size * aspect`. Perspective
objects shrink with distance; orthographic objects keep their projected size.
`aspect_ratio: None` uses the output width/height. `Some(ratio)` fixes the projection
aspect independently of output dimensions; the ratio must be finite and positive.
Pixels still span the full output, so a mismatched output ratio stretches the
image. Letterboxing or matching output dimensions is caller-owned. Projection,
ray, depth-reconstruction, background and framing calculations use the same ratio.

Perspective `far: f32::INFINITY` uses an infinite projection with finite matrix
coefficients. Near clipping remains active. Depth and ray queries have no finite
far limit, while hardware depth precision still limits distinguishable distant
surfaces. Frustum and clipped-bounds queries support this unbounded volume.
`frame_bounds` preserves the aspect setting but fits finite near/far distances
to the supplied bounds.

`up` controls camera roll. A nearly parallel up vector uses a deterministic
world-axis fallback so exact top and bottom views remain defined.

```rust
use gpui::{Bounds, point, px, size};
use gpui_3d::{Camera, Projection};

# fn main() -> Result<(), Box<dyn std::error::Error>> {
let camera = Camera {
    projection: Projection::Orthographic { vertical_size: 4. },
    ..Default::default()
};
let viewport = Bounds::new(point(px(80.), px(40.)), size(px(800.), px(600.)));
let projected = camera.world_to_screen(viewport, [1., 0., 0.])?.unwrap();
let ray = camera.screen_to_ray(viewport, projected.position)?;
let world = camera.screen_to_world(viewport, projected.position, projected.depth)?;
let view = camera.world_to_view([1., 0., 0.])?;
let matrix = camera.view_projection(800. / 600.)?;
# Ok(())
# }
```

These APIs require no `Window`, layout, or GPU:

| API | Result |
| --- | --- |
| `axes` | Camera right, up, and backward unit vectors |
| `view_matrix` | World-to-view matrix |
| `projection_matrix(aspect)` | View-to-clip matrix |
| `view_projection(aspect)` | World-to-clip matrix used by rendering |
| `world_to_view(point)` | Camera-space position; points in front have negative Z |
| `world_to_screen(viewport, point)` | Screen position, NDC, linear forward depth, and frustum membership |
| `screen_to_ray(viewport, position)` | Normalized world-space ray |
| `screen_to_world(viewport, position, depth)` | World position from positive linear camera-forward depth |
| `frustum(aspect)` | Owned camera clip-volume snapshot for repeated world-AABB queries |
| `project_bounds(viewport, bounds)` | Screen rectangle of the clipped world AABB, or `None` for an empty intersection |

Screen coordinates use a top-left origin and include the viewport offset. Use
logical viewport bounds and logical input positions for GPUI handlers. The math
is scale-independent: multiplying both bounds and screen coordinates by the same
DPI scale produces the same ray. Pixel centers in physical image data are at
`(x + 0.5, y + 0.5)`; convert them and the viewport into one coordinate system
before querying. No implicit DPI conversion is performed.

`world_to_screen` returns `None` on or behind the eye plane. Points in front but
outside the viewport or clip planes retain their projected coordinates with
`in_frustum = false`. The near plane is included and the far plane excluded.
Frustum membership is geometric, not proof that a point is unoccluded.

`screen_to_world` inverts screen projection using linear forward depth in scene
units, not normalized hardware depth or distance along a picking ray. Depth must
be finite and strictly positive. Off-screen positions and depths outside the
near/far interval remain valid inputs; reconstruction does not test visibility.
Viewport offsets, DPI-scaled coordinates, and lens shifts follow the same
conventions as `world_to_screen`.

Perspective rays originate at the eye. Orthographic rays originate at the
corresponding point on the eye plane and have parallel directions. Both extend
forward without an intrinsic near/far limit. `screen_to_ray` accepts positions
outside the viewport for captured drags; `Scene::pick` still enforces viewport
bounds and the camera's clip range. UI pointer mapping uses the same projection
for either camera type.

`Ray::new(origin, direction)` accepts arbitrary world rays and normalizes their
direction. `Scene::raycast(ray)` ignores the camera and its clipping planes,
while respecting mesh geometry, constant material alpha, and picking behavior.
It does not resolve images or sample image alpha. Query distance is measured from
the ray origin.

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
still honor `PickBehavior` and constant material alpha; accepting an `Ignore`
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

### Bounds overlap and distance

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

### Focal length and lens shift

`Projection::from_focal_length(focal_length, sensor_height)` constructs a
perspective projection. Both dimensions must be positive and finite and use the
same units, such as millimeters. The vertical FOV is
`2 * atan(sensor_height / (2 * focal_length))`.
`projection.focal_length(sensor_height)` performs the inverse conversion for
perspective views. Orthographic views have no focal length. Invalid inputs return
`InvalidProjection`; values outside representable f32 optics return
`CameraError::Unrepresentable`.

Viewport aspect determines horizontal coverage. For a 36 × 24 mm sensor, use
aspect `36 / 24` to retain the full sensor gate. Other output aspects preserve
vertical FOV and change horizontal coverage; cropping or fitting a sensor gate
is an application decision.

```rust
use gpui_3d::{Camera, Projection};

let camera = Camera {
    projection: Projection::from_focal_length(50., 24.)?,
    lens_shift: [0.4, -0.2],
    ..Default::default()
};
let matrix = camera.projection_matrix(36. / 24.)?;
let focal_mm = camera.projection.focal_length(24.)?;
# Ok::<(), gpui_3d::CameraError>(())
```

`lens_shift` defaults to `[0, 0]`. Its X/Y values move the projection center in
camera-right/up directions, measured in half-viewport spans. A value of one
moves coverage by half the full width or height. The optical axis projects to
NDC `[-shift_x, -shift_y]`, so positive X shifts objects left on screen and
positive Y shifts them down. Any finite shift is accepted, including views whose
optical axis lies outside the image.

Shift applies to both projection kinds without moving or rotating the camera.
Matrices, picking rays, frustum culling, environment backgrounds, and captured UI
mapping use the same shifted projection. Orthographic background directions stay
parallel. Orbit, pan, dolly, and zoom retain the shift; optical zoom remains
centered around the shifted principal point rather than the viewport center.

### Projecting bounds

```rust
use gpui::{Bounds, point, px, size};
use gpui_3d::{Aabb, Camera};

let camera = Camera::default();
let viewport = Bounds::new(point(px(20.), px(40.)), size(px(800.), px(600.)));
let bounds = Aabb::new([-1.; 3], [1.; 3]).unwrap();
let frustum = camera.frustum(800. / 600.)?;
if frustum.intersects(bounds) {
    let screen_bounds = frustum.project_bounds(viewport, bounds)?;
    // Use the optional rectangle for a screen-space annotation or region query.
    assert!(screen_bounds.is_some());
}
# Ok::<(), gpui_3d::CameraError>(())
```

`Frustum::intersects` conservatively tests an AABB against six camera planes.
Boundary contacts and numerically uncertain separation remain candidates; a box
near a frustum corner can pass this test without intersecting the clip volume.
It is a broad-phase query, not a mesh intersection or an occlusion result.

`project_bounds` clips the box against the camera volume before projection. It
handles bounds crossing the eye or near plane, and bounds enclosing the entire
frustum. Empty intersections return `None`; boundary contacts can produce a
zero-area rectangle. Both clip endpoints are included, unlike the far-exclusive
point membership reported by `world_to_screen`. Numerical plane comparisons use
a conservative tolerance relative to the rendered matrix's precision.

Coordinates use the viewport's top-left origin and the same pixel units as its
bounds. Rectangle endpoints are rounded outward to representable values, so they
can extend slightly beyond the viewport. A projected rectangle does not establish
that the mesh occupies all of that area or that any of it is unoccluded. It does
not account for material alpha, hidden nodes, or other objects.

`Camera::project_bounds` derives the aspect ratio from the viewport. A reusable
`Frustum` retains the view-projection matrix at creation; later camera edits do
not change it. `Frustum::project_bounds` only scales its normalized image into the
supplied viewport and does not replace the snapshot's aspect ratio. Recreate the
frustum when the camera or rendering aspect changes. Invalid viewports and camera
parameters return `CameraError`; singular rendered matrices and unrepresentable
screen bounds return `CameraError::Unrepresentable`.

### Framing bounds

```rust
use gpui_3d::{Aabb, Camera};

# fn main() -> Result<(), Box<dyn std::error::Error>> {
let bounds = Aabb::new([-2., -1., -1.], [3., 2., 1.]).unwrap();
let camera = Camera::orbit(0.4, 0.3, 8.).frame_bounds(bounds, 16. / 9., 1.2)?;
# Ok(())
# }
```

`frame_bounds` preserves viewing direction, up, projection kind, and lens shift.
It centers the box in the image and adjusts eye, target, near/far planes, and
orthographic span as needed. With a lens shift, the target is offset from the
box center to preserve the viewing direction. Margin is a finite screen-space
multiplier of at least one.
The result contains all eight corners for the supplied aspect ratio, without
changing any scene objects. Reframe when a changed output aspect requires it.

Use `EvaluatedScene::bounds()` to frame visible geometry, a node's `bounds` to
frame one mesh, or `subtree_bounds` to include its descendants. Subtree bounds
include hidden geometry. Framing an empty group requires the caller to choose
another target; zero-extent boxes use a small finite framing extent.

`Camera::transformed(affine)` maps local eye and target positions and the
orthogonalized up direction into another coordinate space. The result has a
right-handed orthogonal view basis, including under shear or reflection.
Projection, lens shift, near/far distances, and orthographic span are unchanged
by scale.
Invalid inputs or coordinates that lose a representable view return `CameraError`.

## Camera controls

`OrbitController` owns a camera and applies input immediately by default, without
a window or animation clock. Read `camera()` when building a scene and notify the
view when an operation returns `true`.

```rust
use gpui::{Bounds, MouseButton, point, px, size};
use gpui_3d::{Camera, OrbitController, OrbitSettings};

# fn main() -> Result<(), Box<dyn std::error::Error>> {
let mut controls = OrbitController::new(Camera::orbit(0.4, 0.3, 8.))?;
controls.set_settings(OrbitSettings {
    distance: 0.5..=50.,
    pitch: -1.4..=1.4,
    ..Default::default()
})?;
let viewport = Bounds::new(point(px(40.), px(80.)), size(px(800.), px(600.)));
controls.begin_drag(MouseButton::Right, point(px(300.), px(200.)), viewport)?;
let changed = controls.update_drag(
    point(px(325.), px(210.)), Some(MouseButton::Right), viewport,
)?;
controls.end_drag(MouseButton::Right);
let camera = controls.camera();
# Ok(())
# }
```

| Operation | Effect |
| --- | --- |
| `orbit_by([dx, dy])` | Rotate around the current target and up axis, preserving distance |
| `pan_by(viewport, delta)` | Translate eye and target so target-plane points follow the pointer |
| `dolly(factor)` | Multiply eye-to-target distance without changing projection |
| `zoom(factor)` | Multiply orthographic span or perspective tangent half-FOV without moving the camera |
| `scroll(pixels)` | Dolly in perspective or zoom in orthographic; positive pixels zoom out |

Default drag bindings are right-button orbit and middle-button pan; left-button
input is unassigned. Each binding can be changed or disabled with `None`.
`dolly_button` optionally assigns a vertical drag to distance control in either
projection. Positions, viewport bounds, and displacements use logical pixels;
convert wheel line deltas with `event.delta.pixel_delta(...)` before calling
`scroll`. `pan_speed` scales target-plane motion, `orbit_speed` is radians per
pixel, and `zoom_speed` controls logarithmic wheel/dolly sensitivity.

Settings constrain distance, pitch, orthographic span, and perspective FOV.
Default pitch limits are -1.5 to 1.5 radians, keeping orbit input below the poles.
`set_camera` preserves the supplied pose exactly, including an off-origin target
or a pose outside the configured limits. Further input can move an out-of-range
value toward its range but cannot move it farther away. Clipping planes are not
changed by controls; choose them for the navigable scene or frame bounds before
calling `set_camera`. Invalid settings, cameras, or direct-operation inputs return
`OrbitError` without changing the camera.

### Damping

`set_damping(Some(half_life))` enables exponential target following for orbit,
pan, dolly, and optical zoom. The half-life must be nonzero. `None` selects
immediate input. `camera()` is the displayed pose; `target_camera()` is the
destination accumulated by input operations. With damping enabled, an input
operation returning `true` means the destination changed, not that the displayed
camera has already moved.

```rust
use gpui_3d::{Camera, OrbitController};
use std::time::Duration;

# fn main() -> Result<(), Box<dyn std::error::Error>> {
let mut controls = OrbitController::new(Camera::default())?;
controls.set_damping(Some(Duration::from_millis(80)))?;
controls.orbit_by([30., 10.])?;

// Supply elapsed time from the application's frame clock.
let changed = controls.advance(Duration::from_millis(16))?;
let camera = controls.camera();
let needs_next_frame = controls.is_animating();
# Ok(())
# }
```

Advance to an input event's time before applying that input, then advance by the
time since the previous update when rendering. Do not count idle time before the
first input as animation time. Notify after accepted input and request another
animation frame only while `is_animating()` is true. A held, stationary gesture
does not keep requesting frames. The controller does not create tasks or timers;
callers choose whether hidden or paused views advance their clocks.

The response halves the remaining displacement each half-life and settles
exactly after sixteen half-lives without further input. Zero elapsed time does
nothing; a long time step can finish the response in one call. Sampling the same
input history at different frame intervals produces the same response. Orbit
uses the shortest azimuth path and interpolates pitch around the camera's up
axis; target translation is linear, while distance and optical scale are
logarithmic. Orbit retains its radius, optical zoom retains eye and target, and
all operations preserve lens shift and clipping planes. This is a finite
target-following response, not velocity extrapolation beyond the input target.

`end_drag` releases ownership without discarding pending motion. `cancel_drag`,
a successful `set_settings` or `set_damping`, and a newly claimed gesture freeze
at the displayed pose and discard the destination. `set_camera` cancels movement
and uses the supplied pose immediately. If an intermediate pose cannot be
represented, `advance` returns `OrbitError` and cancels motion at the last valid
displayed pose.

### Input ownership

`begin_drag` claims only a configured button pressed inside a valid viewport.
An active gesture cannot be replaced by another button, and wheel input is
ignored until it ends. `update_drag` accepts positions outside the viewport when
the caller provides pointer capture. A changed viewport, missing/mismatched
pressed button, or invalid movement cancels the gesture. Only a matching button
release ends it through `end_drag`.

The caller owns event routing and pointer capture. Use bubbling handlers so
embedded UI can consume its input first, and stop propagation when claiming a
gesture. Call `cancel_drag` on window deactivation or capture loss, and on pointer
leave when not using capture. Successful `set_camera` and `set_settings` calls
also cancel active gestures. Neither the controller nor its camera requires a
background frame loop.

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

### Identity and editing

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

### Reusable subtrees

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

### Transforms and bounds

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

### Evaluation and picking

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

### Camera and light nodes

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

### Transform tracks

`VectorTrack` samples XYZ values and `RotationTrack` samples XYZW quaternions at
an explicit `Duration`. Tracks are immutable and clones share keyframe storage.
Keyframes must be nonempty, strictly ordered by time, and contain finite values
and tangents. Sampling before or after the key range holds the nearest endpoint;
a single key is constant. Sampling has no playback history.

| Interpolation | Vectors | Rotations |
| --- | --- | --- |
| `Step` | Hold the preceding key until the next timestamp. | Hold the normalized preceding quaternion. |
| `Linear` | Component interpolation. | Normalized shortest-arc spherical interpolation. |
| `CubicSpline` | Cubic Hermite interpolation. | Component Hermite interpolation, then normalization. |

`Keyframe::tangents(incoming, outgoing)` supplies component derivatives per
second, not per segment; the default tangents are zero. Cubic quaternion values
and tangents retain their supplied signs and magnitudes. Use unit quaternion
keys for orientation curves. A curve passing through a zero quaternion returns
`AnimationError::InvalidSample`, as does a vector result outside finite `f32`
range. Step and linear rotations accept finite nonzero quaternions of any length.

`TransformTrack` combines independent channels with an explicit `TransformPose`.
Missing channels retain the base translation, rotation, or scale. `sample` returns
a pose, and `sample_transform` returns its validated affine transform. Signed
scale is supported, but singular or unrepresentable transforms return an error,
including interpolated scales crossing zero. No affine matrix decomposition is
performed.

```rust
use gpui_3d::{
    Interpolation, Keyframe, Node, SceneGraph, TransformPose, TransformTrack, VectorTrack,
};
use std::time::Duration;

let mut graph = SceneGraph::new();
let root = graph.insert(None, Node::new())?;
let track = TransformTrack::new(TransformPose::default())?.translation(VectorTrack::new(
    [
        Keyframe::new(Duration::ZERO, [0., 0., 0.]),
        Keyframe::new(Duration::from_secs(2), [4., 1., 0.]),
    ],
    Interpolation::Linear,
)?);
let local = track.sample_transform(Duration::from_millis(750))?;
let pose = graph.evaluate_with_transforms([(root, local)])?;
let scene = pose.scene(gpui_3d::Camera::default());
# let _ = scene;
# Ok::<(), Box<dyn std::error::Error>>(())
```

`evaluate_with_transforms` replaces local transforms only for the supplied nodes.
It does not modify authored nodes or the graph revision. Omitted nodes retain
their authored transforms; descendants inherit the resulting parent transforms.
Duplicate, foreign, and expired handles return `SceneError`. World transforms,
normal matrices, bounds, rendering, and queries use the same evaluated pose.
Each evaluation has its own spatial index; previous snapshots remain unchanged.
The evaluated revision identifies source graph edits, not the sampled time or
overrides, so equal revisions do not imply equal poses.

The caller owns the clock, time origin, looping, pause, and seek policy. Sample
all channels before evaluating a snapshot. Additive or relative motion can be
expressed by composing a sampled transform with an authored affine transform
before passing it to `evaluate_with_transforms`.

### Pose blending

`TransformPose::blend(target, weight)` blends complete local TRS values at a finite
weight in `[0, 1]`. Translation and signed scale interpolate componentwise;
rotation uses normalized shortest-arc interpolation. Weight zero and one retain
the exact input representations. Both inputs and the result must form valid,
invertible transforms, even at endpoint weights. A scale crossing zero returns
`AnimationError::InvalidTransform`; weights outside the allowed range are errors,
not clamped values. The operation does not decompose affine matrices or preserve
shear from an unrelated authored transform.

`Pose` is an immutable, ordered collection of `(NodeHandle, TransformPose)` entries.
`Pose::new` rejects duplicate handles and invalid TRS values. Entries contain full
local transforms, not partial animation channels. Resolve missing track channels
against an explicit base pose before constructing the collection. Empty poses
are valid; clones share storage. `get` retrieves a local pose, `poses` iterates
entries, and `transforms` provides validated affine overrides for scene evaluation.

```rust
use gpui_3d::{Node, Pose, PoseMask, SceneGraph, TransformPose};
use std::f32::consts::FRAC_1_SQRT_2;

let mut graph = SceneGraph::new();
let root = graph.insert(None, Node::new())?;
let joint = graph.insert(Some(root), Node::new())?;
let local = TransformPose {
    translation: [0., 1., 0.],
    ..Default::default()
};
let base = Pose::new([(root, TransformPose::default()), (joint, local)])?;
let target = Pose::new([(joint, TransformPose {
    rotation: [0., 0., FRAC_1_SQRT_2, FRAC_1_SQRT_2],
    ..local
})])?;
let mask = PoseMask::new(0., [(joint, 1.)])?;
let mixed = base.blend(&target, 0.5, Some(&mask))?;
let evaluated = graph.evaluate_with_transforms(mixed.transforms())?;
# let _ = evaluated;
# Ok::<(), Box<dyn std::error::Error>>(())
```

`base.blend(target, weight, mask)` retains the base's node set and insertion order.
Target entries may be sparse; omitted nodes retain their base poses. Every target
node and every explicit mask node must exist in the base, including at zero
weight. Unknown nodes produce `PoseError::MissingNode` instead of blending against
an implicit identity pose. The inputs remain unchanged if any result fails
validation. Node liveness and graph membership are checked when passing the
overrides to `SceneGraph`, not when storing a pose.

`PoseMask::new(default_weight, entries)` assigns finite `[0, 1]` weights to stable
node handles, with an explicit default for unspecified nodes. Effective blend
weight is the global weight multiplied by the node's mask weight. With no mask,
all target nodes use the global weight. A default of zero includes only listed
nodes; a default of one can exclude selected nodes with zero-weight entries.
Mask weights apply to local poses and do not expand through the hierarchy. A
masked-out child still inherits its parent's final transform.

Successive `blend` calls form ordered override layers. They are not a normalized
multi-way mean, so changing layer order can change the result. Sample tracks at
explicit times, construct the input poses, apply layers and local overrides,
then pass `mixed.transforms()` to `evaluate_with_transforms` or
`evaluate_with_constraints`. Use the resulting world transforms for attachments
and skinning. Clip selection, per-clip timing, automatic subtree masks, and
animation-state machines remain caller policy.

### Additive poses

`TransformPose::additive(sample, reference, weight)` applies a reference-relative
change to the current local pose. The weight must be finite and within `[0, 1]`.
For base `B`, sample `S`, reference `R`, and weight `w`:

| Channel | Composition |
| --- | --- |
| Translation | `B + w * (S - R)`, in the parent's coordinate system |
| Rotation | `B * slerp(identity, inverse(R) * S, w)`, with normalized XYZW quaternions and the shortest arc |
| Scale | `B * ((1 - w) + w * (S / R))`, componentwise |

The rotation delta acts in the base rotation's local frame. Translation is not
rotated or scaled by the base. This is TRS channel composition, not full affine
delta multiplication. Negative scales are supported, but a weighted scale ratio
crossing zero returns `AnimationError::InvalidTransform`. All input and output
poses must be invertible and representable, even at zero weight. Zero weight or
identical sample/reference poses retain the exact base representation. Full
weight with an identical base/reference retains the exact sample representation.

`base.additive(sample, reference, weight, mask)` applies this operation to sparse
node poses. Every sample node must exist in both the base and the reference;
missing references return `PoseError::MissingReference`, including when global
or mask weights are zero. Extra reference nodes are ignored. Mask weights and
node validation follow `Pose::blend`; there is no implicit bind or identity pose.
The output retains the base's node set and order, and failures leave all inputs
unchanged.

```rust
use gpui_3d::{Node, Pose, PoseMask, SceneGraph, TransformPose};

let mut graph = SceneGraph::new();
let node = graph.insert(None, Node::new())?;
let local = TransformPose { translation: [0., 1., 0.], ..Default::default() };
let base = Pose::new([(node, local)])?;
let reference = Pose::new([(node, TransformPose::default())])?;
let sample = Pose::new([(node, TransformPose {
    translation: [0., 0.2, 0.],
    ..Default::default()
})])?;
let mask = PoseMask::new(0., [(node, 1.)])?;
let mixed = base.additive(&sample, &reference, 0.5, Some(&mask))?;
let evaluated = graph.evaluate_with_transforms(mixed.transforms())?;
# let _ = evaluated;
# Ok::<(), Box<dyn std::error::Error>>(())
```

Additive and override layers can be combined explicitly. Layer order matters,
particularly for rotations. Keep reference poses explicit, sample tracks at the
desired absolute time, apply layers, then evaluate constraints and world poses.
The operations retain no playback history.

### Weight tracks

`WeightTrack` samples a runtime-sized array of weights from absolute timestamps.
Keys use `Keyframe<Vec<f32>>`; every key has the same nonzero component count.
Step holds the preceding value, Linear interpolates components, and CubicSpline
uses Hermite interpolation with derivatives per second. Empty derivative arrays
mean zeros and are expanded during construction; nonempty arrays must match the
weight count. All supplied values and derivatives must be finite, including
derivatives unused by Step or Linear. Timestamps must be strictly increasing.

```rust
use gpui_3d::{Interpolation, Keyframe, Mesh, MorphTarget, MorphTargets, WeightTrack};
use std::time::Duration;

let base = Mesh::plane();
let targets = MorphTargets::new(base.clone(), [
    MorphTarget {
        positions: Some(vec![[0., 0., 0.2]; base.vertex_count()].into()),
        ..Default::default()
    },
    MorphTarget {
        positions: Some(vec![[0.1, 0., 0.]; base.vertex_count()].into()),
        ..Default::default()
    },
])?;
let track = WeightTrack::new([
    Keyframe::new(Duration::ZERO, vec![0., 0.]),
    Keyframe::new(Duration::from_secs(2), vec![1., -0.5]),
], Interpolation::Linear)?;
let mut weights = vec![0.; track.weight_count()];
track.sample_into(Duration::from_millis(750), &mut weights)?;
let mesh = targets.evaluate(&weights)?;
# let _ = mesh;
# Ok::<(), Box<dyn std::error::Error>>(())
```

Weights are signed and neither clamped nor normalized. Cubic curves can overshoot
their keys. `sample(time)` returns an owned vector; `sample_into(time, output)`
uses a caller-owned slice of exactly `weight_count()` components without
allocating. A length mismatch or unrepresentable sample returns `AnimationError`
without changing the output slice. Sampling outside the key range retains the
first or last key. Clones share immutable keys but no playback or sampling state.

Weight order must match `MorphTargets::targets()`. Sample each instance's weights
at the chosen time, evaluate from the base targets, then apply `Skin` if needed.
The resulting mesh supplies geometry for rendering, bounds, and queries. The
track does not bind itself to nodes, mutate mesh assets, or own looping and clip
time conversion.

### Transform constraints

`SceneGraph::evaluate_with_constraints(transforms, constraints)` evaluates local
pose overrides and stateless world-transform constraints in one snapshot. Each
constraint is paired with the handle of the node it controls. `Follow` computes
`target.world * offset`, replacing the controlled node's world transform. Its
authored or supplied local transform is not applied; encode the attachment
offset explicitly. Full affine offsets preserve scale, shear, and reflection.

```rust
use gpui_3d::{AffineTransform, Node, SceneGraph, TransformConstraint};

let mut graph = SceneGraph::new();
let target = graph.insert(None, Node::new())?;
let parent = graph.insert(None, Node::new())?;
let attached = graph.insert(Some(parent), Node::new())?;
let target_pose = AffineTransform::from_translation([2., 0., 0.])?;
let offset = AffineTransform::from_translation([0., 0.5, 0.])?;
let pose = graph.evaluate_with_constraints(
    [(target, target_pose)],
    [(attached, TransformConstraint::Follow { target, offset })],
)?;
let world = pose.node(attached).unwrap().world;
assert_eq!(world.transform_point([0.; 3]), [2., 0.5, 0.]);

let parent_world = pose.node(parent).unwrap().world;
let released_local = parent_world.inverse().compose(world)?;
let released = graph.evaluate_with_constraints([(attached, released_local)], [])?;
assert_eq!(released.node(attached).unwrap().world, world);
# Ok::<(), Box<dyn std::error::Error>>(())
```

Targets use their final constrained transforms, including their own ancestors
and pose overrides. Input order does not determine evaluation order. Chained
attachments are supported, while self references, descendant targets, and
indirect dependency cycles return `SceneError::ConstraintCycle` with a closed
path of participating handles. Both parent and target links are dependencies.
Expired or foreign targets identify the owner and target in
`InvalidConstraintTarget`; duplicate owner constraints are rejected. A node
may have both a local pose override and a constraint, but only one of each.

Constraints do not reparent nodes or transfer visibility from targets. Hidden
targets are evaluated normally; controlled nodes retain visibility inherited
from their authored hierarchy. Children inherit the constrained world transform.
Camera and light properties, bounds, spatial queries, and renderer preparation
use the same final transforms. Hierarchy traversal order and frame-local object
identity ordering are unchanged.

The graph, its revision, and retained snapshots are not modified. Callers own
the constraint list and can evaluate any sampled pose without playback history.
Omitting a constraint restores the authored or supplied local transform. To
release an attachment while preserving its current world transform, supply
`parent.world.inverse() * node.world` as the replacement local transform and
retain the same evaluated parent pose; for roots, use the world transform itself.
Persistent hierarchy changes use `reparent` instead.

#### Aim and LookAt

`AimSettings::solve(world_transform, world_target)` rotates a transform around
its own origin without changing translation, scale, shear, or handedness.
It returns an `AimResult` containing the transform and an `AimStatus`. No graph,
camera, GPU, or playback history is required.

The default local forward is `-Z`, local up is `+Y`, and world up is `+Y`.
Custom axes need not be normalized. Forward aligns with the target direction;
the component of transformed local up perpendicular to forward aligns with
projected world up. The solver applies a world-space rigid rotation to the
entire linear transform, without decomposing it into TRS. Up need not become
perpendicular to forward when the input affine shape contains shear.

`max_angle` limits the shortest **total orientation correction**, including
roll, relative to the supplied pose. Its range is `0..=pi` radians; the default
`pi` allows a full correction. Zero preserves the supplied transform, while
still validating the inputs. This is not a yaw/pitch cone or an angular speed.
`AimStatus` reports the requested angle, applied angle, and whether the result
was limited. A limited result may not face the target exactly. Exact half-turns
use a deterministic rotation axis.

```rust
use gpui_3d::{AffineTransform, AimSettings};

let result = AimSettings {
    local_forward: [1., 0., 0.],
    max_angle: std::f32::consts::FRAC_PI_4,
    ..Default::default()
}.solve(AffineTransform::IDENTITY, [0., 0., -2.])?;
assert!(result.status.limited);
let transform = result.transform;
# Ok::<(), gpui_3d::AimError>(())
```

Within a graph, `TransformConstraint::Aim` uses the node's local pose composed
with its final parent transform. `target_offset` is a point in the target node's
local coordinates; the target's final world transform supplies its world position.
The `world_up` setting is a world-space vector and does not follow the target's
orientation. Both target and parent participate in dependency-cycle detection.

```rust
use gpui_3d::{AimSettings, ConstraintStatus, Node, SceneGraph, TransformConstraint};

let mut graph = SceneGraph::new();
let node = graph.insert(None, Node::new())?;
let target = graph.insert(None, Node::new())?;
let pose = graph.evaluate_with_constraints([], [(node, TransformConstraint::Aim {
    target,
    target_offset: [2., 0., -3.],
    settings: AimSettings::default(),
})])?;
if let Some(ConstraintStatus::Aim(status)) = pose.constraint_status(node) {
    assert!(!status.limited);
}
# Ok::<(), gpui_3d::SceneError>(())
```

`EvaluatedScene::constraint_status` retains `Follow` or `Aim` outcomes only for
constrained nodes; unconstrained or absent nodes return `None`. Each snapshot
keeps its own results after later evaluations. To constrain an animated pose,
sample its local tracks independently for each requested time; feeding previous
solver outputs back as base poses instead accumulates rotation across calls.

Non-finite or zero axes, coincident targets, invalid limits, and unrepresentable
results return `AimError`; graph evaluation wraps it in `SceneError::InvalidAim`
with the controlled node handle. Transformed local forward/up and target/world-up
pairs must have a sine angle greater than `1e-6`. Parallel or nearly parallel
pairs are errors, not automatic alternate-axis selections.

#### Two-bone inverse kinematics

`TwoBoneIkSettings::solve([root, middle, tip], target, pole)` accepts three world
transforms and world-space target/pole points. The supplied joint distances
define the two bone lengths. The result contains three world transforms,
`reachable_target`, `IkReach`, and `target_error` in scene units. No graph,
skeleton naming convention, or playback state is required.

```rust
use gpui_3d::{AffineTransform, IkReach, TwoBoneIkSettings};

let source = [
    AffineTransform::IDENTITY,
    AffineTransform::from_translation([1., 0., 0.])?,
    AffineTransform::from_translation([2., 0., 0.])?,
];
let result = TwoBoneIkSettings { weight: 1. }
    .solve(source, [1., 1., 0.], [0., 0., 2.])?;
assert_eq!(result.reach, IkReach::Reachable);
let [root_world, middle_world, tip_world] = result.transforms;
let middle_local = root_world.inverse().compose(middle_world)?;
let tip_local = middle_world.inverse().compose(tip_world)?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

The root position is fixed. The pole selects the side of the root-to-target
axis toward which the middle joint bends. The solver uses shortest-swing root
and middle rotations, preserving each joint's affine shape and handedness.
The tip inherits both rotations; there is no separate tip-orientation target.
Exact opposed directions use a deterministic orthogonal rotation axis.

`weight` is in `0..=1`, defaulting to one. It blends the root rotation and the
middle's relative rotation, then evaluates the chain. Intermediate weights
preserve bone lengths rather than interpolating joint positions. Zero returns
the supplied transforms unchanged, but inputs and pole geometry are still
validated. Always solve from the independently sampled source pose when seeking;
feeding prior solver results back as inputs accumulates motion.

For lengths `a` and `b`, the reachable radial interval is `[abs(a-b), a+b]`.
Targets outside it are projected onto the closest boundary without stretching,
with `IkReach::TooClose` or `TooFar`. `Reachable` describes geometry before
blending, not an assurance that a partially blended tip reaches the target.
`target_error` is the `f64` distance from the returned, rounded tip to the
original target. `reachable_target` is the radial projection before blending.

At the root, equal-length bones fold completely with the middle joint toward
the pole. Unequal-length bones retain the supplied root-to-tip direction when
choosing the nearest point on their inner reach boundary. A pole coincident
with the root is invalid. For bent solutions its direction must have a sine
angle greater than `1e-6` from the target axis; straight or fully folded radial
boundary solutions do not need a perpendicular pole. No joint-angle limits or
additional twist controls are imposed.

Zero-length bones, non-finite target/pole data, invalid weights, and ambiguous
bend poles return `TwoBoneIkError`. Calculations use widened intermediates;
outputs remain `f32` affine transforms. If rounding changes either bone length
by more than `1e-4` relative to its supplied length, or a transform cannot be
represented, the solver returns `Unrepresentable`.

To apply results to a three-node chain, convert each world result into its
parent's local space and pass the replacements to `evaluate_with_transforms`.
The root uses its evaluated external parent's inverse (or identity when it has
no parent); the middle and tip use the solved root and middle inverses as above.
Retain other sampled parent overrides. This also permits passing solved world
poses directly into skinning without owning a `SceneGraph`. Applications own
multi-chain ordering and any conflicting pose overrides.

## Materials and light

- `Material::color(color)` creates a lit solid surface.
- `Material::image(source)` maps the first decoded image frame onto UVs. Keep the
  image source stable across renders. An object is omitted while its image is unavailable.
- `Material::ui()` samples the viewport's captured UI without lighting.
- `.unlit(true)` disables lighting for any material.
- `.tint(color)` sets an sRGB tint, decoded before multiplication; its alpha multiplies texture alpha.
- `.base_color_texture(texture)` replaces the base image and sampling without
  resetting tint, image encoding, lighting, other maps, alpha mode, or face visibility.
- `.alpha_mode(mode)` selects `AlphaMode::Opaque`, `Mask`, or `Blend`.
- `.alpha_cutoff(value)` selects `Mask` and preserves a finite nonnegative threshold.
  Zero accepts every alpha value; values above one discard the entire surface.
- `.double_sided(false)` discards back faces in color, shadow, depth, normal and
  ID outputs, and in screen/world-ray queries. Materials are double-sided by default.

Front faces use local counterclockwise triangle winding. A negative-determinant
world transform reverses the raster winding convention, preserving the authored
front side under mirrored instances. Shading and queried normals face the visible
side of double-sided surfaces. Bounds and frustum queries remain conservative
and do not apply face visibility.

`Light` supplies a world-space direction toward the light, color, intensity and
ambient strength. Materials use basic diffuse shading unless `.pbr(parameters)`
is selected. Distinct opaque surfaces occlude each
other independently of object submission order. Coplanar surfaces should be
separated to avoid depth conflicts.

### Transparency

The default is `AlphaMode::Mask` with a cutoff of 0.5.

| Mode | Fragment alpha | Depth writes |
| --- | --- | --- |
| `Opaque` | Ignored; the surface is opaque | Yes |
| `Mask` | Below cutoff is discarded; survivors are opaque | Yes |
| `Blend` | Zero is discarded; nonzero values blend continuously | No |

```rust
use gpui::rgba;
use gpui_3d::{AlphaMode, Material};

let material = Material::color(rgba(0x65e0f580))
    .alpha_mode(AlphaMode::Blend);
```

Texture alpha and tint alpha are multiplied and clamped to `[0, 1]`. Blending
uses premultiplied source-over in the linear HDR target, before exposure and
tone mapping. It applies to both lit and unlit materials.

Opaque and masked objects render first. Blended objects then render from far
to near, ordered by the forward camera depth of each transformed mesh-bounds
center. Equal-depth blended objects retain submission order. All modes depth-test
against opaque and masked surfaces. Sort order does not change object IDs.

Sorting is per object, not per triangle. Intersecting meshes, cyclic overlaps,
and self-overlapping transparent meshes can produce incorrect layer ordering.
Separate independently ordered surfaces into objects. Order-independent
transparency, refraction, and transparent shadows are not provided.

Viewport image picking and headless object IDs select the nearest surviving
surface, even when its blended opacity is small; they do not choose the largest
color contributor. `Opaque` ignores alpha, `Mask` uses the cutoff, and `Blend`
passes through only zero-alpha regions. Captured UI picking remains geometric.

### Metallic-roughness materials

```rust
use gpui::rgb;
use gpui_3d::{Material, PbrMaterial};

let material = Material::color(rgb(0xdca773)).pbr(PbrMaterial {
    metallic: 1.,
    roughness: 0.35,
    emissive: [0.; 3],
});
```

`PbrMaterial` enables a GGX microfacet distribution, height-correlated Smith
visibility, Schlick Fresnel, and Fresnel-weighted Lambert diffuse response.
Dielectrics use normal-incidence reflectance 0.04. Increasing metallic blends
that reflectance toward the linear base color and removes diffuse reflection.
The base color comes from the material's solid color or sampled image and tint.

Metallic and perceptual roughness must be finite and in `[0, 1]`. Defaults are
metallic 0 and roughness 0.5. Shading limits perceptual roughness to at least
0.045 for finite highlights. Emissive is additive linear RGB radiance, defaults
to zero, and accepts finite components in `[0, 65504]`. It is independent of base
color, tint and scene lighting, but receives scene exposure and tone mapping.
Invalid parameters cause rendering to fail. `.unlit(true)` bypasses all PBR
terms, including emissive, and displays the sampled base color.

Specular response follows the world-space viewing direction. Perspective cameras
use the eye-to-surface vector; orthographic cameras use a constant direction.
Both viewport and headless rendering use these conventions. PBR does not alter
alpha cutout, depth writes, object IDs, or picking.

Direct lighting supports a single `Light` or an explicit `PunctualLight` list,
alongside uniform ambient and optional diffuse environment illumination. Pure
metals receive no diffuse ambient or environment light. Environment reflections
are not provided. One directional source can use a shadow map.
Emission does not illuminate other objects or add a glow outside the surface.

### Direct lights

```rust
use gpui::rgb;
use gpui_3d::{Light, PunctualLight, Scene};

let scene = Scene::new()
    .light(Light { ambient: 0.05, ..Default::default() })
    .lights([
        PunctualLight::directional([-0.5, 0.8, 0.7]).intensity(0.3),
        PunctualLight::point([1., 1., 2.])
            .color(rgb(0xffbb88)).intensity(4.).range(Some(6.)),
        PunctualLight::spot([-1., 1., 2.], [0., -0.3, -1.])
            .cone_angles(0.2, 0.5).intensity(6.),
    ]);
```

`Scene::lights` replaces the direct-light list, preserving uniform ambient and
diffuse environment illumination. `MAX_PUNCTUAL_LIGHTS` is eight; rendering
rejects larger lists rather than silently truncating them. An empty list disables
direct illumination. `Scene::light` sets the single directional light and ambient
multiplier and clears the explicit list. Neither method changes the environment.

Light positions and directions are world-space values, independent of object
transforms. A directional vector points **toward the source**. A spot vector
points **outward along the beam**. Directions need not be normalized, but must be
finite and nonzero. Point lights ignore direction. All positions must be finite.
The core does not attach lights to scene nodes or load lighting from asset files.

`color` is sRGB RGB in `[0, 1]` with alpha ignored. `intensity` is a finite linear
multiplier in `[0, 65504]`, defaulting to one. Direct illumination is summed in
linear HDR before exposure and tone mapping. Basic lit materials use Lambert
shading; PBR materials use the same GGX/Smith/Schlick response for every source.
The intensity scale follows the material's existing shading model; it is not a
calibrated photometric exposure system.

Directional light has no distance attenuation. Point and spot lights use
`1 / max(distance, minimum_distance)^2`. `minimum_distance` defaults to 0.01
world units and accepts `[0.0001, 65504]`, keeping near-source intensity finite
without creating an area light. At the exact source position the direction is
undefined and the direct contribution is zero.

`range(Some(r))` sets a finite positive cutoff for point/spot lights; `None`
means unlimited range. Attenuation is multiplied by
`(1 - min(distance/r, 1)^4)^2`, reaching zero smoothly at the cutoff.
Spot half-angles satisfy `0 <= inner < outer <= pi/2`. Their cosines must remain
distinct at f32 precision. The default angles are zero and pi/4. Angular weight
is `clamp((cos(theta)-cos(outer))/(cos(inner)-cos(outer)), 0, 1)^2`, where theta
is measured from the outward beam axis. Distance and cone attenuation follow
the conventions of [KHR_lights_punctual](https://github.com/KhronosGroup/glTF/blob/main/extensions/2.0/Khronos/KHR_lights_punctual/README.md).

AO attenuates indirect illumination only, not these direct sources. Unlit
materials bypass every light. Lights do not change emission, alpha, picking,
object IDs, depth, or geometric normals. Point and spot lights do not compute
shadows. Viewport and headless rendering share the same light list.

The `lighting` example switches directional, point, and spot sources in the same
scene. Move the pointer to move the light, adjust its range and cone, or add a
directional fill. Right-drag to orbit and scroll to zoom.

### Directional shadows

```rust
use gpui_3d::{DirectionalShadow, Light, Scene};

let scene = Scene::new()
    .light(Light { direction: [-1., 2., 1.], ..Default::default() })
    .directional_shadow(Some(DirectionalShadow {
        resolution: 2048,
        softness: 1.5,
        depth_bias: 0.0005,
        normal_bias: 0.01,
        ..DirectionalShadow::new([0., 0., 0.], [4., 4., 6.])
    }));
```

Shadows are optional and default to off. `light_index` selects a directional
source in `Scene::lights`; zero selects the source configured by `Scene::light`.
Rendering rejects missing, point, or spot sources. Reordering or replacing the
light list does not change the index. `directional_shadow(None)` disables the map.

`center` is a world-space point. `half_extent` gives half-width, half-height and
half-depth along the light's local axes, with each extent finite and at least
0.0001. Local Z points toward the source. The projection uses world Y as its up
reference, or world Z when the direction is near vertical. Receivers outside
this volume remain lit; casters outside it cannot contribute to its map. Include
both casters and receivers in the covered volume, including casters outside the
view camera. The volume does not automatically follow the camera or fit scene bounds.

`resolution` accepts powers of two from 256 to 4096, subject to device limits.
The default is 2048. A smaller covered volume or larger map gives finer detail.
`softness` accepts `[0, 4]` in shadow texels: zero uses one hard depth comparison;
positive values spread a 3-by-3 PCF kernel with bilinear depth comparisons.
This is filtered shadow mapping, not physical area-light penumbra simulation.

`depth_bias` offsets receiver depth toward the source in normalized light depth;
it accepts `[0, 0.05]`. `normal_bias` is a finite nonnegative world-space offset
along the geometric surface normal, weighted by the light/surface angle. Defaults
are 0.0005 and 0.01. Increase offsets to suppress self-shadowing artifacts; excessive
values detach shadows from their casters. Normal maps do not change this offset.

`Object::cast_shadows` and `Object::receive_shadows` default to true. The same
builders on `Node` control its own mesh, not its descendants, and are retained
by evaluated scenes and subtree copies. Opaque and Mask materials cast shadows;
Mask uses the base texture's alpha, tint alpha, cutoff and UV sampling, including
captured UI textures. Blend materials receive shadows when lit but never cast
them. Unlit meshes can cast shadows but bypass shadow reception.

The shadow affects only its selected light's direct diffuse and specular terms.
Other lights, ambient/environment illumination, emission, output alpha, picking,
object IDs, depth and geometric normal outputs are unchanged. Viewport and
headless color rendering share the depth pass and sampling implementation.
Maps are reused by resolution within a renderer; disabled shadows allocate no
full-size map. There is one shadowed directional source per scene, without
cascades, contact shadows, or colored transparent shadows.

Run `cargo run -p gpui_3d --example lighting`. Move the pointer to steer sunlight,
right-drag to orbit, and scroll to zoom. Controls toggle shadows and soft edges,
cycle map resolution, and lift the objects above the ground.

### Diffuse environment lighting

```rust
use gpui_3d::{DiffuseEnvironment, Light, Scene};

// Row-major, linear HDR RGB radiance, with the top row facing +Y.
let pixels = [
    [0.2, 0.6, 2.0], [0.2, 0.6, 2.0],
    [0.4, 0.1, 0.02], [0.4, 0.1, 0.02],
];
let environment = DiffuseEnvironment::from_equirectangular([2, 2], &pixels)?;
let scene = Scene::new()
    .light(Light { ambient: 0., intensity: 0., ..Default::default() })
    .diffuse_environment(environment.intensity(1.5).rotation_y(0.5));
# Ok::<(), gpui_3d::EnvironmentError>(())
```

`DiffuseEnvironment` accepts decoded equirectangular radiance or nine precomputed
real spherical-harmonic coefficients. Source RGB must be finite and in
`[0, 65504]`; values above one retain their HDR energy. Decode image formats and
convert nonlinear color inputs to linear RGB before projection. The core does
not load environment files or render them as a background.

Projection integrates each piecewise-constant texel over its spherical area
and convolves the first three SH bands with the cosine kernel divided by pi.
Perform this step once when the source changes, then reuse the value across
scenes and frames. The GPU evaluates nine RGB coefficients per shaded pixel;
environment image dimensions do not affect per-frame storage or sampling cost.

Coordinates use `theta = pi*v`, `phi = 2*pi*u - pi`, and direction
`(sin(theta)*cos(phi), cos(theta), sin(theta)*sin(phi))`. Thus the upper pole is
`+Y`, the image center is `+X`, `u=0.75` is `+Z`, and the seam is `-X`.
`.rotation_y(radians)` applies a right-handed environment-to-world rotation
around `+Y` without reprojection. `.intensity(value)` is a finite linear
multiplier in `[0, 65504]`; zero disables the contribution. Neither control
changes the camera, objects, or GPUI background.

`from_coefficients` expects **irradiance divided by pi**, not raw radiance.
The coefficient order and orthonormal real SH basis are:

| Index | Basis |
| --- | --- |
| 0 | `0.28209479` |
| 1 | `0.48860251 * y` |
| 2 | `0.48860251 * z` |
| 3 | `0.48860251 * x` |
| 4 | `1.09254843 * x*y` |
| 5 | `1.09254843 * y*z` |
| 6 | `0.31539157 * (3*z*z - 1)` |
| 7 | `1.09254843 * x*z` |
| 8 | `0.54627422 * (x*x - y*y)` |

Each coefficient component must be finite and in `[-262016, 262016]`.
`coefficients()` returns the projected coefficients before intensity and rotation.
The representation follows the low-order irradiance approximation described in
[An Efficient Representation for Irradiance Environment Maps](https://graphics.stanford.edu/papers/envmap/).
It captures broad directional illumination, not sharp environment features.
Negative reconstructed irradiance from SH ringing is clamped to zero.

Basic lit materials add `base * irradiance/pi` to existing lighting. PBR materials
add `base * (1 - metallic) * (1 - F0) * irradiance/pi`, where
`F0 = mix(0.04, base, metallic)`, evaluated with the shading normal, including an
active normal map. This diffuse approximation is view-independent; it does not
provide specular IBL, roughness-dependent reflections, or geometry-derived occlusion.
Uniform ambient and direct lighting remain additive. Unlit materials bypass
environment lighting. Alpha, picking, object IDs, depth, and geometric normal
outputs are unchanged. Viewport and headless color rendering share the same path.

The `lighting` example toggles a colored HDR environment independently of direct
sources. Rotate the environment with the toolbar to inspect the illumination.

### Environment background

`EnvironmentMap` retains shared, immutable decoded linear RGB radiance. Its
equirectangular orientation matches `DiffuseEnvironment`: the top is +Y,
the middle column faces +X, and increasing U turns toward +Z. Texels must be
finite and in `[0, 65504]`; dimensions and pixel count are validated at construction.
Decoding and file/resource selection belong to the caller.

```rust
use gpui_3d::{DiffuseEnvironment, EnvironmentBackground, EnvironmentMap, Scene};

let map = EnvironmentMap::from_equirectangular([2, 1], vec![
    [0.2, 0.5, 1.5], [4.0, 1.5, 0.3],
])?;
let illumination = DiffuseEnvironment::from_map(&map)?.intensity(0.5);
let background = EnvironmentBackground::new(map)
    .intensity(0.25)
    .rotation_y(0.4);
let scene = Scene::new()
    .diffuse_environment(illumination)
    .background(Some(background));
# Ok::<(), gpui_3d::EnvironmentError>(())
```

`Scene::background(None)` leaves uncovered pixels transparent. A visible
background is opaque, including at zero intensity, which draws black. Background
intensity and world-Y rotation do not change diffuse illumination, direct lights,
shadow maps, or object picking. A single map can feed both background and diffuse
projection, with independent settings.

The background is infinitely distant: camera rotation and perspective field of
view change the sampled directions, but translating the camera and target together
does not introduce parallax. Orthographic rays are parallel, so the background is
a constant direction across the viewport. Background color is linearly filtered,
wraps across the horizontal seam, and clamps at the poles. Maps use RGBA16Float
GPU storage with binary16 precision; dimensions must fit the device texture limit.
Shared maps are uploaded once while in use and evicted from the background cache
when absent from the prepared scenes. Brightness, orientation, and camera changes
reuse the texture.

The background is composited before scene geometry in linear HDR. Transparent
objects blend over it; opaque surfaces cover it. Exposure and tone mapping apply
to the combined display result. Headless `LINEAR_COLOR` includes the background
before display mapping; object ID, linear depth, and world normal remain zero
where no geometry survives. The background neither writes depth nor casts shadows.
Viewport clipping, subtree effects, and group opacity apply to the combined output.

Use the `lighting` example's background visibility, brightness, and rotation
controls independently of the environment illumination controls.

### Specular environment lighting

`SpecularEnvironment` supplies distant reflections for metallic-roughness PBR
materials. Background visibility and diffuse illumination are independent.

```rust,no_run
use gpui_3d::{EnvironmentMap, Scene, SpecularEnvironment, SpecularPrefilter};

let map = EnvironmentMap::from_equirectangular([2, 1], vec![
    [0.1, 0.4, 1.0], [4.0, 1.0, 0.2],
])?;
let reflections = SpecularEnvironment::from_map(&map, SpecularPrefilter {
    resolution: 128,
    samples: 256,
})?.intensity(0.7).rotation_y(0.4);
let scene = Scene::new().specular_environment(Some(reflections));
# Ok::<(), gpui_3d::EnvironmentError>(())
```

Prefiltering is explicit, synchronous CPU work. Run it during asset preparation,
outside the frame loop; applications may use their own loading workers. Clones
share the resulting cube levels. Rotation and intensity changes reuse these
levels. `None` or zero intensity disables reflections. `SpecularPrefilter`
defaults to 128-pixel faces and 256 GGX samples per filtered texel. Face size
must be a power of two in `[16, 512]`, samples must be in `[16, 4096]`, and
`2 * resolution² * samples` must not exceed 64 million. Invalid quality or source
data returns `EnvironmentError`.

Level zero samples the source sharply. Successive cube levels cover evenly spaced
perceptual roughness values through one. Source mip reduction weights texels by
spherical area and handles non-power-of-two dimensions; filtered importance
sampling reduces aliasing from concentrated radiance. Higher sample counts reduce
prefilter noise but increase preparation cost. Higher face resolution retains
more detail in smooth reflections.

The renderer uses the [split-sum IBL approximation](https://google.github.io/filament/Filament.md.html),
combining prefiltered radiance with a 128 × 128 RG16Float BRDF lookup. The lookup
integrates GGX, height-correlated Smith visibility, and Schlick Fresnel using
512 samples per texel and is generated once when first needed by the renderer.
Cube storage is RGBA16Float, with linear filtering across faces and roughness
levels. The PBR path uses the reflected view direction and shading normal,
including normal maps, then applies metallic/F0, roughness maps, intensity, and
ambient occlusion. Direct light, emission, and geometry output channels are
unchanged. Basic diffuse and unlit materials do not receive specular IBL.

This is a single-scattering, infinitely distant environment approximation. It
does not reflect nearby scene objects or add local occlusion, parallax-corrected
probes, or multiple-scattering energy compensation. Ambient occlusion attenuates
indirect reflections as a scalar approximation. Exposure and display mapping
follow the normal color pipeline; `LINEAR_COLOR` retains linear reflected energy.

#### Prefiltered inputs

External preprocessors can supply `SpecularEnvironmentMap::from_prefiltered`
and pass the result to `SpecularEnvironment::from_prefiltered`. The input is a
complete power-of-two mip chain, ending at one texel per face, with base face
size at most 512. Each level is six contiguous square faces in the order below.
RGB is finite linear radiance in `[0, 65504]`. Let `s` and `t` span `[-1, 1]`
from left to right and top to bottom; normalize each listed direction.

| Face | Direction |
| --- | --- |
| +X | `(1, -t, -s)` |
| -X | `(-1, -t, s)` |
| +Y | `(s, 1, t)` |
| -Y | `(s, -1, -t)` |
| +Z | `(s, -t, 1)` |
| -Z | `(-s, -t, -1)` |

For `N` levels, level `i` represents perceptual roughness `i / (N - 1)`.
One-level inputs provide the same radiance at every roughness. The map contains
normalized GGX radiance convolution, without Fresnel or BRDF response baked in.
`map().levels()` exposes the prepared data for inspection and reuse. GPU caches
retain shared maps while used by prepared scenes and release unused cube maps.

The `lighting` example controls reflections, reflection rotation, and surface
roughness independently of the background and diffuse environment.

### Material textures

```rust,no_run
use gpui::rgb;
use gpui_3d::{Material, MaterialTexture, PbrMaterial, TextureAddressMode, TextureSampling};

let material = Material::color(rgb(0xdca773))
    .pbr(PbrMaterial { metallic: 1., roughness: 1., emissive: [2.; 3] })
    .metallic_roughness_texture(MaterialTexture::new("surface.png").sampling(TextureSampling {
        address_u: TextureAddressMode::Repeat,
        address_v: TextureAddressMode::Repeat,
        ..Default::default()
    }))
    .emissive_texture(MaterialTexture::new("emission.png"));
```

`MaterialTexture` accepts the same image sources as `Material::image` and uses
the first decoded frame. Each map has its own `TextureSampling`, including UV
transform, addressing and filtering. All maps use mesh UVs; base-color sampling
does not affect other maps.

| Input | Channels | Color interpretation | Applied to |
| --- | --- | --- | --- |
| Metallic-roughness | G: roughness, B: metallic | Linear data | Corresponding PBR factors |
| Emissive | RGB | sRGB decoded before filtering | Linear emissive factor |

Map values multiply the material factors. A zero factor remains zero; an absent
map uses a multiplier of one. R in the metallic-roughness map and alpha in both
maps are ignored. Alpha cutout and picking visibility depend only on base-color
alpha and tint. These maps do not add occlusion, surface displacement, or normals.

Metallic-roughness and emissive maps are loaded only for lit PBR materials.
Until all required images are ready, the viewport omits the object from rendering
and visible picking. Direct
headless rendering requires decoded `ImageSource::Render` inputs for every map
and returns an error for unresolved inputs. `.unlit(true)` and diffuse materials
ignore these maps without requesting their resources.

### Ambient occlusion maps

```rust,no_run
use gpui::rgb;
use gpui_3d::{Material, MaterialTexture};

let material = Material::color(rgb(0xd6c3a5))
    .occlusion_texture(MaterialTexture::new("occlusion.png"))
    .occlusion_strength(0.8);
```

Occlusion maps use linear R, where zero blocks indirect light and one leaves it
unchanged. G, B and alpha are ignored. `occlusion_strength` defaults to one and
must be finite and in `[0, 1]`; rendering rejects invalid values. The multiplier
is `1 + strength * (R - 1)`, following the
[glTF occlusion convention](https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html#materialocclusiontextureinfo).
An absent map or zero strength leaves indirect lighting unchanged.

Both basic lit and PBR materials apply this multiplier to uniform ambient and
diffuse environment illumination. Direct diffuse/specular light and PBR
emission are unaffected. Unlit materials bypass occlusion. The texture does not
alter base color, transparency, geometry, picking, depth or geometric normals.
It represents authored or baked occlusion, not dynamic shadows or screen-space AO.

Use `MaterialTexture::sampling` for independent UV transforms, addressing and
filtering. An ORM image can be shared between `occlusion_texture` (R) and
`metallic_roughness_texture` (G/B). No mesh tangents are required for occlusion.
Disabled maps request no resources. Active maps use the same readiness and
decoded-image requirements as other material maps in viewport/headless rendering.

The `materials` example toggles occlusion independently of normal, metallic-roughness,
and emission maps. Occlusion uses the R channel of the shared ORM image.

### Normal maps and tangents

```rust,no_run
use gpui::rgb;
use gpui_3d::{Material, MaterialTexture, Mesh, PbrMaterial};

let mesh = Mesh::plane();
let material = Material::color(rgb(0x79a6b0))
    .pbr(PbrMaterial { roughness: 0.35, ..Default::default() })
    .normal_texture(MaterialTexture::new("normal.png"))
    .normal_scale(0.8);
```

Normal-map RGB is linear vector data, decoded from `[0, 1]` to `[-1, 1]` and
normalized after filtering. R points along the tangent, G along the bitangent,
and B along the surface normal; alpha is ignored. `normal_scale` multiplies XY
before normalization. It defaults to 1 and accepts finite nonnegative values.
Zero disables the normal map and its resource requests. A zero decoded vector
uses the interpolated mesh normal.

Normal maps affect lit PBR shading only. They do not move vertices, alter
silhouettes or depth, or change object IDs, ray intersections, and picking normals.
They use independent `MaterialTexture` sampling. UV transforms and addressing
change sampled locations, not the tangent frame or the decoded vector axes.

`Mesh::plane()` and `Mesh::cube()` provide tangents. Custom meshes attach one
`[f32; 4]` tangent per vertex using `mesh.with_tangents(data)`, returning a new
mesh with shared vertex/index storage and unchanged geometry queries. XYZ is
orthogonalized against the vertex normal and normalized; W is exactly -1 or +1.
The bitangent is `cross(normal, tangent.xyz) * tangent.w`. Read the resulting
data with `mesh.tangents()`.

Invalid counts, non-finite or undefined bases, zero vertex normals, and mixed W
signs within a triangle return `TangentError`. Split vertices at tangent-space
seams before supplying data. Rendering does not generate missing tangent bases.
Rendering an active normal map without mesh tangents returns an error.

`Mesh::generate_tangents()` generates MikkTSpace frames synchronously and returns
`GeneratedTangents`. It uses normalized copies of indexed normals and preserves
the stored positions, normals, and UVs. Shared vertices split when face-corner
tangent frames differ, including mirrored UV seams. Triangle order and winding
are unchanged, preserving triangle IDs for queries and per-triangle metadata.
Existing tangents are replaced without modifying the source mesh.

```rust
use gpui_3d::Mesh;

let source = Mesh::new(Mesh::plane().vertices().to_vec(), Mesh::plane().indices().to_vec());
let generated = source.generate_tangents()?;
let source_weights = vec![0.5_f32; source.vertex_count()];
let weights: Vec<_> = generated.source_vertices().iter()
    .map(|&source| source_weights[source as usize])
    .collect();
let (mesh, source_vertices) = generated.into_parts();
# Ok::<(), gpui_3d::TangentGenerationError>(())
```

Output vertices follow first use and omit unreferenced vertices; output bounds
therefore exclude unused positions. `source_vertices()[output_index]` identifies
the original vertex. Use this mapping for external vertex attributes, morph
deltas, and skin influences before constructing deformation inputs. Distinct
source vertices are not merged even when all their mesh attributes match.

`TangentGenerationError` identifies zero indexed normals, zero geometric or UV
area, unrepresentable f32 intermediates, and undefined output frames. Degenerate
triangles are rejected, not removed or assigned arbitrary tangents. Finite UVs
outside `[0, 1]` are supported. Generate during asset preparation and share the
result across objects; generation is not part of per-frame rendering.

Tangents follow the model transform; normals use the inverse transpose. Shading
reorthogonalizes the world-space frame, adjusts handedness for reflected transforms,
and reverses the mapped normal on back faces. Vertex-normal shading uses the same
reflection-aware face orientation.

### Color and exposure

```rust
use gpui_3d::{ColorOutput, Material, Scene, TextureColorSpace, ToneMapping};

let material = Material::image("albedo.png")
    .image_color_space(TextureColorSpace::Srgb);
let scene = Scene::new().color_output(ColorOutput {
    exposure: -1.,
    tone_mapping: ToneMapping::Reinhard,
});
```

Solid colors, material tints and light colors use sRGB RGB values in `[0, 1]`;
values outside that range are clamped. Image RGB defaults to `TextureColorSpace::Srgb`
and is decoded before Nearest or Linear filtering. `TextureColorSpace::Linear`
treats image RGB as linear values without transfer-function conversion. Alpha is
always linear and is unaffected by the color-space setting. The setting does not
interpret texture channels as normals, roughness, or other material parameters.

Lighting and image filtering operate in linear space. Light intensity and ambient
strength are linear multipliers and may exceed one. Shading uses an `Rgba16Float`
intermediate, retaining values up to 65504 per channel. Exposure and display
mapping are applied to each MSAA sample before averaging premultiplied display
colors. Captured UI colors are unpremultiplied and decoded
for shading; ordinary GPUI elements outside the viewport are not processed.

`ColorOutput` defaults to zero exposure and `ToneMapping::None`. Exposure is in
stops: +1 doubles intensity and -1 halves it. Rendering rejects non-finite exposure
or values outside `[-16, 16]`. `None` clamps the exposed result to the display
range. `Reinhard` compresses each channel with `x / (1 + x)` before sRGB encoding.
Both operate on straight color, then restore premultiplied coverage for GPUI
composition. Unlit materials bypass lighting, but still receive exposure and
tone mapping. Default output preserves unlit source colors up to numeric precision.

Viewport and headless rendering use the same color pipeline. Exposure and tone
mapping do not affect alpha cutout, depth, object IDs, or picking. Display outputs
are sRGB-encoded SDR. Headless `LINEAR_COLOR` exports premultiplied linear HDR
before exposure and tone mapping.

### Image sampling

```rust
use gpui_3d::{Material, TextureSampling, TextureAddressMode, TextureFilter, TextureMipFilter, UvTransform};

# fn main() -> Result<(), gpui_3d::UvTransformError> {
let sampling = TextureSampling {
    transform: UvTransform::from_scale_rotation_translation([2., 2.], 0.2, [-0.25, 0.])?,
    address_u: TextureAddressMode::Repeat,
    address_v: TextureAddressMode::Mirror,
    filter: TextureFilter::Linear,
    mip_filter: TextureMipFilter::Linear,
    max_anisotropy: 8,
};
let material = Material::image("tile.png").image_sampling(sampling);
# Ok(())
# }
```

`TextureSampling` defaults to identity UVs, Clamp on both axes, Linear texel filtering,
no mipmaps, and isotropic sampling (`max_anisotropy: 1`).
`UvTransform` applies scale, rotation about UV origin, then translation. Rotation
is in radians, positive clockwise in top-left-origin coordinates. `from_rows`
accepts two affine rows `[u, v, offset]`, including shear, reflection and zero
scale. Constructors reject non-finite coefficients. `transform(uv)` returns the
unaddressed coordinates, or `None` for non-finite input or overflow.

Clamp extends edge texels. Repeat tiles every unit interval, with linear
interpolation across the first/last texel seam. Mirror alternates forward and
reflected copies; negative coordinates follow the same period. U and V modes
are independent. Image UVs describe texel edges: texel `i` is centered at
`(i + 0.5) / extent`. Nearest selects one texel and Linear interpolates four
neighbors. Sampling remains inside the image, without accessing neighboring atlas tiles.

`mip_filter` selects `None`, `Nearest`, or `Linear`: the original image only,
the nearest mip level, or interpolation between adjacent levels. With mipmaps
enabled, `max_anisotropy` controls the maximum sampling ratio for oblique surfaces,
from 1 through 16. Values above 1 require both `filter` and `mip_filter` to be
`Linear`. Invalid combinations return a scene-preparation error before resource
resolution. Each material-map slot has its own sampling configuration.
Actual anisotropic filtering is backend-dependent; WGPU uses isotropic sampling
on devices without anisotropic-filtering support.

The WGPU renderer generates independent RGBA16Float mip chains on first use and
reuses them while referenced by prepared scenes. Images are decoded to linear
RGB before reduction; normal, metallic-roughness, and occlusion maps retain their
linear channel values. Alpha is averaged independently. Area-weighted reduction
includes odd image edges and supports one-pixel axes. Image identity, atlas
allocation generation, and color interpretation distinguish cached chains;
sampling changes reuse a chain while mipmapping remains enabled. Chain storage
uses eight bytes per texel summed across all levels, in addition to atlas storage.

GPU level selection uses derivatives of transformed, unwrapped UVs. Color,
object-ID, depth, and normal outputs share the same image-alpha sampling at a
given output resolution. Shadow maps select levels using their own projected
footprint. Mip generation does not preserve alpha-test coverage or sharpen normal
maps; use an appropriate cutoff and texture content for distant masked surfaces.

These settings apply only to image materials. Captured UI textures retain their
identity UV mapping and linear edge-clamped sampling, including pointer routing.
`Hit::uv` always contains the original mesh UVs. Viewport image-alpha picking
applies the material's UV transform, addressing, and texel filter at level zero
before evaluating its alpha mode. CPU ray queries do not have a screen-space
sampling footprint, so mipmapped alpha masks can differ from GPU visibility
during minification. Interpolation near a cutoff can also differ at floating-point
precision boundaries between CPU and GPU.

## UI texture

```rust
use gpui::{ParentElement, Styled, div, px, rgb, size};
use gpui_3d::{Material, Mesh, Object, Scene, viewport3d};

let world = Scene::new().object(
    Object::new(Mesh::plane(), Material::ui()).scale([3., 2., 1.]),
);
let viewport = viewport3d("panel", world)
    .size_full()
    .ui_texture_size(size(px(600.), px(400.)))
    .ui_texture_scale(2.)
    .ui_texture(div().size_full().bg(rgb(0x20314b)).child("Hello, space."));
```

`ui_texture_size` sets logical layout dimensions independently of the viewport.
When omitted, layout follows the viewport size. Choose the mesh aspect ratio to
match the logical texture dimensions to preserve text and shape proportions.
Window resizing and camera movement do not reflow a fixed-size UI texture.

`ui_texture_scale` controls raster density relative to display scale and defaults
to `1`. For example, a 600 × 400 layout at density `2` on a 1× display renders to
1200 × 800 pixels without changing font sizes or line wrapping. Raster density
is reduced uniformly when either texture dimension would exceed 2048 pixels.
Smaller densities reduce texture memory and rasterization costs; larger densities
preserve more detail when a surface fills the viewport.

One live UI capture is shared by all UI materials in the viewport. Text, images
and ordinary descendants are captured into a transparent, independently sized
target. Source layout is clipped to its own dimensions, not the window or
ancestor masks. Ancestor clipping applies to the final 3D viewport. Deferred
overlays are not part of the texture.

UI textures are decorative by default. Pointer hits inside a decorative source
subtree are disabled without blocking camera controls on the enclosing viewport.

## UI pointer interaction

Give the UI object a unique ID and select it with `interactive_ui`:

```rust
use gpui::{ParentElement, Styled, div, prelude::*, px, size};
use gpui_3d::{Material, Mesh, Object, Scene, viewport3d};

let world = Scene::new().object(
    Object::new(Mesh::plane(), Material::ui())
        .id("controls")
        .scale([4., 3., 1.]),
);
let viewport = viewport3d("world", world)
    .size_full()
    .ui_texture_size(size(px(640.), px(480.)))
    .ui_texture(
        div().size_full().child(
            div().id("button").child("Apply").on_click(|_, _, _| {}),
        ),
    )
    .interactive_ui("controls");
```

Ordinary GPUI pointer handlers receive source UI coordinates through the mesh UVs.
Buttons, hover styles, pointer-capturing sliders, and scrollable descendants use
their existing event handling. `window.mouse_position()` also returns source
coordinates inside those handlers; `raw_mouse_position()` remains window-relative.
Only the selected UI object is interactive; other objects sharing the capture
remain decorative. The selected object must use `PickBehavior::Target`.

Nearest-surface picking handles mesh occlusion and image alpha cutouts. Ancestor
clipping and ordinary 2D overlays also limit pointer hits. Captured UI alpha is
not sampled: transparent areas of a UI material still participate geometrically.

Left-button gestures that start on the interactive surface belong to the UI.
The starting triangle's projection is retained until release, allowing captured
sliders to continue outside the mesh and viewport without switching to a different
surface. If the projection becomes parallel or points behind the camera, the
last valid source position is retained. Changing the target or logical layout
size, removing the selected object, or deactivating the window cancels routing.

Left-button gestures and scrolling over the surface do not bubble to enclosing
camera handlers. Right-button gestures remain available for orbit controls.
Use normal bubbling handlers for the camera; ancestor capture-phase listeners
run before the UI and should not claim its left-button gestures. Object click
callbacks do not fire for clicks consumed by the UI surface.

Text editing, input-method placement, keyboard focus ownership, tooltips, menus,
and deferred overlays are not part of UI pointer routing. Avoid global pointer
listeners and overlays inside the source; keep those controls in ordinary 2D UI.

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

Viewport callbacks sample the first image frame's alpha using the material's UV
transform, addressing and filter, matching the material shader. Sampled
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
handling. It respects picking behavior and constant material alpha but does not
resolve images or sample texture alpha. Both arguments use logical window
coordinates; the caller supplies the viewport bounds and handles UI clipping
and input routing. Queries scan the scene's triangles on the CPU, so use modest
meshes for interactive picking.

## Resource preparation

`Scene::prepare(aspect, ui_texture, resolver)` produces a `PreparedScene` without
opening a window or requiring a GPU. It shares validation, culling, transforms,
material selection, and output IDs with viewport and headless rendering.

The resolver receives a `TextureRequest` containing the original object index,
output ID, application ID, node handle, material slot, and borrowed source.
Return `TextureState::Ready` with a matching `ResolvedTexture`, or
`TextureState::Pending` while the input is unavailable. Solid sources use
`ResolvedTexture::None`, images use `ResolvedTexture::Image(tile)`, and UI
captures use `ResolvedTexture::Subtree`.

```rust
use gpui_3d::{Material, Mesh, Object, ResolvedTexture, Scene, TextureSource, TextureState};

let scene = Scene::new().object(
    Object::new(Mesh::plane(), Material::image("cover.png")).id("cover"),
);
let prepared = scene.prepare(16. / 9., None, |request| {
    Ok(match request.source {
        TextureSource::Solid => TextureState::Ready(ResolvedTexture::None),
        TextureSource::Image(_) | TextureSource::Ui => TextureState::Pending,
    })
})?;
assert!(!prepared.is_ready());
let pending = &prepared.pending_textures()[0];
assert_eq!(prepared.object(pending.output_id).unwrap().id, Some("cover".into()));
# Ok::<(), gpui_3d::PrepareError>(())
```

All active inputs of an eligible object are requested even when another input
is pending. An object enters `frame().objects` only when every active input is
ready. Other objects can render while it waits. Disabled maps and meshes outside
both camera and shadow coverage make no requests. Readiness applies to the
current view, not every resource in the scene. Call `prepare` again after resource
completion or scene changes; preparation does not start tasks or schedule redraws.

Resolver errors return `PrepareError::Resource` with the object index, slot, and
underlying error. Incompatible ready texture kinds return `InvalidResolution`;
invalid scene inputs return `InvalidScene`. Errors return no prepared frame, but
uploads or other resolver side effects are not rolled back. There is no implicit
fallback material or retry policy.

`objects()` and `identities()` retain the complete original object mapping,
including pending and culled objects. `object(id)` returns `None` for zero or an
unknown ID. A later preparation does not mutate an earlier frame or its mapping.

Atlas tiles are renderer-local references. Upload decoded images with
`Window::prepare_effect_image` or the atlas exposed by
`WgpuScene3dRenderer::sprite_atlas`, and retain those allocations through render
submission. `PreparedScene` does not own atlas residency and must not be submitted
to a different renderer. Pass `frame()` to `WgpuScene3dRenderer::render`, or use
`into_frame()` with `Window::with_scene3d` and the corresponding UI capture.
Retain `identities()` with custom outputs before consuming the prepared scene.
File resolution, decoding, cache eviction, cancellation, and retries belong to
the resource manager.

### Retained preparation

`PreparationCache` retains one `Arc<PreparedScene>`. Unchanged `Scene` clones and
scenes created from the same `EvaluatedScene` snapshot reuse validation, culling,
matrices, and identity mapping. Camera values, aspect ratio, UI logical dimensions
and raster density are compared separately. Content builders invalidate the
preparation; constructing a new scene or evaluating a new graph snapshot also
requires preparation, even when its values happen to match an older scene.

```rust
use gpui_3d::{Material, Mesh, Object, PreparationCache, ResolvedTexture, Scene, TextureState};

let scene = Scene::new().object(Object::new(Mesh::cube(), Material::color(gpui::white())));
let mut cache = PreparationCache::new();
let first = cache.prepare(&scene, 1., None, |_| {
    Ok(TextureState::Ready(ResolvedTexture::None))
})?;
let next = cache.prepare(&scene.clone(), 1., None, |_| {
    Ok(TextureState::Ready(ResolvedTexture::None))
})?;
assert!(std::sync::Arc::ptr_eq(&first, &next));
# Ok::<(), gpui_3d::PrepareError>(())
```

Every call invokes the resolver once per active input, including cache hits.
Pending/ready transitions and changed atlas tile references rebuild the output.
An error clears the cache and returns no frame. `clear()` releases the retained
CPU inputs without affecting previously returned preparations or renderer caches.

Viewports retain this cache under their element ID. Keep a scene or evaluated
snapshot in application state and clone it when rendering. UI layout, painting,
and pick-surface resolution still run normally. This is a CPU input cache, not a
rendered-image cache: unchanged tile references do not imply unchanged pixels.
Resource managers must keep resolving current residency, retain allocations
through submission, and request redraws when asynchronous inputs change.

### CPU preparation benchmarks

```sh
cargo bench -p gpui_3d --bench scene
```

The workloads prepare 1,024 and 16,384 objects with shared geometry, mixed PBR
factors, mostly off-camera placement, and pending image inputs. Measurements
include scene validation, culling, resource callbacks, output construction, and
identity mapping. The `retained_preparation` group measures steady-state cache
hits with the same resource callbacks. Scene construction is outside the timed
region. These CPU-only
measurements do not include GPU uploads, draw encoding, shading, or readback.

### Draw statistics

With the `wgpu` feature, `Scene3dDrawStatistics::plan(frame, channels, limit)`
runs the WGPU mesh planner without an adapter. Pass a prepared frame and a
positive maximum instance count per batch. A renderer exposes its device-specific
limit through `max_instances_per_batch()`; an explicit limit also allows offline
comparisons of batch sizes. Planning does not validate scene inputs, atlas
residency, or device capabilities.

Statistics report camera and shadow draw calls, instance counts, and submitted
triangle counts. `batches`, `instance_upload_bytes`, and `uniform_upload_bytes`
describe per-submission payloads, not allocated buffer capacity. A batch shared
by camera and shadow work uploads once. Color and linear color share one shaded
pass; each selected object-ID, depth, or normal output has its own mesh pass.
Consequently, multi-channel counts include repeated work across outputs.

`RenderedFrame::gpu().draw_statistics()` retains the counts from the actual
submission's prepared plans without a GPU readback. Statistics do not include
fullscreen background/display draws, texture or geometry uploads, GPU timings,
occlusion, or pixel coverage. Frustum-culled meshes contribute no work, while
occluded meshes can still contribute draws and triangles.

```sh
cargo bench -p gpui_3d --features wgpu --bench scene -- draw_planning
```

These CPU workloads compare shared geometry, mixed materials, culled scenes, and
ordered transparency. Each workload verifies its expected instance and draw
counts before timing. Scene preparation is outside the measured region.

### GPU submission benchmarks

The `draw_encoding` group requires an explicit GPU opt-in:

```sh
GPUI_3D_GPU_BENCH=1 cargo bench -p gpui_3d --features wgpu --bench scene -- draw_encoding
```

It renders 1,024 and 16,384 cube instances at 256 × 256 with one sample per pixel,
using shared geometry, mixed PBR materials, mostly culled objects, and fixed-topology
vertex updates. Color-only and color/ID/depth/normal outputs run separately.
`vertex_updates` replaces the shared cube's vertices each iteration while retaining
its index storage; it measures upload and buffer-reuse overhead, not bulk transfer
bandwidth. Throughput counts visible instances across all selected output passes.

Timing covers `WgpuScene3dRenderer::render`: validation, retained draw preparation,
resource preparation, command encoding, output allocation, and queue submission.
Scene construction, vertex generation, initial pipeline warm-up, GPU completion
waits, result validation, and output release are outside the measured interval.
Only one submission is outstanding at a time, with a 30-second completion timeout.
These are CPU submission timings, not GPU timestamps, throughput under a deep
queue, readback latency, or visual validation.

The harness prints the selected adapter and checks expected draw/instance counts
against each submitted output. Unsupported selected channels fail explicitly.
Criterion filters select individual workloads, for example
`draw_encoding/shared_geometry/color/1024`. Without `GPUI_3D_GPU_BENCH=1`, this
group does not create a device or submit GPU work.

## Rendering and support

Linux WGPU supports these viewports. Check `window.supports_scene3d()` before
displaying 3D content; unsupported backends draw no mesh scene. Native Metal and
DirectX backends do not currently implement the mesh pass.

Each viewport has isolated depth visibility and is composited into GPUI's normal
paint order. Ancestor opacity applies once to the final image, and ancestor
clipping still applies. Mesh edges default to four samples when supported,
otherwise one. Captured viewports can be nested in other subtree effects.

### Viewport effects

Wrap a viewport with `gpui_effects::subtree_effect_chain` to apply Bloom and color
adjustment to its composed image. The wrapper shares the viewport's layout;
effect padding expands capture space without changing its camera aspect ratio
or pointer coordinates. Put toolbars outside the wrapper to leave them unaffected.

```no_run
use gpui::{prelude::*, px};
use gpui_3d::{Scene, viewport3d};
use gpui_effects::{BloomOptions, EffectStage, SubtreeColorOptions, subtree_effect_chain};

let view = subtree_effect_chain(
    viewport3d("scene", Scene::new()).size_full(),
    [
        EffectStage::bloom(BloomOptions {
            threshold: 0.7,
            radius: px(32.),
            ..Default::default()
        }),
        EffectStage::color_adjust(SubtreeColorOptions {
            saturation: 0.8,
            ..Default::default()
        }),
    ],
)
.map_interaction(true);
```

Stages run in order: color adjustment after Bloom also changes the halo's color.
Both stages preserve geometry and support identity pointer mapping; the halo does
not create additional interactive surfaces. Ancestor clipping also clips the
halo. Use stage-level `enabled(false)` to remove a pass, or disable the wrapper
to paint the viewport directly. The wrapper does not request animation frames
for these static stages.

The input is the viewport's display-encoded image after scene exposure and tone
mapping, including its environment background. These stages do not read the
linear HDR, depth, normal, or object-ID outputs, and they do not change geometric
picking. For HDR processing before display mapping or depth-dependent effects,
use the headless GPU output textures in a same-device rendering pipeline.

Check `window.supports_subtree_effects()` in addition to 3D support. On a backend
without subtree effects, the wrapper paints its content directly. The `lighting`
example exposes Bloom and Natural/Monochrome/Vivid color controls on the viewport.

### Raster quality

`resolution_scale` sets mesh raster density relative to physical render-surface pixels;
`color_samples` requests one or four samples per mesh pixel. Defaults are `1.0`
and `4`. These controls do not change logical layout, camera aspect ratio,
geometric picking, pointer routing, or UI capture density.

```no_run
use gpui_3d::{Scene, viewport3d};

let viewport = viewport3d("preview", Scene::new())
    .resolution_scale(0.5)
    .color_samples(1);
```

Scale must be finite and positive; invalid scale or sample count panics at the
builder call. Dimensions are scaled uniformly to fit the device texture limit,
rounded up, and kept at least one pixel on each axis. Lower scales reduce mesh
attachment memory and raster work; higher scales increase them. Non-native
resolutions use bilinear reconstruction of premultiplied display color. This is
not an area-filtered downsampling chain for large scale factors. Device limits
are not memory budgets; applications remain responsible for total GPU memory use.

Four samples fall back to one when unavailable. Use
`window.scene3d_support().capabilities()` and
`capabilities.color_samples_for(ViewportQuality::new(scale, samples))` to query
the effective count. Viewports in one window may use different configurations.
Low-level frames carry `Scene3dFrame::viewport_quality`; direct headless outputs
use `Scene3dOutputConfig` instead. Use `ui_texture_scale` independently to adjust
the raster density of captured UI.

### Allocation and visibility

Rendering conservatively rejects indexed mesh bounds outside the camera frustum
before allocating geometry buffers or uploading instance data. Bounds touching
a clip plane or crossing the camera plane remain eligible. Local bounds follow
vertex snapshots and are transformed with the object's full matrix, including
shear and reflections. Numerically uncertain cases remain eligible for GPU
clipping. Unreferenced vertices do not enlarge render-culling bounds.

Directional shadows use their own light-space clip volume. A mesh outside the
camera can still cast a visible shadow; disabling shadow casting or using Blend
removes that shadow-only work. Scene preparation resolves material images only
for meshes eligible for the camera or shadow volume. Moving a camera, changing
geometry, or changing shadow coverage reevaluates visibility on the next render.
This does not hide scene nodes, alter bounds or world-ray queries, or renumber
output IDs. It is not occlusion culling: geometry behind other objects still
participates in depth testing.

Geometry buffers are reused for shared meshes. Intermediate HDR color and depth
targets cover the viewport's pixel bounds intersected with the render surface,
rounded outward to whole pixels. Fractional layout positions keep their pixel
alignment. Viewports with the same target dimensions and sample count share
temporary attachments; other configurations have separate attachments retained
only while used by the current scene. A window resize preserves attachments whose viewport dimensions remain
unchanged. Fully off-surface viewports do not allocate mesh attachments.

Instance buffers grow in power-of-two steps capped by the device batch limit.
An active batch reuses its capacity while demand stays above one quarter of it;
at or below that threshold, the next preparation allocates a smaller buffer.
Removed batches release their buffers. The same policy applies to viewport and
direct-output passes, without changing draw order or object identities. Queued
commands retain any replaced resources they still reference.

Mesh output is placed back into surface coordinates for subtree composition and
enclosing effects. Generic subtree-composition textures remain surface-sized, so many
nested captures can still consume substantial GPU memory. UI capture and composition
run when GPUI repaints; there is no autonomous background render loop. UI layout,
texture sampling coordinates, picking, and pointer routing
are independent of mesh attachment dimensions.
UI texture targets and their rendering resources are reused while attached;
pixel-size changes resize the capture targets independently of the window.

### Submitted viewport outputs

WGPU retains visibility, transparent ordering, and instance-batch plans for
unchanged object snapshots, camera/shadow clip matrices, output mode, and batch
limits. Lighting or display settings that do not affect those inputs preserve
the plan. Unused plans are evicted on preparation. This CPU reuse does not depend
on command submission and does not skip resource checks or request UI frames.

WGPU window rendering reuses submitted mesh pixels when the immutable
`Scene3dFrame`, raster region, and referenced atlas generations are unchanged.
`Viewport3d` preserves frame identity across CPU preparation cache hits with the
same raster quality. Camera, material, lighting, geometry, or quality changes
produce a new frame and redraw the mesh.

Meshes sampling UI also compare captured paint content and image-pass inputs.
Equivalent freshly painted scenes can reuse mesh output. Changes to geometry,
style, clipping, draw order, shader parameters, nested frames, or referenced atlas
generations invalidate it. Particle, fluid, feedback, particle-transition, and
external-surface inputs bypass output reuse.

Independent UI textures retain submitted pixels separately from mesh outputs.
An unchanged capture does not redraw when the camera moves or unrelated UI
repaints. Text, images, paths, background blur, and stateless subtree effects are
eligible; animation time and effect parameters participate in content comparison.
Capture-size changes replace the texture. Layout, paint callbacks, resource
resolution, hit testing, and focus/event handling remain active; reuse skips only
GPU capture rendering. No application-managed dirty flag is required.

The mesh-output cache holds only viewport-covered display pixels. Each WGPU
window or external renderer has a shared 64 MiB budget by default, including
mesh outputs nested inside its independent UI captures. Other windows have
independent budgets even when they share a device.
Inputs must repeat before a pixel texture is allocated; continuously changing
snapshots do not allocate output-cache textures. Surface-size, transparency-mode,
and subpixel-layout changes invalidate retained outputs.
Entries are retained only for the current visible viewport list; oversized or
uncacheable inputs render normally. This limit excludes intermediate attachments,
UI capture textures, atlas resources, geometry, and outputs still held only by
submitted GPU commands. Abandoned
encodings never make an output reusable.

`Window::set_scene3d_output_cache_budget(bytes)` changes the shared limit; zero
disables mesh pixel reuse. Changing the limit releases all existing mesh output
entries immediately without clearing UI pixels, atlas images, or mesh buffers.
Setting the same limit preserves entries. The setting survives WGPU device
recovery and does not request a frame or wait for GPU completion.
`Window::scene3d_output_cache_stats()` returns the budget, retained bytes, and
texture count, or `None` when the backend does not expose this cache. Counts include
allocated cache textures awaiting submission, not total physical GPU memory.
`WgpuRenderer` and `WgpuOffscreenRenderer` expose the same controls and statistics.

`WgpuRenderer::draw_external` clears, renders, and submits an external target,
enabling the same output reuse without a native window. `WgpuOffscreenRenderer`
uses this path before readback. `encode_external` leaves submission to its caller
and does not reuse mesh or UI capture pixels. Subsequent captures use a different
texture after caller-owned encoding, isolating cached pixels from late submission
of older commands. Direct `HeadlessRenderer` outputs are
independent per-call textures, not viewport pixel-cache entries.

`Window::clear_scene3d_caches()` releases mesh buffers, intermediate mesh targets,
shadow maps, environment uploads, image mip chains, retained mesh output pixels,
and mesh pipelines for all viewports, including those inside UI captures.
Shared 2D atlas entries, UI capture textures, and generic
subtree-effect resources remain intact. Capture contents are invalidated, and the
next mesh draw rebuilds its caches.
The call does not request a repaint or wait for GPU completion, and unsupported
backends do nothing. In-flight commands retain the resources they use, so release
does not guarantee an immediate reduction in physical GPU memory use.

## Headless output

The optional `wgpu` feature provides `HeadlessRenderer` for the same scenes without
a native window or UI layout. It accepts solid and decoded-image materials and
returns independently selectable display-color, linear-HDR, object-ID, linear-depth, and world-normal
textures with a frame-local identity map and bounded
nonblocking CPU readback. See [Headless rendering](headless.md) for formats,
coverage, resource readiness, and ownership.

## Examples

Each example is an independent executable.

| Example | Controls and content |
| --- | --- |
| `scene` | Shared mesh assemblies, hierarchy edits, subtree instances, selection, free/rig cameras, attached spot lights, transform tracks, vertex tapering, two-target morph blending, and two-joint skin bending with independent weights and playback controls. |
| `materials` | Dielectric/metal/emissive spheres, normal and ORM maps, roughness, emission, exposure, tone mapping, UV addressing, mipmaps, anisotropy, and alpha modes. |
| `lighting` | Direct lights, diffuse/specular environments, roughness, independent HDR background, directional shadows, map resolution, soft edges, Bloom, and color adjustment. |
| `ui` | Captured UI buttons, slider and scrolling, occlusion, logical layout size and raster density. |
| `headless` | Window-free display/HDR/ID/depth/normal readback, PNG previews and object identity inspection. |

```sh
cargo run -p gpui_3d --example scene
cargo run -p gpui_3d --example materials
cargo run -p gpui_3d --example lighting
cargo run -p gpui_3d --example ui
cargo run -p gpui_3d --features wgpu --example headless -- /tmp/gpui-3d-outputs
```

In `scene`, hover an assembly to highlight its toolbar control, or click it to
select it. The numbered controls also select assemblies. Move or
tint its body, rotate the assembly, or hide its subtree. Other instances retain
their own properties. Right-drag to orbit, middle-drag to pan, and scroll to zoom.
Projection preserves the apparent size at the target; Frame selected fits the
selected assembly's bounds.
Camera damping toggles an 80 ms response half-life for manual camera controls.

In `materials`, the spheres share geometry and expose different material responses.
Normal and occlusion maps toggle independently of metallic-roughness and emissive
maps. The strip below the spheres shows image alpha over an opaque background.
Cycle Opaque/Mask/Blend, Clamp/Repeat/Mirror, and Nearest/Linear; density and offset
also affect the strip. Exposure and tone mapping apply to the complete 3D scene.

In `lighting`, move the pointer to steer the source. Shadow controls apply only
to the directional source. Lift the objects to inspect detached shadows, or toggle
the environment and fill light to inspect illumination inside shadowed areas.

In `ui`, drag the slider beyond the panel and scroll the notes. Toggle the occluder
to block part of the panel. Density changes raster quality without reflow; canvas
width changes layout and the mesh aspect ratio. Right-drag to orbit, or left-drag
empty space. Scroll outside the panel to zoom.
