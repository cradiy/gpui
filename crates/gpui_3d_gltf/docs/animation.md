## Animation clips

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

### Scene instances

Match `NodeAnimation::node_index()` to `SceneNode::index` from the same document,
then use the chosen `SubtreeInstance` to map its source handle. A clip may target
nodes outside the selected scene; the caller decides whether to skip or reject
those targets. Names are optional metadata, not binding keys.

```no_run
use std::time::Duration;
use gpui_3d::{Pose, SceneGraph};
use gpui_3d_gltf::{AnimationOptions, ImageDecodeLimits, PreparedDocument, SceneOptions};

fn evaluate(document: &PreparedDocument, time: Duration) -> anyhow::Result<()> {
    let clip = document.animation(0, AnimationOptions::default())?;
    let asset = document.scene(None, SceneOptions::default())?
        .decode_images(ImageDecodeLimits::default())?;
    let mut graph = SceneGraph::new();
    let instance = graph.instantiate(None, asset.subtree())?;
    let bindings: std::collections::HashMap<_, _> = asset.nodes().iter()
        .map(|node| (node.index, instance.node(node.handle).unwrap()))
        .collect();
    let mut locals = Vec::new();
    for animation in clip.nodes() {
        if let (Some(&handle), Some(track)) =
            (bindings.get(&animation.node_index()), animation.transform())
        {
            locals.push((handle, track.sample(time)?));
        }
    }
    let pose = Pose::new(locals)?;
    let transforms = graph.evaluate_with_transforms(pose.transforms())?;
    let meshes = asset.skins().iter()
        .map(|skin| skin.evaluate(&instance, &transforms))
        .collect::<anyhow::Result<Vec<_>>>()?;
    for (handle, mesh) in meshes {
        graph.set_mesh(handle, mesh)?;
    }
    let evaluated = graph.evaluate_with_transforms(pose.transforms())?;
    // Use evaluated cameras, meshes, and queries from the same snapshot.
    let _ = evaluated;
    Ok(())
}
```

### Admission and errors

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
