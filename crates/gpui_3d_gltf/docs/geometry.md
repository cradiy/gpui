
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
  and unsigned-short VEC2. All sets retain their original identifiers and values;
  UVs are not flipped or transformed. `tex_coord_sets()` lists authored set IDs
  in ascending order. `Mesh::uv_at(set, vertex)` reads any retained set, while
  `Vertex::uv` contains set zero. If set zero is absent, it is filled with zeros
  without adding it to the authored-set list.
- Skin attributes retain consecutive `JOINTS_n`/`WEIGHTS_n` pairs. Joint indices
  use unsigned-byte/unsigned-short VEC4; weights use float or normalized
  unsigned-byte/unsigned-short VEC4. Weights must be finite and nonnegative, with
  a positive total per vertex. `skin_influences()` exposes output-vertex slices
  after normal/tangent splitting. Indices address a skin's joint array.
- Morph targets retain float VEC3 position, normal, and tangent deltas through
  `morph()`. Target data follows the generated vertex correspondence.
- `COLOR_0` accepts unnormalized float or normalized unsigned-byte/unsigned-short
  VEC3 and VEC4. RGB stays linear; VEC3 uses alpha one. Every component must be
  finite and within `[0, 1]`, including unused vertices. Colors retain their
  correspondence through normal/tangent splits, Morph, and Skin. Missing colors
  use implicit white. Additional color sets and custom attributes are unsupported.
- All attribute counts must match POSITION.

Indexed primitives accept unsigned-byte, unsigned-short, or unsigned-int scalar
indices. Non-indexed primitives use accessor order. Triangle lists retain their
index order; strips and fans expand to triangle lists with the original winding
and triangle sequence. Lines and points return errors. Out-of-range indices and
reserved primitive-restart values are rejected. Degenerate triangles are not
removed. Flat normal generation requires nonzero geometric area; tangent
generation uses the repair policy described below.

Interleaved, sparse, and zero-initialized accessors are supported. Required
extensions other than `KHR_materials_unlit`, `KHR_texture_transform` and
[`KHR_lights_punctual`](lights.md) are rejected. Geometry conversion does not apply
material or lighting effects; those inputs are converted separately.

## Normal and tangent generation

Missing normals produce flat face normals and invalidate authored tangents.
Set `generate_tangents` to generate MikkTSpace tangents from `tangent_uv_set`,
replacing authored tangents. This requires texture coordinates and nonzero
normals. The UV set must correspond to the intended normal
texture; conversion does not infer it from the material or apply UV transforms.
`tangent_uv_set` also identifies the basis of retained authored tangents.
Generating tangents requires that set to exist in the source primitive.

Generation uses `TangentGenerationMode::Repair`: neighboring MikkTSpace frames
are inherited where available, undefined corners use a triangle derivative when
possible, and otherwise receive a deterministic normal-orthogonal basis.
`tangent_repairs()` reports base-mesh triangle/corner positions and repair kinds;
triangle indices refer to the expanded triangle list. A synthesized orthonormal
basis does not recover an undefined authored UV direction. Callers requiring
that fidelity can reject the reported repairs. Invalid normals, numerical range
failures, and incompatible triangle handedness remain errors.
`SceneDefinition::geometries()` exposes unique primitives and these diagnostics
before image decoding. `into_parts()` discards repair diagnostics along with
deformation inputs.

Generation preserves triangle identities and composes vertex mappings across
normal and tangent splits, retaining every UV set through the same mapping.
Unreferenced vertices are omitted when generation
runs. Without generation they remain in the mesh and contribute to its bounds.
Morph primitives use a fixed triangle-corner layout when generating normals or
tangents. `MorphGeometry::evaluate` recomputes generated directions from the
weighted geometry without changing its topology. Authored direction deltas are
used when their corresponding directions are not generated.

## Limits and failures

Defaults admit 4,194,304 input/final vertices and 12,582,912 expanded indices per
primitive. Counts and topology expansion are checked before decoding arrays.
Final vertex admission also applies after normal/tangent splitting; temporary
generation storage is bounded by admitted index counts, not the final vertex
limit. `influence_limit` bounds both input and output joint/weight slots,
including zero weights; its default is 16,777,216. Skin metadata is retained
separately from the core mesh and is discarded by `into_parts()`.
Morph limits default to 1,024 targets and 16,777,216 VEC3 attribute elements per
primitive. Both input and mapped output attribute counts must fit the latter
limit. `into_parts()` also discards Morph inputs.
`tex_coord_limit` defaults to 16,777,216 coordinate pairs across all sets,
including implicit set zero. It bounds input and worst-case triangle-corner
workspace before attribute decoding. When generating normals or tangents, the
larger of input vertex count and expanded index count is charged per set, even
if the final mesh can share vertices.
These are element limits, not an exact process-memory budget. Resource
byte limits remain independent and apply during document preparation.

Errors retain mesh/primitive context and relevant accessor, vertex, or triangle
details. Failed conversion leaves prepared resources unchanged for another
conversion attempt.
