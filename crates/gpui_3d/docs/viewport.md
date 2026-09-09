# 3D viewports

`gpui_3d` embeds depth-tested mesh scenes in ordinary GPUI layouts. A viewport
uses a perspective camera, indexed triangle geometry, one directional light,
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
orbits the origin; `Camera` also exposes eye, target, field of view and clip
distances for explicit positioning.

`Mesh::plane()` is a unit XY square facing positive Z. `Mesh::cube()` is a unit
cube centered at the origin. Both reuse shared geometry. `Mesh::new` accepts
vertices with position, normal and UV, plus counterclockwise triangle indices.
UV `(0, 0)` is at the top left. Faces render from both sides.

Object transforms apply scale, X/Y/Z Euler rotation, then translation. Normals
use inverse-transpose transforms for nonuniform scale. Scale components must be
finite and nonzero. Camera clip distances must satisfy `0 < near < far`.

## Materials and light

- `Material::color(color)` creates a lit solid surface.
- `Material::image(source)` maps the first decoded image frame onto UVs. Keep the
  image source stable across renders. An object is omitted while its image is unavailable.
- `Material::ui()` samples the viewport's captured UI without lighting.
- `.unlit(true)` disables lighting for any material.
- `.alpha_cutoff(value)` discards low-alpha pixels. Remaining pixels are opaque
  and write depth; fractional material transparency is not blended.

`Light` supplies a world-space direction toward the light, color, intensity and
ambient strength. Lighting is a basic diffuse model, without shadows, reflections
or physically based material parameters. Distinct opaque surfaces occlude each
other independently of object submission order. Coplanar surfaces should be
separated to avoid depth conflicts.

## UI texture

```rust
use gpui::{ParentElement, Styled, div, rgb};
use gpui_3d::{Material, Mesh, Object, Scene, viewport3d};

let world = Scene::new().object(
    Object::new(Mesh::plane(), Material::ui()).scale([3., 2., 1.]),
);
let viewport = viewport3d("panel", world)
    .size_full()
    .ui_texture(div().size_full().bg(rgb(0x20314b)).child("Hello, space."));
```

The UI texture is laid out at viewport size. Choose the mesh aspect ratio to
match that layout when preserving proportions matters. One live UI capture is
shared by all UI materials in the viewport. Text, images and ordinary descendants
are captured; deferred overlays are not part of the texture.

UI textures are decorative: pointer hits inside the source subtree are disabled,
without blocking camera controls on the enclosing viewport. Mesh picking and
mesh-to-UI event mapping are not provided. Do not use focusable controls or global
input listeners inside the capture.

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

## Example

```sh
cargo run -p gpui_3d --example orbit
```

Drag to orbit the scene and scroll to change distance. Toggle the UI plane to
inspect the image plane and textured cube behind it. `Reset` restores the camera.
