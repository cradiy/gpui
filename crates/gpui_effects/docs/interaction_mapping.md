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

Layout and keyboard focus are not deformed. Deferred overlays can map their anchors
without scaling their content, as described below.
IME and accessibility geometry follow affine scopes as described below; nonlinear
effects retain untransformed geometry. Mapped child-view caches are invalidated when their
coordinate scope changes unless they opt into affine cache reuse as described below.
Backends without subtree-effect support retain ordinary rendering and pointer behavior.

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

### Transform groups

`transform_group(content, matrix)` applies one affine matrix to both captured pixels
and interaction geometry. The matrix uses logical pixels relative to the content's
top-left layout corner. Input handlers, accessibility nodes and mapped popup anchors
inherit the same coordinate scope, including for nested groups.

```rust
use gpui::{TransformationMatrix, div, prelude::*, px};
use gpui_effects::transform_group;

let viewport = transform_group(
    div().w(px(600.)).h(px(400.)).child("Content"),
    TransformationMatrix {
        rotation_scale: [[1.5, 0.0], [0.0, 1.5]],
        translation: [40.0, 20.0],
    },
);
```

Scale and rotation use the top-left origin. Compose translations around another pivot
when needed. Matrices must be finite and invertible; invalid matrices panic at construction.
Layout size, scroll deltas and keyboard focus order remain unchanged. Both source pixels
and the displayed result are limited to the group's rectangular viewport. A group cannot
reveal uncaptured content outside that source region. Put a background on the parent to
fill space exposed by translation or rotation.

The group captures at the current window density by default. Use
`transform_group(content, matrix).raster_scale(2.0)` for sharper magnified text and
vector content. This changes source rasterization without changing logical layout,
pointer coordinates or IME geometry. Zoom does not automatically choose a density;
magnification beyond the chosen density can still soften text. Keep the multiplier
fixed while zooming to reuse capture textures.

On WGPU, ordinary UI and nested transform groups allocate captures at the group's
bounds, clipped to the window, at both normal and increased density. Backdrop blur,
simulation, 3D, multipass, two-input effects and nested subtrees without an isolated
raster capture retain full-window captures. Shader positions stay in window coordinates.

Each requested multiplier is capped at four and reduced using the full window viewport
to fit 8192 pixels per axis and 16,777,216 pixels (64 MiB for one RGBA8 texture),
without reducing native density. This density limit also applies to cropped captures.
Nested groups share these dimension and area limits. These are per-capture limits,
not a total GPU memory budget; intermediate textures and glyph caches also consume
memory. Custom painting code should use `window.raster_scale_factor()` for device-pixel
shader parameters; layout and input continue to use `window.scale_factor()`.

Equal coordinate scopes reuse cached views;
matrix changes require the explicit cache option below to reuse view content. Platforms
without subtree effects draw and hit-test the original content. Deferred popups stay unscaled and can use
`anchored().map_anchor(true)` to follow the group.

On WGPU, an unchanged source subtree can reuse its captured pixels while the matrix
changes. Transform groups have independent source caches at both normal and increased
density. Ordinary subtrees sharing scratch textures require a single writer at that
capture depth and no multipass, two-input, raster-scaled or 3D layers in the frame.
Content, atlas-image, capture-region and text-rendering changes
invalidate the capture; video and simulation content is redrawn. This uses existing
capture textures and does not prevent child-view cache invalidation.

For an entity-backed view whose content is independent of the enclosing matrix, use
`.cached(style).cache_across_transforms()` inside the group:

```rust,ignore
let content = content_view
    .cached(div().w(px(600.)).h(px(400.)).style().clone())
    .cache_across_transforms();
let viewport = transform_group(content, matrix);
```

This reuses render, prepaint and paint output while updating hitboxes, mouse listeners
and IME handlers to the current affine scope. Event callbacks receive current source
coordinates. Render, layout and paint must not derive content from mapped pointer
positions or retain coordinate mappings. Notify the entity when its content changes.

Layout bounds, clipping, display or capture density, hover changes and explicit refreshes still
invalidate the view cache. A changed matrix also redraws views containing deferred
overlays, tooltips or nested input scopes. Inspector and accessibility rendering bypass
the view cache. Arbitrary mapping callbacks are not eligible for this option.

Run `cargo run -p gpui_effects --example transform_group` to exercise scaling, rotation,
panning, clicks and popup anchors.

### Custom drawing scopes

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
display density and the parent scope also match. Without `cache_across_transforms`, changing
any of these values invalidates the cache. Arbitrary callbacks invalidate cached views in each new
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
original accessibility geometry.

## Deferred overlay anchors

Use `anchored().map_anchor(true)` inside `deferred` to attach an unscaled popup to
an affine source position:

```rust
use gpui::{anchored, deferred, div, point, prelude::*, px};

let popup = deferred(
    anchored()
        .position(point(px(120.), px(80.)))
        .map_anchor(true)
        .offset(point(px(0.), px(6.)))
        .child(div().w(px(200.)).h(px(120.))),
);
```

The originating scope is captured when drawing is deferred. Window or local anchor
coordinates are resolved in that scope and then mapped to displayed window coordinates.
The offset remains a logical-pixel gap. Window-edge fitting runs after mapping, using
the popup's normal size. Painting, pointer hits, text input and accessibility inside
the popup use its final window coordinates.

Nested deferred wrappers preserve the source scope until an anchored element consumes
it. Submenus inside the positioned popup use window coordinates, so the source transform
is not applied twice. Cached views track the deferred anchor scope as well as input scopes.

Anchor mapping is disabled by default. Outside deferred drawing, or for a callback-based
scope with no forward map, it retains ordinary positioning. Mapping an anchor does not
transform the popup's contents or move overlays that do not opt in.

## Example

```sh
cargo run -p gpui_effects --example interaction_mapping
```

Switch between Lens and Stretch, hover or click the counters, and drag the slider
to adjust the effect. Lens ranges from 1× to 3× magnification; Stretch ranges from
zero to full deformation. Mapping on/off compares displayed targets with their
original hit regions.
