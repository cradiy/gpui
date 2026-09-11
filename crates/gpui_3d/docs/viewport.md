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

## Topics

| Topic | Content |
| --- | --- |
| [Geometry](topics/geometry.md) | Coordinates, mesh construction, primitives, normals, and vertex updates. |
| [Scenes](topics/scenes.md) | Node hierarchy, identities, reusable subtrees, cameras, and light nodes. |
| [Cameras](topics/camera.md) | Projection, optics, framing, Orbit controls, and damping. |
| [Animation](topics/animation.md) | Transform and weight tracks, pose blending, Morph, and skinning. |
| [Constraints](topics/constraints.md) | Follow, Aim, joint limits, and IK chains. |
| [GPU deformation](topics/deformation.md) | Morph/Skin computation, retained buffers, resource admission, and CPU readback. |
| [Materials](topics/materials.md) | PBR, transparency, texture sampling, tangent frames, and color output. |
| [Lighting](topics/lighting.md) | Direct lights, shadows, HDR backgrounds, and diffuse/specular environments. |
| [Queries](topics/queries.md) | Rays, bounds, spatial indices, filtering, and object picking. |
| [UI textures](topics/ui.md) | Captured UI sizing, pointer routing, and interaction limits. |
| [Rendering](topics/rendering.md) | Resource preparation, batching, quality, effects, caches, and measurements. |
| [Headless output](topics/headless.md) | Output channels, GPU ownership, readback, and resource limits. |

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

## Headless output

The optional `wgpu` feature provides `HeadlessRenderer` for the same scenes without
a native window or UI layout. It accepts solid and decoded-image materials and
returns independently selectable display-color, linear-HDR, object-ID, linear-depth, and world-normal
textures with a frame-local identity map and bounded
nonblocking CPU readback. See [Headless rendering](topics/headless.md) for formats,
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
