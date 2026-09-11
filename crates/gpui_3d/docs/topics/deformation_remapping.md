# GPU deformation remapping

`GpuDeformationRemap` copies evaluated GPU vertex records through a retained
output-to-source index map. It supports vertex duplication, reordering and
subsets without reading vertices back to the CPU. The destination mesh supplies
indices, UV sets, vertex colors and tangent-coordinate metadata. Positions,
normals, tangent lanes and status words are copied bit-for-bit from the input.

## Corner expansion

Prepare CPU topology once, then reuse the mapping for each evaluated result:

```rust
let expanded = base.expand_corners(vertex_limit)?;
let remap = GpuDeformationRemap::new(
    context.clone(),
    base.clone(),
    expanded.mesh().clone(),
    expanded.source_vertices(),
    limits,
)?;

let corners = remap.evaluate(&deformed)?;
```

The result's `base_mesh()` is the supplied destination allocation. It can feed
`GpuFlatNormals`, `GpuTangentGeneration`, Skin or render packing whose source
metadata uses that allocation and vertex order.

For shared indexed geometry, compute `GpuSmoothNormals` before expanding the
evaluated result. The copied corner normals preserve smooth shading while the
corner layout permits tangent generation. Build `GpuTangentGeneration` against
`expanded.mesh()` and pass `corners` to its `evaluate` method. Smooth normal and
tangent generation require enabled `SHADER_F64`; remapping itself does not.

## Mapping contract

Each destination vertex requires exactly one valid source index. Repeated indices
are allowed; unused source vertices are omitted. The mapping does not interpolate,
weld, transform coordinates, recompute directions or verify geometric equivalence
between the CPU source and destination. Supply destination topology and static
attributes that match the intended mapping and coordinate space. Remap Skin
influences and custom attribute streams separately when needed.

Destination tangent metadata may be absent. If present, its coordinate-set ID
must match the source's tangent metadata. Declaring a tangent basis on a source
without tangents, or relabeling it as another coordinate set, is rejected. Removing
destination tangent metadata does not modify the copied tangent words; consumers
ignore them when the destination has no tangent basis.

Invalid source records are copied, not repaired. Failure statuses propagate only
to vertices that reference them. Consumers such as render packing and deformation
readback perform their usual validation. CPU meshes, bounds and query indices
remain unchanged; recompute output bounds explicitly before publication.

## Admission and ownership

`GpuDeformationRemapMemory::plan` admits positive u32 source/output counts, the
retained mapping plus a 16-byte uniform, and one 64-byte record per output vertex.
The source budget excludes the existing input vertex buffer. Input buffers,
CPU meshes, pipelines, driver overhead, readbacks and other retained outputs are
not included. Construction also checks every mapping entry and enabled device
storage/dispatch limits before allocation.

Evaluation requires the same device and exact source mesh allocation. Every call
allocates an independent output, including identity mappings. Outputs survive
operator destruction and later evaluations. External producers follow the
[deformation buffer ownership contract](deformation.md#outputs-and-cpu-queries).
