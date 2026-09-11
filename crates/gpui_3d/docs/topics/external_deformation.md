# External GPU deformation

With the native `wgpu` feature, application compute results can become
`GpuDeformationOutput` values without CPU vertex readback.

- `from_buffer(context, mesh, buffer, limits)` retains the supplied buffer without
  copying or submitting work. It requires `STORAGE | COPY_SRC` usage.
- `copy_from_buffer(context, mesh, &buffer, limits)` submits a GPU copy into an
  independent result. The source only requires `COPY_SRC` usage. Later queue-ordered
  writes can reuse that source without changing the result.

Both constructors require the complete buffer to contain exactly one
`GpuDeformationVertex` per base-mesh vertex, in the same order. Subranges, interleaved
application records, and alternate strides must be converted before adoption.
The mesh retains topology, UV sets, colors, and tangent presence. Adoption does not
infer a different topology or modify CPU geometry.

## Record layout

Each record occupies 64 bytes:

```wgsl
struct Vertex {
    position: vec4<f32>,
    normal: vec4<f32>,
    tangent: vec4<f32>,
    status: vec4<u32>,
}
```

Position and normal use XYZ, with zero padding in W. Tangent W holds handedness;
use a zero tangent record when the base mesh has no tangents. All floating-point
lanes must be finite. Successful vertices use an all-zero status. Any nonzero
status reports invalid output; constructors preserve status rather than clearing it.
Use the status codes documented by `GpuDeformationVertex` for the corresponding errors.

Constructors validate size, usages, payload limits, and device ownership, not
record contents. Producers are responsible for writing every record correctly.
Bounds reduction checks positions and status; a valid bound does not certify
normals or tangents. Render packing additionally checks full floating-point records
and tangent consistency and suppresses invalid draws. CPU readback checks status
and validates the resulting mesh.

## Ordering and lifetime

Use the same `WgpuContext` for producers and consumers. Submit producer commands
before calling either constructor. No CPU wait for producer completion is needed:
queue ordering places subsequent copies and consumers after those commands.
The constructors do not submit caller-owned encoders or synchronize external queues.

Direct adoption transfers a read-only usage contract, not exclusive ownership of
the underlying GPU allocation. Do not write, map, or destroy an adopted buffer while
the result or queued consumers may use it. WGPU handle clones do not copy its contents.
Use the copy constructor for mutable or pooled producer buffers. Returned snapshot
buffers are also read-only by contract, including handles obtained from `buffer()`.

The result retains its context, base mesh, and buffer. `context()` exposes its device
and queue for compatible processing. Recreate resources after device replacement.
Both source and output payload limits apply to the full attribute buffer. A copied
result additionally occupies one buffer of that size; limits do not bound aggregate
residency across retained frames or account for the CPU mesh and driver overhead.

## Processing and publication

The result works with existing Skin composition, flat-normal reconstruction,
tangent preprocessing, bounds reduction, and render packing under each operation's
topology and device requirements. Render sources can be retained and reused across
outputs that share the same base mesh and material UV selections.

```rust
let output = GpuDeformationOutput::from_buffer(context, mesh, attributes, limits)?;
let source = output.render_source([0; 5], None)?;
let geometry = output.render_geometry(&source)?;
```

Creating a result does not replace scene geometry or publish new bounds. Pair the
packed geometry with conservative bounds for that same result before viewport or
headless submission. Bounds readbacks retain their input version; keep application
frame identities with pending requests and reject stale results. See
[GPU deformation](deformation.md) for processing and viewport attachment.
