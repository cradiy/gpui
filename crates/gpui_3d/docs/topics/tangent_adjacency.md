# GPU tangent adjacency

`GpuTangentAdjacency` pairs directed triangle edges from a
[`GpuTangentWeldOutput`](tangent_weld.md). It uses the native `wgpu` feature and
retains the exact corner map, face derivatives, and vertex snapshot used for pairing.

```rust
# use gpui_3d::{GpuDeformationLimits, GpuTangentAdjacency, GpuTangentWeldOutput};
# fn adjacent(weld: &GpuTangentWeldOutput) -> anyhow::Result<()> {
let faces = weld.derivatives();
let source = GpuTangentAdjacency::new(
    faces.context().clone(),
    faces.base_mesh().clone(),
    faces.uv_set(),
    GpuDeformationLimits::default(),
)?;
let edges = source.evaluate(weld)?;
# Ok(())
# }
```

The source can be reused across samples with the same device, mesh allocation, and
UV set. Every evaluation rebuilds adjacency from the supplied weld result without
CPU readback. Earlier results remain valid after subsequent submissions or source
destruction. Exposed buffers are read-only by contract.

## Pairing

An edge starts at a triangle corner and ends at the next corner of that triangle.
Edges match only when their welded endpoints occur in opposite order. Sharing a
single vertex is insufficient. UV and normal seams remain separated by the weld map.

For each undirected endpoint pair, edges in each direction are ranked by original
corner order. Equal ranks in opposite directions are paired. This handles edges
shared by more than two faces deterministically; excess edges remain unpaired.
Pairing occurs before orientation checks, so mirror boundaries do not redirect
an edge to a different face.

Faces with coincident vertex positions, including signed-zero-equivalent positions,
are excluded. Distinct collinear positions are not excluded by area alone. Undefined
UV/derivative frames retain adjacency for inheritance. Any failing corner or face
derivative excludes all three edges of that face. Excluded edges remain in the output.

## Records

The buffer contains one 64-byte `GpuTangentEdge` per original triangle corner:

| Field | Meaning |
| --- | --- |
| `edge` | Starting corner, welded start, welded end, and opposite edge's starting corner. |
| `adjacency` | Neighbor corners matching start/end, regular-face orientation compatibility, and face eligibility. |
| `classification` | Original face derivative classification. |
| `status` | First failed corner status in the face, otherwise its derivative status. |

Missing edge/corner references use `u32::MAX`. Eligibility is 0 for regular frames,
1 for frames needing inheritance, 2 for coincident positions, and 3 for failed faces.
Orientation compatibility is true only when both frames are regular and have the
same UV orientation. No orientation is assigned to a frame needing inheritance.

[`GpuTangentGroups`](tangent_groups.md) consumes this output to label connected
regular corners. Adjacency does not select inherited frames, compute angle weights,
or publish vertex tangents. CPU and GPU edge pairing use face order for opposite
directions; complete tangent generation also depends on the subsequent frame stages.

## Admission

`GpuTangentAdjacencyMemory::plan` checks counts and budgets without GPU allocation.
The source budget covers all pass uniforms. The output budget covers two power-of-two
sort buffers (`128 * padded_edges` bytes total) and one result (`64 * edges` bytes).
Retained weld, derivative, and vertex buffers and driver overhead are excluded.
Callers bound concurrent evaluations and retained results separately.

Evaluation uses bounded bitonic sorting and rank lookup in one command submission.
It requires compute support, four storage bindings, one uniform binding, and
64-invocation workgroups. Construction checks enabled buffer and dispatch limits;
device replacement requires a new source.
