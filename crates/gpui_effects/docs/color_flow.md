# Color flow

The [color flow example](../examples/color_flow.rs) extracts four alpha-weighted
main colors once from the decoded image. Soft color volumes overlap above an
image-derived dark base, with monochrome dithering for smooth gradients. The
preview displays the extracted palette alongside tone, brightness, speed,
cohesion, diffusion, and pause controls. Rendering defaults to the component's
tone-grouped image sampling; click Source to switch to the extracted palette.
`PreviewSettings::use_extracted_palette` selects the initial mode.

## Run the example

From the workspace root, use the bundled cover:

```sh
cargo run -p gpui_effects --example color_flow --features gpui_platform/runtime_shaders
```

Or supply an image path:

```sh
cargo run -p gpui_effects --example color_flow --features gpui_platform/runtime_shaders -- "/path/to/image.jpg"
```

## Brightness

Background brightness defaults to `1.0` (100%). The Dimmer / Brighter controls
adjust it by `0.05` within `0.0..=2.0`, including while paused. Brightness affects
only the flowing background, not the cover or palette swatches. `0.0` produces a
black background; values above `1.0` brighten it and may clip highlights.

Configure these fields in `PreviewSettings::default()` in the example:

- `effect`: the `ColorFlowOptions` configuration; use
  `ColorFlowOptions::default().brightness(0.6)` for a 60% background.
- `brightness_step`: amount changed by each button click; defaults to `0.05`.
- `brightness_min`: lower control limit; defaults to `0.0`.
- `brightness_max`: upper control limit; defaults to `2.0`.

The settings belong to the preview instance and are not persisted between runs.

## Color depth

The background uses darker palette colors for its base. Local color volumes
reveal more light where they overlap and fade back into that base outside their
shared region. Source highlights use a soft RGB ceiling that preserves channel
ratios; colors below 75% of the ceiling keep their input brightness. Bright
neutral colors have a separate palette weight so white artwork
does not contribute as strongly as its colored areas.

```rust
let options = ColorFlowOptions::default()
    .shadow_level(0.85)
    .highlight_level(0.75)
    .neutral_weight(0.70)
    .brightness(1.0);
```

- Lower `shadow_level` for deeper shadows without dimming the local colors as
  much. `0` uses a black base; `1` uses the full toned dark base.
- Lower `highlight_level` to compress light colors. It is a soft RGB ceiling,
  not an exposure multiplier; `0` makes the background black. Saturation,
  brightness, and dithering are applied afterward and can exceed this ceiling.
- Lower `neutral_weight` to reduce the relative contribution of bright gray and
  white. `1` retains their original weight; `0` excludes fully bright neutral
  entries. Dark neutrals and chromatic colors retain their weight. A palette of
  only neutral colors stays neutral; no replacement hue is synthesized.

The preview has − / + controls for each of these fields, including while paused.
`PreviewSettings::tone_step` sets the increment (default `0.05`); controls clamp
to `0..=1`. These options apply to both tone-grouped image sampling and supplied palettes.
The displayed cover and palette swatches retain their original colors.

## Flow and palette configuration

`PreviewSettings` holds the effect options, playback speed, control limits, and
palette sampling configuration. Slower / Faster adjusts animation
speed, Less diffuse / More diffuse adjusts color blending, and Pause / Resume
stops or continues the animation without resetting its phase.
Looser / Tighter adjusts cohesion; `PreviewSettings::cohesion_step` configures
the button increment (default `0.1`).

The [`color_flow` component](../src/color_flow.rs) accepts named
`ColorFlowOptions`. Individual builder methods preserve the other options:

```rust
use gpui_effects::{ColorFlowOptions, color_flow};

let options = ColorFlowOptions::default()
    .brightness(0.6)
    .diffusion(0.22)
    .saturation(0.9)
    .motion(1.0)
    .cohesion(0.85)
    .drift(0.45)
    .dither(1.0);

let background = color_flow(cover)
    .options(options)
    .time(elapsed_seconds);
```

| Option | Default | Meaning |
| --- | --- | --- |
| `diffusion` | `0.18` | Softness of color blending, `0..=0.3` |
| `saturation` | `1.0` | Nonnegative saturation multiplier; `0` is grayscale |
| `brightness` | `1.0` | Nonnegative background brightness multiplier |
| `shadow_level` | `0.85` | Dark base level, `0..=1`; `0` uses a black base |
| `highlight_level` | `0.75` | Soft RGB ceiling before saturation and brightness, `0..=1` |
| `neutral_weight` | `0.70` | Relative weight of bright neutral colors, `0..=1` |
| `motion` | `1.0` | Movement amplitude, `0..=2`; `0` freezes the field |
| `flow_scale` | `1.0` | Broad flow distortion, `0..=3` |
| `drift` | `0.45` | Travel amplitude, `0..=2`; mainly shared group movement at high cohesion |
| `cohesion` | `0.85` | Shared motion and confinement, `0..=1`; higher values keep colors together |
| `vignette` | `0.18` | Edge darkening, `0..=1` |
| `seed` | `0.37` | Stable variation of the color arrangement and paths |
| `glow` | `0.20` | Nonnegative light density; higher values reveal more local color over the base |
| `dither` | `1.0` | Monochrome noise in 8-bit color steps, `0..=2`; `0` disables it |

Layout, rounding, shadows, and opacity use the normal GPUI `Styled` methods.
Pass elapsed seconds to `.time()`; playback speed is controlled by how quickly
that time advances. Keep it fixed to pause.

For concentrated blending, use high `cohesion` with low `drift`. At
`cohesion = 1.0`, all colors share a small moving region while local deformation
and soft overlap continue within it. Reducing `drift` limits the group's travel
without stopping that internal mixing. `cohesion = 0.0` allows independent color
travel. `diffusion` controls the softness of the overlap, not the travel distance.

## Supplying a palette

`.palette(...)` takes a `ColorFlowPalette`: four `ColorFlowPaletteColor` entries
with named `color` and `weight` fields. Weight controls the contribution of a
color, independently of its alpha; zero excludes it.

```rust
use gpui::rgb;
use gpui_effects::{ColorFlowPaletteColor, color_flow};

let palette = [
    ColorFlowPaletteColor::new(rgb(0x493867), 0.4),
    ColorFlowPaletteColor::new(rgb(0xbd7187), 0.3),
    ColorFlowPaletteColor::new(rgb(0x427e83), 0.2),
    ColorFlowPaletteColor::new(rgb(0xd6a674), 0.1),
];

let background = color_flow(cover)
    .options(options)
    .palette(palette)
    .time(elapsed_seconds);
```

Omit `.palette(...)` or pass `None` to take sixteen stratified image samples and
group them into four bands over the sampled luminance range. Each band's RGB
average and population are alpha-weighted. Empty bands have zero weight. This
keeps distinct tonal areas separate, but similarly bright colors may still
share a band; supply an extracted palette when finer hue selection is needed.
Passing `Some(palette)` is also supported. The preview passes its optional
extracted palette in extracted-palette mode. Colors and weights are clamped to
`0..=1`.

Uniform packing and palette selection are handled inside the component. Use
`.into_effect()` only when an integration requires a low-level `Effect`.
