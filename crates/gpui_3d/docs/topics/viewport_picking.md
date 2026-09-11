# Viewport ID and depth capture

[Rendered-frame picking](picking.md) · [GPU deformation](deformation.md)

`Viewport3d::pick_capture` binds a retained `ViewportPickCapture` to a viewport.
Use one handle per viewport and reuse it across renders. It supports CPU and
GPU-deformed geometry without using CPU mesh hits.

```rust
use gpui_3d::{Scene, Viewport3d, ViewportPickCapture, viewport3d};

fn view(scene: Scene, picks: &ViewportPickCapture) -> Viewport3d {
    viewport3d("model", scene).pick_capture(picks.clone())
}
```

Create the handle with an explicit target payload budget, for example
`ViewportPickCapture::new(64 * 1024 * 1024)`. Call `picks.pick(position)` from a
pointer handler, then poll the returned request with `try_read()`. Positions use
logical viewport input coordinates. No polling or redraw scheduling is implicit.
The capture, retained frames, and requests are UI-thread-owned.

`pick()` returns `Ok(None)` when no matching submitted frame exists or the
position lies outside the viewport/render surface. Capture and readback failures
return errors. `try_read()` returns `Ok(None)` while pending; a completed
background query has `result.frame.hit == None`. Surface results contain the
source `RenderObject`, linear depth, and world position. `result.position` and
`bounds()` retain the logical query position and source layout.

`picks.frame()` retains a `ViewportPickFrame` for a delayed query. Frame/camera,
object mapping, layout, DPI, and render-surface dimensions are paired at paint;
layout changes require a matching backend submission. Pointer conversion uses the
renderer’s snapped physical viewport bounds. Effective camera aspect is retained
independently of raster rounding. Removing the element's retained state
expires the current binding; `clear()` releases it explicitly. Retained frames
and in-flight queries remain usable. Completed results use weak freshness tokens
and do not retain the GPU textures themselves.

`picks.is_current(&result)` requires the same live binding and backend submission.
It becomes false after replacement, including a repaint with new UI texture
pixels. Applications may accept an older click-time result while animation runs;
resolve its original object identity and reject superseded requests. The scene
example uses one pending request and retains only the latest queued click.

Frames, pending requests, and completed picks expose `frame_id()`. This identifies
the submitted ID/depth output, not merely the prepared scene or its camera.
`picks.is_current_frame(request.frame_id())` checks whether that output remains
current before readback finishes. It returns false for removed, failed, cleared,
or not-yet-matched bindings. Tokens retain no GPU textures. See
[frame identity](readback.md#frame-identity).

## Regional queries

`ViewportPickFrame::readback_region(region, config)` reads a rectangle of ID/depth
pixels and retains the capture's camera, projection, object mapping, and frame ID.
The rectangle uses capture-texture coordinates within `frame.size()`, not logical
window coordinates. `frame.pixel_at(position)` converts a logical input position
without reading the GPU; out-of-bounds or clipped positions return `None`.
Only the ID and linear-depth channels are available. The
returned `FrameReadback` shares the capture's pending-readback permit and supports
the same [regional queries](readback.md#regions) as headless output. Use
`picks.is_current_frame(request.frame_id())` when freshness is required.

## Backend capture

The WGPU viewport renderer publishes paired Object ID and camera-forward depth
textures through `gpui::Scene3dFrame::pick_capture`. Both passes use the same frame,
GPU geometry, UI texture, and raster projection as the viewport's color submission.
They use single-sample pixel-center coverage, independently of color MSAA.

## Publication

Attach a `gpui::Scene3dPickCapture::new(max_bytes)` to the frame before sharing it.
Use a distinct capture for each simultaneous viewport. Read the backend result
with `capture.read::<gpui_wgpu::WgpuScene3dPickFrame>()`.

- `None` means no result has been published.
- `Some(Ok(output))` retains the submitted ID/depth textures.
- `Some(Err(error))` reports capture admission, preparation, or scene encoding failure.

Publication follows renderer-owned queue submission, not GPU completion or display
presentation. Window drawing and `WgpuRenderer::draw_external` publish successful
outputs; `encode_external` and direct headless rendering do not. Failed capture
admission does not disable the color viewport. Failed scene encoding publishes an
error to its captures, including nested UI scenes, without revoking retained
outputs. A successful renderer-owned resubmission replaces the error. Unsubmitted
commands never publish a new successful output.

Instance storage exhaustion can request an encoding retry within the device's
buffer limit. Unsupported resources and scene preparation errors do not grow
instance storage. A failed draw submits no frame commands or successful pick output.

`output.matches_frame(&frame)` checks the original immutable frame allocation.
Retain the source camera, object-ID mapping, and layout alongside that frame.
Frame identity alone does not establish current layout, visibility, or application
request validity. Results precede outer clipping, opacity, overlays, and subtree
effects; callers must account for those when accepting a pointer query.

## Coordinates and readback

`output.pixel_at([u, v])` maps normalized full-viewport coordinates to a physical
pixel. The normalized domain is half-open `[0, 1)`. Nonfinite, out-of-domain, and
surface-clipped positions return `None`. Convert logical pointer positions using
the matching viewport bounds first; apply inverse outer effect mappings separately.
`pixel_at_surface([x, y])` accepts physical source-surface coordinates directly and
accounts for snapped viewport bounds. `ViewportPickCapture` uses this path with
the retained logical-to-physical scale.

The output texture size includes surface clipping and actual raster density.
`projection_rect()` gives the full viewport projection rectangle within those
textures. For world reconstruction, compute viewport UV from the pixel center as
`(pixel + 0.5 - rect.origin) / rect.size`, not by dividing by texture dimensions.

`output.gpu().readback_region(...)` supports a one-pixel ID/depth copy with 512
staging bytes and eight decoded bytes. Readback is nonblocking and uses the same
completion, cancellation, and device-loss contract as [frame readback](readback.md).
Captures from one viewport renderer share a pending-readback permit, including
retained older outputs. Independently rendered UI textures have separate renderers.

## Resources and interaction

Each preparation allocates fresh output textures. Consumers can retain an earlier
result through later renders, resizing, capture errors, and renderer destruction.
The backend retains a weak source-frame identity to avoid a frame/capture cycle.

`max_bytes` admits output and depth-attachment payload before allocation: 16 bytes
per raster pixel for the paired passes. It excludes older retained outputs,
transient overlap, readback staging, geometry, images, pipelines, color-rendering
resources, and driver overhead. Zero rejects capture allocation. Frames require
unique nonzero object IDs and supported data-output formats.

The backend interface exposes raw data. `ViewportPickCapture` pairs it with scene
identities and camera reconstruction. Neither interface installs event handlers
or routes captured-UI input. GPU geometry overrides continue to disable CPU hit
handling. Apply outer effect-coordinate mappings and UI hit eligibility before
requesting a pick.
