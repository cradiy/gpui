# Animation playback

`AnimationPlayback` maps caller-provided elapsed time into an imported clip's
authored time range. It owns no clock, tracks, scene, executor or frame scheduler.
Use separate values for independent instances; cloning copies the current state.

```rust
use std::time::Duration;
use gpui_3d_gltf::{AnimationClip, AnimationPlayback};

fn position(clip: &AnimationClip) -> anyhow::Result<Duration> {
    let mut playback = AnimationPlayback::new(clip);
    playback.set_looping(true);
    playback.set_rate(0.5)?;
    playback.play();
    playback.advance(Duration::from_secs(3))?;
    Ok(playback.time())
}
```

`position()` is relative to the first key. `time()` adds the clip's authored start
and is the value to pass to transform and weight tracks. Use that same absolute
time for every channel in a pose before applying Morph and Skin.

## Controls

The initial state is paused at relative zero, with rate `1.0` and looping disabled.

- `play()` starts or resumes. At the terminal endpoint for the current direction,
  it restarts from the opposite endpoint. Repeated calls while playing do nothing.
- `pause()` retains the position. Elapsed time passed while paused is ignored.
- `seek(position)` pauses, clamps to the clip duration and sets a relative position.
- `set_rate(rate)` accepts finite nonzero values; negative rates play backward.
  Position and play/pause state remain unchanged. Use `pause()` rather than zero rate.
- `set_looping(enabled)` preserves the current position and play/pause state.

Without looping, advancement clamps at the directional endpoint and pauses.
With looping, advancement wraps into the half-open relative range `[0, duration)`;
elapsed time may cross multiple cycles. Explicit seeking can still select either
endpoint. A zero-duration clip remains paused at its single authored time.

## Advancement and sampling

`advance(elapsed)` returns whether the position changed. Inspect `is_playing()`
separately to decide whether another frame is needed. At a whole-cycle boundary,
position can remain unchanged while playback continues.

Elapsed time accumulates from the most recent effective control change. Non-unit
speed scales that total once before nanosecond rounding, instead of rounding each
frame independently. Unit forward/reverse speed uses exact `Duration` arithmetic.
Floating-point rates have finite precision. Elapsed or scaled-time overflow returns
an error without changing playback state. Invalid rate updates also leave state
unchanged.

Track sampling remains absolute and independent of playback history. For offline
evaluation or random-access rendering, sample tracks directly at the desired
authored time. For interactive playback, callers choose elapsed-time accounting,
background suspension, scheduling, clip selection, blending and publication policy.

The [model viewer](viewer.md) applies one selected clip to its instance and uses
the resulting deformed snapshot for rendering, bounds, cameras and picking.
