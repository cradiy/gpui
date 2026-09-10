# Model viewer

The `viewer` example loads a local glTF or GLB asset into a GPUI viewport.

```sh
cargo run -p gpui_3d_gltf --example viewer -- /path/to/model.glb
cargo run -p gpui_3d_gltf --example viewer -- /path/to/model.gltf 1
```

The optional second argument is a scene index. Without it, the file must declare
a default scene. `--help` prints usage without opening a window.

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

The example uses the renderer's default inspection lighting. It displays authored
initial Morph/Skin deformation; it does not play animation clips or edit the asset.
Models use the same importer support and limits as [scene conversion](scenes.md).
An unavailable 3D backend is reported in the window.

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
