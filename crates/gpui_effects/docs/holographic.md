# Holographic material

Holographic shading combines procedural surface normals, directional lighting,
and angle-dependent spectral reflections. It supports solid surfaces, monochrome
text and SVG masks, and image stages in a subtree effect chain.

## Surface

```rust,ignore
use gpui::{prelude::*, px, rgb};
use gpui_effects::{HolographicOptions, holographic};

let surface = holographic(rgb(0x596373), HolographicOptions::default())
    .w(px(480.))
    .h(px(320.))
    .rounded(px(24.));
```

Use normal GPUI styling for size, position and corners. The material shades
inside the element bounds and does not add an external glow.

## Text and SVG

```rust,ignore
use gpui::{div, prelude::*, px, rgb, svg};
use gpui_effects::{HolographicOptions, holographic_masked};

let options = HolographicOptions::default();
let title = holographic_masked(
    div().text_size(px(56.)).child("Prism"),
    rgb(0xa7afbf),
    options,
);
let icon = holographic_masked(
    svg().path("icons/star.svg").size(px(48.)),
    rgb(0xa7afbf),
    options,
);
```

Mask shading uses glyph and SVG alpha coverage. It does not create a rectangular
background behind the content. Multicolor images should use an image stage.

## Subtree effect chain

```rust,ignore
use gpui_effects::{EffectStage, HolographicOptions, subtree_effect_chain};

let shaded = subtree_effect_chain(content, [
    EffectStage::holographic(HolographicOptions::default()),
]);
```

The image stage shades the captured RGB and preserves its alpha. Transparent
regions remain transparent. A strength of zero disables the stage. Place text
outside the shaded subtree when it should retain its original color.

## Parameters

- `MaterialSurface::tilt`: horizontal and vertical normal offsets, from -1 to 1.
- `curvature`: curvature across the surface; zero is flat.
- `roughness`: high values broaden highlights; low values concentrate them.
- `texture`: intensity of fine directional grain, filtered at small sizes.
- `MaterialLight::direction`: direction toward the light in right/down/viewer coordinates.
- `color`, `intensity`, `ambient`: direct light color, strength and ambient illumination.
- `iridescence`: spectral coloring; zero produces neutral reflections.
- `scale`, `angle`: spectral frequency and grain orientation in radians.
- `strength`: blend between original color and material shading.

Coordinates follow the surface aspect ratio. Surface normals are procedural;
external normal maps are not sampled. Non-finite scalar parameters use defaults,
and a zero or non-finite light direction uses the default direction.

## Interaction

Update `options.light.direction` to move illumination across a stationary surface.
Map centered pointer coordinates to its X and Y components and keep Z positive
to light the front of the surface. Alternatively, update `options.surface.tilt`
to tilt the surface beneath a fixed light. The effect owns no clock and requests
no animation frames. Applications control interpolation, freezing and redraws.

The shader functions `holographic_shader`, `holographic_mask_shader` and
`holographic_image_shader` are also available. Use
`HolographicOptions::uniforms(base_color)` to configure them; the image variant
uses sampled RGB instead of `base_color`.

## Example

```sh
cargo run -p gpui_effects --example holographic
```

The light direction smoothly follows the pointer across the foil card while the
surface stays fixed. Leaving the window returns the light to its default direction.
Controls provide freeze, roughness, texture, spectral coloring and reset.
