# GPU tangent groups

`GpuTangentGroups` labels connected tangent-space corners from a
[`GpuTangentAdjacencyOutput`](tangent_adjacency.md). It requires the native `wgpu`
feature and retains the exact adjacency, weld map, derivatives, and deformed vertices.

```rust
# use gpui_3d::{GpuDeformationLimits, GpuTangentAdjacencyOutput, GpuTangentGroups};
# fn group(edges: &GpuTangentAdjacencyOutput) -> anyhow::Result<()> {
let faces = edges.weld().derivatives();
let source = GpuTangentGroups::new(
    faces.context().clone(),
    faces.base_mesh().clone(),
    faces.uv_set(),
    GpuDeformationLimits::default(),
)?;
let groups = source.evaluate(edges)?;
# Ok(())
# }
```

The source accepts subsequent snapshots from the same device, mesh allocation, and
UV set. Evaluation performs no CPU readback. Earlier outputs remain valid after
subsequent evaluations or source destruction. Exposed buffers are read-only by contract.

## Connectivity

Corners connect across paired edges with matching UV orientation. Each connection
preserves the welded vertex. Regular faces retain their derivative orientation.
UV and normal seams, mirror boundaries, and point-only contact remain separate.
Non-manifold connectivity follows the supplied adjacency's ranked edge pairs.

Each group is identified by its first regular seed in original corner order. Group IDs
are not compact and can change when deformation alters connectivity or eligibility;
they are not persistent application identities.

Faces with undefined derivative frames can join a group. The first regular seed
to reach such a face fixes its orientation for all three corners. Subsequent
groups with the opposite orientation cannot cross that face. This rule applies
before corner averaging; a face cannot choose conflicting orientations at its
different vertices. A corner with no reachable, orientation-compatible regular
seed stays unassigned. Assigned undefined corners can precede their regular seed
in index order, so the group representative is not necessarily its smallest corner.

Faces with coincident positions and failed faces remain ungrouped. This API does
not calculate direction averages or publish vertex tangents. CPU geometry and
queries remain unchanged; imported MikkTSpace regeneration is a separate operation.

Use [`GpuTangentFrames`](tangent_frames.md) to project and angle-weight the groups'
valid face contributions while retaining original corner correspondence.

## Records

The output contains one 64-byte `GpuTangentGroup` per original triangle corner:

| Field | Meaning |
| --- | --- |
| `identity` | Original corner, welded representative, group representative, and UV orientation. |
| `neighbors` | Outgoing and incoming same-vertex neighbor corners, face eligibility, and zero. |
| `reserved` | Four zeros. |
| `status` | Unmodified face status from the paired adjacency. |

Missing neighbors and unassigned group/orientation values use `u32::MAX`.
Eligibility is 0 for regular frames, 1 for undefined derivative frames, 2 for
coincident positions, and 3 for failed faces. Regular orientations are 0 or 1,
matching the seed face's derivative classification. Eligibility records the input
classification even after successful assignment. Neighbor indices refer to immediate
connections, not the internal propagation jumps.

## Admission

`GpuTangentGroupsMemory::plan` checks triangle-corner counts and payload budgets
without allocating GPU resources. For `C` corners, the source budget covers a
16-byte uniform. The output budget covers two `64 * C` byte buffers and a
`max(4 * ceil(C / 64), 64)` byte inheritance flag buffer. `scratch_bytes` includes
one corner buffer and the flags; `output_bytes` covers the returned corner buffer.
Retained input snapshots, CPU data, and driver
overhead are excluded. Callers bound concurrent evaluations and retained outputs
separately.

Regular-only input uses initialization, `ceil(log2(C))` pointer-doubling passes,
and finalization. A parallel detection pass flags workgroups containing undefined
frames. When present, an ordered GPU traversal rebuilds assignments using the
seed priority above; each corner is queued at most once. The traversal is serial
`O(C)` work and may be costly for large meshes. No CPU readback or recursion is used.
The regular-only path only scans the compact detection flags in this stage.

All passes share one submission. Four storage bindings, one uniform binding,
64-invocation workgroups, and four bytes of workgroup storage are required.
Construction checks enabled buffer and dispatch limits. Device replacement
requires a new source.
