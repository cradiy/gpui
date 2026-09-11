# GPU deformation

[3D viewports](../viewport.md) · [CPU Morph and Skin](animation.md)

## Morph computation

With the native `wgpu` feature, `GpuMorph` uploads a `MorphTargets` binding to
one `WgpuContext`. Base attributes, target deltas, and the compute pipeline are
retained across evaluations. `evaluate(weights)` validates the weight count and
finiteness, submits a compute dispatch, and returns a fresh `GpuDeformationOutput`.
Signed weights and empty target lists are supported. Earlier outputs are never
overwritten by later evaluations and may outlive the uploaded source object.

```rust
use gpui_3d::{GpuDeformationLimits, GpuMorph, Mesh, MorphTarget, MorphTargets, WgpuContext};

let mesh = Mesh::plane();
let targets = MorphTargets::new(mesh.clone(), [MorphTarget {
    positions: Some(vec![[0., 0., 0.2]; mesh.vertex_count()].into()),
    ..Default::default()
}])?;
let context = WgpuContext::new_headless()?;
let gpu = GpuMorph::new(context, targets, GpuDeformationLimits::default())?;
let output = gpu.evaluate(&[0.5])?;
let deformed_mesh = output.readback()?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

The shader blends position, normal, and tangent deltas. Normals with active
deltas are normalized; tangents are orthogonalized against the resulting normal,
preserving handedness. All-zero weights copy the base attributes. GPU arithmetic
uses `f32`, unlike the CPU evaluator's widened intermediates, so cancellation,
overflow, and tiny values can produce different results. CPU evaluation remains
available through `source().evaluate(weights)`; no automatic fallback is performed.

## Skin computation

`GpuSkin` uploads a validated `Skin` influence binding. Each vertex retains its
variable-length influence list, including repeated joint indices. There is no
fixed four-weight limit. Normalized weights are converted to `f32`; values below
the normal `f32` range are rejected during upload. The CPU binding retains its
original `f64` weights and remains available through `source()`.

`palette(mesh_world, joint_world)` composes and validates
`inverse(mesh_world) * joint_world * inverse_bind` on the CPU, then uploads the
matrices. The returned `GpuSkinPalette` is immutable and can be reused for multiple
inputs evaluated by the same `GpuSkin`. Updating a pose creates a new palette;
earlier palettes and outputs are not overwritten. A palette from a different
`GpuSkin` is rejected, even if the joint counts match.

```rust
use gpui_3d::{
    AffineTransform, GpuDeformationLimits, GpuDeformationOutput, GpuSkin,
    Mesh, Skin, SkinInfluence, WgpuContext,
};

let context = WgpuContext::new_headless()?;
let mesh = Mesh::plane();
let binding = Skin::new(
    [AffineTransform::IDENTITY],
    (0..mesh.vertex_count()).map(|_| [SkinInfluence { joint: 0, weight: 1. }]),
)?;
let limits = GpuDeformationLimits::default();
let skin = GpuSkin::new(context.clone(), binding, limits)?;
let input = GpuDeformationOutput::upload(context, mesh, limits)?;
let palette = skin.palette(
    AffineTransform::IDENTITY,
    &[AffineTransform::from_translation([0., 0.2, 0.])?],
)?;
let output = skin.evaluate(&input, &palette)?;
let deformed_mesh = output.readback()?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

Pass a `GpuMorph::evaluate` result directly to `GpuSkin::evaluate` for Morph-before-Skin
composition. Both objects must use the same device. No vertex readback or re-upload
occurs between these stages. Vertex count is checked; matching semantic vertex
order remains the caller's responsibility. Inputs must be bind-space or morphed
geometry, not previous skinned results.

The shader blends affine matrices per vertex, transforms normals with the blended
inverse transpose, and transforms tangents with the blended linear matrix and
reflection-aware handedness. Singular or unrepresentable blended transforms
produce an error status. Normal/tangent directions are normalized, and tangents
are orthogonalized against normals. Input error status is preserved across Skin
evaluation. All vertex arithmetic uses `f32`; CPU evaluation is the reference
path when widened accumulation or final CPU geometry is required. Triangle-wide
tangent handedness consistency is checked when materializing a CPU mesh, not by
the compute dispatch.

## Outputs and CPU queries

`GpuDeformationOutput::buffer()` exposes storage/vertex/copy-source data in source vertex
order. Each 64-byte `GpuDeformationVertex` contains position, normal, tangent,
and status vectors at offsets 0, 16, 32, and 48. Attribute XYZ uses the first
three lanes; tangent W is handedness. Indices, UV sets, and vertex colors remain
in `base_mesh()` and are not duplicated in this buffer. External consumers must
preserve the buffer and inspect status before using results. They must not mutate
the buffer through another GPU binding.

Status X is zero for a valid record, one for detected nonfinite arithmetic,
two for an undefined tangent frame, and three for a singular or unrepresentable
blended Skin transform. Other lanes are reserved and zero. Submission
validation does not prove that the computed attributes are valid.

`readback()` waits for the GPU, checks record status, validates mesh attributes,
and returns a new CPU mesh with recomputed bounds and a fresh lazy query index.
Indices, UV sets, vertex colors, and tangent coordinate-set identity are preserved.
Existing meshes, scenes, and queries are not modified. Pass the materialized mesh
to scene objects or snapshot mesh overrides for matching CPU spatial queries.
This path includes a GPU-to-CPU copy and is not a zero-copy render path.

