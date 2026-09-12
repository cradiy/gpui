# GPU smooth normals

`GpuSmoothNormals` rebuilds area-weighted normals from a
`GpuDeformationOutput`. Shared vertex indices accumulate incident face cross
products in triangle order. Coincident positions with distinct indices remain
separate shading boundaries. The operation preserves vertex order, indices,
positions, UV sets, vertex colors and the source mesh identity.

```rust
let normals = GpuSmoothNormals::new(
    context,
    input.base_mesh().clone(),
    GpuDeformationLimits::default(),
)?;
let output = normals.evaluate(input)?;
let source = output.render_source([0; 5], None)?;
let packed = output.render_geometry(&source)?;
```

Retain the normal source across evaluations with the same topology and base mesh
allocation. Each evaluation returns an independent output; it can consume Morph,
Skin or external deformation results and feed further geometry processing or
render packing without vertex readback. Bounds and CPU queries are not updated.

## Inputs and arithmetic

The device must enable `SHADER_F64` and satisfy the compute/storage limits checked
by `GpuSmoothNormals::check_support`. Positions are decoded from finite f32 bits
and widened before subtraction. Cross products, ordered accumulation and
normalization use f64; normalized components are rounded to f32. This retains
area weighting across large scale differences without float atomics. GPU results
are not guaranteed bit-identical to CPU normal generation.

The source must have no tangents. Rebuild any required tangent basis after
changing normals; smooth-normal reconstruction does not update tangent frames.
CPU `NormalMode::Smooth` removes unused vertices and can reorder the output.
`GpuSmoothNormals` keeps every vertex slot: unreferenced vertices retain their
complete input records, including normals and status.

Use [GPU deformation remapping](deformation_remapping.md) to expand the evaluated
smooth normals into triangle corners before GPU tangent generation, without
reading vertex data back to the CPU.

Every referenced triangle must have nonzero area. Degenerate faces fail even if
other incident faces would provide a valid normal. Opposite contributions that
cancel exactly also fail. Status X is one for a nonfinite position, two for a
cancelled normal and four for a zero-area face. Existing failures on a vertex or
an incident face propagate. Readback and render packing use the standard
[deformation status contract](deformation.md#outputs-and-cpu-queries).

## Memory and lifetime

`GpuSmoothNormalsMemory::plan(vertices, indices, limits)` validates payloads
without a device. Source storage includes a CSR adjacency array, the original
index list and a 16-byte uniform. For V vertices and I indices, adjacency uses
`4 * (V + 1 + I)` bytes and indices use `4 * I` bytes. Each evaluation allocates
`64 * V` output bytes. There are no per-evaluation topology or scratch buffers.

Source and output limits are independent. CPU adjacency construction, input
buffers, pipelines, driver overhead, readbacks and other retained evaluations
are excluded. Each vertex gathers only its incident faces, so total traversal
is linear in the index count; high-valence vertices require longer individual
invocations. The caller owns admission across concurrent sources and results.

Construction rejects unsupported devices and source tangents. Evaluation rejects
foreign devices and different base mesh allocations before binding. Destroying
the operator does not invalidate previously returned outputs.
