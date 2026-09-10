# Scenes

`PreparedDocument::scene(index, options)` converts a selected mesh scene
into a `SceneDefinition`. `None` selects the document's declared default scene;
without a default, pass an explicit index. An absent or out-of-range selection
returns an error. Nodes outside the selected scene are not converted.

Definitions own shared geometry, material parameters and encoded image inputs.
They are `Send + Sync` and may be prepared on a caller-owned background worker.
No filesystem, image decoding, native window or GPU operation occurs during
scene conversion. `materials()` exposes active material definitions in first-use
order for dependency inspection.

`SceneDefinition::resolve_images(decode)` creates a `SceneAsset`. The callback
supplies straight-alpha BGRA images under the material decoding contract. Each
active image index is decoded once across all materials in that call. Definitions
remain usable after the prepared document is dropped or image resolution fails.
No partial asset is returned; callback side effects are not rolled back.

```rust
use std::sync::Arc;
use gpui::RenderImage;
use gpui_3d::{AffineTransform, Camera, SceneGraph};
use gpui_3d_gltf::{EncodedImage, PreparedDocument, SceneOptions};

fn build(
    document: PreparedDocument,
    decode: impl FnMut(usize, &EncodedImage) -> anyhow::Result<Arc<RenderImage>>,
) -> anyhow::Result<SceneGraph> {
    let definition = document.scene(None, SceneOptions::default())?;
    let asset = definition.resolve_images(decode)?;
    let mut graph = SceneGraph::new();
    let first = graph.instantiate(None, asset.subtree())?;
    let second = graph.instantiate(None, asset.subtree())?;
    graph.set_transform(second.root(), AffineTransform::from_translation([3., 0., 0.])?)?;

    if let Some(primitive) = asset.primitives().first() {
        let handle = first.node(primitive.handle).unwrap();
        graph.set_visible(handle, false)?;
    }
    let scene = graph.evaluate()?.scene(Camera::default());
    // The same scene can be passed to a viewport or headless renderer.
    let _ = scene;
    Ok(graph)
}
```

## Hierarchy and identity

The asset contains one synthetic identity-transform root. Each original glTF node
becomes a group with its local transform. Mesh primitives become identity-transform
children of that group, before its original children. Root order, original sibling
order and primitive order are preserved. Column-major matrices, quaternion TRS and
negative scales retain their coordinate conventions without axis conversion.

`nodes()` associates original node indices and optional names with source-subtree
handles. `primitives()` associates each occurrence with its original node, mesh,
primitive and material indices. Duplicate names are allowed and do not become
application IDs. The synthetic root has no original node index.

Use `SubtreeInstance::node(source_handle)` to map these records to an instance.
Core evaluation, picking and frame-output mappings then identify the instantiated
primitive nodes. Application IDs can be assigned through
`SceneGraph::instantiate_with_ids`; they are not inferred from names or file indices.

Repeated references to the same mesh primitive share converted geometry. Instances
share geometry and decoded images while owning independent transforms, visibility
and material values. Removing an instance does not remove another instance or the
asset. Retained evaluated scenes preserve their previous state.

## Admission and supported content

`SceneOptions` bounds one conversion:

- `node_limit` counts the synthetic root, original nodes and every primitive
  occurrence, including repeated mesh instances.
- `vertex_limit` and `index_limit` apply across unique converted mesh primitives,
  including vertices split by normal or tangent generation. Repeated occurrences
  do not consume additional geometry quota.
- `tex_coord_limit` counts retained coordinate pairs across all sets of unique
  primitives, including implicit set zero. Each primitive's input and worst-case
  normal/tangent generation workspace must fit the remaining quota. Its default
  is 16,777,216 pairs.
- `influence_limit` counts retained geometry joint/weight slots and each unique
  `(skin, primitive)` binding's slots, including zero weights.
- `joint_limit` counts joints in each unique skin definition and each unique
  `(skin, primitive)` binding. Repeated occurrences share bindings and do not
  consume additional joint/influence quota.
