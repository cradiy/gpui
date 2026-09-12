# gpui_macos

AppKit windows and Metal rendering for GPUI.

The default renderer supports GPU fluid, particles, subtree effects, background
blur, and 3D scenes. These capabilities require no additional Cargo feature.
Applications can query `Window::supports_gpu_fluid()`,
`Window::supports_gpu_particles()`, `Window::supports_subtree_effects()`, and
the scene's 3D capabilities before presenting an effect.

CoreVideo BGRA, RGBA, and NV12 frames can be sampled by subtree effects and
3D UI captures through shared Metal textures. GPU information is available
through the platform window's `gpu_specs()` method.

`WindowKind::AnchoredPopup` uses a child panel with parent-relative placement,
screen-edge adjustments, and automatic cleanup when its parent closes. Grabbing
popups must open during a mouse press; Escape or application deactivation
dismisses them. Closing popups for clicks elsewhere in the same application
is the caller's responsibility.

`Window::promote_active_drag_to_system()` starts an AppKit drag session between
GPUI windows. Its typed payload stays in the process. The drag image follows
the active drag view, including size, scale and hotspot changes.

Run the native-window example:

```sh
cargo run -p gpui --example native_windows
```

The example opens two windows for dragging and dropping, plus a menu button.
F2 opens a passive anchored popup; Command-Q exits.

Wayland layer-shell, X11 selections, and compositor decoration protocols remain
Linux-specific. macOS uses AppKit decorations and CoreText grayscale text.
