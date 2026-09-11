# GPU vertex streams

[Geometry](geometry.md) · [GPU deformation](deformation.md)

With the native `wgpu` feature, `WgpuScene3dGeometry::with_attributes` creates a
render-source snapshot with independently replaced UV sets and vertex colors.
It accepts borrowed `Scene3dVertexUpdate` values and uploads only those streams.
Positions, normals, tangents, and indices are not uploaded again.

```rust
use gpui_3d::Scene3dVertexUpdate;

let source = output.render_source([0; 5], None)?;
let updated = source.with_attributes(
    &[
        Scene3dVertexUpdate::Uv { set: 0, coordinates: &animated_uvs },
        Scene3dVertexUpdate::Color(&vertex_colors),
    ],
    Some(32 * 1024 * 1024),
)?;
let geometry = output.render_geometry(&updated)?;
```

## Stream contract

Each replacement contains exactly one value per base-mesh vertex, including
unused vertices. UVs must be finite but may extend outside `[0, 1]`. Colors are
normalized linear, straight-alpha RGBA multipliers; all components must be finite
and within `[0, 1]`. Supply white to remove vertex-color modulation.

Only coordinate sets selected by the source's `uv_sets()` are accepted. The five
selections correspond to base color, metallic/roughness, emission, normal, and
occlusion textures. Updating one set replaces every slot selecting that set with
one uploaded stream. Duplicate UV sets, duplicate colors, missing selections,
invalid values, and mismatched counts return errors before GPU allocation.
Omitted streams retain their current values. Empty updates share the source
without buffer allocation or submission.

The source retains its original base mesh and topology identity. CPU mesh values,
CPU queries, and deformation readbacks do not include these render-only updates.
Pass the resulting packed geometry to the viewport or headless renderer; color,
shadow, depth, normal, and ID passes use the same updated vertex inputs and surface
coverage. Use GPU ID/depth picking when UV or alpha changes affect visible coverage.

UV updates do not regenerate tangent frames. If a normal map uses changed
tangent-space coordinates, supply matching tangents in the deformation result
used for packing. Normal/tangent processors initialized from the original CPU
mesh still use its original coordinate sets.

## Versions and memory

Each nonempty update copies the interleaved source into a new GPU allocation and
updates selected fields there. Earlier sources and packed results remain unchanged.
The base mesh, index buffer, packing pipeline, and lazily prepared attribute-update
pipeline are shared. Updates may be chained; no source retains its predecessor's
vertex buffer solely to preserve the chain.

The working-byte limit covers the new 96-byte-per-vertex source plus a temporary
upload of 32 header bytes, 8 bytes per vertex per supplied UV set, and 16 bytes per
vertex when colors are supplied. Repeated material selections do not duplicate UV
upload storage. Existing sources, packed results, shared resources, CPU storage,
and driver overhead are outside this per-call budget. Enabled device buffer,
storage-binding, and dispatch limits are checked separately.

Calls submit on the source's queue without vertex readback or a CPU wait for GPU
completion. Packing submitted afterward observes the updated source. Recreate
sources after device replacement. This interface uses the renderer's fixed UV and
color formats; it does not declare application-specific vertex attributes.
