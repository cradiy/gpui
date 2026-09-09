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
the ray origin. Queries still scan triangles directly; no spatial acceleration
structure is provided.

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
Asset import, model-instance resources, and camera/light attachments are not
provided by the graph.

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
- `.tint(color)` sets the sampled RGBA multiplier; its alpha participates in cutout.
- `.alpha_cutoff(value)` discards low-alpha pixels. Remaining pixels are opaque
  and write depth; fractional material transparency is not blended.

`Light` supplies a world-space direction toward the light, color, intensity and
ambient strength. Lighting is a basic diffuse model, without shadows, reflections
or physically based material parameters. Distinct opaque surfaces occlude each
other independently of object submission order. Coplanar surfaces should be
separated to avoid depth conflicts.

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

Viewport callbacks sample the first image frame's alpha with clamped UVs and
bilinear filtering, matching the material shader's sampling convention. Sampled
alpha is multiplied by material alpha and compared with `alpha_cutoff`; discarded
regions allow hits on surfaces behind them. This also applies to occluders.
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

## Example

```sh
cargo run -p gpui_3d --example camera
```

Compare six equal cubes at different depths with perspective and orthographic
projection. Click a cube to select it, frame the selection or the whole scene,
and switch between front, top, and oblique views. Right-drag to orbit; scrolling
moves a perspective camera or changes the orthographic span. The footer reports
the selected node's projected position and linear forward depth.

```sh
cargo run -p gpui_3d --example hierarchy
```

Rotate groups A and B, toggle group A's visibility, and reparent the coral cube
with either keep-world or keep-local behavior. Remove and restore coral to create
a new handle. Click geometry to inspect its node identity. Right-drag to orbit
and scroll to zoom. The graph is reevaluated after edits, not camera movement.

```sh
cargo run -p gpui_3d --example ui_interaction
```

Click the panel buttons, drag the level slider beyond the panel, and scroll the
notes. Toggle the occluder to block part of the panel. Right-drag to orbit; left
dragging empty space also orbits. Scroll outside the panel to zoom the camera.

```sh
cargo run -p gpui_3d --example ui_texture
```

Change density to compare fine-line and text detail without reflowing the panel.
The layout button switches between 640 × 400 and 960 × 400 logical pixels, with
a matching mesh aspect ratio. Resize the window, drag to orbit, or scroll to zoom.
The toolbar shows the allocated texture dimensions in physical pixels.

```sh
cargo run -p gpui_3d --example orbit
```

Drag to orbit the scene and scroll to change distance. Toggle the UI plane to
inspect the image plane and textured cube behind it. `Reset` restores the camera.

```sh
cargo run -p gpui_3d --example picking
```

Hover to highlight a cube, click to select it, left- or right-drag to orbit, and
scroll to zoom. Movement beyond four logical pixels starts an orbit gesture and
suppresses selection on release. Hover and selection change colors without
changing geometry.

The halo uses an image with a transparent center, with a selectable backplate
behind the scene. Compare the three mode buttons while pointing at the solid rim:
selectable highlights the halo, occluder yields no target, and pass-through
highlights an object behind the halo. The pointer target and selected object are
shown below the viewport. The transparent center passes through in all modes.