- `deformed_vertex_limit` counts initial Morph/Skin output vertices per primitive
  occurrence, including repeated meshes. Its default is 4,194,304.
- `morph_target_limit` and `morph_attribute_limit` bound aggregate target and
  retained VEC3 attribute counts across unique primitives. Defaults are 8,192
  targets and 16,777,216 attribute elements. Each primitive's input attributes
  must also fit the remaining attribute quota.

These are conversion limits, not device-memory quotas. Document/encoded-resource
limits and callback-owned decoded-image limits remain separate. Conversion may
allocate bounded temporary attribute arrays; counts do not represent exact peak
bytes. Core instantiation into a destination graph has its own caller-owned policy.

Primitives retain every authored UV set under its original identifier, including
sets unused by active materials. Missing set zero is filled with zero UVs.
Each material slot uses its selected set; the active normal map determines the
tangent basis. Missing normals are generated, and active normal maps generate
tangents when required. Material and
geometry compatibility is checked before image decoding. Local/world transforms
and world mesh bounds must be representable by the core.

Duplicate roots, multiply referenced nodes within the selected scene, hierarchy
cycles, combined matrix/TRS properties and singular transforms return contextual
errors. Traversal is iterative. Different scenes may reference the same node.

Skin bindings retain joint order and initialize mesh geometry from the authored
joint pose after applying default Morph weights. Node weights override mesh
weights; omitted mesh weights are zero. `skins()` and `morphs()` expose shared
deformation inputs, and `deform()` evaluates them for an instance. Animation clips are not
sampled during scene conversion: ordinary transform-animated
nodes use their declared base transforms. `PreparedDocument::animation` converts
tracks separately for explicit instance-pose evaluation. Required unsupported
extensions are rejected; optional unknown extensions retain only their core glTF fallback.
Scene assets do not own animation playback, asset catalogs, file watching, loading
queues or image/GPU cache policies.

## Cameras

`PreparedDocument::camera(index)` returns a core camera in local coordinates:
eye at the origin, looking down -Z with +Y up. Scene conversion attaches this
camera to its original node. `SceneNode::camera_index` retains the source camera
index; several nodes may reference the same camera while keeping independent
world poses and instance overrides. Camera nodes add no renderable primitive or
Object ID coverage. Camera selection is explicit.

```rust
use anyhow::Context;
use gpui_3d::{Scene, SceneGraph};
use gpui_3d_gltf::SceneAsset;

fn camera_view(asset: &SceneAsset, camera_index: usize) -> anyhow::Result<Scene> {
    let source = asset.nodes().iter()
        .find(|node| node.camera_index == Some(camera_index))
        .context("camera is not present in the selected scene")?;
    let mut graph = SceneGraph::new();
    let instance = graph.instantiate(None, asset.subtree())?;
    let handle = instance.node(source.handle).context("missing instance camera")?;
    Ok(graph.evaluate()?.scene_from_camera(handle)?)
}
```

Perspective `yfov` maps to the vertical field of view without changing units.
An explicit `aspectRatio` fixes the core projection ratio; otherwise the output
width/height determines it. An absent `zfar` becomes positive infinity. Explicit
clip parameters must be finite and satisfy the core camera's range requirements.

Orthographic `ymag` gives half the full vertical span. The fixed aspect is
`xmag / ymag`, preserving both authored magnitudes independently of output shape.
Magnitudes must be positive and finite. The core requires positive near depth,
so orthographic `znear = 0` is unsupported and returns an error. No clip distance
is substituted silently.

Node/world transforms use the core camera transform contract: eye, viewing
direction and up follow the hierarchy; projection and clip distances are not
scaled. Invalid world camera poses are rejected during scene conversion before
image decoding. A fixed-ratio camera still fills the output rectangle; use a
matching viewport/output ratio or caller-owned letterboxing to avoid stretching.
