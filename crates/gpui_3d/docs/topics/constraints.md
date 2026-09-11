# Constraints and IK

[3D viewports](../viewport.md)

## Transform constraints

`SceneGraph::evaluate_with_constraints(transforms, constraints)` evaluates local
pose overrides and stateless world-transform constraints in one snapshot. Each
constraint is paired with the handle of the node it controls. `Follow` computes
`target.world * offset`, replacing the controlled node's world transform. Its
authored or supplied local transform is not applied; encode the attachment
offset explicitly. Full affine offsets preserve scale, shear, and reflection.

```rust
use gpui_3d::{AffineTransform, Node, SceneGraph, TransformConstraint};

let mut graph = SceneGraph::new();
let target = graph.insert(None, Node::new())?;
let parent = graph.insert(None, Node::new())?;
let attached = graph.insert(Some(parent), Node::new())?;
let target_pose = AffineTransform::from_translation([2., 0., 0.])?;
let offset = AffineTransform::from_translation([0., 0.5, 0.])?;
let pose = graph.evaluate_with_constraints(
    [(target, target_pose)],
    [(attached, TransformConstraint::Follow { target, offset })],
)?;
let world = pose.node(attached).unwrap().world;
assert_eq!(world.transform_point([0.; 3]), [2., 0.5, 0.]);

let parent_world = pose.node(parent).unwrap().world;
let released_local = parent_world.inverse().compose(world)?;
let released = graph.evaluate_with_constraints([(attached, released_local)], [])?;
assert_eq!(released.node(attached).unwrap().world, world);
# Ok::<(), Box<dyn std::error::Error>>(())
```

Targets use their final constrained transforms, including their own ancestors
and pose overrides. Input order does not determine evaluation order. Chained
attachments are supported, while self references, descendant targets, and
indirect dependency cycles return `SceneError::ConstraintCycle` with a closed
path of participating handles. Both parent and target links are dependencies.
Expired or foreign targets identify the owner and target in
`InvalidConstraintTarget`; duplicate owner constraints are rejected. A node
may have both a local pose override and a constraint, but only one of each.

Constraints do not reparent nodes or transfer visibility from targets. Hidden
targets are evaluated normally; controlled nodes retain visibility inherited
from their authored hierarchy. Children inherit the constrained world transform.
Camera and light properties, bounds, spatial queries, and renderer preparation
use the same final transforms. Hierarchy traversal order and frame-local object
identity ordering are unchanged.

`evaluate_with_constraints_and_meshes(transforms, constraints, meshes)` also
accepts replacement geometry without editing mesh nodes. For skinning, evaluate
the constrained joint pose first, compute the deformed meshes, and pass the same
transform and constraint inputs to this method. The returned snapshot preserves
constraint outcomes while using the replacement meshes for bounds, rendering,
and picking. See [Scene evaluation](evaluation.md) for mesh admission and snapshot
semantics.

The graph, its revision, and retained snapshots are not modified. Callers own
the constraint list and can evaluate any sampled pose without playback history.
Omitting a constraint restores the authored or supplied local transform. To
release an attachment while preserving its current world transform, supply
`parent.world.inverse() * node.world` as the replacement local transform and
retain the same evaluated parent pose; for roots, use the world transform itself.
Persistent hierarchy changes use `reparent` instead.

#### Aim and LookAt

`AimSettings::solve(world_transform, world_target)` rotates a transform around
its own origin without changing translation, scale, shear, or handedness.
It returns an `AimResult` containing the transform and an `AimStatus`. No graph,
camera, GPU, or playback history is required.

The default local forward is `-Z`, local up is `+Y`, and world up is `+Y`.
Custom axes need not be normalized. Forward aligns with the target direction;
the component of transformed local up perpendicular to forward aligns with
projected world up. The solver applies a world-space rigid rotation to the
entire linear transform, without decomposing it into TRS. Up need not become
perpendicular to forward when the input affine shape contains shear.

`max_angle` limits the shortest **total orientation correction**, including
roll, relative to the supplied pose. Its range is `0..=pi` radians; the default
`pi` allows a full correction. Zero preserves the supplied transform, while
still validating the inputs. This is not a yaw/pitch cone or an angular speed.
`AimStatus` reports the requested angle, applied angle, and whether the result
was limited. A limited result may not face the target exactly. Exact half-turns
use a deterministic rotation axis.

