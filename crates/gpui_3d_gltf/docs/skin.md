## Skeletal skins

`PreparedDocument::skin(index, options)` converts a skin's document-local joint
indices, optional skeleton/name, and inverse bind matrices into `SkinDefinition`.
Joint order is preserved. Missing inverse binds use identity; an explicit accessor
must contain at least one float MAT4 per joint. Only the first `joint_count`
matrices are retained. Matrices must be finite, affine, and invertible.

`SkinDefinition::bind(&geometry)` consumes the converted primitive's retained
influences from the same document and returns a core `Skin`. Normal/tangent
splitting preserves source correspondence. All source joint indices, including
unused vertices and zero-weight slots, must address the skin's joint array.
Weights are normalized across every set of each vertex by the core binding.

Scene conversion requires each referenced skin to have a mesh and every joint
to be reachable in the selected scene under a common root. An explicit skeleton
must be an ancestor of all its joints. Duplicate joints, missing attributes,
invalid references, and incompatible bindings produce contextual errors before
image decoding. Core failures while evaluating the initial pose are returned by
`resolve_images`/`decode_images`.

### Instances and pose evaluation

`SceneAsset::subtree()` contains geometry evaluated at the authored joint pose
and default Morph weights.
`SceneAsset::skins()` retains undeformed base meshes and shared bindings for
subsequent evaluation. Each `SceneSkin` identifies its original skin index,
source primitive handle, and source joint handles. `SceneNode::skin_index` and
`ScenePrimitive::skin_index` retain file-level associations.

`SceneSkin::evaluate(instance, poses)` maps the source handles into one instance
and evaluates its undeformed base mesh from that snapshot's world transforms.
It does not apply Morph targets. It returns a
primitive handle and a replacement local mesh without mutating the graph. The
mesh-node transform is canceled during skinning; rendering under that same
transform applies only the joint transforms to the final surface. Transforms on
a shared instance ancestor still move the whole rig.

```no_run
use gpui_3d::{EvaluatedScene, NodeHandle, Pose, SceneGraph, SubtreeInstance, TransformConstraint};
use gpui_3d_gltf::SceneAsset;

fn evaluate(
    graph: &SceneGraph,
    asset: &SceneAsset,
    instance: &SubtreeInstance,
    pose: &Pose,
    constraints: &[(NodeHandle, TransformConstraint)],
) -> anyhow::Result<EvaluatedScene> {
    let transforms = graph.evaluate_with_constraints(
        pose.transforms(), constraints.iter().copied(),
    )?;
    let replacements = asset.deform(instance, &transforms, &[])?;
    Ok(graph.evaluate_with_constraints_and_meshes(
        pose.transforms(), constraints.iter().copied(), replacements,
    )?)
}
```

Use the final evaluated snapshot for bounds, picking, color, shadow, and geometry
outputs. An empty constraint list uses the sampled local pose. Skinning and final
scene evaluation use the same constraint inputs. For multiple instances, compute
all replacements from the same pose snapshot before final evaluation. Graph mutations
between sampling and replacement require the caller to resample; missing or
foreign handles return errors.

`SceneAsset::deform` evaluates imported Morph targets before Skin, using authored
weights unless overridden. Always deform source geometry, not the previously
skinned result. For caller-owned Morph data, evaluate Morph first, then use
`binding().evaluate_world` with the same instance-space mapping. Evaluation is
synchronous CPU work; callers own scheduling and time selection.

### Limits

`SkinOptions::joint_limit` defaults to 65,536 joints per definition.
`GeometryOptions::influence_limit` bounds input and generated vertex slots.
`SceneOptions` applies aggregate joint/influence admission, with shared
definitions and unique `(skin, primitive)` bindings charged once. Initial skinned
vertices have a separate per-occurrence `deformed_vertex_limit`; repeated
primitives share bindings and source joint handles, not their deformed outputs.
These are
element limits, not process-memory or GPU budgets. Skin conversion performs no
filesystem, image decoding, or GPU operations.
