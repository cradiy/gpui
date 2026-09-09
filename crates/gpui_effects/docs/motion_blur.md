# Motion blur

`subtree_motion_blur` applies centered directional blur to a UI subtree.
Supply translation velocity in logical pixels per second, using the captured
content's horizontal and vertical axes.

```rust
use gpui::{div, point, px};
use gpui_effects::{MotionBlurOptions, subtree_motion_blur};

let content = subtree_motion_blur(
    div().child("Night drive"),
    MotionBlurOptions {
        velocity: point(px(900.), px(0.)),
        ..Default::default()
    },
);
```

## Options

| Field | Default | Meaning |
| --- | --- | --- |
| `velocity` | Zero | Translation velocity in logical pixels per second |
| `exposure` | 1/120 second | Shutter duration, independent of frame rate |
| `strength` | 1 | Exposure multiplier, limited to 0–8 |
| `max_distance` | 24 px | Total support length, limited to 0–128 logical pixels |
| `samples` | 33 | Sampling budget, limited to 3–129 and rounded up to an odd count |

Support length is `min(speed × exposure × strength, max_distance)`.
Half extends in each direction from the current position. A soft shutter profile
keeps the center strongest. Sampling is reduced for short distances; higher
budgets improve thin-line continuity at large distances and device scales.

Zero velocity, exposure, strength or maximum distance skips the stage entirely.
Non-finite velocity or scalar controls disable the stage. Set velocity to zero
when motion stops. The component neither measures movement nor requests frames;
the application owns animation timing and velocity estimation.

## Composition and boundaries

Use `EffectStage::motion_blur(options)` inside `subtree_effect_chain` to combine
motion blur with other effects. Colors are accumulated with premultiplied alpha.
The wrapper adds paint-only capture padding; ancestor clipping still applies.
Layout, pointer targets and accessibility geometry remain unchanged. Transparent
blurred edges do not extend the element's interactive area.

The effect samples the current subtree along a translation vector. It does not
capture historical content, rotational motion or per-pixel deformation velocities.
Unsupported renderers display the original subtree.

## Example

```sh
cargo run -p gpui_effects --example motion_blur
```

Drag either card to move both copies. Release to return them to the center.
The exposure presets control the right-hand copy.
