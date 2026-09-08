# Contour relief

`subtree_contour_relief` shades bevels derived from the alpha contour of text,
icons and transparent artwork. Positive depth raises the shape; negative depth
recesses it. Source alpha is preserved, including holes and antialiased edges.

```rust
use gpui::{div, prelude::*, px};
use gpui_effects::{ContourReliefOptions, subtree_contour_relief};

let title = subtree_contour_relief(
    div().text_size(px(48.)).child("Relief"),
    ContourReliefOptions {
        width: px(5.),
        depth: px(3.),
        ..Default::default()
    },
);
```

## Surface and light

- `width`: inward bevel width in logical pixels, from 0 to 128.
- `depth`: surface height in logical pixels, from -128 to 128. Zero disables shading.
- `roughness`: highlight spread, from 0 to 1.
- `specular`: reflected highlight strength, from 0 to 2.
- `light`: `MaterialLight` direction, color, intensity and ambient illumination.
- `strength`: blend with the source, from 0 to 1. Zero disables shading.
- `threshold`: alpha contour level, from 0.001 to 0.999; default 0.5.

Light directions use surface coordinates: X points right, Y down, and Z toward
the viewer. Update `light.direction` and notify the owning view to follow pointer
movement. The effect does not schedule animation frames.

Capture foreground content without an opaque background to shade individual
glyphs and icons. With an opaque background, the captured panel defines the
contour. Lower `threshold` for translucent content below the default alpha level.

This is height-based lighting, not extruded geometry. It does not move pixels,
cast shadows onto neighboring elements, or change layout and hit regions.
Parent clipping and final layer opacity apply normally. Unsupported renderers
paint the source without shading.

## Composition

Use `EffectStage::contour_relief` in an effect chain. Distance generation uses
the same GPUI distance-field stage as contour light.

`contour_surface_wgsl()` supplies two reusable WGSL functions for custom
`EffectStage::distance_field` composites:

- `contour_height(distance, width, depth)` maps signed distance to surface height.
- `contour_normal(input, width, depth, sample_step)` samples the distance field
  in image two and returns a surface normal pointing toward the viewer.

All distances supplied to these functions are device pixels. Use
`uniform_pixels` for logical-pixel configuration.

## Example

```sh
cargo run -p gpui_effects --example contour_relief
```

Move across any panel to illuminate the flat, raised and recessed samples with
the same light direction. Controls adjust bevel width, depth and roughness.
