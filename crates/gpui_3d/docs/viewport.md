# 3D viewports

[Overview and documentation](../README.md) · [Examples](examples.md)

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
physical texture dimension, and captured-UI texture limit. The WGPU mesh renderer
uses four color samples when its linear-color and depth formats support them,
otherwise one. UI raster density is uniformly reduced to fit both the 2048-pixel
capture limit and the device limit without changing logical layout.

Unavailable states distinguish an unimplemented backend, absent renderer
resources, observed device loss, and missing device limits or format features.
Unsupported viewports retain layout and outer styling but do not paint a mesh or
capture UI; object callbacks do not report hits and pending UI routing is cleared
on the next prepaint. The application chooses its fallback content.

Linux X11, Wayland, Windows DX12, and the macOS Metal compositor query their
current WGPU mesh renderer. Windows D3D11 fallback reports `RendererUnavailable`.
Other platform windows report `BackendUnsupported` unless their renderer implements this capability.
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
