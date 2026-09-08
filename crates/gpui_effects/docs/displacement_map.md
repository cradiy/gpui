# Displacement maps

`subtree_displacement_map` distorts painted text, images and child elements using
an RG control texture. The built-in shader handles displacement, texture addressing
and optional masking.

```rust
use gpui::{div, prelude::*, px};
use gpui_effects::{DisplacementMapPreset, subtree_displacement_map};

let preset = DisplacementMapPreset::Water;
let content = subtree_displacement_map(
    div().text_size(px(48.)).child("Refraction"),
    preset.image(),
    preset.options(),
).time(1.5);
```

`Water` provides broad overlapping waves. `Heat` provides narrow streams with
mostly horizontal distortion and upward texture movement. Their 256 × 256 maps
are generated once and shared across elements. Time is supplied by the caller;
the effect does not schedule animation frames.

## External maps

The map accepts `ImageSource`, including file paths, decoded images and custom
loaders. Use a lossless RGBA image for predictable control values.

- Red controls the horizontal source-sampling offset; green controls the vertical offset.
- Channel value `128` is neutral. `255` gives the positive maximum and `0` the negative maximum.
- Alpha scales displacement strength. Transparent texels leave content unchanged.
- Blue is unused. Values are sampled as data, without sRGB decoding.

A positive offset samples content farther right or down, so the visible content
moves left or up. Negative amplitude reverses the corresponding axis.

```rust
use std::path::PathBuf;
use gpui::{div, point, prelude::*, px};
use gpui_effects::{DisplacementMapOptions, subtree_displacement_map};

let content = subtree_displacement_map(
    div().child("Moving light"),
    PathBuf::from("assets/flow-map.png"),
    DisplacementMapOptions {
        amplitude: point(px(10.), px(4.)),
        velocity: point(0.03, -0.05),
        ..Default::default()
    },
).time(2.0);
```

## Configuration

- `amplitude`: maximum sampling offset on each axis in logical pixels, clamped to ±256.
- `scale`: map repetitions across the capture on each axis, clamped to 0.01–64.
- `offset`: normalized map coordinates added after scaling.
- `velocity`: normalized map-coordinate movement per second. Zero produces a static map.
- `sampling`: `Repeat` tiles with seam-aware bilinear filtering; `Clamp` extends edge texels.
- `source_edge`: `Transparent` returns transparency beyond the capture; `Clamp` extends its edge texels.

Map coordinates cover the full capture, including paint padding. Changing a
chain's total padding changes this mapping. Padding is reserved automatically
for displacement; `.capture_padding(...)` overrides it. Clamping extends the
capture edge, which can itself be transparent when padding is present.

Input images use their first decoded frame. An unavailable or failed input skips
only its stage, preserving the preceding content. Other stages continue to run.
Image replacement follows the same rule while the new image loads.

## Local masking and composition

```rust
use std::path::PathBuf;
use gpui::{div, prelude::*};
use gpui_effects::{DisplacementMapPreset, EffectStage, subtree_effect_chain};

let preset = DisplacementMapPreset::Water;
let content = subtree_effect_chain(
    div().child("Surface currents"),
    [EffectStage::masked_displacement_map(
        preset.image(),
        PathBuf::from("assets/soft-mask.png"),
        preset.options(),
    )],
).time(1.0);
```

The local mask multiplies strength by its red channel and alpha. White enables
displacement; black disables it. It stretches over the capture independently
of the moving map. Masking changes displacement, not content opacity.

`EffectStage::displacement_map` and `masked_displacement_map` compose with blur,
Bloom, color adjustment and other stages. Parent clipping and final opacity
still apply. Layout and pointer targets remain at their original coordinates.
Renderers without subtree support paint the original content.

## Custom image stages

`EffectStage::with_images(shader, images)` binds the captured source first, then
one external image for `EffectShader::wgsl_two_images`, or three for
`EffectShader::wgsl_four_images`.

`sample_effect_image` reads the captured source. External samplers are
`sample_effect_second_image`, `sample_effect_third_image` and
`sample_effect_fourth_image`, each with `_cover` and `_repeat` variants.
External samples preserve straight alpha and stay within their atlas tile.
The ordinary sampler clamps at tile edges; `_repeat` filters across tile seams.

## Example

```sh
cargo run -p gpui_effects --example displacement_map
cargo run -p gpui_effects --example displacement_map -- /path/to/flow-map.png /path/to/mask.png
```

The example compares original and displaced content, with Water and Heat maps,
pause, strength, density, speed and an optional local mask. The second path is
optional; without it, the example supplies a soft circular mask.
