# Water ripple

`subtree_ripples` refracts the pixels of an element subtree with outward-moving
radial waves. Text, images and decorations share the same displacement field.
Each wave has a soft spatial envelope and fades to zero over its lifetime.

## Usage

```rust,ignore
use std::time::Duration;
use gpui::point;
use gpui_effects::{Ripple, RippleOptions, subtree_ripples};

let surface = subtree_ripples(content, RippleOptions::default(), [
    Ripple {
        center: point(0.5, 0.5),
        elapsed: Duration::from_millis(300),
    },
]);
```

`center` is normalized to the capture rectangle: `(0, 0)` is its top-left corner,
and `(1, 1)` is its bottom-right corner. Convert pointer positions by subtracting
the capture origin and dividing by its width and height. If the effect chain uses
capture padding, include that padding in the rectangle used for conversion.

Keep emission times in the owning view and derive `elapsed` from the application
clock. Rebuild the stage and request animation frames while waves remain active.
The stage does not start a timer or retain playback state. Remove expired waves
and stop requesting frames when the surface becomes idle. For paused playback,
keep elapsed times unchanged and exclude paused time when resuming.

One stage combines up to `MAX_RIPPLES` (four) live waves in a single pass. When
more are supplied, it uses the last four live entries. Non-finite centers and
expired waves are ignored. Empty input or zero amplitude disables the stage.

## Configuration

`RippleOptions` controls:

- `amplitude`: maximum combined displacement, default `12 px`.
- `wavelength`: distance between peaks, default `72 px`.
- `width`: wave-packet half-width, default `110 px`.
- `speed`: outward travel per second, default `340 px` per second.
- `duration`: wave lifetime, default `2.2 s`.
- `edge_fade`: fade distance at capture edges, default `32 px`.

Spatial values use logical pixels and follow the window scale. Amplitude and
speed are clamped to nonnegative values; wavelength, width and edge fade to at
least one pixel; duration to at least one millisecond. Displacement fades at the
capture edges, and sampling is clamped to the input rectangle. The stage does
not add capture padding or change the input colors.

## Composition

```rust,ignore
use gpui_effects::{BloomOptions, EffectStage, subtree_effect_chain};

let surface = subtree_effect_chain(content, [
    EffectStage::ripples(options, waves),
    EffectStage::bloom(BloomOptions::default()),
]);
```

Stages run in order. Ripple coordinates refer to the shared capture rectangle,
including padding contributed by other stages. `.time()` on the chain does not
replace the per-wave `elapsed` values.

The effect changes rendered pixels only; layout and hit testing retain their
original coordinates. Linux WGPU and macOS Metal support subtree effects. On unsupported
backends the input is drawn normally; check `window.supports_subtree_effects()`.

## Example

```sh
cargo run -p gpui_effects --example ripple
```

Click the artwork or text to emit a wave. The example offers three displacement
strengths and a clear button.
