# 3D viewports

`gpui_3d` embeds depth-tested mesh scenes in ordinary GPUI layouts. A viewport
supports perspective and orthographic cameras, indexed triangle geometry, one directional light,
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

## Coordinates and geometry

World coordinates are right-handed, with positive Y up. The default camera is
at `[0, 0, 6]`, looking toward the origin. Angles are radians. `Camera::orbit`
orbits the origin; `Camera` exposes `eye`, `target`, `up`, `projection`, and clip
distances for explicit positioning. Projection matrices are column-major, with
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

## Camera projection and queries

`Camera::projection` selects `Projection::Perspective { vertical_fov }` in radians
or `Projection::Orthographic { vertical_size }` in scene units. Orthographic size
is the full vertical span; horizontal span is `vertical_size * aspect`. Perspective
objects shrink with distance; orthographic objects keep their projected size.
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
camera updates avoids rebuilding the object hierarchy. New evaluations rebuild
their object index lazily; incremental refitting is not provided. Heavily
overlapping object or triangle bounds can still require broad traversal. These
indices accelerate queries, not render submission or GPU draw batching.

Invalid camera, viewport, and point inputs return `CameraError` from the public
matrix/projection/query methods. Invalid rays return `RayError`.

### Framing bounds

```rust
use gpui_3d::{Aabb, Camera};

# fn main() -> Result<(), Box<dyn std::error::Error>> {
let bounds = Aabb::new([-2., -1., -1.], [3., 2., 1.]).unwrap();
let camera = Camera::orbit(0.4, 0.3, 8.).frame_bounds(bounds, 16. / 9., 1.2)?;
# Ok(())
# }
```

`frame_bounds` preserves viewing direction, up, and projection kind. It centers
the target on the box and adjusts eye distance, near/far planes, and orthographic
span as needed. Margin is a finite screen-space multiplier of at least one.
The result contains all eight corners for the supplied aspect ratio, without
changing any scene objects. Reframe when a changed output aspect requires it.

Use `EvaluatedScene::bounds()` to frame visible geometry, a node's `bounds` to
frame one mesh, or `subtree_bounds` to include its descendants. Subtree bounds
include hidden geometry. Framing an empty group requires the caller to choose
another target; zero-extent boxes use a small finite framing extent.

## Camera controls

`OrbitController` owns a camera and applies input immediately, without a window,
animation clock, or continuous redraw. Read `camera()` when building a scene and
notify the view when an operation returns `true`.

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

`SceneGraph` manages group and mesh nodes independently of a window or GPU.
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

Instantiation copies editable scene nodes while sharing resources; it does not
batch them into an instanced GPU draw call.

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

The evaluated state contains static transforms and visibility. Animation time,
constraints, asset readiness, and direct no-window 3D rendering are separate
capabilities. The viewport still resolves image resources during rendering.

## Materials and light

- `Material::color(color)` creates a lit solid surface.
- `Material::image(source)` maps the first decoded image frame onto UVs. Keep the
  image source stable across renders. An object is omitted while its image is unavailable.
- `Material::ui()` samples the viewport's captured UI without lighting.
- `.unlit(true)` disables lighting for any material.
- `.tint(color)` sets an sRGB tint, decoded before multiplication; its alpha multiplies texture alpha.
- `.alpha_mode(mode)` selects `AlphaMode::Opaque`, `Mask`, or `Blend`.
- `.alpha_cutoff(value)` selects `Mask` and sets its threshold, clamped to `[0.001, 1]`.

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
seams before supplying data. Tangents are explicit inputs: the core does not
generate MikkTSpace tangents or reconstruct missing bases from derivatives.
Rendering an active normal map without mesh tangents returns an error.

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
are sRGB-encoded SDR; the internal HDR texture is not exposed as an output channel.

### Image sampling

```rust
use gpui_3d::{Material, TextureSampling, TextureAddressMode, TextureFilter, UvTransform};

# fn main() -> Result<(), gpui_3d::UvTransformError> {
let sampling = TextureSampling {
    transform: UvTransform::from_scale_rotation_translation([2., 2.], 0.2, [-0.25, 0.])?,
    address_u: TextureAddressMode::Repeat,
    address_v: TextureAddressMode::Mirror,
    filter: TextureFilter::Linear,
};
let material = Material::image("tile.png").image_sampling(sampling);
# Ok(())
# }
```

`TextureSampling` defaults to identity UVs, Clamp on both axes and Linear filtering.
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
neighbors. Sampling remains inside the image's atlas rectangle.

These settings apply only to image materials. Captured UI textures retain their
identity UV mapping and linear edge-clamped sampling, including pointer routing.
`Hit::uv` always contains the original mesh UVs. Viewport image-alpha picking
applies the material's image sampling before evaluating its alpha mode. Color and
object-ID outputs use the same sampling and discard rules. Interpolation near
a cutoff can differ at floating-point precision boundaries between CPU and GPU.
Mipmaps and anisotropic filtering are not provided; both magnification and
minification use mip level zero.

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
Both faces are pickable; backface normals follow the renderer's flipped shading
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

## Rendering and support

Linux WGPU supports these viewports. Check `window.supports_scene3d()` before
displaying 3D content; unsupported backends draw no mesh scene. Native Metal and
DirectX backends do not currently implement the mesh pass.

Each viewport has isolated depth visibility and is composited into GPUI's normal
paint order. Ancestor opacity applies once to the final image, and ancestor
clipping still applies. Mesh edges use four-sample MSAA. Captured viewports can
be nested in other subtree effects.

Geometry buffers are reused for shared meshes. Intermediate color and depth
targets are reused at a stable window size; they are recreated on resize and
device recovery. Window-sized offscreen targets consume GPU memory, so use a
small number of simultaneous viewports. UI capture, mesh rendering and composition
run when GPUI repaints; there is no autonomous background render loop.
UI texture targets and their rendering resources are reused while attached;
pixel-size changes resize the capture targets independently of the window.

## Headless output

The optional `wgpu` feature provides `HeadlessRenderer` for the same scenes without
a native window or UI layout. It accepts solid and decoded-image materials and
returns independently selectable color, object-ID, linear-depth, and world-normal
textures with a frame-local identity map and bounded
nonblocking CPU readback. See [Headless rendering](headless.md) for formats,
coverage, resource readiness, and ownership.

## Examples

Each example is an independent executable.

| Example | Controls and content |
| --- | --- |
| `scene` | Shared mesh assemblies, hierarchy edits, subtree instances, selection, perspective/orthographic projection, framing, orbit and pan. |
| `materials` | Dielectric/metal/emissive spheres, normal and ORM maps, roughness, emission, exposure, tone mapping, UV addressing/filtering, and alpha modes. |
| `lighting` | Directional/point/spot sources, fill light, environment rotation, directional shadows, map resolution and soft edges. |
| `ui` | Captured UI buttons, slider and scrolling, occlusion, logical layout size and raster density. |
| `headless` | Window-free color/ID/depth/normal readback, PNG previews and object identity inspection. |

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
