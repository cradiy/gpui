# Scene evaluation

`SceneGraph::evaluate_with_overrides(transforms, meshes)` creates an immutable
snapshot from replacement local transforms and mesh resources. Omitted nodes
use their authored values. The graph and its revision remain unchanged on both
success and failure; earlier snapshots remain usable.

```rust
use gpui_3d::{AffineTransform, EvaluatedScene, Mesh, NodeHandle, SceneError, SceneGraph};

fn sample(
    graph: &SceneGraph,
    transforms: Vec<(NodeHandle, AffineTransform)>,
    meshes: Vec<(NodeHandle, Mesh)>,
) -> Result<EvaluatedScene, SceneError> {
    graph.evaluate_with_overrides(transforms, meshes)
}
```

Transform replacements are local to the authored parent. Descendants inherit
the evaluated parent transform. Mesh replacements use mesh-local coordinates
and must target nodes that already have geometry. They retain the node's
material, identities, visibility, picking behavior, camera, light, and shadow
settings. Replacement meshes retain their own indices and vertex attributes;
they need not share topology with the authored mesh.

Rendering, world and subtree bounds, and spatial queries all use the replacement
geometry. Hidden mesh nodes retain evaluated bounds without producing render
objects. `prepare_spatial_index_from` can refit an earlier snapshot's index using
the new geometry and world transforms. Preparation caches distinguish snapshots
even when their source graph revisions are equal.

Duplicate handles within either input return `DuplicateTransform` or
`DuplicateMesh`. The same node may occur once in each input. Foreign or expired
handles return `InvalidHandle`; mesh replacements for non-mesh nodes return
`NoMesh`. Invalid composed transforms or transformed bounds also fail evaluation.
Material compatibility is checked during scene preparation.

## Deformation inputs

For Morph and Skin, sample local transforms first and evaluate the joint pose
with `evaluate_with_transforms`. Compute mesh replacements from that pose, then
pass the same local transforms and the resulting meshes to
`evaluate_with_overrides`. Use the final snapshot for rendering and queries.
For multiple instances, gather their transforms and mesh replacements before
evaluating the final scene.

The caller owns sampling, deformation, input resource limits, and scheduling.
Evaluation performs synchronous CPU work; it does not run a clock, upload meshes,
or retain prior samples as defaults. See [Animation](animation.md) for tracks,
pose layers, Morph and Skin, and [Scenes](scenes.md) for hierarchy ownership.
