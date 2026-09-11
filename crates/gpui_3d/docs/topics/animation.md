# Animation and deformation

[3D viewports](../viewport.md)

## Morph targets

`MorphTargets` binds immutable target deltas to a base `Mesh`.
`MorphTarget::positions`, `normals`, and `tangents` contain optional dense XYZ
arrays in mesh-local space. Each supplied array must contain one finite delta
per base vertex, including unused vertices. A target needs at least one attribute;
the target list itself may be empty. Tangent deltas require base mesh tangents
and never modify handedness W. Clones share the base mesh and delta arrays.

```rust
use gpui_3d::{Mesh, MorphTarget, MorphTargets};

let base = Mesh::plane();
let targets = MorphTargets::new(base.clone(), [
    MorphTarget {
        positions: Some(vec![[0., 0., 0.5]; base.vertex_count()].into()),
        ..Default::default()
    },
    MorphTarget {
        positions: Some(vec![[0.25, 0., 0.]; base.vertex_count()].into()),
        ..Default::default()
    },
])?;
let mesh = targets.evaluate(&[0.7, -0.2])?;
# let _ = mesh;
# Ok::<(), gpui_3d::MorphError>(())
```

Evaluation computes `base + sum(weight * delta)` before node transforms. Weights
must be finite and match the target count; negative weights and values above one
are supported without clamping or normalization. Omitted attributes contribute
zero, and UVs and triangle identities remain unchanged. Normals with active
deltas are normalized after the complete blend. Tangents are orthogonalized
against the resulting normals with base handedness. Position-only targets do
not regenerate normals. Zero normals are supported without tangents; undefined
tangent bases return an error.

`evaluate` is synchronous and CPU-only, returning an ordinary immutable `Mesh`
with shared index storage and an independent lazy query index. Use the result in
`Object::new` or pass it to `SceneGraph::evaluate_with_overrides` for evaluated
world bounds. The same vertex snapshot is used by viewport/headless rendering
and ray queries. Zero weights return the shared base mesh. Keep a sampled mesh while its
weights are unchanged; the evaluator has no history, internal cache, or clock.
Work scales with vertex count and the number of nonzero targets.

`MorphError` reports invalid target/attribute/vertex offsets, mismatched weights,
nonfinite inputs, unrepresentable positions, and invalid resulting tangent data.
Failed evaluation does not modify the base or previous results. File-format
decoding, sparse-array expansion, weight animation, and playback policy belong
to the caller.

## Skeletal skinning

`Skin` stores shared inverse bind matrices and per-vertex `SkinInfluence` lists.
An inverse bind matrix maps mesh bind-space coordinates into one joint's
bind-local space. Influence joint indices address this array. The vertex lists
must include unused vertices and match the vertex order of every sampled mesh.
The binding does not own a mesh, so a morph result can be sampled directly.

```rust
use gpui_3d::{AffineTransform, Mesh, Skin, SkinInfluence};

let mesh = Mesh::plane();
let bind = AffineTransform::from_translation([0., 0.25, 0.])?;
let skin = Skin::new(
    [AffineTransform::IDENTITY, bind.inverse()],
    mesh.vertices().iter().map(|v| {
        let weight = v.position[1] + 0.5;
        [
            SkinInfluence { joint: 0, weight: 1. - weight },
            SkinInfluence { joint: 1, weight },
        ]
    }),
)?;
let bent_joint = AffineTransform::from_trs(
    [0., 0.25, 0.], [0., 0., 0.3_f32.sin(), 0.3_f32.cos()], [1.; 3],
)?;
let posed_mesh = skin.evaluate(&mesh, &[AffineTransform::IDENTITY, bent_joint])?;
# let _ = posed_mesh;
# Ok::<(), Box<dyn std::error::Error>>(())
```

Weights must be finite and nonnegative, with a positive total for each vertex.
They are normalized per vertex during construction using f64 accumulation.
Zero entries are discarded after their joint indices are validated; repeated
joint entries contribute additively. There is no fixed joint or influence limit.
Rigid vertices can reference one joint with a positive weight. Empty joint arrays,
empty vertex lists, missing influences, and out-of-range joints return `SkinError`.

`Skin::vertex_influences(vertex)` borrows a slice of `NormalizedSkinInfluence`
values containing `joint: usize` and `weight: f64`. These are the same normalized
contributions used by evaluation, including for unused mesh vertices. Positive
inputs retain their order; repeated joints remain separate and contribute
additively. Zero-weight inputs are absent. Reading does not allocate or round
weights to f32. An index at or beyond `vertex_count()` returns
`SkinError::VertexIndex` with the requested index and vertex count.

