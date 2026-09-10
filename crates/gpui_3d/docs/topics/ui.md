# UI textures and interaction

[3D viewports](../viewport.md)

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

## Related topics

[Spatial queries and picking](queries.md).
