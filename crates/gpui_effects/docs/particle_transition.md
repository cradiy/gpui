# Particle transition

`subtree_particle_transition` fragments painted text, icons and images into
colored particles. Increasing progress scatters them; decreasing it gathers
them into the source. Zero is the complete source and one is fully transparent.

```rust
use gpui::{div, prelude::*, px};
use gpui_effects::{ParticleTransitionOptions, subtree_particle_transition};

let content = subtree_particle_transition(
    div().text_size(px(72.)).child("Stardust"),
    0.4,
    ParticleTransitionOptions::default(),
);
```

## Progress and reversal

The caller owns progress and animation timing. Animate `0 → 1` to scatter and
`1 → 0` to gather. Progress can be paused, scrubbed or reversed at any point.
The effect does not schedule frames or integrate a simulation clock.

Each source cell has a fixed home coordinate, departure delay and curved path
derived from its grid coordinate and seed. The renderer evaluates these paths
directly from progress, without CPU pixel readback or history textures. Repeating
the same progress with the same source, layout and options produces the same
image, regardless of previous frames.

Keep source content, capture dimensions and options stable during a transition.
The source is captured on each paint, not frozen as a persistent snapshot.
Changing content or resizing changes the fragment mapping. Update such inputs
at an endpoint when continuity matters.

## Options

- `cell_size`: source fragment spacing in logical pixels, clamped to 1–12;
  default 3. Smaller cells produce finer particles.
- `scatter`: main displacement in logical pixels; each axis is clamped to
  ±512. The default is `(100, -70)`.
- `spread`: randomized path spread, clamped to 0–256; default 60 pixels.
- `radius`: luminous particle radius, clamped to 0.25–8; default 0.85 pixels.
- `streak`: short light-streak length, clamped to 0–32; default 6 pixels.
- `seed`: stable seed for departure delays and trajectories.

Source fragments gradually blend into soft particles with their sampled color
and alpha. Transparent cells contribute no light. Particle colors come from
cell centers; fine details smaller than a cell may fade during fragmentation.
The renderer limits each capture to 131,072 cells by increasing spacing for
large captures.

Paint padding is reserved automatically for movement and particle falloff.
Parent clipping still applies. Layout, accessibility and pointer targets remain
at their original coordinates; particles are not separate interactive elements.
Content outside the viewport is not available to the source capture.
Unsupported renderers paint the original content.

## Composition

`EffectStage::particle_transition(progress, options)` can be placed in a subtree
effect chain. It fragments the preceding stage's output and passes the particle
image to later stages, including Bloom and color adjustment.

## Example

```sh
cargo run -p gpui_effects --example particle_transition
```

Scatter and Gather change the playback direction without resetting progress.
Pause freezes playback; clicking the progress track selects a paused frame.
Controls select text or artwork, duration, spread and fragment size.
