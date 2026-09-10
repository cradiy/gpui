# Scenes

`PreparedDocument::scene(index, options)` converts a selected static mesh scene
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

These are conversion limits, not device-memory quotas. Document/encoded-resource
limits and callback-owned decoded-image limits remain separate. Conversion may
allocate bounded temporary attribute arrays; counts do not represent exact peak
bytes. Core instantiation into a destination graph has its own caller-owned policy.

Each active material determines its primitive's UV set. Untextured primitives
retain their lowest available UV set, or zero UVs if none exists. Missing normals are
generated, and active normal maps generate tangents when required. Material and
geometry compatibility is checked before image decoding. Local/world transforms
and world mesh bounds must be representable by the core.

Duplicate roots, multiply referenced nodes within the selected scene, hierarchy
cycles, combined matrix/TRS properties and singular transforms return contextual
errors. Traversal is iterative. Different scenes may reference the same node.

Skins, morph targets/weights and camera nodes are not converted and return errors
when encountered. Animation clips are not sampled: ordinary transform-animated
nodes use their declared base transforms. Required unsupported extensions are
rejected; optional unknown extensions retain only their core glTF fallback.
Scene assets do not own animation playback, asset catalogs, file watching, loading
queues or image/GPU cache policies.