```rust
fn joint_weight(skin: &gpui_3d::Skin, vertex: usize, joint: usize)
    -> Result<f64, gpui_3d::SkinError>
{
    Ok(skin.vertex_influences(vertex)?.iter()
        .filter(|influence| influence.joint == joint)
        .map(|influence| influence.weight)
        .sum())
}
```

Cloned bindings share the borrowed data. Weight editing creates a new `Skin`
from caller-owned `SkinInfluence` inputs; normalization and validation apply to
the new binding without changing existing bindings or meshes. `SkinInfluence`
accepts f32 weights, so converting normalized f64 values back to editing inputs
can lose precision, including very small contributions. Imported bindings are
available through `gpui_3d_gltf::SceneSkin::binding()`.

`evaluate` accepts current joint-local-to-mesh-local transforms. Each is multiplied
by its inverse bind matrix before per-vertex linear blending. `evaluate_world`
instead accepts `mesh_world` and current world-space joint transforms, computing
`inverse(mesh_world) * joint_world * inverse_bind`. Supply the complete joint
array in binding order. With `SceneGraph`, first evaluate the joint hierarchy,
collect each joint's `EvaluatedNode::world`, sample the skin, then pass the
result to `evaluate_with_overrides` with the same local transforms. Render under
the same `mesh_world` used for sampling.

Positions use the blended affine transform. Normals use its inverse transpose
and are normalized; zero source normals remain zero without tangents. Tangent XYZ
uses the blended linear transform and is orthogonalized against the output normal.
Reflections flip tangent handedness. Triangle order is preserved, so skinning does
not repair folded geometry or reversed winding. Mixed tangent handedness within
one triangle returns a mesh validation error. Normals are not regenerated from
deformed triangles or spatial weight gradients.

All input and blended transforms must satisfy `AffineTransform`'s invertibility
and finite f32 constraints. Even invertible joint transforms can blend to a
singular matrix; this returns `SkinError::InvalidVertexTransform` with the vertex
offset. Matrix composition overflow and out-of-range positions are also errors.
Failed samples leave the source and previous results unchanged.

Sampling is synchronous CPU linear-blend skinning, not dual-quaternion skinning
or a GPU deformation pass. Work scales with joints, vertices, and retained
influences. Results are immutable `Mesh` snapshots with shared indices, fresh
bounds and query indices, and the existing topology-compatible GPU buffer reuse.
Rendering and picking consume the same geometry. Apply morph targets first, then
skin the morph result; do not feed a previous skinned result into the next sample.
Keep the sampled mesh while the pose is unchanged. File loading, joint selection,
animation playback, and pose caching remain caller-owned.

## Transform tracks

`VectorTrack` samples XYZ values and `RotationTrack` samples XYZW quaternions at
an explicit `Duration`. Tracks are immutable and clones share keyframe storage.
Keyframes must be nonempty, strictly ordered by time, and contain finite values
and tangents. Sampling before or after the key range holds the nearest endpoint;
a single key is constant. Sampling has no playback history.

| Interpolation | Vectors | Rotations |
| --- | --- | --- |
| `Step` | Hold the preceding key until the next timestamp. | Hold the normalized preceding quaternion. |
| `Linear` | Component interpolation. | Normalized shortest-arc spherical interpolation. |
| `CubicSpline` | Cubic Hermite interpolation. | Component Hermite interpolation, then normalization. |

`Keyframe::tangents(incoming, outgoing)` supplies component derivatives per
second, not per segment; the default tangents are zero. Cubic quaternion values
and tangents retain their supplied signs and magnitudes. Use unit quaternion
keys for orientation curves. A curve passing through a zero quaternion returns
`AnimationError::InvalidSample`, as does a vector result outside finite `f32`
range. Step and linear rotations accept finite nonzero quaternions of any length.

`TransformTrack` combines independent channels with an explicit `TransformPose`.
Missing channels retain the base translation, rotation, or scale. `sample` returns
a pose, and `sample_transform` returns its validated affine transform. Signed
scale is supported, but singular or unrepresentable transforms return an error,
including interpolated scales crossing zero. No affine matrix decomposition is
performed.

```rust
use gpui_3d::{
    Interpolation, Keyframe, Node, SceneGraph, TransformPose, TransformTrack, VectorTrack,
};
use std::time::Duration;

let mut graph = SceneGraph::new();
let root = graph.insert(None, Node::new())?;
let track = TransformTrack::new(TransformPose::default())?.translation(VectorTrack::new(
    [
        Keyframe::new(Duration::ZERO, [0., 0., 0.]),
        Keyframe::new(Duration::from_secs(2), [4., 1., 0.]),
    ],
    Interpolation::Linear,
)?);
let local = track.sample_transform(Duration::from_millis(750))?;
let pose = graph.evaluate_with_transforms([(root, local)])?;
let scene = pose.scene(gpui_3d::Camera::default());
# let _ = scene;
# Ok::<(), Box<dyn std::error::Error>>(())
```

