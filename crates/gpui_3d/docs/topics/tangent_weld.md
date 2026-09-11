# GPU tangent welding

`GpuTangentWeld` builds an exact-match corner map from the vertex snapshot retained
by a [`GpuTangentDerivativeOutput`](tangent_derivatives.md). It uses the native
`wgpu` feature and performs no CPU readback or topology replacement.

```rust
# use gpui_3d::{GpuDeformationLimits, GpuTangentDerivativeOutput, GpuTangentWeld};
# fn weld(faces: &GpuTangentDerivativeOutput) -> anyhow::Result<()> {
let source = GpuTangentWeld::new(
    faces.context().clone(),
    faces.base_mesh().clone(),
    faces.uv_set(),
    GpuDeformationLimits::default(),
)?;
let corners = source.evaluate(faces)?;
# Ok(())
# }
```

Reuse the source across evaluations of the same base mesh and coordinate set.
Cross-device, foreign-mesh, and different-UV inputs are rejected. Every evaluation
reads the paired deformed positions and normals; there is no bind-pose matching
cache. The output retains its derivative buffer and exact input vertex snapshot.

## Matching and records

Keys consist of position XYZ, normalized normal XYZ, and selected UV XY. Matching
uses exact `f32` bits, preserves signed zero, and applies no positional tolerance.
Normals are normalized with scaled `f32` arithmetic. CPU normalization can round
differently; this API alone does not establish CPU MikkTSpace parity.

The buffer contains one 64-byte `GpuTangentWeldRecord` per original triangle corner:

| Field | Meaning |
| --- | --- |
| `key` | Eight scalar bit patterns: position, normalized normal, and UV. |
| `identity` | Original corner, source vertex, earliest matching corner, and failure flag. |
| `status` | Source vertex status; `[1, 0, 0, 0]` for nonfinite arithmetic or `[2, 0, 0, 0]` for a zero normal. |

Corners are numbered in index-buffer order, not vertex-storage order. Repeated
indices and duplicated vertices can share a representative. Failed corners remain
separate and represent themselves. Ignore their keys. Derivative face failures are
retained separately in `derivatives()` and must also be checked by consumers.

UV seams and differing normal directions prevent matching. Matching itself does
not account for triangle adjacency or mirrored orientation: those require separate
connected groups. It does not compute corner weights, smooth tangent directions,
inherit degenerate frames, or publish renderable tangents.

## Work and memory

Evaluation builds corner keys, sorts them with a bitonic network, then resolves the
earliest representative and restores original corner order. For `P` corners rounded
up to a power of two, sorting uses `log2(P) * (log2(P) + 1) / 2` passes and two scratch
buffers. All passes share one command submission; there is no per-pass readback.

`GpuTangentWeldMemory::plan` reports admission without allocating GPU resources.
The source budget covers UVs, indices, and uniform records. The output budget covers
both scratch buffers (`128 * P` bytes total) plus the result (`64 * corners` bytes).
These limits exclude retained input/derivative buffers, CPU data, and driver
overhead. Bound concurrent evaluations and retained results separately.

`check_support` checks compute support, four storage bindings, one uniform binding,
and 64-invocation workgroups. Construction also checks enabled buffer sizes and
dispatch limits. Device replacement requires rebuilding the source.
