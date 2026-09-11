# GPU tangent groups

`GpuTangentGroups` labels connected regular-face corners from a
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

Corners connect across paired edges only when both faces have regular derivative
frames and matching UV orientation. Each connection preserves the welded vertex.
UV and normal seams, mirror boundaries, and point-only contact remain separate.
Non-manifold connectivity follows the supplied adjacency's ranked edge pairs.

Each group is identified by its minimum original triangle-corner index. Group IDs
are not compact and can change when deformation alters connectivity or eligibility;
they are not persistent application identities.

Frames needing direction inheritance, faces with coincident positions, and failed
faces remain ungrouped. This API does not assign inherited directions, calculate
corner weights, or publish final vertex tangents. CPU geometry and queries remain
unchanged, and imported MikkTSpace regeneration is not enabled by this stage.

Use [`GpuTangentFrames`](tangent_frames.md) to project and angle-weight the regular
groups' face contributions while retaining original corner correspondence.

## Records

The output contains one 64-byte `GpuTangentGroup` per original triangle corner:

| Field | Meaning |
| --- | --- |
| `identity` | Original corner, welded representative, group representative, and UV orientation. |
| `neighbors` | Outgoing and incoming same-vertex neighbor corners, face eligibility, and zero. |
| `reserved` | Four zeros. |
| `status` | Unmodified face status from the paired adjacency. |

Missing neighbors and unassigned group/orientation values use `u32::MAX`.
Eligibility is 0 for regular frames, 1 for frames needing inheritance, 2 for
coincident positions, and 3 for failed faces. Regular orientations are 0 or 1,
matching the face derivative classification. Neighbor indices refer to immediate
connections, not the internal propagation jumps.

## Admission

`GpuTangentGroupsMemory::plan` checks triangle-corner counts and payload budgets
without allocating GPU resources. For `C` corners, the source budget covers a
16-byte uniform. The output budget covers two `64 * C` byte buffers: one scratch
buffer and one returned result. Retained input snapshots, CPU data, and driver
overhead are excluded. Callers bound concurrent evaluations and retained outputs
separately.

Evaluation uses initialization, `ceil(log2(C))` pointer-doubling passes, and
finalization in one command submission. It requires four storage bindings, one
uniform binding, and 64-invocation workgroups. Construction checks enabled buffer
and dispatch limits. Replacing the device requires a new source.
