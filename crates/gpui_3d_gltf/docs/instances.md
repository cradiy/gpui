# Scene instances

`SceneAsset::instantiate(&mut graph, parent)` creates a `SceneInstance` containing
shared asset data and the handles of one editable graph subtree. Each instance
has independent transforms, visibility, materials and deformation output. Mesh
and decoded-image storage remain shared until replaced.

```rust
use gpui::rgb;
use gpui_3d::{AffineTransform, Material, SceneGraph};
use gpui_3d_gltf::SceneAsset;

fn instantiate(asset: &SceneAsset, graph: &mut SceneGraph) -> anyhow::Result<()> {
    let first = asset.instantiate(graph, None)?;
    let second = asset.instantiate(graph, None)?;
    graph.set_transform(
        second.root(),
        AffineTransform::from_translation([3., 0., 0.])?,
    )?;
    for node in first.material_nodes(Some(0)) {
        graph.set_material(node, Material::color(rgb(0x4080ff)))?;
    }
    Ok(())
}
```

The instance retains its asset after the original `SceneAsset` is dropped.
`asset()` exposes source metadata and shared deformation inputs.
`subtree_instance()` provides the core mapping for APIs that accept
`SubtreeInstance`. Direct instantiation through `SceneGraph` remains available
when only the core mapping is needed.

## Source and destination identities

| Method | Mapping |
| --- | --- |
| `root()` | Synthetic instance root, without an original glTF node index. |
| `node(node_index)` | Original glTF node index to its instance group handle. |
| `primitive(node_index, primitive_index)` | Original node and mesh-local primitive index to one instance primitive handle. |
| `source_node(handle)` | Instance group handle to the original `SceneNode` metadata. |
| `source_primitive(handle)` | Instance primitive handle to the original `ScenePrimitive` metadata. |
| `material_nodes(material_index)` | Instance primitive handles associated with an authored material, in scene order. |

Primitive occurrences are distinguished by node index: two nodes referencing
the same mesh retain separate editable handles. Names may be duplicated and are
not used for lookup. Reverse mappings reject handles from other instances or
graphs. Group and primitive mappings are separate; neither includes the synthetic
root. Source metadata's `handle` remains a source-subtree handle, not a destination
handle.

Picking and frame-output object mappings report destination primitive handles.
Pass those handles to `source_primitive()` to recover the original node, mesh,
primitive, material and skin indices.

`material_nodes(Some(index))` selects occurrences of that original glTF material.
`None` selects primitives with an implicit material; it does not select all
materials. Assigning a new material in the graph leaves this authored association
unchanged. A nonexistent source index yields no handles. Individual graph
mutations are not a multi-node transaction.

`instantiate_with_ids()` accepts the same source-handle callback as the core
graph API. IDs remain caller-defined, including the synthetic root and primitive
children. Duplicate IDs and invalid parents leave the graph and revision
unchanged; callback side effects are not rolled back.

## Deformation and lifetime

[`AnimationClip::bind`](animation.md#scene-instances) creates reusable track
bindings for this instance with an explicit policy for scene-external targets.
Each binding samples independent local poses and Morph weights.

`instance.deform(&poses, &weights)` evaluates the retained asset's Morph and Skin
inputs against the instance's final pose snapshot. Weight targets use destination
group handles, obtainable with `node(index)`. Omitted weights use authored
defaults. The method returns primitive mesh replacements without modifying the
graph; pass them to `SceneGraph::evaluate_with_overrides` with the same local
transforms used for the pose snapshot. See [Morph](morph.md) and
[Skin](skin.md) for evaluation and admission constraints.

The graph owns nodes. Dropping an instance does not remove its subtree, and
removing nodes does not mutate retained instance metadata. Lookups describe the
original instantiation and do not validate liveness. Use graph methods to validate
handles; stale generational handles cannot identify newly inserted nodes.

Remove a hierarchy explicitly with `graph.remove_subtree(instance.root())`.
Ordinary graph reparenting rules apply: descendants moved elsewhere are not
removed with that root, while nodes attached beneath it are. Retained evaluated
scenes and other instances preserve their own state.
