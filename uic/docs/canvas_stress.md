# Canvas workload

```sh
cargo run -p uic --example canvas_stress
```

The canvas contains 100 nodes with editable text, image thumbnails, anchored menus
and popovers. Use the wheel to zoom, middle-button drag to pan, and the toolbar to
add or remove nodes, toggle cached views and enable nested transform groups.
Click a node title for its menu or `＋` for its popover. Text fields support the
normal input method and selection interactions.

`Animate` continuously changes the transform. `Pause` returns to event-driven
drawing. `Print / reset stats` prints the current sample window and clears it.
The sample window retains at most 600 distinct completed frames. Values update
when the application draws; the diagnostics do not create an idle polling loop.

## Repeatable workload

```sh
cargo run -p uic --example canvas_stress -- --automated
cargo run -p uic --example canvas_stress -- --automated --no-cache
```

The automated workload runs warmup, pan, zoom, nested transforms, empty canvas
and restored canvas phases for 90 animation steps each, prints each phase's
statistics, then exits. Keep window size, display scale, GPU, build profile and
focus state the same when comparing runs. `--no-cache` disables node view caches;
renderer source-capture caching still operates. Use `--release` before `--` to
measure an optimized build. The automated path does not simulate clicks or IME.

Accessibility integration stays enabled by default. Cached views retain their
accessibility nodes and actions; activation or focus changes rebuild that data.
The panel and logs identify those misses. `--no-accessibility` disables
accessibility only in the example application. Use the same setting on both
runs when comparing measurements.

Check menus and popovers while zooming manually, edit text using an input method,
and repeat clear/restore while watching retained capture storage. The source
canvas is 1000 × 600 logical pixels and clips nodes outside its captured region;
zooming out cannot recover source content clipped before capture.

## Measurements

- **Build p50 / p95:** CPU wall time constructing the scene, including raster-budget retries.
- **Platform p50 / p95:** CPU wall time in the platform draw call, including command
  encoding, submission and any surface waits. This is not GPU execution time or FPS.
- **Views:** per-frame hit/miss decisions for explicitly cached views. Reusing a
  parent skips its descendants, so counts are not a percentage of all nodes.
  Logs also separate cold, accessibility, refresh, dirty and context rebuilds.
- **Captures:** actual source-capture reuse decisions and retained UI/scratch texture
  dimensions and storage estimates across nested WGPU renderers. A reused parent
  does not count its skipped child captures as hits.
- **Density:** each capture's local raster multiplier, after capture limits apply.
  Nested multipliers compose; device scale is additional. The panel lists up to
  four targets, while storage totals include all retained capture targets.

Capture estimates count uncompressed texels. They exclude atlas, path/MSAA,
backdrop, media, 3D, simulation, presentation buffers and driver overhead, and
must not be interpreted as process memory or total GPU memory. A zero capture
total after clearing the canvas only confirms release of those capture targets.

The window API is `set_frame_diagnostics_enabled(true)` followed by
`submitted_frame_diagnostics()` for the last scene passed to platform draw.
The panel retains that snapshot while a newer scene is awaiting platform draw.
`frame_diagnostics()` exposes the latest scene build, whose platform statistics
may still be pending. Tracking is disabled by default. Renderer details are
optional and currently supplied by Linux and Web WGPU; other platforms still
provide scene-build timing and view-cache counts. Measurements do not wait for
GPU completion. Opening Inspector bypasses view caching and changes the workload.
