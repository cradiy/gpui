# GPU deformation

The native `wgpu` feature exposes `GpuSceneDeformation`, a retained adapter for
imported Morph and Skin data. It uses a caller-owned `WgpuContext` and submits
immutable core `GpuDeformationOutput` buffers. Vertex evaluation stays on the GPU;
resource loading, animation sampling, render publication, and CPU queries remain
caller-owned.

```toml
gpui_3d_gltf = { path = "../gpui_3d_gltf", features = ["wgpu"] }
```

## Preparation

`GpuSceneDeformation::check_asset(&asset)` checks direction-regeneration policies
without creating a device or allocating GPU resources. `new(context, &asset,
limits)` performs this check before any upload, then prepares each deformable
primitive in scene order. Static primitives are omitted. The adapter retains
geometry, authored weights, and skin bindings, not materials or decoded images.
It can evaluate multiple instances of the same asset.

Supported inputs include authored normal/tangent deltas, flat normal
reconstruction for imported triangle-corner geometry, Skin without Morph, and
Morph followed by Skin. MikkTSpace tangent regeneration is unsupported and causes
preparation to fail, even when authored weights are zero. Use
[`SceneAsset::deform`](morph.md#scene-weights-and-deformation) when CPU direction
generation is required; there is no automatic fallback.

`GpuDeformationLimits` applies to each core source and each result. It is not an
aggregate scene budget. Repeated primitive occurrences have independent GPU
sources; instances evaluated through one adapter reuse those sources. Outputs,
intermediate buffers, palettes, render packing, and pending readbacks also consume
memory. Bound outstanding evaluations and retained results in the caller.

## Evaluation

```no_run
use gpui_3d::{
    EvaluatedScene, GpuDeformationLimits, GpuDeformationOutput, NodeHandle,
    SubtreeInstance, WgpuContext,
};
use gpui_3d_gltf::{GpuSceneDeformation, SceneAsset};

fn prepare(
    context: WgpuContext,
    asset: &SceneAsset,
) -> anyhow::Result<GpuSceneDeformation> {
    GpuSceneDeformation::new(context, asset, GpuDeformationLimits::default())
}

fn sample(
    gpu: &GpuSceneDeformation,
    instance: &SubtreeInstance,
    poses: &EvaluatedScene,
    weights: &[(NodeHandle, Vec<f32>)],
) -> anyhow::Result<Vec<(NodeHandle, GpuDeformationOutput)>> {
    gpu.evaluate(instance, poses, weights)
}
```

`poses` contains final world transforms, including animation, constraints, or
caller-supplied pose overrides. Weight overrides use original glTF node handles
mapped through `SubtreeInstance::node`, not primitive-child handles. One override
applies to every primitive of its node. Omitted overrides use authored defaults
on every call. Weights are finite, signed, and must match the target count.
Duplicate, unknown, foreign-instance, and missing snapshot targets return errors.

Evaluation starts from bind-space geometry, applies Morph, rebuilds flat normals
for nonzero-weight samples when required, and then applies Skin. All-zero Morph
samples retain the base directions. Skin uses instance-mapped joints in binding
order and applies mesh-world cancellation and inverse binds once. No previous
sample is used as input.

Returned pairs follow deformable primitive order and contain mapped primitive
handles. Earlier outputs remain valid after another evaluation or destruction of
the adapter. CPU graphs and snapshots are never changed. An error may occur after
work has been submitted for earlier primitives; their outputs are not published.
Successful submission does not prove valid shader arithmetic: core vertex status
is checked during render packing or explicit readback.

## Rendering and queries

Each output's `base_mesh()` is its GPU source identity. The imported scene's
initial mesh may already contain authored Morph and Skin deformation and must not
be substituted for this source when binding GPU draws.

```no_run
# use gpui_3d::{EvaluatedScene, GpuDeformationOutput, NodeHandle};
# fn render_pose(poses: &EvaluatedScene, outputs: &[(NodeHandle, GpuDeformationOutput)])
#     -> anyhow::Result<EvaluatedScene> {
let render_pose = poses.with_meshes(
    outputs.iter().map(|(handle, output)| (*handle, output.base_mesh().clone())),
)?;
# Ok(render_pose)
# }
```

Use `Scene::geometry_inputs()` to obtain each object's node, source mesh, output ID,
and five active texture-coordinate selections before image resolution. Create render
sources with those selections, pack each result with `render_geometry`, and bind
the packed geometry to its scene object. Output IDs are scene object indices plus
one, not indices in the adapter's result vector. Supply conservative mesh-local
bounds for the same deformation sample, or obtain them through
`GpuDeformationBounds`. See
[core GPU deformation](../../gpui_3d/docs/topics/deformation.md#render-vertex-packing)
for packing, headless rendering, and viewport binding.

The replacement snapshot above contains source meshes, not final CPU geometry.
Its CPU bounds and picking do not describe deformed surfaces. GPU viewport
overrides disable CPU picking and captured-UI pointer routing. Use explicit mesh
readback to build a CPU-query snapshot when needed; no selection synchronization
is performed by this adapter. The [model viewer](viewer.md#gpu-deformation) combines
this adapter with GPU bounds and submitted-frame ID/depth selection.

Use the window's shared context for viewport rendering and the renderer's context
for headless output. Sources and results belong to that device. Rebuild the
adapter and render sources after device replacement; cross-device reuse is
rejected.
