# Rendered-frame picking

[Headless output](headless.md) · [Frame readback](readback.md) · [GPU deformation](deformation.md)

`RenderedFrame::pick([x, y])` reads one physical pixel's Object ID and linear depth
without copying either full image or materializing CPU geometry. It selects the
nearest surviving rendered surface, including GPU-deformed meshes passed to
`HeadlessRenderer::render_with_geometry`.

## Request and completion

Render both `Scene3dChannels::OBJECT_ID` and `Scene3dChannels::LINEAR_DEPTH`. The
default output configuration does not include depth. Requests fail for missing
channels, out-of-bounds pixels, a lost device, or an already-pending readback.
Coordinates have a top-left origin and refer to the full source output. Convert
window or logical pointer positions to those physical pixels before requesting a
pick; no DPI, layout, or effect-coordinate conversion is implicit.

```rust
use gpui_3d::{FramePickReadback, RenderedFrame};

fn request(frame: &RenderedFrame, pixel: [u32; 2]) -> anyhow::Result<FramePickReadback> {
    frame.pick(pixel)
}

fn poll(pending: &mut FramePickReadback) -> anyhow::Result<()> {
    if let Some(result) = pending.try_read()? {
        match result.hit {
            Some(hit) => {
                let object = hit.object;
                let world_position = hit.world_position;
            }
            None => {}
        }
    }
    Ok(())
}
```

`try_read()` returns `None` while pending. A completed background query returns
`Some(FramePick { hit: None, .. })`. A surface result contains the original
`RenderObject`, camera-forward `linear_depth`, and pixel-center `world_position`.
The result also retains the full output `size`, queried `pixel`, and source
camera through `camera()`.

Unknown nonzero IDs, malformed samples, inconsistent background ID/depth,
nonfinite or invalid surface depths, and unrepresentable world positions are
errors. An object ID distinguishes a zero-depth orthographic surface from
background. Perspective surface depths must be positive. World reconstruction
uses the original camera's projection, lens shift, and aspect policy, not the
one-pixel copy dimensions.

## Ownership and limits

A request copies two aligned rows: 512 staging bytes and eight decoded channel
bytes, independent of output resolution. `memory()` reports this payload. Source
textures, retained identity metadata, object/result storage, and driver or
allocator overhead are additional memory.

Picks share the renderer's single pending-readback permit with full-frame and
regional reads, including requests from older frames. Invalid requests do not
consume it. Polling pumps callbacks without waiting; completion or failure is
terminal. Dropping a pending request cancels mapping, not submitted GPU work, and
the permit becomes available after outstanding callbacks finish.

The request retains the source camera and complete object mapping until
completion. It survives frame and renderer destruction, later renders, and
resizing. Numeric IDs are frame-local: resolve them using the request's mapping,
never a newer frame's. A returned node handle may have been removed from the live
graph. Keep application request generations or frame timestamps when deciding
whether an asynchronous result is still relevant.

## Coverage and interaction

Picking follows Object ID/depth coverage at the pixel center, including face
visibility and material alpha discard. It is not color-MSAA coverage or an
alpha-weighted choice: a low-opacity blended surface can be the nearest hit.
CPU `PickBehavior` and query filters do not change rendered IDs.

Results identify objects and world positions, not triangle indices, barycentric
coordinates, or surface UVs. They do not route captured-UI events or update CPU
BVHs. GPU overrides disable `Viewport3d` CPU hit handling.
Viewport event scheduling, selection state, and synchronization with the displayed
frame remain caller-owned. Use final mesh readback for CPU queries that require
deformed topology.
