# Contour light

`subtree_contour_glow` adds light around the alpha contour of text, icons and
transparent artwork. The source keeps its original colors. Brightness is
concentrated at the edge and fades to zero within the configured radius.

```rust
use gpui::{div, prelude::*, px, rgb};
use gpui_effects::{ContourGlowOptions, subtree_contour_glow};

let title = subtree_contour_glow(
    div().text_size(px(48.)).child("Outline"),
    ContourGlowOptions {
        color: rgb(0x76deff),
        radius: px(18.),
        edge_width: px(1.5),
        ..Default::default()
    },
);
```

## Configuration

- `color`: glow color and opacity.
- `radius`: outer support in logical pixels, from 0 to 256.
- `edge_width`: bright edge width, from 0 to the support radius.
- `intensity`: light strength, from 0 to 4. Zero disables processing.
- `threshold`: alpha level defining the contour, from 0.001 to 0.999; default 0.5.

Capture content without an opaque background to light its individual outlines.
Holes and disconnected shapes participate in the same distance field. An opaque
panel produces a panel contour instead. Use a lower threshold for translucent
content whose alpha does not reach 0.5.

The wrapper reserves paint-only padding. Parent clipping still applies; layout,
accessibility and child hit regions are unchanged. Layer opacity is applied after
the effect. An empty input produces no light. Unsupported renderers paint the
source normally.

## Composition

`EffectStage::contour_glow(options)` can be included in a subtree effect chain.
Each stage operates on the previous stage's output.

For custom distance-based shading, use
`EffectStage::distance_field(two_image_shader, threshold)`. Image one contains
the source. Image two contains a full-resolution signed distance field:

- R: distance in device pixels, negative inside the contour.
- G: one if a contour exists, zero otherwise.
- B: reserved.
- A: one.

Sample the field with `sample_effect_second_image`. Distances use a jump-flood
approximation to Euclidean distance with subpixel alpha crossings. Set
`capture_padding` for outward effects and use `uniform_pixels` for shader
parameters expressed in logical pixels.

The WGPU backend reuses scratch textures across sequential stages and releases
them when no distance-field stage is present. The effect does not schedule
animation frames; input changes trigger rendering through normal GPUI updates.

## Example

```sh
cargo run -p gpui_effects --example contour_glow
```
