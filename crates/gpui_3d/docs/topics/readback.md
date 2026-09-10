# Frame readback

[Headless output](headless.md)

`RenderedFrame::readback()` reads every channel rendered into the frame.
`readback_with(config)` reads a nonempty subset and checks caller-supplied payload
budgets before acquiring the renderer's readback permit or allocating buffers.
The lower-level `Scene3dGpuOutput` provides the same methods.

```rust
use gpui_3d::{FrameReadback, RenderedFrame, Scene3dChannels, Scene3dReadbackConfig};

fn read_geometry(frame: &RenderedFrame) -> anyhow::Result<FrameReadback> {
    frame.readback_with(Scene3dReadbackConfig {
        channels: Scene3dChannels::OBJECT_ID | Scene3dChannels::LINEAR_DEPTH,
        max_staging_bytes: Some(32 * 1024 * 1024),
        max_cpu_bytes: Some(32 * 1024 * 1024),
    })
}
```

Selected channels must exist in the frame. Empty, unknown, or unavailable channel
selections return errors rather than producing partial selections. Unselected
textures remain on the GPU; they are not copied or mapped. No rerender occurs.
`Scene3dReadbackConfig::new(channels)` disables the optional payload limits with
`None`; device limits and the single-pending-request rule still apply.

## Memory admission

`config.memory(size)` computes a `Scene3dReadbackMemory` report and checks the
configured budgets without a device. It reports:

- `staging_bytes`: all selected GPU copy buffers, including 256-byte row alignment.
- `cpu_bytes`: the tightly packed decoded channel buffers.
- `max_buffer_bytes`: the largest individual aligned copy buffer.

Display color, Object ID, and linear depth each require four CPU bytes per pixel.
World normals require sixteen. Linear HDR color uses eight staging bytes per
pixel before row alignment, but sixteen CPU bytes because binary16 components
are widened to `f32`.

Budgets are per request, not global residency limits. They exclude source
textures, retained earlier results, object identities, allocator/driver overhead,
and application processing. During decoding, staging buffers and the growing set
of decoded channels coexist; allow for both payloads. Decoding writes directly
into the result buffers without an intermediate packed image. Allocation failure
returns an error without publishing a partial frame.

Actual admission also checks frame channel availability and the device's
per-buffer limit. `FrameReadback::memory()` and `Scene3dReadback::memory()` retain
the admitted report, including after completion or failure.

## Completion and ownership

Starting readback submits copies and requests asynchronous mapping without
waiting. `try_read()` polls callbacks and returns `None` until all selected
channels are ready, then returns the complete result once. Unselected CPU channel
fields are `None`. Dimensions, camera, and the original complete object identity
mapping remain associated with that frame. Integer IDs retain all 32 bits;
depth, normals, and HDR color bypass display conversion.

At most one readback may remain pending per renderer, including older retained
frames. Selecting fewer channels does not create another queue. Invalid admission
does not acquire this permit. Dropping a pending readback cancels mapping; the
permit is released after outstanding callbacks finish. Later start/poll calls
pump callbacks without blocking. Completion or failure is terminal for that
readback; retry through the source frame if needed.

The source frame and its GPU textures are unchanged. Retained CPU results are
independent of subsequent frames, requests, and renderer destruction. The caller
owns polling cadence and any worker execution; large CPU decoding work happens
inside `try_read()`, not in the GPU mapping callback.
