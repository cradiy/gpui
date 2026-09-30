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

Layout, keyboard focus and deferred overlays are not deformed.
IME and accessibility geometry follow affine scopes as described below; nonlinear
effects retain untransformed geometry. Mapped child-view caches are invalidated when their coordinate
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

## Affine coordinate scopes

Use `PointerTransform::affine` for a numeric source-to-display matrix. Coordinates
and translation are logical window pixels, independent of display density:

```rust
use gpui::{PointerTransform, TransformationMatrix};

let transform = PointerTransform::affine(TransformationMatrix {
    rotation_scale: [[2., 0.], [0., 2.]],
    translation: [80., 0.],
}).expect("finite, invertible transform");
```

Pass the transform to `window.with_pointer_transform` in both prepaint and paint,
or attach it to a custom stage with matching shader geometry. It supplies input
mapping only; it does not move or scale the painted content. Capture bounds still
clip both displayed and sampled positions.

Equal affine matrices and chains reuse cached child views when bounds, clipping,
display density and the parent scope also match. Changing any of these values
invalidates the cache. Arbitrary callbacks invalidate cached views in each new
scope, including shared callbacks that may capture mutable state.

`source_to_display` on `PointerTransform` or a hitbox's `PointerMapping` returns
the forward-mapped position for affine chains without clipping. It returns `None`
when a scope has no known forward map. Nonfinite or singular matrices are rejected
by the affine constructor.

## Text input geometry

Input handlers registered with `window.handle_input` retain the current coordinate
scope, including when a child view is cached. Return text and element bounds in
window-relative source coordinates. Affine scopes map these rectangles to displayed
window coordinates before passing them to the platform. Rectangles enclose all four
transformed corners; rotation and shear use an axis-aligned bounding rectangle.
Display-density and screen-origin conversion remain platform responsibilities.

IME composition anchors are resolved in source coordinates before transformation.
Changing the focused handler's scope refreshes the platform candidate position after
painting, without requiring another keystroke. Platform character-position queries
are mapped back to source coordinates without clipping.

Scopes containing a callback or decorative transform have no forward geometry map,
so text bounds retain their source coordinates.

## Accessibility geometry

Accessible elements retain their affine coordinate scopes. AccessKit receives relative
node transforms, with translation converted to physical pixels. Nested nodes in the
same scope inherit the transform once. Synthetic children inherit their owner's scope,
including text runs with character positions; authored node transforms are preserved.

GPUI's fallback accessibility click uses the resulting displayed bounds, and automation
snapshots report those bounds in logical window pixels. Nonlinear scopes retain their
original accessibility geometry. Layout and deferred overlay placement are unchanged.

## Example

```sh
cargo run -p gpui_effects --example interaction_mapping
```

Switch between Lens and Stretch, hover or click the counters, and drag the slider
to adjust the effect. Lens ranges from 1× to 3× magnification; Stretch ranges from
zero to full deformation. Mapping on/off compares displayed targets with their
original hit regions.