```rust
use gpui_3d::{AffineTransform, AimSettings};

let result = AimSettings {
    local_forward: [1., 0., 0.],
    max_angle: std::f32::consts::FRAC_PI_4,
    ..Default::default()
}.solve(AffineTransform::IDENTITY, [0., 0., -2.])?;
assert!(result.status.limited);
let transform = result.transform;
# Ok::<(), gpui_3d::AimError>(())
```

Within a graph, `TransformConstraint::Aim` uses the node's local pose composed
with its final parent transform. `target_offset` is a point in the target node's
local coordinates; the target's final world transform supplies its world position.
The `world_up` setting is a world-space vector and does not follow the target's
orientation. Both target and parent participate in dependency-cycle detection.

```rust
use gpui_3d::{AimSettings, ConstraintStatus, Node, SceneGraph, TransformConstraint};

let mut graph = SceneGraph::new();
let node = graph.insert(None, Node::new())?;
let target = graph.insert(None, Node::new())?;
let pose = graph.evaluate_with_constraints([], [(node, TransformConstraint::Aim {
    target,
    target_offset: [2., 0., -3.],
    settings: AimSettings::default(),
})])?;
if let Some(ConstraintStatus::Aim(status)) = pose.constraint_status(node) {
    assert!(!status.limited);
}
# Ok::<(), gpui_3d::SceneError>(())
```

`EvaluatedScene::constraint_status` retains `Follow` or `Aim` outcomes only for
constrained nodes; unconstrained or absent nodes return `None`. Each snapshot
keeps its own results after later evaluations. To constrain an animated pose,
sample its local tracks independently for each requested time; feeding previous
solver outputs back as base poses instead accumulates rotation across calls.

Non-finite or zero axes, coincident targets, invalid limits, and unrepresentable
results return `AimError`; graph evaluation wraps it in `SceneError::InvalidAim`
with the controlled node handle. Transformed local forward/up and target/world-up
pairs must have a sine angle greater than `1e-6`. Parallel or nearly parallel
pairs are errors, not automatic alternate-axis selections.

#### Two-bone inverse kinematics

`TwoBoneIkSettings::solve([root, middle, tip], target, pole)` accepts three world
transforms and world-space target/pole points. The supplied joint distances
define the two bone lengths. The result contains three world transforms,
`reachable_target`, `IkReach`, and `target_error` in scene units. No graph,
skeleton naming convention, or playback state is required.

