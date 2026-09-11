# GPU tangent publication

`GpuTangents` converts [corner frames](tangent_frames.md) into fixed-order
`GpuDeformationVertex` records with tangent XYZ and handedness W. It requires
the native `wgpu` feature. Outputs feed Skin, bounds reduction, render vertex
packing, and explicit CPU readback without reading intermediate vertices back.

## Source preparation

Each source vertex must appear exactly once in the index buffer. Index order
may differ from vertex order. Shared and unused vertices are rejected. Prepare
unshared triangle corners and remap Morph targets, Skin influences, and external
attributes before constructing the compute sources.

Construction generates initial CPU tangents with the selected UV set and
`TangentGenerationMode`, then remaps them back to source vertex order. The
retained `output_mesh()` carries this initial basis and its UV-set identity.
Initial generation errors are returned by the constructor. This is source
preparation, not per-evaluation CPU deformation.

The fixed UV topology also retains zero-area classification computed with CPU
`f64` differences and products. Evaluation does not reclassify UV degeneracy from
a rounded GPU determinant.

```rust,no_run
# use gpui_3d::{GpuDeformationLimits, GpuTangentFramesOutput, GpuTangents, TangentGenerationMode};
# fn publish(frames: &GpuTangentFramesOutput) -> anyhow::Result<()> {
let faces = frames.groups().adjacency().weld().derivatives();
let source = GpuTangents::new(
    faces.context().clone(),
    faces.base_mesh().clone(),
    faces.uv_set(),
    TangentGenerationMode::Inherit,
    GpuDeformationLimits::default(),
)?;
let output = source.evaluate(frames)?;
let deformation = output.deformation();
let render_source = deformation.render_source([faces.uv_set(); 5], None)?;
let geometry = deformation.render_geometry(&render_source)?;
# Ok(())
# }
```

Reuse the publisher for frames from the same original mesh allocation, selected
UV set, and device. Evaluation reads the vertex snapshot retained by those frames.
Position, normal, indices, UVs, colors, and vertex order are preserved. The output's
base mesh is `output_mesh()`, not the original allocation. Create render packing
sources and scene mesh bindings with that output mesh identity. CPU bounds and
queries still describe initial geometry until explicitly updated.

## Policies

Every usable corner tangent is normalized and projected against its current
vertex normal. A projected length at or below one millionth of the incoming
direction's length is undefined. UV orientation determines handedness.

| Mode | Behavior |
| --- | --- |
| `Strict` | Requires nonzero geometric area, a nonzero UV determinant, and usable frames for all corners. |
| `Inherit` | Accepts usable inherited frames, including degenerate faces; rejects unresolved corners. |
| `Repair` | Replaces unresolved frames with a projected triangle derivative when usable, otherwise a normal-orthogonal basis. |

Repair chooses the least-aligned normal axis, breaking ties by X, then Y, then Z.
Repaired signs use the first usable frame in triangle-corner order; without one,
they use the UV determinant sign, or positive handedness for a zero determinant.
Existing usable signs are not changed. Incompatible signs within a triangle reject
the entire triangle in every mode.

Input vertex failures, nonfinite arithmetic, and zero normals cannot be repaired.
Failure propagates to every vertex of the triangle. Status X = 1 indicates
nonfinite arithmetic, X = 2 an undefined frame or inconsistent handedness,
X = 4 a zero-area triangle rejected by `Strict`, and X = 5 tangent input outside
the supported numeric range. Original input failures preserve their status vector.
Rejected tangents are zero and must not be consumed.

Numeric admission requires a normal `f32` UV determinant, except exact zero on
a zero-UV face. All three edge squared lengths and both unnormalized derivative
squared lengths must be positive normal `f32` values. Zero vectors are allowed
only on geometrically degenerate or zero-UV faces. Regular faces also require
finite derivative magnitudes. Underflow, overflow, and nonzero subnormal squared
lengths are rejected in every mode before repair. Dynamic geometric degeneracy
uses scaled `f32` cross products; CPU geometry classification uses wider arithmetic.

`GpuTangentsOutput::repair_buffer()` is a read-only storage/copy-source buffer
containing one `u32` per original source vertex:

| Value | Meaning |
| --- | --- |
| 0 | Generated or inherited frame; also used for failed vertices. Inspect vertex status separately. |
| 1 | Repaired with the triangle derivative. |
| 2 | Repaired with a normal-orthogonal basis. |

Successful repairs have zero vertex status and remain drawable. Repair tags do
not occupy reserved vertex status lanes. `into_deformation()` transfers the
canonical output and drops the repair buffer; borrow `deformation()` to retain
both. Subsequent Skin or packing operations consume the canonical records, not
the repair buffer.

## Admission and lifetime

For `V` vertices, source admission covers `16 * V` topology bytes and a 16-byte
uniform. Output admission covers `64 * V` vertex bytes plus `4 * V` repair bytes.
`GpuTangentsMemory::plan` performs payload checks without GPU allocation. Initial
CPU mesh generation, retained inputs, pipelines, and driver overhead are excluded.
Callers separately bound retained outputs and concurrent evaluations.

Construction requires five storage bindings, one uniform binding, and
64-invocation compute workgroups. Each invocation owns one triangle; the fixed
corner mapping makes its output vertices disjoint. Evaluation uses one linear
compute pass and returns independent buffers that may outlive the source.
Rebuild compute sources after replacing the device.

The GPU pipeline uses `f32` arithmetic and its documented welding and inheritance
rules. CPU MikkTSpace equivalence is not guaranteed. Imported primitives requiring
MikkTSpace regeneration remain rejected by the glTF GPU adapter.