`evaluate_with_transforms` replaces local transforms only for the supplied nodes.
It does not modify authored nodes or the graph revision. Omitted nodes retain
their authored transforms; descendants inherit the resulting parent transforms.
Duplicate, foreign, and expired handles return `SceneError`. World transforms,
normal matrices, bounds, rendering, and queries use the same evaluated pose.
Each evaluation has its own spatial index; previous snapshots remain unchanged.
The evaluated revision identifies source graph edits, not the sampled time or
overrides, so equal revisions do not imply equal poses.

For combined local-transform and mesh replacements, use
[`evaluate_with_overrides`](evaluation.md). The final snapshot uses the supplied
geometry for rendering, bounds, and picking without editing the graph.

The caller owns the clock, time origin, looping, pause, and seek policy. Sample
all channels before evaluating a snapshot. Additive or relative motion can be
expressed by composing a sampled transform with an authored affine transform
before passing it to `evaluate_with_transforms`.

## Pose blending

`TransformPose::blend(target, weight)` blends complete local TRS values at a finite
weight in `[0, 1]`. Translation and signed scale interpolate componentwise;
rotation uses normalized shortest-arc interpolation. Weight zero and one retain
the exact input representations. Both inputs and the result must form valid,
invertible transforms, even at endpoint weights. A scale crossing zero returns
`AnimationError::InvalidTransform`; weights outside the allowed range are errors,
not clamped values. The operation does not decompose affine matrices or preserve
shear from an unrelated authored transform.

`Pose` is an immutable, ordered collection of `(NodeHandle, TransformPose)` entries.
`Pose::new` rejects duplicate handles and invalid TRS values. Entries contain full
local transforms, not partial animation channels. Resolve missing track channels
against an explicit base pose before constructing the collection. Empty poses
are valid; clones share storage. `get` retrieves a local pose, `poses` iterates
entries, and `transforms` provides validated affine overrides for scene evaluation.

```rust
use gpui_3d::{Node, Pose, PoseMask, SceneGraph, TransformPose};
use std::f32::consts::FRAC_1_SQRT_2;

let mut graph = SceneGraph::new();
let root = graph.insert(None, Node::new())?;
let joint = graph.insert(Some(root), Node::new())?;
let local = TransformPose {
    translation: [0., 1., 0.],
    ..Default::default()
};
let base = Pose::new([(root, TransformPose::default()), (joint, local)])?;
let target = Pose::new([(joint, TransformPose {
    rotation: [0., 0., FRAC_1_SQRT_2, FRAC_1_SQRT_2],
    ..local
})])?;
let mask = PoseMask::new(0., [(joint, 1.)])?;
let mixed = base.blend(&target, 0.5, Some(&mask))?;
let evaluated = graph.evaluate_with_transforms(mixed.transforms())?;
# let _ = evaluated;
# Ok::<(), Box<dyn std::error::Error>>(())
```

`base.blend(target, weight, mask)` retains the base's node set and insertion order.
Target entries may be sparse; omitted nodes retain their base poses. Every target
node and every explicit mask node must exist in the base, including at zero
weight. Unknown nodes produce `PoseError::MissingNode` instead of blending against
an implicit identity pose. The inputs remain unchanged if any result fails
validation. Node liveness and graph membership are checked when passing the
overrides to `SceneGraph`, not when storing a pose.

`PoseMask::new(default_weight, entries)` assigns finite `[0, 1]` weights to stable
node handles, with an explicit default for unspecified nodes. Effective blend
weight is the global weight multiplied by the node's mask weight. With no mask,
all target nodes use the global weight. A default of zero includes only listed
nodes; a default of one can exclude selected nodes with zero-weight entries.
Mask weights apply to local poses and do not expand through the hierarchy. A
masked-out child still inherits its parent's final transform.

Successive `blend` calls form ordered override layers. They are not a normalized
multi-way mean, so changing layer order can change the result. Sample tracks at
explicit times, construct the input poses, apply layers and local overrides,
then pass `mixed.transforms()` to `evaluate_with_transforms` or
`evaluate_with_constraints`. Use the resulting world transforms for attachments
and skinning. Clip selection, per-clip timing, automatic subtree masks, and
animation-state machines remain caller policy.

## Additive poses

`TransformPose::additive(sample, reference, weight)` applies a reference-relative
change to the current local pose. The weight must be finite and within `[0, 1]`.
For base `B`, sample `S`, reference `R`, and weight `w`:

