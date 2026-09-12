# GPUI

This is an independently maintained GPUI repository extracted from [Zed](https://github.com/zed-industries/zed).

GPUI is a GPU-accelerated UI framework written in Rust. It provides core building blocks for element layout, text rendering, window management, input handling, state management, and asynchronous tasks. This repository retains the GPUI core, platform backends, and supporting crates while removing code specific to the Zed editor.

It currently includes support for Linux, macOS, Windows, and the web, along with the WGPU rendering backend and Tokio integration.

## What This Fork Adds

Development after the initial GPUI import focuses on embedded 3D rendering,
reusable visual effects, media playback, UI components, and desktop integration.

### `gpui_3d`

[`gpui_3d`](crates/gpui_3d/README.md) brings 3D scenes into ordinary GPUI
layouts, with the same scene data available to a window-free renderer:

- Scene hierarchies, shared mesh instances, perspective/orthographic cameras,
  Orbit controls, and spatial queries.
- PBR materials, image textures, HDR environment lighting, direct lights,
  directional shadows, and configurable antialiasing.
- Application-defined WGSL materials, texture and parameter bindings, custom
  vertex attributes, and additional mesh passes for effects such as outlines.
- Animation tracks, pose blending, joint limits and IK, CPU/GPU Morph and Skin,
  and external GPU deformation inputs.
- Retained color, HDR, depth, normal, and object-ID outputs, GPU label maps,
  and asynchronous picking against rendered geometry.
- Captured GPUI surfaces with UV-mapped pointer interaction, alongside normal
  2D application controls.

[`gpui_3d_gltf`](crates/gpui_3d_gltf/README.md) adds glTF/GLB import,
materials and images, cameras and lights, animation, and reusable model instances.
The core remains format-independent; application workflows and physics solvers
build on its scene, geometry, and pose interfaces.

Embedded 3D rendering currently targets **Linux WGPU**. GPUI's broader platform
support does not imply 3D viewport support on every backend. Native GPU compute,
custom material programs, and direct headless output use the optional `wgpu`
feature and require compatible device capabilities.

Start with the [3D overview](crates/gpui_3d/README.md),
[viewport integration](crates/gpui_3d/docs/viewport.md),
[examples](crates/gpui_3d/docs/examples.md), or the
[model viewer](crates/gpui_3d_gltf/docs/viewer.md).

### `gpui_effects`

[`gpui_effects`](crates/gpui_effects) extends GPUI with reusable GPU-backed
visual components:

- Extensible WGSL effects with uniforms and zero-, one-, two-, or four-image
  inputs.
- Built-in Aurora, Plasma, Color Orbs, Album Glow, and Album Ripples effects.
- Shader effects and gradient fills masked by arbitrary elements, including
  ready-to-use text and SVG helpers.
- `FrostedGlass` panels with strong backdrop blur, light/dark appearances,
  normal `div()` layout behavior, and mergeable rounded surfaces.
- A page-flip component with rigid, soft, and curl styles, single- or
  double-page layouts, lazy content providers, and preloading.
- `MotionLayer` for coordinated movement along linear, curved, or custom paths.
- Timed text with character or word timing, gradient reveal, grouped emphasis,
  and playback-clock integration.

See the [glass guide](crates/gpui_effects/docs/glass.md) and the complete
[`gpui_effects` documentation](crates/gpui_effects/README.md).

### `gpui_media`

[`gpui_media`](crates/gpui_media) provides reusable video playback for GPUI
applications. See its [documentation](crates/gpui_media/README.md) for usage.

### Other Extensions

- Additional rendering and styling primitives, including per-side border
  colors, animated gradients, color SVGs, and platform backdrop effects.
- Desktop integrations such as native system trays, Wayland internal drag and
  drop, drag icons, and screen capture.
- [`uic`](uic), a reusable component library with generated Lucide icons and
  components such as `ColorPicker`.

## Getting Started

The Rust toolchain is defined in `rust-toolchain.toml`. After cloning the repository, run an example with:

```sh
cargo run --example hello_world
```

More examples:

```sh
cargo run --example image_gallery
cargo run --example text
cargo run --example svg
```

Check the entire workspace with:

```sh
cargo check --workspace
```

Example source code is available in [`crates/gpui/examples`](crates/gpui/examples).

For 3D scenes, custom materials, and local models:

```sh
cargo run -p gpui_3d --features wgpu --example scene
cargo run -p gpui_3d --features wgpu --example materials
cargo run -p gpui_3d_gltf --features wgpu --example viewer -- /path/to/model.glb
```

For rendering without a window:

```sh
cargo run -p gpui_3d --features wgpu --example headless -- /tmp/gpui-3d-outputs
```

## License

This is a mixed-license repository:

- GPUI-related code derived from Zed remains licensed under the Apache License
  2.0. See [LICENSE-APACHE](LICENSE-APACHE).
- The independently developed `gpui_3d`, `gpui_3d_gltf`, `gpui_effects`,
  `gpui_media`, `uic`, and `uic-macros` crates declare the MIT License in their
  package metadata. See each crate's `Cargo.toml` and included license files.
- Third-party assets retain their original licenses. In particular, the Lucide
  icons bundled by `uic` retain the Lucide ISC and Feather MIT license text in
  [`uic/assets/icons/LICENSE`](uic/assets/icons/LICENSE).
