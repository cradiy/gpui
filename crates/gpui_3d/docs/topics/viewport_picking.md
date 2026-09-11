# Viewport ID and depth capture

[Rendered-frame picking](picking.md) · [GPU deformation](deformation.md)

The WGPU viewport renderer can publish paired Object ID and camera-forward depth
textures through `gpui::Scene3dFrame::pick_capture`. Both passes use the same frame,
GPU geometry, UI texture, and raster projection as the viewport's color submission.
They use single-sample pixel-center coverage, independently of color MSAA.

## Publication

Attach a `gpui::Scene3dPickCapture::new(max_bytes)` to the frame before sharing it.
Use a distinct capture for each simultaneous viewport. Read the backend result
with `capture.read::<gpui_wgpu::WgpuScene3dPickFrame>()`.

- `None` means no result has been published.
- `Some(Ok(output))` retains the submitted ID/depth textures.
- `Some(Err(error))` reports capture admission or preparation failure.

Publication follows renderer-owned queue submission, not GPU completion or display
presentation. Window drawing and `WgpuRenderer::draw_external` publish results;
`encode_external` and direct headless rendering do not. Failed capture admission
does not disable the color viewport. An abandoned encoding does not replace the
last submitted result with a new successful output.

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

This interface exposes backend data. It does not install `Viewport3d` event
handlers, resolve IDs to scene nodes, or route captured-UI input. GPU geometry
overrides continue to disable CPU hit handling.
