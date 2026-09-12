# Cameras

[3D viewports](../viewport.md)

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

Perspective near depth must be finite and positive. Orthographic near depth may
be zero, including surfaces on the eye plane, and far depth must be finite and
greater than near. Negative near depths are rejected. Zero-near orthographic
frames use a `-1` linear-depth background sentinel so zero remains a valid
surface depth; see [Headless output](headless.md).

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
| `screen_to_world(viewport, position, depth)` | World position from linear camera-forward depth |
| `frustum(aspect)` | Owned camera clip-volume snapshot for repeated world-AABB queries |
| `project_bounds(viewport, bounds)` | Screen rectangle of the clipped world AABB, or `None` for an empty intersection |

Screen coordinates use a top-left origin and include the viewport offset. Use
logical viewport bounds and logical input positions for GPUI handlers. The math
is scale-independent: multiplying both bounds and screen coordinates by the same
DPI scale produces the same ray. Pixel centers in physical image data are at
`(x + 0.5, y + 0.5)`; convert them and the viewport into one coordinate system
before querying. No implicit DPI conversion is performed.

`world_to_screen` returns `None` behind the eye plane, and on it for perspective
projection. Orthographic eye-plane points retain their coordinates. Points
outside the viewport or clip planes retain their projected coordinates with
`in_frustum = false`. The near plane is included and the far plane excluded.
Frustum membership is geometric, not proof that a point is unoccluded.

`screen_to_world` inverts screen projection using linear forward depth in scene
units, not normalized hardware depth or distance along a picking ray. Depth must
be finite, positive for perspective, and nonnegative for orthographic projection.
Off-screen positions and depths outside the near/far interval remain valid inputs;
reconstruction does not test visibility.
Viewport offsets, DPI-scaled coordinates, and lens shifts follow the same
conventions as `world_to_screen`.

Perspective rays originate at the eye. Orthographic rays originate at the
corresponding point on the eye plane and have parallel directions. Both extend
forward without an intrinsic near/far limit. `screen_to_ray` accepts positions
outside the viewport for captured drags; `Scene::pick` still enforces viewport
bounds and the camera's clip range. UI pointer mapping uses the same projection
for either camera type.

## Focal length and lens shift

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

## Projecting bounds

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

## Framing bounds

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

## Damping

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

## Input ownership

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

## Related topics

[Spatial queries and picking](queries.md).