| Channel | Composition |
| --- | --- |
| Translation | `B + w * (S - R)`, in the parent's coordinate system |
| Rotation | `B * slerp(identity, inverse(R) * S, w)`, with normalized XYZW quaternions and the shortest arc |
| Scale | `B * ((1 - w) + w * (S / R))`, componentwise |

The rotation delta acts in the base rotation's local frame. Translation is not
rotated or scaled by the base. This is TRS channel composition, not full affine
delta multiplication. Negative scales are supported, but a weighted scale ratio
crossing zero returns `AnimationError::InvalidTransform`. All input and output
poses must be invertible and representable, even at zero weight. Zero weight or
identical sample/reference poses retain the exact base representation. Full
weight with an identical base/reference retains the exact sample representation.

`base.additive(sample, reference, weight, mask)` applies this operation to sparse
node poses. Every sample node must exist in both the base and the reference;
missing references return `PoseError::MissingReference`, including when global
or mask weights are zero. Extra reference nodes are ignored. Mask weights and
node validation follow `Pose::blend`; there is no implicit bind or identity pose.
The output retains the base's node set and order, and failures leave all inputs
unchanged.

```rust
use gpui_3d::{Node, Pose, PoseMask, SceneGraph, TransformPose};

let mut graph = SceneGraph::new();
let node = graph.insert(None, Node::new())?;
let local = TransformPose { translation: [0., 1., 0.], ..Default::default() };
let base = Pose::new([(node, local)])?;
let reference = Pose::new([(node, TransformPose::default())])?;
let sample = Pose::new([(node, TransformPose {
    translation: [0., 0.2, 0.],
    ..Default::default()
})])?;
let mask = PoseMask::new(0., [(node, 1.)])?;
let mixed = base.additive(&sample, &reference, 0.5, Some(&mask))?;
let evaluated = graph.evaluate_with_transforms(mixed.transforms())?;
# let _ = evaluated;
# Ok::<(), Box<dyn std::error::Error>>(())
```

Additive and override layers can be combined explicitly. Layer order matters,
particularly for rotations. Keep reference poses explicit, sample tracks at the
desired absolute time, apply layers, then evaluate constraints and world poses.
The operations retain no playback history.

## Weight tracks

Use [`WeightPose`](weight_layers.md) to combine sampled arrays with sparse
override layers, reference-relative additive layers, and node masks.

`WeightTrack` samples a runtime-sized array of weights from absolute timestamps.
Keys use `Keyframe<Vec<f32>>`; every key has the same nonzero component count.
Step holds the preceding value, Linear interpolates components, and CubicSpline
uses Hermite interpolation with derivatives per second. Empty derivative arrays
mean zeros and are expanded during construction; nonempty arrays must match the
weight count. All supplied values and derivatives must be finite, including
derivatives unused by Step or Linear. Timestamps must be strictly increasing.

```rust
use gpui_3d::{Interpolation, Keyframe, Mesh, MorphTarget, MorphTargets, WeightTrack};
use std::time::Duration;

let base = Mesh::plane();
let targets = MorphTargets::new(base.clone(), [
    MorphTarget {
        positions: Some(vec![[0., 0., 0.2]; base.vertex_count()].into()),
        ..Default::default()
    },
    MorphTarget {
        positions: Some(vec![[0.1, 0., 0.]; base.vertex_count()].into()),
        ..Default::default()
    },
])?;
let track = WeightTrack::new([
    Keyframe::new(Duration::ZERO, vec![0., 0.]),
    Keyframe::new(Duration::from_secs(2), vec![1., -0.5]),
], Interpolation::Linear)?;
let mut weights = vec![0.; track.weight_count()];
track.sample_into(Duration::from_millis(750), &mut weights)?;
let mesh = targets.evaluate(&weights)?;
# let _ = mesh;
# Ok::<(), Box<dyn std::error::Error>>(())
```

Weights are signed and neither clamped nor normalized. Cubic curves can overshoot
their keys. `sample(time)` returns an owned vector; `sample_into(time, output)`
uses a caller-owned slice of exactly `weight_count()` components without
allocating. A length mismatch or unrepresentable sample returns `AnimationError`
without changing the output slice. Sampling outside the key range retains the
first or last key. Clones share immutable keys but no playback or sampling state.

Weight order must match `MorphTargets::targets()`. Sample each instance's weights
at the chosen time, evaluate from the base targets, then apply `Skin` if needed.
The resulting mesh supplies geometry for rendering, bounds, and queries. The
track does not bind itself to nodes, mutate mesh assets, or own looping and clip
time conversion.

## Related topics

[Constraints and IK](constraints.md) · [GPU deformation](deformation.md).
