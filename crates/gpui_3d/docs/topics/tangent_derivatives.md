# GPU surface derivatives

`GpuTangentDerivatives` computes per-triangle surface derivatives from a
`GpuDeformationOutput` and a selected UV set. It uses the native `wgpu` feature.
The source retains indices and coordinates; each evaluation reads deformed
positions and submits an independent face buffer without CPU readback.

```rust
# use gpui_3d::{GpuDeformationOutput, GpuDeformationLimits, GpuTangentDerivatives, WgpuContext};
# fn derivatives(context: WgpuContext, input: &GpuDeformationOutput) -> anyhow::Result<()> {
let source = GpuTangentDerivatives::new(
    context,
    input.base_mesh().clone(),
    0,
    GpuDeformationLimits::default(),
)?;
let faces = source.evaluate(input)?;
# Ok(())
# }
```

The input must belong to the same device and base mesh allocation. Shared vertices,
reordered indices, and nonzero UV sets are supported. Normals, existing tangents,
vertex colors, indices, and the CPU scene are not modified.

## Records

The output buffer contains one 64-byte `GpuTangentDerivative` per source triangle:

| Field | Meaning |
| --- | --- |
| `tangent` | Unit dP/du direction in XYZ and derivative magnitude in W. |
| `bitangent` | Unit dP/dv direction in XYZ and derivative magnitude in W. |
| `classification` | Boolean lanes for zero geometric area, zero UV determinant, positive UV orientation, and an undefined derivative pair. |
| `status` | First failing input corner's status; X = 1 for detected nonfinite input arithmetic, X = 5 for an unsupported derivative squared length or nonfinite magnitude. Other lanes are zero for locally detected failures. |

Inspect `status` before consuming any other field. Degenerate inputs are classified
separately, not discarded or repaired. Undefined derivative pairs contain zero
vectors. Classification and arithmetic use `f32`; this stage does not establish
equivalence with CPU tangent-generation policies at numeric limits.

A regular derivative pair requires both the absolute UV determinant and each
derivative magnitude to exceed `f32::MIN_POSITIVE` (`2^-126`). Values equal to
that boundary remain undefined, even when finite and nonzero. The zero-UV lane
still records exact zero independently of derivative eligibility. Undefined pairs
participate in frame inheritance rather than seeding regular orientation groups.

Eligible nonzero derivatives use an unscaled f32 squared length, square root,
and reciprocal multiplication for direction normalization. Magnitude is the
length divided by the absolute UV determinant. A squared length that is zero,
subnormal, or nonfinite produces status 5 before grouping; it is not rescued by
rescaling. A nonfinite resulting magnitude also produces status 5. This status
remains a failure under every publication repair mode.

These are unprojected triangle derivatives, not final vertex tangents. Welding,
orientation groups, corner-angle weighting, normal projection, and degenerate-frame
inheritance are outside this API. Do not bind its buffer as a renderable mesh or
substitute its directions for MikkTSpace tangents.

Use [GPU tangent welding](tangent_weld.md) to derive exact position/normal/UV
corner matches from the paired input snapshot. Orientation groups and final
tangent generation remain separate operations.

`input_buffer()` retains the exact immutable vertex snapshot used for evaluation.
`base_mesh()` supplies topology and attribute identity; `uv_set()` identifies the
coordinate selection. The result remains valid after later evaluations or source
destruction. Exposed buffers are read-only by contract.

## Admission

`GpuTangentDerivativesMemory::plan` checks counts and payload limits without a GPU.
Source payload is 8 bytes per vertex, 4 bytes per index, and 16 uniform bytes.
Output payload is 64 bytes per triangle. The input buffer, retained CPU mesh, and
driver overhead are excluded; limits are per source/result, not aggregate residency.
Callers bound concurrently retained outputs and their input snapshots.

`check_support` requires compute support, four storage bindings, one uniform
binding, and 64-invocation workgroups. Construction checks enabled storage sizes
and dispatch limits before uploading topology. Device loss and cross-device input
are errors; rebuild sources on the replacement device.
