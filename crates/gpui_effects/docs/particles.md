# GPU particles

`Particles` manages a simulation clock and a bounded emission queue. GPU compute
updates particle positions, velocities and lifetimes in persistent buffers;
instanced rendering draws soft light points and velocity-aligned streaks.

## State and emission

Keep one system in the owning view:

```rust,ignore
use gpui_effects::Particles;

let particles = Particles::new(4096);
```

Emit a burst from an input handler, then notify the view:

```rust,ignore
use gpui::{point, px, rgb};
use gpui_effects::ParticleSpawn;

let position = point(px(180.), px(120.));
self.particles.emit(ParticleSpawn {
    from: position,
    to: position,
    count: 160,
    speed: px(70.)..px(280.),
    color: rgb(0xb8dfff),
    ..Default::default()
});
cx.notify();
```

Positions are relative to the particle surface's top-left corner, in logical
pixels. Different `from` and `to` positions distribute particles along a segment.
The GPU samples radial speed, lifetime and radius from their configured ranges.
`velocity` adds a directional initial velocity; `stretch` controls the duration
of the velocity-aligned tail. Set `stretch` to zero for round light points.
Colors and shape settings are captured at emission and do not alter existing particles.

`emit` returns false for zero count, while paused, or when 32 commands are already
queued for the next frame. Each command is limited to the system capacity.
Capacity is clamped to `1..=65_536`. New emissions overwrite the oldest ring-buffer
slots once capacity is reached; there is no unbounded particle allocation.

## Rendering

```rust,ignore
use gpui::prelude::*;
use gpui_effects::particles;

let surface = particles(&mut self.particles).size_full();
```

For custom layout or paint integration, obtain `self.particles.frame()` during
render and call `window.paint_particles(bounds, frame)` from a canvas paint callback.
Use one occurrence of each system per rendered scene. Separate surfaces require
separate states. Layout and hit testing belong to the canvas, not to individual particles.

## Mask emission

`subtree_particles` samples the painted alpha of text, icons and transparent
images, then composites particles over the unchanged source:

```rust,ignore
use gpui::{div, prelude::*, px};
use gpui_effects::{ParticleMask, subtree_particles};

let title = subtree_particles(
    div().text_size(px(72.)).child("Luminous"),
    &mut self.particles,
    ParticleMask::default(),
).capture_padding(px(100.));
```

Queue emissions with `Particles::emit`. The mask replaces `from` and `to`;
all other spawn settings and simulation forces apply normally.

- `threshold`: minimum source alpha, clamped to 0.001–0.999; default 0.5.
- `edge_width`: approximate inward edge band in logical pixels, clamped to
  0–128. The default is 2; zero samples the whole opaque shape.
- `inherit_color`: samples source RGB and multiplies source alpha by emission
  opacity. When false, particles use the configured spawn color.

Keep opaque panel backgrounds outside the captured element when emitting from
individual glyphs or icons. Transparent gaps and holes emit nothing. An empty
mask leaves existing particles to finish their lifetimes.

Sampling and simulation stay on the GPU. Each emission frame samples at most
131,072 locations on a regular grid; large captures use wider spacing, so very
thin features can be missed. Edge selection compares alpha in eight directions.
Source colors and positions are retained at birth. Changing the source does not
move existing particles. Only the visible viewport portion supplies new samples.

Particles are clipped to the capture bounds. Reserve transparent space with
`capture_padding`, or capture a larger transparent container. Force coordinates
are relative to the padded capture's top-left corner. Layout and hit regions do
not change. Unsupported renderers paint the source without particles.

Use `EffectStage::masked_particles(frame, mask)` to compose emission with other
stages. It samples the preceding stage and passes both source and particles to
subsequent stages such as Bloom. Each particle system may appear once per scene,
including ordinary particle canvases and masked stages.

## Forces and playback

```rust,ignore
let mut physics = self.particles.physics();
physics.attractor = pointer_local;
physics.strength = px(750.);
physics.radius = px(180.);
self.particles.set_physics(physics);
```

Positive force strength attracts; negative strength repels. The local force
smoothly fades to zero at its radius. Uniform `acceleration` is measured in logical
pixels per second squared. `drag` is exponential velocity damping per second.

- `set_paused(true)` freezes simulation and discards pending emissions. Resuming
  excludes paused time from the clock.
- `clear()` clears particles on the next paint, including while paused.
- `advance(delta)` supplies an application-controlled simulation delta instead
  of `frame()`'s wall clock.

Notify the view after changing state. Visible, unpaused surfaces request animation
frames until the latest possible particle expiry. Empty and paused systems do not
request them. When using an external clock, pause the system while that clock stops.

Physics uses substeps no longer than `1/60 s`, with at most eight substeps per
update. Lifetimes account for the full elapsed time even after a longer gap.

## Effect chains

```rust,ignore
use gpui_effects::{BloomOptions, EffectStage, particles, subtree_effect_chain};

let surface = subtree_effect_chain(
    particles(&mut self.particles).size_full(),
    [EffectStage::bloom(BloomOptions::default())],
);
```

Particle content can also feed a `Feedback` stage for retained trails. Emit into
feedback on frames whose particle image should be accumulated. Keep feedback's
clock and pause/clear controls synchronized with the particle system as needed.

Parent clipping and opacity apply to the visible output, without changing the
retained simulation state. A surface keeps its logical coordinates when moved;
changing its size, device scale or capacity resets its GPU state. Removing it
from a rendered scene releases its buffers. Device recovery also resets them.

Linux WGPU supports GPU particles. Check `window.supports_gpu_particles()`;
unsupported renderers do not draw the particle surface.

## Example

```sh
cargo run -p gpui_effects --example particles
```

Move the pointer to scatter stars or click to emit a burst. Controls select
free motion, attraction or repulsion; light points or streaks; color and Bloom.
Pause and Clear control the particle system.

```sh
cargo run -p gpui_effects --example particle_mask
```

Switch between text and transparent artwork, edge and fill emission, or source
and fixed colors. Controls adjust emission density, speed and lifetime.
