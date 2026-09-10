
# Primitive geometry

`PreparedDocument::geometry(mesh_index, primitive_index, options)` converts one
glTF primitive into an immutable `gpui_3d::Mesh`. Indices refer to the original
glTF arrays. Conversion performs synchronous CPU work without image decoding,
node construction, material conversion, or GPU access.

```rust
use gpui_3d_gltf::{GeometryOptions, PreparedDocument, PrimitiveGeometry};

fn convert(prepared: &PreparedDocument) -> anyhow::Result<PrimitiveGeometry> {
    prepared.geometry(0, 0, GeometryOptions::default())
}
```

The result retains mesh, primitive, and optional material indices.
`source_vertices()` maps each output vertex to its original accessor element,
including vertices split during normal or tangent generation. `into_parts()`
transfers the mesh and this mapping. Cloned meshes share core geometry storage.
Bounds are computed from converted vertices, not accessor min/max metadata.

## Attributes and topology

- `POSITION` is required as unnormalized float VEC3.
- `NORMAL` accepts unnormalized float VEC3. Finite nonzero normals are normalized.
- `TANGENT` accepts unnormalized float VEC4, with a finite nonzero tangent basis
  and handedness of exactly -1 or 1. Handedness must agree within each triangle.
- Texture coordinates accept unnormalized float VEC2 or normalized unsigned-byte
  and unsigned-short VEC2. `tex_coord_set` chooses the set copied into the mesh;
  UVs are not flipped or transformed. When no sets exist, vertices use zero UVs
  and the result reports `tex_coord_set() == None`. When sets exist but the
  requested set does not, conversion returns an error.
- All attribute counts must match POSITION. Vertex colors, skin attributes,
  custom attributes, and morph targets are unsupported and return errors.

Indexed primitives accept unsigned-byte, unsigned-short, or unsigned-int scalar
indices. Non-indexed primitives use accessor order. Triangle lists retain their
index order; strips and fans expand to triangle lists with the original winding
and triangle sequence. Lines and points return errors. Out-of-range indices and
reserved primitive-restart values are rejected. Degenerate triangles are not
removed; normal or tangent generation reports them as errors.

Interleaved, sparse, and zero-initialized accessors are supported. Required
extensions other than `KHR_materials_unlit` and `KHR_texture_transform` are
rejected. Accepting those material extensions here does not apply their effects;
the caller still converts materials and sampling state separately.

## Normal and tangent generation

Missing normals produce flat face normals and invalidate authored tangents.
Set `generate_tangents` to generate MikkTSpace tangents from the selected UV set,
replacing authored tangents. This requires texture coordinates and nondegenerate
geometry and UV triangles. The UV set must correspond to the intended normal
texture; conversion does not infer it from the material or apply UV transforms.

Generation preserves triangle identities and composes vertex mappings across
normal and tangent splits. Unreferenced vertices are omitted when generation
runs. Without generation they remain in the mesh and contribute to its bounds.
Vertex correspondence does not itself transform external normal/tangent deltas
into a regenerated basis.

## Limits and failures

Defaults admit 4,194,304 input/final vertices and 12,582,912 expanded indices per
primitive. Counts and topology expansion are checked before decoding arrays.
Final vertex admission also applies after normal/tangent splitting; temporary
generation storage is bounded by admitted index counts, not the final vertex
limit. These are element limits, not an exact process-memory budget. Resource
byte limits remain independent and apply during document preparation.

Errors retain mesh/primitive context and relevant accessor, vertex, or triangle
details. Failed conversion leaves prepared resources unchanged for another
conversion attempt.