```rust
use gpui_3d::{AffineTransform, IkReach, TwoBoneIkSettings};

let source = [
    AffineTransform::IDENTITY,
    AffineTransform::from_translation([1., 0., 0.])?,
    AffineTransform::from_translation([2., 0., 0.])?,
];
let result = TwoBoneIkSettings { weight: 1. }
    .solve(source, [1., 1., 0.], [0., 0., 2.])?;
assert_eq!(result.reach, IkReach::Reachable);
let [root_world, middle_world, tip_world] = result.transforms;
let middle_local = root_world.inverse().compose(middle_world)?;
let tip_local = middle_world.inverse().compose(tip_world)?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

The root position is fixed. The pole selects the side of the root-to-target
axis toward which the middle joint bends. The solver uses shortest-swing root
and middle rotations, preserving each joint's affine shape and handedness.
The tip inherits both rotations; there is no separate tip-orientation target.
Exact opposed directions use a deterministic orthogonal rotation axis.

`weight` is in `0..=1`, defaulting to one. It blends the root rotation and the
middle's relative rotation, then evaluates the chain. Intermediate weights
preserve bone lengths rather than interpolating joint positions. Zero returns
the supplied transforms unchanged, but inputs and pole geometry are still
validated. Always solve from the independently sampled source pose when seeking;
feeding prior solver results back as inputs accumulates motion.

For lengths `a` and `b`, the reachable radial interval is `[abs(a-b), a+b]`.
Targets outside it are projected onto the closest boundary without stretching,
with `IkReach::TooClose` or `TooFar`. `Reachable` describes geometry before
blending, not an assurance that a partially blended tip reaches the target.
`target_error` is the `f64` distance from the returned, rounded tip to the
original target. `reachable_target` is the radial projection before blending.

At the root, equal-length bones fold completely with the middle joint toward
the pole. Unequal-length bones retain the supplied root-to-tip direction when
choosing the nearest point on their inner reach boundary. A pole coincident
with the root is invalid. For bent solutions its direction must have a sine
angle greater than `1e-6` from the target axis; straight or fully folded radial
boundary solutions do not need a perpendicular pole. No joint-angle limits or
additional twist controls are imposed.

Zero-length bones, non-finite target/pole data, invalid weights, and ambiguous
bend poles return `TwoBoneIkError`. Calculations use widened intermediates;
outputs remain `f32` affine transforms. If rounding changes either bone length
by more than `1e-4` relative to its supplied length, or a transform cannot be
represented, the solver returns `Unrepresentable`.

To apply results to a three-node chain, convert each world result into its
parent's local space and pass the replacements to `evaluate_with_transforms`.
The root uses its evaluated external parent's inverse (or identity when it has
no parent); the middle and tip use the solved root and middle inverses as above.
Retain other sampled parent overrides. This also permits passing solved world
poses directly into skinning without owning a `SceneGraph`. Applications own
multi-chain ordering and any conflicting pose overrides.

## Joint rotation limits

`JointRotationLimits::solve(rotation)` limits a joint-local-to-parent quaternion
relative to `reference_rotation`. Both use `[x, y, z, w]` order and accept finite,
nonzero inputs that are normalized on entry. `twist_axis` is expressed in the
reference joint's local space and need not be normalized.

The relative rotation is decomposed as
`reference.inverse() * rotation = swing * twist`. The swing rotates about an
axis perpendicular to `twist_axis`; the twist rotates around `twist_axis`.
`max_swing` defines a circular cone half-angle in `0..=pi` radians.
`twist_range` is an ordered `[min, max]` interval within `[-pi, pi]`. The default
reference is identity, the default axis is `+X`, and both components are unrestricted.
Zero swing and a `[0, 0]` twist interval lock the rotation to the reference pose.

```rust
use gpui_3d::{JointRotationLimits, TransformPose};

let limits = JointRotationLimits {
    twist_axis: [0., 1., 0.],
    max_swing: 45_f32.to_radians(),
    twist_range: [-20_f32.to_radians(), 30_f32.to_radians()],
    ..Default::default()
};

fn constrain_pose(
    mut pose: TransformPose,
    limits: JointRotationLimits,
) -> Result<TransformPose, gpui_3d::JointRotationLimitError> {
    pose.rotation = limits.solve(pose.rotation)?.rotation;
    Ok(pose)
}
# Ok::<(), gpui_3d::JointRotationLimitError>(())
```

The returned `rotation` is the constrained local orientation. `swing` and `twist`
each report `requested_angle`, `applied_angle`, and `limited`. Angles are computed
in `f64`; the output quaternion is rounded to `f32`. Diagnostics describe the
angular clamp before output rounding. Quaternion sign does not change the result's
orientation. No translation, scale, graph state, or mesh data is modified.

Twist uses the signed principal angle in `(-pi, pi]`, choosing positive `pi` for
an exact half-turn. Intervals are clamped numerically and do not wrap across that
seam. This is not a multi-turn counter or a closest-orientation optimization.
The `f32` representations of `pi` and `-pi` are accepted as interval endpoints.

At a half-turn swing the twist decomposition is not unique. When the norm of
the quaternion's scalar and axial projection is at most `1e-12`, the solver
selects zero twist and sets `twist_degenerate`. Limits then apply to that
decomposition. The convention is deterministic, not a continuity guarantee near
the singularity or the principal-angle seam; no previous pose is retained.

Invalid rotations, reference rotations, axes, or angular limits return the
corresponding `JointRotationLimitError`. Callers apply the output to a
`TransformPose` before creating local transform overrides. Arbitrary affine
matrices are not decomposed into joint orientations. This solver does not
automatically constrain `TwoBoneIkSettings` or preserve an IK target after
clamping; chain solving and constraint order are caller-controlled.

## Related topics

[Scene hierarchy](scenes.md).
