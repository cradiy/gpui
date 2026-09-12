# GPU fluid

`Fluid` manages the clock and input queue for a two-dimensional fluid surface.
Velocity, pressure and dye remain on the GPU. A built-in solver advects the fields,
enhances vortices and projects velocity through a pressure solve.

## State and input

Keep one state in the owning view:

```rust,ignore
use gpui_effects::{Fluid, FluidOptions};

let fluid = Fluid::new(FluidOptions::default());
```

Inject color and momentum from an input handler, then notify the view:

```rust,ignore
use gpui::{point, px, rgb};
use gpui_effects::FluidSplat;

self.fluid.splat(FluidSplat {
    from: previous_pointer,
    to: pointer,
    velocity: point(px(180.), px(40.)),
    radius: px(32.),
    amount: 0.8,
    color: rgb(0x48dfff),
});
cx.notify();
```

Positions are relative to the surface's top-left corner in logical pixels.
Each injection covers the segment from `from` to `to`, with a Gaussian brush
profile. Equal endpoints create a spot. `velocity` adds momentum in logical
pixels per second. `amount` controls dye density and is clamped to `0..=4`.
Color alpha scales density without changing the injected velocity. Set `amount`
to zero to stir existing dye without adding color.

`splat` returns false while paused, for non-finite coordinates, or when 32
commands are already queued. Commands are consumed on the next simulation update.

## Rendering

```rust,ignore
use gpui::prelude::*;
use gpui_effects::fluid;

let surface = fluid(&mut self.fluid).size_full();
```

For custom paint integration, obtain `self.fluid.frame()` during render and call
`window.paint_fluid(bounds, frame)` from a canvas paint callback. One state may
appear only once per rendered scene. Use separate states for independent surfaces.

The surface has a transparent background. Parent clipping and opacity affect its
output without changing the simulation. Moving the surface preserves local
coordinates; changing its size, device scale or grid resolution clears the fields.
Removing it from a rendered scene releases its buffers. Device recovery resets them.

## Configuration and playback

| Setting | Default | Range / behavior |
| --- | --- | --- |
| `resolution` | 256 | Longest grid dimension, `32..=512`; follows surface aspect ratio |
| `update_hz` | 60 | Maximum simulation frequency, `15..=120` |
| `pressure_iterations` | 24 | Pressure solver iterations, `8..=60` |
| `velocity_decay` | 0.35 | Exponential momentum decay per second, `0..=20` |
| `dye_decay` | 0.7 | Exponential density decay per second, `0..=20`; zero retains dye |
| `vorticity` | 12 | Vortex confinement strength, `0..=30` |

Use `options()` and `set_options()` to update settings. Grid resolution is
independent of window resolution. Lower grid resolution and fewer pressure
iterations reduce GPU work. The update frequency limits simulation work, not
the window's display refresh rate.

- `set_paused(true)` freezes simulation and discards pending input. Resuming
  excludes paused time.
- `clear()` clears velocity and dye on the next paint, including while paused.
- `advance(delta)` supplies an application-controlled elapsed interval instead
  of the wall clock used by `frame()`.

Notify the view after changing state. Empty and paused surfaces do not request
animation frames. Decaying surfaces stop after their dye lifetime; zero decay
keeps animation active until paused or cleared. After long gaps, one update
advances transport by at most `1/15 s`, while decay accounts for the full interval.

## Effect chains

```rust,ignore
use gpui_effects::{BloomOptions, EffectStage, fluid, subtree_effect_chain};

let surface = subtree_effect_chain(
    fluid(&mut self.fluid).size_full(),
    [EffectStage::bloom(BloomOptions::default())],
);
```

Linux WGPU and macOS Metal support GPU fluid without an extra Cargo feature.
Check `window.supports_gpu_fluid()` before offering the effect; unsupported
renderers draw no surface.

## Example

```sh
cargo run -p gpui_effects --example fluid
```

Drag to inject ink. Choose a color or switch to Stir to move existing ink.
The controls adjust vortex strength, grid resolution and Bloom, with pause,
resume and clear actions.
