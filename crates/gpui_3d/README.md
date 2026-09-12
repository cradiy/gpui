# gpui_3d

Low-level 3D scenes, animation, queries, and rendering for GPUI applications.
Embed a depth-tested viewport in an ordinary layout, or render the same scene
without a native window. Geometry, evaluated poses, materials, and frame outputs
can be retained independently of the UI that presents them.

The core is format-independent. Importers such as
[`gpui_3d_gltf`](../gpui_3d_gltf/README.md) translate assets into its public
types. Editors, character systems, physics integration, and asset-management
workflows remain application-owned.

## Capabilities

- **Scenes and geometry:** hierarchical nodes, stable handles, shared instances,
  affine transforms, indexed meshes, primitives, normal/tangent generation,
  and fixed-topology updates.
- **Cameras and interaction:** perspective and orthographic projection, framing,
  Orbit/pan/dolly controls, CPU spatial queries, and rendered-frame ID/depth picking.
- **Materials and lighting:** metallic-roughness PBR, normal and occlusion maps,
  mipmaps and anisotropy, direct lights, directional shadows, HDR environments,
  and linear-color output.
- **Rendering extensions:** custom WGSL shading, retained parameters and textures,
  independent vertex streams, and additional mesh passes with configurable depth,
  blending, culling, and normal expansion.
- **Animation and deformation:** absolute-time tracks, pose and weight blending,
  constraints and IK, CPU/GPU Morph and Skin, external GPU results, and coupled
  geometry/bounds publication.
- **Output and composition:** GPUI viewport layout and effects, captured UI
  surfaces, headless color/HDR/depth/normal/ID textures, label maps, and bounded
  asynchronous readback.

## Start here

From the repository root:

```sh
cargo run -p gpui_3d --features wgpu --example scene
cargo run -p gpui_3d --features wgpu --example materials
cargo run -p gpui_3d --features wgpu --example headless -- /tmp/gpui-3d-outputs
```

The [example reference](docs/examples.md) covers the scene, materials, lighting,
UI, and headless executables and their controls. For existing model files, use
the [glTF/GLB viewer](../gpui_3d_gltf/docs/viewer.md).

For application integration, start with [3D viewports](docs/viewport.md) or
[headless rendering](docs/topics/headless.md). Both consume `Scene` values;
`SceneGraph` provides hierarchy editing and retained evaluated snapshots.

## Documentation

### Scenes and animation

| Topic | Content |
| --- | --- |
| [Geometry](docs/topics/geometry.md) | Coordinates, meshes, primitives, normals, UVs, and vertex updates. |
| [Scenes](docs/topics/scenes.md) | Hierarchy, identities, instances, cameras, and light nodes. |
| [Evaluation](docs/topics/evaluation.md) | Final poses, mesh replacement, and immutable scene snapshots. |
| [Cameras](docs/topics/camera.md) | Projection, optics, framing, controls, and damping. |
| [Animation](docs/topics/animation.md) | Tracks, pose blending, CPU Morph and Skin, and weight layers. |
| [Constraints](docs/topics/constraints.md) | Follow, Aim, joint limits, and IK chains. |
| [GPU deformation](docs/topics/deformation.md) | Morph/Skin, generated directions, packing, bounds, and readback. |
| [External deformation](docs/topics/external_deformation.md) | Application-produced GPU vertices, ordering, and ownership. |

### Materials and rendering

| Topic | Content |
| --- | --- |
| [Materials](docs/topics/materials.md) | PBR, transparency, texture sampling, tangent frames, and color output. |
| [Lighting](docs/topics/lighting.md) | Direct lights, shadows, HDR backgrounds, and environment lighting. |
| [Material programs](docs/topics/material_programs.md) | WGSL surface/shading functions and standard renderer inputs. |
| [Material bindings](docs/topics/material_bindings.md) | Uniforms, textures, samplers, snapshots, and device ownership. |
| [Custom attributes](docs/topics/material_attributes.md) | Typed vertex streams and interpolation for application shaders. |
| [UV and color streams](docs/topics/vertex_streams.md) | Independent CPU/GPU attribute updates for deformed geometry. |
| [Additional mesh passes](docs/topics/mesh_passes.md) | Outline-capable draws, raster state, and coverage ownership. |
| [Object submissions](docs/topics/submissions.md) | Coherent transform, geometry, material, and bounds updates. |
| [Rendering and resources](docs/topics/rendering.md) | Preparation, batching, quality, effects, caches, and measurements. |

### Interaction and output

| Topic | Content |
| --- | --- |
| [Viewport integration](docs/viewport.md) | Layout, scheduling, and window backend support. |
| [UI textures](docs/topics/ui.md) | Captured UI sizing, pointer routing, and interaction limits. |
| [Spatial queries](docs/topics/queries.md) | Rays, bounds, filtering, BVHs, and CPU picking. |
| [Rendered-frame picking](docs/topics/picking.md) | Asynchronous selection from retained ID/depth outputs. |
| [Viewport capture](docs/topics/viewport_picking.md) | Submitted-frame identity and viewport-local pixel queries. |
| [Headless output](docs/topics/headless.md) | Output channels, coverage rules, GPU ownership, and limits. |
| [Readback](docs/topics/readback.md) | Channel selection, regions, completion, and memory admission. |
| [Labels](docs/topics/labels.md) / [GPU labels](docs/topics/gpu_labels.md) | Application-defined segmentation from object IDs. |

## Features and support

Scene data, CPU geometry/animation evaluation, and ordinary viewport APIs do not
require the crate's `wgpu` feature. Enable it for native GPU deformation, custom
material bindings, viewport ID/depth capture, and `HeadlessRenderer`.

Embedded mesh rendering is implemented by the Linux WGPU backend. Query
`Window::scene3d_support()` for the current window rather than inferring support
from the operating system or enabled Cargo features. Direct rendering has its
own device/format checks through `Scene3dDeviceCapabilities`.

Some GPU direction-generation stages require device-enabled `SHADER_F64`.
Capability checks and resource budgets apply before construction or submission;
GPU failures remain explicit. See [deformation admission](docs/topics/deformation.md#resource-admission).

Captured UI supports one live source and one interactive surface per viewport.
Text input, IME positioning, menus, and focus management on 3D surfaces are not
part of pointer routing. Blended surfaces use object-level sorting rather than
order-independent transparency. See the corresponding topic pages for contracts
and [TODO](TODO.md) for planned work and outstanding validation.
