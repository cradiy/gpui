# Model viewer

The `viewer` example loads a local glTF or GLB asset into a GPUI viewport.

```sh
cargo run -p gpui_3d_gltf --example viewer -- /path/to/model.glb
cargo run -p gpui_3d_gltf --example viewer -- /path/to/model.gltf 1
cargo run -p gpui_3d_gltf --example viewer -- /path/to/model.glb 0 2
```

The optional second argument is a scene index. Without it, the file must declare
a default scene. A third argument selects one animation index. Without an animation
index the viewer displays the authored initial pose. `--help` prints usage without
opening a window.

## Navigation and inspection

The scene is framed after loading using the viewport's actual dimensions. The
viewport and inspector share a bounded layout; resizing does not change model
geometry or continuously reset the camera.

- Right-drag to orbit, middle-drag to pan, and scroll to change distance.
- **Frame all** fits the scene; **Frame selected** fits the clicked primitive.
- **Perspective / Ortho** changes the free camera's projection while preserving
  its target-plane scale.
- **Asset camera** selects the first imported camera. **Next camera** cycles
  through the remaining cameras and back to Orbit. Free-camera gestures are
  disabled while an asset camera is selected. Fixed authored aspect ratios fit
  inside the viewport with unused space instead of stretching the projection.
- Click a surface to see its original node, mesh, primitive and material indices,
  node/material names, base color, metallic/roughness factors and alpha settings.

The example uses imported node lights when present, otherwise the renderer's
default inspection source. Default ambient illumination is retained. It does not
edit the asset. See [punctual lights](lights.md) for supported parameters and limits.
Models use the same importer support and limits as [scene conversion](scenes.md).
An unavailable 3D backend is reported in the window.

## Animation

A selected clip is converted on the loading worker and starts paused at its first
authored key. **Play / Pause**, **Start**, **−0.25 s / +0.25 s**, **Once / Loop** and
the rate button control [playback time](playback.md). Rates cycle through 0.5×, 1×,
2× and reverse −1×. Seeking pauses playback. Losing window activation pauses it;
resuming requires Play.

The displayed position is clip-relative. Sampling adds the authored start time,
evaluates all TRS and weight channels, applies Morph before Skin, and publishes one
snapshot for meshes, bounds, cameras and picking. Node-track groups outside the
selected scene are skipped and counted in the controls. Invalid sampling pauses
playback, reports the error and retains the last successful frame and position.
Animation frames are requested while playing or awaiting GPU results. Camera
framing is explicit after initial loading; animation does not continuously reframe
the model.

## GPU deformation

Build with the native `wgpu` feature to enable the **CPU deformation / GPU
deformation** toggle:

```sh
cargo run -p gpui_3d_gltf --features wgpu --example viewer -- /path/to/model.glb 0 0
```

CPU mode evaluates final vertices on the CPU. GPU mode samples transforms and
weights on the CPU, then evaluates Morph and Skin through
[`GpuSceneDeformation`](gpu_deformation.md). It retains uploaded sources and packs
render vertices without reading them back to the CPU. Material coordinate sets
are bound independently of image loading.

GPU mode keeps one pending evaluation batch and the last complete display batch.
Each primitive's packed geometry is validated and its bounds are reduced on the
GPU, with fixed-size status and bounds readbacks. Geometry, material coordinate
selections, bounds, transforms, lights and cameras become visible together after
every primitive passes both checks. A failed preparation retains the previous
display batch and reports the error. The timeline shows the requested
sample; the displayed pose can lag while work completes. Before the first batch
is ready, the viewport shows a preparation message. **Frame all** and **Frame
selected** use bounds from the displayed batch.

Left-click selection reads ID/depth from the submitted viewport frame at the click
position, then maps its primitive back to imported node and material details.
Selection is asynchronous, with one pending request and one latest queued click.
Only the latest click updates selection. Background clicks clear selection;
pending or failed queries leave it unchanged. Superseded query errors are ignored.
Mode changes and successful reloads discard outstanding selection requests.
Captured-UI pointer routing is not enabled for GPU-deformed surfaces.

Unsupported direction generation, backend limits, and device changes are reported
in the window; GPU mode does not silently substitute CPU geometry. Assets requiring
tangent regeneration need device-enabled `SHADER_F64`. After device replacement,
disable and re-enable GPU deformation to rebuild its resources. Reloaded models
start in CPU mode.

The example uses default per-source deformation limits, a 256 MiB limit per render
source, a 256 MiB preparation limit per primitive, and a 256 MiB pick-target budget.
Render sources are reused only for matching node, texture-coordinate selections,
and mesh allocation. Replacement sources do not invalidate the displayed batch.
Preparation includes packed vertices, indirect/validation storage, and 96 bytes
for bounds reduction and both staging buffers. Existing sources and driver
overhead are excluded. These limits are not an aggregate GPU residency budget.

## Loading and resources

Parsing, resource reads, scene conversion and image decoding run on GPUI background
workers through a shared [load queue](load_queue.md). The queue allows two active
pipelines and two waiters. Final material resolution, instantiation and publication
run on the owner thread. [Load slots](load_slots.md) reject superseded results.

**Reload** reads the same path again. **Cancel** abandons the current request.
Failed or cancelled replacements leave the last successful model visible; errors
appear in the window. Synchronous file reads and decoding are not preempted by
cancellation, but their late results cannot replace a newer model.

The examples share bounded local-file reads and relative URI resolution. External
resources must resolve to regular files within the asset directory after
canonicalization. Parent-directory escapes, escaping symlinks, remote URLs, query
strings and fragments are rejected. This path policy is not a filesystem sandbox
against concurrent path replacement. Embedded resources use the importer's normal
validation. No network requests are made.

An [image cache](image_cache.md) reuses matching decoded content across reloads
under its default limits. Encoded resources are reread, so reloads do not depend
on path-only freshness keys. Resource, scene and image-decode admission use their
default limits independently of the cache capacity.

For command-line inspection and absolute-time animation evaluation without a
window, use the `inspect` example.
