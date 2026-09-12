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
call `poses.with_meshes(meshes)`. Use the returned snapshot for rendering and
queries. For multiple instances, compute their deformations against the same
pose snapshot and combine the replacement lists.

With Follow or Aim constraints, evaluate the joint pose using
`evaluate_with_constraints`, then compute deformation and call `with_meshes` on
that constrained snapshot. World transforms and constraint outcomes are retained
without running the solver again. If transforms, constraints, and replacement
meshes are already available together, the graph also accepts them through
`evaluate_with_constraints_and_meshes(transforms, constraints, meshes)`.
See [Constraints](constraints.md) for dependencies and errors.

## Mesh replacement on a snapshot

`EvaluatedScene::with_meshes` takes mesh-local replacements and returns an
independent snapshot without requiring the original graph. It preserves world
transforms, parent relationships, visibility, material and shadow settings,
cameras, lights, identities, constraint outcomes, and the source graph revision.
Omitted nodes retain their current snapshot geometry, not authored defaults.

Visible bounds, per-node bounds, descendant bounds, and spatial-query inputs
follow the replacements. Hidden mesh nodes accept updates and contribute to
descendant bounds without becoming visible or queryable. Resources remain shared;
replacement topology and attributes are retained without copying mesh buffers.
Preparation identity is refreshed and the new spatial index starts unprepared;
`prepare_spatial_index_from(&poses)` can refit a prepared earlier index. Empty
replacement input returns a clone sharing preparation and spatial-query identity.

Targets are validated against the snapshot: a node removed from the live graph
can still be updated in an older snapshot that contains it. Absent handles,
non-mesh nodes, duplicates, and unrepresentable transformed bounds return
`SceneError`. Earlier snapshots remain unchanged on success or failure. Repeated
replacements act on the receiver's current geometry and may be chained; retain
an authored or earlier snapshot when a reset is needed.

This operation performs no animation sampling, constraint solving, world-transform
evaluation, or GPU upload. It traverses snapshot metadata to refresh bounds and
query inputs; it is not a constant-time geometry swap.

The caller owns sampling, deformation, input resource limits, and scheduling.
Evaluation performs synchronous CPU work; it does not run a clock, upload meshes,
or retain prior samples as defaults. See [Animation](animation.md) for tracks,
pose layers, Morph and Skin, and [Scenes](scenes.md) for hierarchy ownership.
