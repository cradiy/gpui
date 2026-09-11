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

Projected vectors and accumulated directions use an unscaled f32 squared length
and reciprocal-square-root normalization. Vectors whose components are all at
most `f32::MIN_POSITIVE` in magnitude retain their components. When normalization
is required, a squared length that rounds to zero makes it undefined; the vector
is not rescaled into a unit direction.

Contributions are sorted by group representative and original corner. Each
destination corner accepts contributors from its connected group, excluding
exactly opposing projected tangent or bitangent directions. Its own contribution
is always included. Accepted directions are angle-weighted, summed in original
corner order, and normalized independently. Derivative magnitudes are averaged
using the same weights. UV orientation comes from the connected group, not from
the magnitude lanes.

A zero accumulated bitangent does not invalidate a usable tangent. Its direction
remains zero; canonical tangent publication uses the tangent and orientation to
define the basis. A zero accumulated tangent or zero angle weight is undefined.
Undefined projected directions do not match regular neighboring subgroups.
An accepted contribution with an undefined angle or tangent makes the destination
frame undefined, allowing explicit publication repair. An undefined bitangent
alone does not invalidate the encoded tangent; its unusable direction is zero.

Assigned corners with undefined derivative frames accept every valid contributor
in their group, without adding their own direction or angle weight. This also
allows otherwise separated regular contributions to connect through an undefined
frame with a compatible inherited orientation. Unassigned corners remain unresolved.

## Collapsed-face inheritance

After regular frames are evaluated, faces containing coincident positions can
inherit them. Each corner selects the smallest original corner index on a
noncollapsed face with the same [welded position, normal, and UV key](tangent_weld.md).
An integer minimum reduction makes donor selection independent of workgroup order.
No edge connection is required. Attribute seams still prevent matching.

Inherited frames copy the donor's directions, magnitudes, orientation, weight,
source identity, and status. Donor selection does not skip undefined or failed
frame accumulation in favor of a later usable frame. Undefined donors remain
undefined until explicit publication repair; arithmetic failures remain failures.
Faces with failed input geometry and failed destinations do not participate.
Collapsed faces never donate to another corner. If no donor exists, the destination
remains unresolved; no default basis is substituted. Corners of one collapsed
triangle can inherit different orientations, which final vertex publication must
validate rather than silently changing the signs.

Faces with distinct positions and undefined UV frames use their assigned groups
instead of this donor rule. [Frame repair and fixed-vertex publication](tangent_publication.md)
consume the corner output separately from imported MikkTSpace regeneration. CPU meshes,
bounds, and picking are unchanged. Do not bind this corner buffer as render vertices.

## Records

There is one 64-byte `GpuTangentFrame` per original triangle corner:

| Field | Meaning |
| --- | --- |
| `tangent` | Unit projected direction in XYZ; weighted mean derivative magnitude in W. |
| `bitangent` | Independently normalized direction in XYZ; weighted mean magnitude in W. |
| `identity` | Frame source corner, source group representative, and UV orientation. |
| `angle_weight` | Total accepted corner angle in radians. |
| `status` | Input failure or arithmetic/frame failure. |

Consume a frame only when status is zero and its group is not `u32::MAX`.
Array indices are destination corners; `identity[0]` differs only for inherited
frames, including unresolved inherited frames. Unassigned corners use `u32::MAX`
for both group and orientation, retain input or donor status, and contain zero
directions and weight. Regular orientations are 0 or 1. A contribution with an
input or detected nonfinite projection failure rejects every frame in its group.
Undefined normalization is handled within accepted subgroups. Accumulation
failures are reported per destination: X = 1 reports detected nonfinite arithmetic
and X = 2 reports an undefined accumulated frame. These statuses also propagate
through collapsed-face inheritance. Remaining status lanes are reserved.
Failed frame directions and weights must not be used.

## Admission and work

`GpuTangentFramesMemory::plan` checks counts and budgets without GPU allocation.
For `C` corners and `P` rounded up to a power of two, initialization and accumulation
each use one pass; sorting uses `log2(P) * (log2(P) + 1) / 2` passes. Source admission
covers those 16-byte uniforms. Three further passes clear donors, select donors,
and apply inheritance, reusing the accumulation uniform. Output admission covers
both sort buffers (`128 * P` bytes total), the donor map (`max(4 * C, 64)` bytes),
and the final result (`64 * C` bytes). `scratch_bytes` includes the donor map;
`donor_bytes` reports its size separately. Accumulation reuses one sort buffer.

Every corner scans only its sorted group. A group with `k` corners requires
`O(k²)` contribution comparisons; high-valence geometry can therefore be expensive.
Accumulation uses no floating-point atomics. Donor selection and inheritance each
visit every corner once. Existing input snapshots, CPU allocations, pipelines,
and driver overhead are excluded from payload admission.
Callers separately bound retained results and concurrent evaluations.

Construction requires four storage bindings, one uniform binding, and
64-invocation compute workgroups, with enabled buffer and dispatch limits checked
before allocation. Replacing the device requires rebuilding the source.
