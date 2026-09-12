# gpui_3d_gltf

glTF 2.0 and GLB import for [`gpui_3d`](../gpui_3d/README.md), with explicit
resource loading, shared model assets, instance mappings, and animation data.
Parsing and scene conversion do not require a window or GPU.

## Capabilities

- Indexed triangle geometry, all authored UV sets, vertex colors, and generated
  normals and MikkTSpace tangents with preserved vertex correspondence.
- Metallic-roughness and unlit materials, image sampling, texture transforms,
  cameras, and punctual lights.
- Scene hierarchy, reusable assets, independent instances, and mappings back to
  original node, mesh, primitive, and material indices.
- Absolute-time TRS and Morph-weight animation, Skin bindings, CPU deformation,
  and GPU Morph/direction-generation/Skin evaluation.
- Bounded resource and image loading, caches, asynchronous load queues, cancellation,
  and replacement slots that retain the previous successful asset.

The importer rejects unsupported required extensions and exposes diagnostics for
ignored optional extensions. URI policy, scheduling, application identities,
playback, and asset catalogs belong to the caller. The loading and playback
helpers are optional building blocks, not a global asset manager.

## Try a model

Run these commands from the repository root with a local asset path:

```sh
cargo run -p gpui_3d_gltf --features wgpu --example viewer -- /path/to/model.glb
cargo run -p gpui_3d_gltf --features wgpu --example viewer -- /path/to/model.glb 0 0
```

The optional arguments select a scene and then an animation. Without a scene
index, the document must declare a default scene. The viewer includes camera
controls, material inspection, playback, and a CPU/GPU deformation toggle.
See [viewer controls and limits](docs/viewer.md).

Inspect metadata and sample animation without opening a window:

```sh
cargo run -p gpui_3d_gltf --example inspect -- /path/to/model.glb
cargo run -p gpui_3d_gltf --example inspect -- /path/to/model.glb --scene 0 --animation 0 --time 0 --time 1 --time 0
```

The inspector evaluates poses and CPU geometry; it does not validate GPU rendering.
Both examples accept `--help`. Their local resolver confines external resources
to the asset directory; the library itself delegates external URI resolution to
the application.

## Loading an asset

1. Parse bytes with `Document::from_slice` and inspect its diagnostics.
2. Resolve encoded buffers and images with `prepare` or `prepare_async`.
3. Convert a selected scene with `PreparedDocument::scene`.
4. Decode images or supply an image resolver to produce a `SceneAsset`.
5. Instantiate the asset into a core `SceneGraph`, evaluate a pose, and pass its
   scene to a viewport or headless renderer.

Animation clips are converted separately and bound to instances from the same
document. Sample transforms and weights, evaluate the hierarchy, and apply Morph
before Skin. The [instance](docs/instances.md), [animation](docs/animation.md),
and [GPU deformation](docs/gpu_deformation.md) references cover these interfaces.

## Documentation

### Import and scene data

| Topic | Content |
| --- | --- |
| [Resources](docs/resources.md) | Parsing, URI resolution, limits, and command-line inspection. |
| [Diagnostics](docs/diagnostics.md) | Optional extensions, source locations, and import failures. |
| [Geometry](docs/geometry.md) | Attributes, topology, normal/tangent generation, and limits. |
| [Quantization](docs/quantization.md) | Integer attribute decoding and coordinate conventions. |
| [Materials](docs/materials.md) | Factors, texture semantics, sampling, and ownership. |
| [Images](docs/images.md) | Decoding, pixel layout, budgets, and scheduling. |
| [Scenes](docs/scenes.md) | Hierarchy, cameras, supported content, and source identities. |
| [Lights](docs/lights.md) | Punctual light conversion and renderer limits. |
| [Instances](docs/instances.md) | Reusable assets, source mappings, and material overrides. |

### Animation and loading

| Topic | Content |
| --- | --- |
| [Animation](docs/animation.md) | Clip conversion, instance binding, and absolute-time samples. |
| [Playback](docs/playback.md) | Caller-advanced time, looping, seeking, and signed rates. |
| [Morph](docs/morph.md) | Target attributes, authored weights, and generated directions. |
| [Skin](docs/skin.md) | Joint bindings, pose mapping, and Morph-before-Skin evaluation. |
| [GPU deformation](docs/gpu_deformation.md) | Source admission, evaluation, rendering, and query synchronization. |
| [Asynchronous loading](docs/loading.md) | Scheduling, byte limits, cancellation, and retry. |
| [Resource cache](docs/cache.md) / [Image cache](docs/image_cache.md) | Encoded and decoded resource identity and residency. |
| [Load queue](docs/load_queue.md) / [Load slots](docs/load_slots.md) | Bounded work and publication of replacement assets. |
| [Model viewer](docs/viewer.md) | Camera controls, inspection, animation, GPU mode, and reload. |

## Features and support

Enable `wgpu` for native `GpuSceneDeformation`; ordinary import and CPU evaluation
do not use it. GPU evaluation checks the selected asset's required stages and
device limits, including `SHADER_F64` for generated tangents. It does not silently
fall back to CPU evaluation.

Viewport and headless rendering use the
[core renderer's support and output contracts](../gpui_3d/README.md#features-and-support).
The viewer currently targets Linux WGPU. Application-specific formats, retargeting,
character controllers, and physics are outside this importer.
