# History feedback

`Feedback` retains a surface across frames. New input is composited over fading
history, producing trails and afterimages from text, images or custom drawing.
The built-in WGSL shader handles accumulation and time-based decay.

## State and input

Keep one `Feedback` in the owning view:

```rust,ignore
use gpui_effects::{Feedback, FeedbackOptions};
use std::time::Duration;

let feedback = Feedback::new(FeedbackOptions {
    fade_duration: Duration::from_millis(1200),
    downsample: 2,
});
```

Call `emit()` when the next rendered input should be recorded, then notify the
view. Without `emit`, the stage only fades the retained pixels. Multiple calls
before a frame record that frame's input once.

```rust,ignore
// Input handler
self.feedback.emit();
cx.notify();

// Render
let content = gpui_effects::subtree_feedback(drawing, &mut self.feedback);
```

Use a fixed-size transparent drawing region. Put backgrounds and controls
outside it: opaque input covers previous history wherever it is recorded.
For continuous strokes, include the segment between input positions in the
new frame's drawing.

## Effect chains

```rust,ignore
use gpui::{px, prelude::*};
use gpui_effects::{BloomOptions, EffectStage, subtree_effect_chain};

let content = subtree_effect_chain(drawing, [
    self.feedback.stage(),
    EffectStage::bloom(BloomOptions {
        radius: px(24.),
        ..Default::default()
    }),
]).capture_padding(px(32.));
```

This order stores the drawing and adds Bloom to the visible trail. Placing Bloom
before feedback stores the processed input instead. History captures the stage
input, not the later effects or the wrapper's final opacity.

Keep capture padding stable when toggling downstream effects if history should
remain in place. Each feedback state may occur only once in a rendered scene;
use separate states for independent drawing regions.

## Playback

- `stage()` advances using wall-clock elapsed time.
- `advance(delta)` builds a stage using an application-supplied frame delta instead.
- `set_paused(true)` freezes retained pixels and ignores new `emit` calls.
  Resuming excludes the paused time from decay.
- `clear()` clears history on the next paint, including while paused.
- `set_fade_duration(duration)` changes how long trails remain visible.

Notify the view after changing playback state. Visible, unpaused feedback stages
request animation frames while a trail remains. When it expires, history clears
and those requests stop. Pause feedback when stopping an application-driven clock.

`fade_duration` is clamped to at least one millisecond. Retained alpha decays
exponentially to `1/1024` over that duration; smaller values are discarded.
`downsample` is clamped to `1..=8`. The default `2` stores half-width, half-height
history; use `1` for finer details.

## Resources and coordinates

History uses two reusable `RGBA16Float` textures per active surface. Their sizes
follow the window's device-pixel dimensions divided by `downsample`. No CPU
readback is needed for accumulation or expiry.

Changing capture bounds, padding, device scale, window size or history resolution
clears retained pixels. Removing the stage from a rendered scene releases its
history textures. Device recovery also starts with empty history.

The effect retains pixels, not interactive elements. Hit testing and layout stay
with the current content. Parent clipping applies to the visible trail.

Linux Wayland/X11 and macOS Metal windows support feedback. Check
`window.supports_subtree_effects()` for availability. Unsupported backends paint
the current input directly.

## Example

```sh
cargo run -p gpui_effects --example feedback
```

Drag to draw light trails. Controls change the color and fade duration, pause or
clear history, and toggle Bloom.
