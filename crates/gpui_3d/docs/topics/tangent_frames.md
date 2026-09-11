# GPU corner tangent frames

`GpuTangentFrames` computes angle-weighted frames from a
[`GpuTangentGroupsOutput`](tangent_groups.md), using the native `wgpu` feature.
It retains the connected groups, face derivatives, welded corners, and exact
deformed vertex snapshot. Evaluation does not read vertices back to the CPU.

```rust
# use gpui_3d::{GpuDeformationLimits, GpuTangentFrames, GpuTangentGroupsOutput};
# fn frames(groups: &GpuTangentGroupsOutput) -> anyhow::Result<()> {
let faces = groups.adjacency().weld().derivatives();
let source = GpuTangentFrames::new(
    faces.context().clone(),
    faces.base_mesh().clone(),
    faces.uv_set(),
    GpuDeformationLimits::default(),
)?;
let frames = source.evaluate(groups)?;
# Ok(())
# }
```

Reuse the source for subsequent groups from the same device, mesh allocation, and
coordinate set. Other inputs are rejected. Outputs remain valid after further
evaluation or source destruction; their exposed buffers are read-only by contract.

## Weighting

Each regular corner projects its two incident edges and face derivative directions
onto the plane orthogonal to its normalized vertex normal. The angle between the
projected unit edges supplies the contribution weight. A zero projected edge uses
a zero direction in the angle calculation.

Contributions are sorted by group representative and original corner. Each
destination corner accepts contributors from its connected group, excluding
exactly opposing projected tangent or bitangent directions. Its own contribution
is always included. Accepted directions are angle-weighted, summed in original
corner order, and normalized independently. Derivative magnitudes are averaged
using the same weights. UV orientation comes from the connected group, not from
the magnitude lanes.

This operation only evaluates regular groups. Ungrouped degenerate corners have
no frame; they are not repaired or assigned a default basis. Degenerate-frame
inheritance, fixed-vertex publication, and imported MikkTSpace regeneration are
separate operations. CPU meshes, bounds, and picking remain unchanged. Do not bind
this corner buffer as render vertices.

## Records

There is one 64-byte `GpuTangentFrame` per original triangle corner:

| Field | Meaning |
| --- | --- |
| `tangent` | Unit projected direction in XYZ; weighted mean derivative magnitude in W. |
| `bitangent` | Independently normalized direction in XYZ; weighted mean magnitude in W. |
| `identity` | Original corner, connected group representative, and UV orientation. |
| `angle_weight` | Total accepted corner angle in radians. |
| `status` | Input failure or arithmetic/frame failure. |

Consume a frame only when status is zero and its group is not `u32::MAX`.
Nonregular corners use `u32::MAX` for both group and orientation, retain their
input status, and contain zero directions and weight. Regular orientations are
0 or 1. A failed contribution rejects every frame in its group. Accumulation
failures are reported per destination: X = 1 reports detected nonfinite arithmetic
and X = 2 reports an undefined regular frame. Remaining status lanes are reserved. Failed frame
directions and weights must not be used.

## Admission and work

`GpuTangentFramesMemory::plan` checks counts and budgets without GPU allocation.
For `C` corners and `P` rounded up to a power of two, initialization and accumulation
each use one pass; sorting uses `log2(P) * (log2(P) + 1) / 2` passes. Source admission
covers one 16-byte uniform per pass. Output admission covers both sort buffers
(`128 * P` bytes total) and the final result (`64 * C` bytes).

Every corner scans only its sorted group. A group with `k` corners requires
`O(k²)` contribution comparisons; high-valence geometry can therefore be expensive.
Accumulation uses no floating-point atomics. Existing input snapshots, CPU
allocations, pipelines, and driver overhead are excluded from payload admission.
Callers separately bound retained results and concurrent evaluations.

Construction requires four storage bindings, one uniform binding, and
64-invocation compute workgroups, with enabled buffer and dispatch limits checked
before allocation. Replacing the device requires rebuilding the source.
