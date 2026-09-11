# GPU vertex streams

[Geometry](geometry.md) · [GPU deformation](deformation.md)

With the native `wgpu` feature, `WgpuScene3dGeometry::with_attributes` creates a
render-source snapshot with independently replaced UV sets and vertex colors.
It accepts borrowed `Scene3dVertexUpdate` values, uploading CPU streams or copying
external GPU buffers without readback.
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
one copied stream. Duplicate UV sets, duplicate colors, missing selections,
invalid CPU values, and mismatched counts return errors before GPU allocation.
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

## External buffers

`UvBuffer { set, buffer }` copies packed Float32x2 coordinates; `ColorBuffer(buffer)`
copies packed Float32x4 colors. Inputs are `WgpuResource<wgpu::Buffer>` values
created through the source's `WgpuContext`. Each allocation must have exactly one
record per base vertex and include `COPY_SRC` usage. Device ownership is checked
before backend buffer access. CPU and buffer updates can be mixed in one call;
duplicate streams are rejected across both forms.

```rust
let updated = source.with_attributes(
    &[
        Scene3dVertexUpdate::UvBuffer { set: 0, buffer: &gpu_uvs },
        Scene3dVertexUpdate::ColorBuffer(&gpu_colors),
    ],
    Some(32 * 1024 * 1024),
)?;
```

Submit producers on the source's queue before calling. Later writes submitted on
that queue can reuse input buffers without changing the returned snapshot. Inputs
must remain unmapped and undestroyed until their copies complete; dropping handles
does not explicitly destroy the buffers. The returned source owns copied values,
not the external buffers.

GPU values are not checked on the CPU. Packing checks every vertex, including
unused vertices, and suppresses the entire indirect draw if UVs are nonfinite or
color lanes fall outside `[0, 1]`. This does not turn `with_attributes` or packing
into a CPU error result. Replacing invalid streams with valid ones produces a
usable source; no values are clamped and earlier snapshots remain unchanged.
Use the packed result's [validation status](deformation.md#geometry-validation)
to inspect rejection reasons without reading vertex data.

## Versions and memory

Each nonempty update copies the interleaved source into a new GPU allocation and
updates selected fields there. Earlier sources and packed results remain unchanged.
The base mesh, index buffer, packing pipeline, and lazily prepared attribute-update
pipeline are shared. Updates may be chained; no source retains its predecessor's
vertex buffer solely to preserve the chain.

The working-byte limit covers the new 96-byte-per-vertex source plus a temporary
copy buffer of 32 header bytes, 8 bytes per vertex per supplied UV set, and 16 bytes per
vertex when colors are supplied. Repeated material selections do not duplicate UV
copy storage. External inputs count toward this temporary allocation just like
CPU inputs. Existing sources, input buffers, packed results, shared resources, CPU storage,
and driver overhead are outside this per-call budget. Enabled device buffer,
storage-binding, and dispatch limits are checked separately.

Calls submit on the source's queue without vertex readback or a CPU wait for GPU
completion. Packing submitted afterward observes the updated source. Recreate
sources after device replacement. This interface uses the renderer's fixed UV and
color formats. [Custom vertex inputs](material_attributes.md) use separately
declared streams and immutable material-bound snapshots.