Readback is synchronous and waits up to 30 seconds for device completion, followed
by up to one second for the mapping callback. Use it outside the interactive render
loop. Retaining only the GPU buffer does not synchronize CPU bounds or picking.
Device loss, weight mismatches, malformed output, and invalid mesh attributes
are returned as errors.

## Render vertex packing

`output.render_source(uv_sets, byte_limit)` creates a reusable
`WgpuScene3dGeometry` containing static material attributes and indices.
The five coordinate sets select base color, metallic/roughness, emission, normal,
and occlusion inputs. `output.render_geometry(&source)` combines those inputs
with GPU positions, normals, and tangents, returning a `Scene3dGpuGeometry` without
CPU readback. Reuse the source across outputs from the same base mesh allocation
and device; mismatches are rejected.

```rust
# use gpui_3d::{GpuDeformationOutput, WgpuScene3dGeometry};
# fn pack(output: &GpuDeformationOutput) -> anyhow::Result<()> {
let source = output.render_source([0; 5], Some(64 * 1024 * 1024))?;
let geometry = output.render_geometry(&source)?;
# Ok(())
# }
```

Packed vertex layout matches `Scene3dGpuGeometry::vertex_layout()`. UVs and vertex
colors are preserved; vertices and indices have `VERTEX` and `INDEX` usages.
The `draw()` buffer supplies `draw_indexed_indirect` arguments at offset zero,
with first instance zero. Keep the pipeline's per-object instance binding aligned
with that index. Buffers are immutable by contract. Earlier results survive later
evaluations and destruction of the source.

The packing dispatch disables the entire draw if any attribute record has a
nonzero status, nonfinite values, an undefined tangent frame, or inconsistent
triangle tangent handedness. It does not replace invalid geometry with the bind
pose. This check stays on the GPU and is not reported as a synchronous `Err`.
The original deformation output remains available for explicit diagnostic
readback. Consumers must use the indirect arguments to honor this draw suppression.

## Direct rendering

`HeadlessRenderer::render_with_geometry(scene, config, draws)` accepts packed
outputs without vertex readback. Each `Scene3dGpuDraw` supplies an `output_id`,
an `Arc<Scene3dGpuGeometry>`, and conservative mesh-local `bounds` as `[min, max]`.
IDs are scene object indices plus one. Geometry must share the object's base mesh
allocation, active material coordinate sets, and renderer device.

Bounds are finite and ordered. They control camera and shadow culling and
transparent-object sorting, including objects whose original mesh is outside the
camera. Each overridden object draws independently; shared source meshes can use
different deformation outputs. Color, shadows, IDs, depth, and normals consume the
same packed geometry. CPU scene geometry and spatial queries remain unchanged.
Use output ID/depth readback for screen-space selection or materialize meshes for
CPU queries at the deformed pose.

`WgpuScene3dRenderer::render_with_geometry` accepts the same draws for a prepared
`Scene3dFrame`. Its input must still contain every overridden object; objects
removed by earlier preparation cannot be recovered. `Viewport` requires CPU mesh
inputs and does not accept packed overrides.

Rendered geometry memory includes packed vertices, indices, and indirect arguments,
counted once per distinct active packed output. Source upload buffers and deformation
attributes have separate admission limits. Draw statistics count submissions,
including indirect draws suppressed by invalid GPU attributes.

`Scene3dGpuGeometryMemory::plan` reports 96 bytes per vertex for the static source,
another 96 bytes per packed vertex, four bytes per index, and 20 bytes per draw
argument buffer. The optional byte limit covers one source and one output, not
all retained results or the separate 64-byte deformation attributes. Compute,
indirect execution, five storage bindings, and device buffer/dispatch limits are
required. No global cache retains these objects.

## Resource admission

`GpuMorphMemory::plan(vertices, targets, limits)` checks counts and payload budgets
without a GPU. Vertex count must be positive; vertex, target, and flattened-delta
indexing must fit `u32`. It reports base, delta, weight, output, and uniform bytes.
Base and output each use 64 bytes per vertex; deltas use 64 bytes per vertex per
target. Empty delta/weight bindings use one padded record/scalar. Uniforms use
16 bytes. Weights are allocated per evaluation.

`GpuDeformationLimits` defaults to 256 MiB of source payload and 64 MiB per output.
The source budget includes base, deltas, and uniforms. The output limit also bounds
one optional readback staging buffer. For Skin, the source budget covers the
influence binding, one palette, and two 16-byte uniform blocks for meshes with
and without tangents. `GpuSkinMemory::plan` reports binding, palette, output,
and retained uniform bytes. The influence binding
uses `4 * (vertices + 1) + 8 * influences` bytes; each palette uses 64 bytes per
joint. Mesh inputs have independent admission through Morph or
`GpuDeformationOutput::upload`, which checks its attribute payload against both
source and output limits.

Device storage-buffer and dispatch limits
are checked before source packing and GPU allocation. These are per-object payload
limits, not a total residency quota: retained outputs, CPU inputs, staging copies,
concurrent evaluations, and driver overhead consume additional memory. Drop source
objects and outputs when no longer needed; no global cache owns them.

## Validation

Shader/host layout, admission, and readback decoding can be checked without a GPU:

```sh
cargo test -p gpui_3d --features wgpu --lib geometry::gpu_
cargo test -p gpui_wgpu --lib scene3d_renderer::gpu_geometry::tests
```

The compute comparison requires an explicit GPU run:

```sh
cargo test -p gpui_3d --features wgpu --lib compute_morph_matches_cpu -- --ignored
cargo test -p gpui_3d --features wgpu --lib compute_skin_composes_morph -- --ignored
cargo test -p gpui_wgpu --lib gpu_geometry_preserves_material -- --ignored
```
