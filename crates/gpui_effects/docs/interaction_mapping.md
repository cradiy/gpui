# Interaction mapping

`.map_interaction(true)` aligns pointer targets with a subtree's rendered geometry.
Hover, mouse button events and drag positions use the same displayed-to-source
mapping as the effect. The option is disabled by default.

```rust
use gpui::{div, prelude::*};
use gpui_effects::{LensOptions, subtree_lens};

let content = div().id("target").child("Open");
let surface = subtree_lens(content, LensOptions::default())
    .map_interaction(true);
```

## Supported stages

`EffectStage::lens` and `EffectStage::deformation` supply inverse coordinate maps.
`identity`, `blur`, `bloom` and `color_adjust` preserve target geometry. These stages can
be composed with `subtree_effect_chain`; pointer mapping runs through the chain
in reverse order. Nested wrappers map outer coordinates before inner coordinates.

Every enabled stage needs a mapping. Other built-ins require an explicit
`EffectStage::pointer_transform` matching their geometry. External-image stages
are unsupported. Enabling mapping on an unsupported chain panics when rendered
on a backend with subtree-effect support.

## Event coordinates

Inside a mapped listener, `event.position` and `window.mouse_position()` are
window-relative source coordinates. Subtract the control's layout origin to
obtain local coordinates. Pointer positions continue to be mapped outside the
capture while dragging; normal hit testing respects capture bounds and clipping.
Scroll deltas, button state and modifiers are unchanged.

Use `window.raw_mouse_position()` for displayed window coordinates, such as when
positioning an independent cursor-following lens.

Layout, keyboard focus, accessibility bounds, IME placement and deferred overlays
are not deformed. Mapped child-view caches are invalidated when their coordinate
scope changes. Backends without subtree-effect support retain ordinary rendering
and pointer behavior.

## Custom stages

`gpui::PointerTransform::new` receives the displayed position, snapped logical
capture bounds including padding, and the device scale factor. Return the
window-relative position sampled by the shader. The callback must use the same
parameters and animation time as the shader.

Set `.pointer_transform(transform)` after `.uniforms`, `.uniform` and
`.uniform_pixels`; these parameter setters clear any existing mapping. Rebuild
built-in stages from their options when changing their geometry.

Custom elements can use `window.with_pointer_transform` around both prepaint and
paint to scope hitbox insertion and event registration.

## Example

```sh
cargo run -p gpui_effects --example interaction_mapping
```

Switch between Lens and Stretch, hover or click the counters, and drag the slider
to adjust the effect. Lens ranges from 1× to 3× magnification; Stretch ranges from
zero to full deformation. Mapping on/off compares displayed targets with their
original hit regions.
