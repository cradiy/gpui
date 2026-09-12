# Animation clips

`PreparedDocument::animation(index, options)` converts one animation into shared,
CPU-only `AnimationClip` data. Translation, XYZW rotation, scale, and Morph weight
channels support Step, Linear, and CubicSpline interpolation. Cubic derivatives
remain per-second values; rotations use the core quaternion sampling rules.

`nodes()` groups channels by original glTF node index, in first-channel order.
Each entry exposes an optional `TransformTrack` and an optional `WeightTrack`.
Missing TRS channels retain the node's authored values. Tracks can be converted
without converting meshes or selecting a scene, including tracks targeting joints.
Weight tracks do not load Morph geometry or apply deformation.

Times remain absolute seconds from the file, converted to nanosecond `Duration`.
`start()` and `end()` span all referenced channels; `duration()` is their difference.
Sampling outside an individual channel retains its endpoint value. Looping,
speed, offsets, blending, and clip selection belong to the caller. Distinct
timestamps that collapse at nanosecond precision are rejected.

[`AnimationPlayback`](playback.md) provides optional caller-advanced time controls
for pause, seek, signed speed and looping without changing track data.

## Scene instances

`AnimationClip::bind(instance, policy)` maps tracks to a `SceneInstance` once.
`AnimationTargetPolicy::RequireAll` rejects targets outside that scene;
`SkipMissing` excludes them and records their original indices in
`BoundAnimation::missing_nodes()`, in first-channel order. A binding may contain
no active tracks when all targets are explicitly skipped.

Clips and assets must originate from the same parsed `Document`. Clones,
preparations, selected scenes, decoded assets, and repeated instances preserve
this source identity. Independently parsing identical bytes creates a different
identity and cannot be bound automatically. Names and numeric indices alone do
not establish compatibility. Explicit retargeting can use `NodeAnimation` tracks
and caller-owned handle mappings.

`BoundAnimation::sample(time)` returns an owned `AnimationSample` containing a
local `Pose` and destination-node Morph weights. Unanimated TRS channels use
authored values; unanimated weights are omitted for deformation to use asset
defaults. `into_parts()` returns the owned pose and weight list for composition.
The default sample is empty. Each call samples absolute time independently;
failure returns no partial sample and leaves previous samples usable.

`weight_pose()` exposes a core [`WeightPose`](../../gpui_3d/docs/topics/weight_layers.md)
for masked override or reference-relative additive mixing. Supply a complete base
for every layered target; sparse clip samples do not insert unanimated defaults.
Pass the mixed collection's `weights()` into instance deformation.

## Authored bases

`SceneInstance::authored_pose()` builds a core `Pose` containing every
TRS-authored node in the selected scene, including unanimated nodes. Values use
destination handles and parent-first scene order. Signed scales and authored
quaternion representations are retained. Matrix-authored nodes are excluded
without decomposition; their graph matrices remain in effect during evaluation.
The synthetic instance root and primitive children are excluded, so an outer
placement transform is not reset by applying the base.

`authored_weights()` builds a core `WeightPose` with one array per Morph node,
regardless of primitive count. Node weights override mesh defaults; absent
defaults become zero. Values remain signed and are not normalized. Nodes without
Morph targets are excluded. Both methods use retained asset data, not current
graph edits or clip samples, and do not require an animation clip. Graph evaluation
and deformation validate destination-handle liveness.

Build and retain these bases once per instance. To crossfade clips with different
target sets, expand each sparse sample against the same base before blending:

```rust
use gpui_3d::{Pose, WeightPose};
use gpui_3d_gltf::AnimationSample;

fn crossfade(
    base_pose: &Pose,
    base_weights: &WeightPose,
    a: &AnimationSample,
    b: &AnimationSample,
    weight: f32,
) -> anyhow::Result<(Pose, WeightPose)> {
    let pose_a = base_pose.blend(a.pose(), 1., None)?;
    let pose_b = base_pose.blend(b.pose(), 1., None)?;
    let weights_a = base_weights.blend(a.weight_pose(), 1., None)?;
    let weights_b = base_weights.blend(b.weight_pose(), 1., None)?;
    Ok((
        pose_a.blend(&pose_b, weight, None)?,
        weights_a.blend(&weights_b, weight, None)?,
    ))
}
```

This gives omitted targets their authored values on each side of the fade.
For layered overrides, blend sparse samples directly onto the current base with
the desired weight and optional mask. The authored bases can also be retained
as additive references.

## Scene evaluation

```no_run
use std::time::Duration;
use gpui_3d::SceneGraph;
use gpui_3d_gltf::{
    AnimationOptions, AnimationTargetPolicy, ImageDecodeLimits, PreparedDocument, SceneOptions,
};

fn evaluate(document: &PreparedDocument, time: Duration) -> anyhow::Result<()> {
    let clip = document.animation(0, AnimationOptions::default())?;
    let asset = document.scene(None, SceneOptions::default())?
        .decode_images(ImageDecodeLimits::default())?;
    let mut graph = SceneGraph::new();
    let instance = asset.instantiate(&mut graph, None)?;
    let binding = clip.bind(&instance, AnimationTargetPolicy::RequireAll)?;
    let sample = binding.sample(time)?;
    let transforms = graph.evaluate_with_transforms(sample.pose().transforms())?;
    let meshes = instance.deform(&transforms, sample.weights())?;
    let evaluated = transforms.with_meshes(meshes)?;
    // Use evaluated cameras, meshes, and queries from the same snapshot.
    let _ = evaluated;
    Ok(())
}
```

Bindings share clip tracks and mapped handles across clones without retaining
the parsed document, encoded buffers, scene geometry, or the graph. They can be
sampled on worker threads. Graph deletion does not retarget a binding; scene
evaluation rejects expired handles. Bind separately for each instance and combine
sampled poses and mesh replacements before final scene evaluation. Playback,
layer mixing, constraints, and release of graph nodes remain caller-owned.

## Admission and errors

`AnimationOptions` bounds channels, aggregate keyframes, and retained f32
values/derivatives per conversion. Shared samplers are charged per channel.
Scalar admission includes three component arrays per key even for noncubic
tracks, whose derivatives are zero. These are conversion limits, not a total
process-memory quota; document and encoded resources have separate limits.

Input accessors must contain finite, nonnegative, strictly increasing float
seconds. Outputs must match the target's dimensions, component format, key count,
and interpolation. Rotation and weight outputs also accept normalized 8/16-bit
signed or unsigned components. Weight counts must match every primitive of the
target mesh. Duplicate node/property targets and matrix-authored animated nodes
are rejected. Errors retain animation and channel indices; definitions remain
usable after failed conversion.

The core requires invertible transforms. Zero-scale keys are rejected; curves
crossing singular scales or zero quaternions return sampling errors. Skin and
Morph scene conversion is not provided by clip conversion. No graph mutation,
image decoding, I/O, clock advancement, or GPU work occurs while importing tracks.
