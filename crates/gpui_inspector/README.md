# GPUI Inspector

An optional inspector panel for GPUI applications. Pick elements on screen or
browse the collapsible element tree. Rows show element IDs, direct text previews,
and measured dimensions. The selected element remains highlighted.

- **Layout**: resolved box model, configured size constraints, Flex/Grid alignment,
  spacing, and clipping.
- **Style**: color swatches, opacity, borders, corner radii, and shadows.
- **Text**: inherited base typography, direct text previews, and text layout bounds.
- **Source**: Rust type, element identity, and source location.

**Parent** selects the containing element. **Copy report** copies all four pages
and resolved spacing for the selected element. **Hide overlay** shows the
application's original colors while preserving the selection.

```rust,ignore
gpui_inspector::init(cx);
// In an application-owned button or action handler:
window.toggle_inspector(cx);
```

The application controls opening the panel and any keyboard bindings. The crate
enables GPUI's `inspector` feature, including in release builds. Applications that
do not depend on this crate do not include its UI.

Run the example:

```sh
cargo run -p gpui_inspector --example inspector
```

Picking intercepts pointer events in the application area. Scroll while picking
to select an ancestor. Stop picking to interact with the application normally.
Custom element details can be added with `App::register_inspector_element`.

The tree contains laid-out elements with source locations, including the visible
portion of virtualized lists. Deferred elements appear as separate roots.
Bounds are logical layout coordinates before subtree effects. The box model
shows resolved spacing in pixels; configured values preserve percentages and
`auto`. Text previews contain up to 256 Unicode scalar values per layout. Font
information describes the inherited base style and requested font stack; per-span
overrides and per-glyph fallback faces are not listed. View caching is disabled
while the panel is open so inspection reflects current layout.
