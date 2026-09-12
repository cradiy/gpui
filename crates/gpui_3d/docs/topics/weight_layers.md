# Weight layers

`WeightPose` stores immutable weight arrays indexed by `NodeHandle`, in insertion
order. Each array is nonempty and finite. Values may be negative or greater than
one; they are never normalized or clamped. Different nodes may have different
array lengths. An empty collection is valid, and clones share storage.

Use `get(node)` for one array, `weights()` for borrowed deformation inputs, or
`into_weights()` to take the arrays. Taking shared storage copies its arrays;
taking uniquely owned storage does not.

## Override and additive layers

`base.blend(target, weight, mask)` applies a sparse override layer:

```text
t = weight * mask.weight(node)
result = base * (1 - t) + target * t
```

Without a mask, `t` equals `weight`. The blend weight and `PoseMask` values must be
finite and in `[0, 1]`. Every target and explicit mask node must exist in the base,
including at zero weight. Target arrays must match their base array lengths.
Omitted targets retain base values. Output nodes keep the base's insertion order.

```rust
use gpui_3d::{Node, PoseMask, SceneGraph, WeightPose};

let mut graph = SceneGraph::new();
let node = graph.insert(None, Node::new())?;
let base = WeightPose::new([(node, vec![0., -0.2])])?;
let target = WeightPose::new([(node, vec![1., 0.6])])?;
let mask = PoseMask::new(0., [(node, 0.8)])?;
let mixed = base.blend(&target, 0.5, Some(&mask))?;
let values = mixed.get(node).unwrap();
# let _ = values;
# Ok::<(), Box<dyn std::error::Error>>(())
```

`base.additive(sample, reference, weight, mask)` applies
`base + (sample - reference) * t` componentwise. Each sample node must exist in
both base and reference with equal array lengths. Extra reference nodes are
ignored. Layers are ordered operations, not a normalized multi-clip average.
A mask affects only its addressed arrays; it does not expand through a hierarchy.

## Validation and deformation

`WeightPoseError` identifies duplicate or missing nodes, empty arrays, nonfinite
values, component-count mismatches, invalid blend weights, and unrepresentable
results. Validation still applies to zero-weight layers. A failure leaves every
input unchanged and returns no partial collection.

The collection does not inspect graphs or geometry. The caller supplies base
defaults for all layered nodes; the deformation consumer checks node liveness
and Morph target counts. Pass the resulting arrays into Morph evaluation before
Skin, then use one final scene snapshot for rendering and queries.

For imported clips, `AnimationSample::weight_pose()` exposes the same collection;
`weights()` remains the borrowed input for `SceneInstance::deform`. See
[Animation](animation.md) for weight tracks and transform layers, and
[Scene evaluation](evaluation.md) for final mesh snapshots.
