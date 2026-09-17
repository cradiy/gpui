# Edit overlays

With the `wgpu` feature, points and line segments can be attached to an
[`EditOcclusionGroup`](edit_occlusion.md). Elements use world coordinates and
caller-assigned nonzero `u32` identities, unique across that group's points and
lines. They do not create scene objects or change model geometry or CPU picking.

```rust,ignore
use gpui_3d::{EditHiddenStyle, EditLine, EditPoint, EditStyle};

let style = EditStyle {
    hidden: EditHiddenStyle::Dashed { dash: 5., gap: 5. },
    hidden_color: gpui::rgb(0x304a60),
    ..EditStyle::new(2., gpui::rgb(0x70d4e9))
};
let group = group
    .lines([EditLine { id: 1, start: [-1., 0., 0.], end: [1., 0., 0.], style }])
    .points([EditPoint {
        id: 2,
        position: [1., 0., 0.],
        style: EditStyle::new(6., gpui::rgb(0xffc267)),
    }]);
let scene = scene.edit_occlusion([group]);
```

## Appearance and visibility

Point diameters, line widths and dash lengths use logical pixels. Viewports apply
window DPI and compensate for render-resolution scaling; zooming the camera does
not change their pixel width. Headless output uses one physical pixel per logical
pixel by default. `EditOcclusionGroup::pixel_scale` supplies an additional scale,
including for high-density headless output. Sizes must be positive and at most
4096 logical pixels, with at most 1,048,576 pixels after scaling; invalid IDs,
coordinates, styles or scales fail preparation.

Points are analytic circles; lines are analytic round-ended capsules. Both use
fragment coverage antialiasing, independent of mesh MSAA. Near/far clipping occurs
before projection. Depth along a perspective segment uses perspective-correct
interpolation, not a midpoint or a linear interpolation of endpoint depths in
screen space. Segments clipped to zero length render as points.

Visibility is evaluated for each fragment using the group's independent depth:

- Self-occlusion follows the projected point or segment centerline at that fragment.
  A local, subpixel slope tolerance accounts for sampling the auxiliary surface.
  Expanding the line width does not move its geometry or change its depth.
- Foreign geometry masks the actual expanded fragment footprint.
- `depth_tolerance` adds an explicit nonnegative tolerance in view-depth units;
  it applies to all occluders. Keep it small relative to the scene's geometry.

Visible fragments use `color`. Occluded fragments use `hidden_color` with `Hide`,
`Solid`, or `Dashed` coverage. Dashed points use a solid hidden marker. Dashes use
projected screen-space distance from the start endpoint; near/far clipping resets
that anchor to the clipped start, while viewport-side clipping preserves it.
Dash periods smaller than a raster pixel use their average coverage.

Groups composite in insertion order. Within a group, lines draw first, then
points, each in insertion order. Later elements take precedence at overlaps;
elements are not depth-sorted against each other. Colors are linear RGBA in
`0..=1`, composited into linear scene color before its exposure and tone mapping.
Applications express selection and hover by choosing each element's style.

## Element identities

`Scene3dOcclusionOutput::elements()` returns an independent GPU output containing
R32Uint element IDs and R32Float view depth. The buffers are generated from the
same projected primitives and visibility calculation as color. Zero ID is
background; use the output's depth-background convention when ID is zero.
IDs cover the interior with at least 50% analytic coverage. Partially covered
antialiased fringes can contribute color without an ID. Nonzero color opacity
does not enlarge or shrink the selectable footprint. Hidden gaps have no ID.

For a viewport, attach `ViewportPickCapture`, obtain its submitted frame and map
the logical pointer with `ViewportPickFrame::pixel_at`. Use that same pixel in
any group's element output. A one-pixel, ID-only request avoids reading an image:

```rust,ignore
let pixel = frame.pixel_at(pointer).expect("inside viewport");
let group = &frame.occlusion_groups()[0];
let elements = group.elements().expect("group has elements");
let pending = elements.readback_region(
    Scene3dReadbackRegion { origin: pixel, size: [1, 1] },
    Scene3dReadbackConfig {
        channels: Scene3dChannels::OBJECT_ID,
        max_staging_bytes: Some(256),
        max_cpu_bytes: Some(4),
    },
)?;
```

Retain `group.parent_frame_id()` alongside the request to associate the result
with the primary frame. Element outputs also have their own allocation identity.
Readbacks retain output resources and obey the renderer's existing queue and
admission rules. Mouse handlers, selection state, topology edits and snapping
remain application-owned.

Groups with elements draw in viewports even without a capture. Capture is needed
only to expose their submitted output handles to the application. Headless color
and linear-color outputs include overlays, while primary ID, depth and normal
outputs remain model-only. Each nonempty element group adds an 8-byte-per-pixel
ID/depth output and an instance buffer. Depth-source passes and instance buffers
are currently prepared for each submission; request only active editing groups.

For frontmost selection across overlapping groups, inspect their nonzero element
IDs in reverse group order, matching color composition order.

## Example

```sh
cargo run -p gpui_3d --features wgpu --example editing
```

Right-drag to orbit; switch projection, hidden style or the foreign occluder.
Click a point or edge to highlight its independent element identity.
