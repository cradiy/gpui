# Contour shadow

`subtree_contour_shadow` projects a directional soft shadow from the painted
alpha contour of text, icons and transparent images. The source is composited
over the shadow without displacement.

```rust
use gpui::{div, point, prelude::*, px};
use gpui_effects::{ContourShadowOptions, subtree_contour_shadow};

let title = subtree_contour_shadow(
    div().text_size(px(72.)).child("Float"),
    ContourShadowOptions {
        offset: point(px(20.), px(28.)),
        softness: px(12.),
        ..Default::default()
    },
);
```

## Projection

- `offset`: shadow direction and maximum reach in logical pixels. Positive X
  projects right; positive Y projects down. Vector length is limited to 256.
- `contact_softness`: edge softness near the source; default 1 logical pixel.
- `softness`: edge softness at the far end; default 12 logical pixels.
- `color`: shadow RGB and opacity. Zero opacity disables the stage.
- `threshold`: source alpha contour level, clamped to 0.001–0.999; default 0.5.

Both softness values are clamped to 0–128. Antialiasing remains enabled at zero.
The shadow softens along the projection and fades toward its far end. It uses
the thresholded silhouette rather than proportional transmission through
translucent pixels. Lower the threshold to include translucent content.

Keep opaque backgrounds outside the captured content when shadowing individual
glyphs or icons. Holes and disconnected shapes contribute their own contours.
Projected shadows can enter holes or overlap neighboring parts of the source.
This is a two-dimensional contour projection, not a scene-wide 3D light solver.

The effect automatically reserves paint padding for the projection and softness.
Parent clipping still applies; layout and hit regions are unchanged. Content
outside the viewport cannot supply contours to the capture. Unsupported
renderers paint the source normally.

## Composition and animation

Use `EffectStage::contour_shadow(options)` in a subtree effect chain. It receives
the preceding stage's alpha contour and passes source plus shadow onward.
Place it after `EffectStage::contour_relief` to combine bevel lighting and shadow
without using the shadow itself as a bevel source.

The shadow vector points away from the light. Update `offset` and notify the
owning view for pointer-driven lighting. The effect does not schedule animation
frames. Smooth interpolation and playback timing belong to the caller.

For a custom distance-field stage, `contour_shadow_shader()` consumes the source
and its distance field. Uniform slot 0 is shadow RGBA; slot 1 is
`[offset_x_px, offset_y_px, contact_softness_px, softness_px]` in device pixels.

## Example

```sh
cargo run -p gpui_effects --example contour_shadow
```

Move the pointer over the panel to guide the light. Controls adjust projection
distance, softness and opacity; shadow and relief can be enabled independently.
