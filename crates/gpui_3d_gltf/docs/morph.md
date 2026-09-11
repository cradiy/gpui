## Morph geometry

`PrimitiveGeometry::morph()` returns optional shared `MorphGeometry`. Each target
retains float VEC3 position, normal, and tangent deltas mapped to the converted
mesh's vertex order. Tangent deltas contain XYZ only; authored handedness is
preserved. Weights are finite, signed, and neither clamped nor normalized.

Inputs may use float accessors or the signed integer formats defined by
[`KHR_mesh_quantization`](quantization.md). Retained deltas are floating-point
values in the same coordinate space as the decoded base geometry.

`evaluate(weights)` returns a mesh without changing its inputs. Weight count
must equal target count. Zero weights share the base mesh; other results retain
its index storage. Evaluation starts from the base on every call, independently
of previous samples. Clones share target arrays and base geometry.

Missing normals are generated as flat face normals from the weighted geometry.
Authored tangents and tangent deltas are ignored when base normals are absent.
When tangent generation is requested, MikkTSpace uses the weighted positions,
normals, and selected UVs with the same repair policy as base-mesh conversion.
All coordinate sets and the selected tangent-set identifier remain unchanged
across samples.
These generation policies use fixed triangle-corner vertices, preserving
correspondence with skin influences across all samples. Base-mesh repair reports
do not describe subsequent Morph samples. Flat normal generation still requires
nonzero geometric area. Invalid normals, incompatible tangent handedness, or
unrepresentable results return errors without changing topology or dropping
triangles.

### Attribute inputs

`attribute_targets()` exposes the shared core `MorphTargets` before direction
regeneration. Cloning it retains the converted vertex order and target arrays.
`regenerates_normals()` and `regenerates_tangents()` identify the additional work
required for nonzero-weight samples.

When both flags are false, these inputs can be evaluated directly with core
`MorphTargets` or uploaded to `GpuMorph` with the `gpui_3d/wgpu` feature. GPU
evaluation uses floating-point shader arithmetic; it is not bit-identical to
CPU evaluation. When either flag is true, attribute evaluation alone does not
produce the complete imported surface: flat normals or MikkTSpace tangents must
also be regenerated before Skin and rendering. `MorphGeometry::evaluate` performs
that work on the CPU.

`GpuFlatNormals` can rebuild the flat normals of imported triangle-corner inputs
without changing their vertex order. It does not replace MikkTSpace tangent
generation; assets with `regenerates_tangents()` still require that stage.

[`GpuSceneDeformation`](gpu_deformation.md) composes imported Morph, flat normal
reconstruction, and Skin with the native `wgpu` feature. It preserves authored
defaults and instance mappings, and rejects MikkTSpace regeneration requirements
before uploading resources.

Generated tangents are absent from `attribute_targets().base_mesh()`, even when
`MorphGeometry::base_mesh()` contains them. For zero weights, `evaluate` returns
the complete base mesh, including its generated directions.

### Scene weights and deformation

`SceneAsset::morphs()` associates each morphable primitive with its source node,
geometry, and authored weights. Node weights take precedence over mesh weights;
otherwise weights are zero. Scene conversion initializes meshes with those
weights before applying any skin binding.

`SceneAsset::deform(instance, poses, weights)` returns mesh replacements in scene
primitive order. It applies Morph before Skin using one instance's mapped node
handles and final world transforms. Overrides address the original glTF **node**
mapped into the instance, not its primitive children. One override applies to all
primitives belonging to that node. Missing overrides use authored defaults, not
the last sampled weights. Unknown nodes, duplicate overrides, missing snapshot
nodes, and invalid values return errors without mutating the graph.

```no_run
use gpui_3d::{AffineTransform, EvaluatedScene, NodeHandle, SceneGraph, SubtreeInstance};
use gpui_3d_gltf::SceneAsset;

fn deform(
    graph: &SceneGraph,
    asset: &SceneAsset,
    instance: &SubtreeInstance,
    transforms: &[(NodeHandle, AffineTransform)],
    weights: &[(NodeHandle, Vec<f32>)],
) -> anyhow::Result<EvaluatedScene> {
    let poses = graph.evaluate_with_transforms(transforms.iter().copied())?;
    let replacements = asset.deform(instance, &poses, weights)?;
    Ok(graph.evaluate_with_overrides(transforms.iter().copied(), replacements)?)
}
```

Use the final snapshot for rendering, bounds, and picking. For several instances,
compute their poses and replacements before evaluating the final scene. The
authored graph remains unchanged. Graph edits between pose sampling and final
evaluation require resampling. See
[Scene evaluation](../../gpui_3d/docs/topics/evaluation.md) for override semantics.

### Validation and limits

Each target attribute requires its corresponding base attribute and matching
accessor count. Position targets require finite, ordered VEC3 min/max metadata;
computed mesh bounds use actual positions. All source deltas must be finite,
including unused vertices and ignored tangent attributes. All primitives of a
mesh must have equal target counts, and authored weights must match that count.
Morph UVs, colors, and custom attributes are unsupported and produce conversion
errors even when supported attributes occur in the same target.

`GeometryOptions` bounds targets and input/retained attribute elements.
`SceneOptions` additionally bounds unique target data and initial deformed output
vertices per occurrence. Repeated instances share target arrays, not their
evaluated geometry. Limits are element counts, not exact process-memory budgets.
Evaluation is synchronous CPU work; scheduling, playback, and GPU upload remain
caller-owned.
