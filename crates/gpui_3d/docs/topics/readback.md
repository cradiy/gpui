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

## Frame identity

`RenderedFrame::frame_id()` and `Scene3dGpuOutput::frame_id()` identify one output
allocation with a `Scene3dFrameId`. Separate renders receive distinct identities
even when the scene, camera, size, object IDs, and cached resources are unchanged.
Clones of a token compare equal and can be used as map keys. Tokens are
process-local, do not order frames, and do not indicate GPU completion.

Full-frame and regional readback requests expose the same `frame_id()`. Completed
`ReadFrame`, `FramePick`, `FrameCoverage`, `FrameLabels`, and `RenderedLabels`
retain that identity. Compare it with the expected output before applying an
asynchronous result. An older result remains valid for its original frame.

Identity tokens retain no textures, scene geometry, or device. Manually creating
a token does not render anything. They describe provenance, not content hashes:
editing a `ReadFrame`'s public pixel buffers does not change its source identity.
Raw `Scene3dPixels` contain no provenance; when using lower-level regional reads,
retain `Scene3dReadback::frame_id()` alongside the returned pixels.

## Regions

`frame.gpu().readback_region(region, config)` copies a nonempty rectangle from
selected channels. `Scene3dReadbackRegion { origin, size }` uses top-left-origin
physical pixels. `region.validate(output_size)` checks containment without a
device. Invalid rectangles are rejected rather than clipped, including empty
sizes and overflowing extents.

Admission uses `config.memory(region.size)`, not full output dimensions. The
result's `Scene3dPixels::size` equals the rectangle size, and pixel coordinates
are local to that rectangle. `Scene3dReadback::region()` retains the original
source rectangle after completion. No camera is cropped or recomputed; use the
full source dimensions and add `region.origin` when reconstructing world points.

Regional reads share the same pending-request permit and cancellation behavior
as full-frame reads. All available channel formats are supported. For an ID/depth
query with retained object and camera metadata, use
[`RenderedFrame::pick`](picking.md).

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
