# Depth parallax

`depth_parallax` displays an image using a registered depth map and a normalized
view offset. It returns a styled `Effect`; give it an explicit size or a bounded
parent.

```rust
use std::path::PathBuf;
use gpui::{Styled, point, px};
use gpui_effects::{DepthParallaxOptions, depth_parallax};

let image = depth_parallax(
    PathBuf::from("landscape.png"),
    PathBuf::from("landscape-depth.png"),
    DepthParallaxOptions {
        offset: point(0.4, -0.2),
        ..Default::default()
    },
).w(px(800.)).h(px(500.));
```

## Inputs and framing

The color and depth images must describe the same framing. Different resolutions
are supported, but their normalized coordinates must correspond. The depth map's
red channel is read as data without gamma conversion: zero is far and one is near.
`invert_depth` reverses that convention. Transparent depth pixels use the focus
plane. Both image inputs must be available before the effect can draw.

The image uses centered cover fitting. The same crop is applied to both inputs.
A fixed overscan based on strength and focus reserves enough space for the full
view-offset range. Moving the pointer or returning to zero does not change zoom.
Setting strength to zero removes both displacement and overscan.

## Options

| Field | Default | Meaning |
| --- | --- | --- |
| `offset` | `(0, 0)` | View offset, each axis limited to −1–1 |
| `strength` | `0.045` | Full depth-range displacement relative to viewport size, limited to 0–0.15 |
| `focus` | `0.5` | Stationary plane, limited to 0–1 |
| `invert_depth` | `false` | Use black as near |
| `steps` | `32` | Front-to-back depth search steps, limited to 8–64 |

Positive X moves near content right and far content left; positive Y moves near
content down. Actual displacement includes the cover and overscan scale.
The caller owns pointer tracking, smoothing and frame scheduling. The effect
does not move layout or interactive regions.

Depth search selects the frontmost sampled surface and refines its intersection.
This is a single-image, 2.5D effect: hidden scenery cannot be reconstructed, and
large offsets or sharp depth discontinuities may stretch or repeat edge content.
Use modest strength and a depth map aligned with visible boundaries.

## Example

```sh
cargo run -p gpui_effects --example depth_parallax
cargo run -p gpui_effects --example depth_parallax -- /path/color.png /path/depth.png
```

Move the pointer over the landscape. `Parallax` toggles movement while retaining
the same crop; `Depth map` displays the depth input. The strength presets control
the displacement range.
