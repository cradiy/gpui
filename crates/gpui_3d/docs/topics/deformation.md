# GPU deformation

[3D viewports](../viewport.md) · [CPU Morph and Skin](animation.md)

Application-produced buffers use the [external deformation](external_deformation.md)
adoption and GPU snapshot interfaces.

[GPU vertex streams](vertex_streams.md) update render-source UVs and colors
independently of deformation records.

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

## Flat normal reconstruction

`GpuFlatNormals` retains triangle topology and rebuilds face normals from a
`GpuDeformationOutput` on the same device. Each vertex must appear exactly once
in the index buffer; shared or unused vertices are rejected. Index order may
differ from vertex order. Prepare triangle-corner geometry and remap external
Morph/Skin attributes before creating the compute sources.

The input must use the same base mesh allocation and have no tangents. Evaluation
preserves vertex order, positions, indices, coordinate sets, and colors, and
returns an independent output. Existing vertex failures propagate to every
corner of their triangle. Zero-area faces and nonfinite arithmetic produce error
status; they are not dropped or replaced with an arbitrary normal.

```rust,no_run
# use gpui_3d::{GpuDeformationOutput, GpuFlatNormals, GpuDeformationLimits, WgpuContext};
# fn rebuild(context: WgpuContext, morphed: &GpuDeformationOutput) -> anyhow::Result<GpuDeformationOutput> {
let normals = GpuFlatNormals::new(
    context, morphed.base_mesh().clone(), GpuDeformationLimits::default(),
)?;
let output = normals.evaluate(morphed)?;
# Ok(output)
# }
```

Reuse the source across samples. Run reconstruction after position Morph and
before Skin when the asset requires generated face normals. This operation does
not generate smooth normals or MikkTSpace tangents. It runs even for zero Morph
weights; callers preserving an authored zero-weight base may bypass it.

`GpuFlatNormalsMemory::plan` admits two four-byte topology entries per vertex,
a 16-byte uniform, and a 64-byte output record per vertex. The input buffer is
separately owned. Constructors check enabled compute limits and topology before
GPU allocation. GPU calculations use `f32`, with different numerical limits from
the CPU normal generator's widened arithmetic. No CPU query or bound is updated.

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
two for an undefined tangent frame, three for a singular or unrepresentable
blended Skin transform, and four for a zero-area face during flat normal
reconstruction. Other lanes are reserved and zero. Submission
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

### Nonblocking readback

`request_readback(max_staging_bytes)` returns an owned `GpuDeformationReadback`.
The optional staging limit is checked before allocation; `staging_bytes()` reports
the admitted payload. Each request owns its own staging buffer and may outlive the
deformation output and compute source. Concurrent requests are independent;
the caller controls their count and total memory use.

`try_read()` pumps device callbacks without waiting for GPU completion. It returns
`Ok(None)` while pending and `Ok(Some(mesh))` once the attributes have been validated
and materialized. CPU decoding and mesh validation occur during the ready call and
scale with vertex count. Use a worker thread when this work exceeds the interaction
budget. Success or failure releases staging resources; subsequent calls return an
error. Dropping a pending request cancels mapping and releases its resources without
canceling already submitted GPU commands.

The returned mesh has fresh bounds and query state. Publishing it to a scene or
pose snapshot is explicit; no existing scene or query is mutated. Keep the last
accepted snapshot while waiting, and associate requests with application revisions
when older results must not replace newer poses.

### Bounds readback

`GpuDeformationBounds` reduces mesh-local bounds on the GPU. Reuse the reducer
for outputs on the same device. Each request allocates a 32-byte result buffer
and 32-byte staging buffer, independently of vertex count. The optional working
byte limit covers both buffers, excluding the existing input, pipeline, workgroup
memory, and driver overhead. The caller controls concurrent requests and total
residency.

```rust,no_run
# use gpui_3d::{GpuDeformationBounds, GpuDeformationOutput, WgpuContext};
# fn example(context: WgpuContext, output: &GpuDeformationOutput) -> anyhow::Result<()> {
let reducer = GpuDeformationBounds::new(context)?;
let mut request = reducer.request(output, Some(64))?;

// Poll during a later application update, retaining the request while pending.
if let Some(bounds) = request.try_read()? {
    let local_bounds = [bounds.min(), bounds.max()];
    // Use local_bounds with the packed geometry from this output.
}
# Ok(())
# }
```

The reduction includes every vertex, including vertices not referenced by
triangles. Nonzero vertex status or nonfinite positions fail the entire result.
Bounds describe positions only; they do not validate other vertex attributes or
materialize CPU geometry. Mesh queries and existing scene snapshots remain
unchanged.

`try_read()` does not wait for GPU completion and decodes a fixed-size payload.
Success or failure releases staging storage and makes the request terminal.
Requests may outlive the reducer and input. Dropping a request cancels mapping,
not already submitted commands.

Associate each result with its original deformation output. A previous pose's
bounds need not contain a newer pose. While awaiting a result, render the last
complete geometry/bounds pair or provide a conservative envelope for the current
pose. Pass the accepted bounds as `[min, max]` to `Scene3dGpuDraw`; do not substitute
them for the undeformed CPU mesh's query bounds.

## Render vertex packing

For per-triangle derivative inputs to tangent processing, see
[GPU surface derivatives](tangent_derivatives.md).

`output.render_source(uv_sets, byte_limit)` creates a reusable
`WgpuScene3dGeometry` containing static material attributes and indices.
The five coordinate sets select base color, metallic/roughness, emission, normal,
and occlusion inputs. `output.render_geometry(&source)` combines those inputs
with GPU positions, normals, and tangents, returning a `Scene3dGpuGeometry` without
CPU readback. Reuse the source across outputs from the same base mesh allocation
and device; mismatches are rejected.

`Scene::geometry_inputs()` exposes every object's output ID, optional node and
application identity, source mesh, and active coordinate sets before image
resolution or raster culling. Use these inputs to bind GPU geometry even while
textures are loading. Inactive material slots use coordinate set zero. Inspection
does not validate geometry or materials; preparation and rendering perform those
checks.

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

Render Object ID and linear depth to use [single-pixel picking](picking.md) on
deformed surfaces without materializing CPU meshes. Results retain the submitted
frame's camera and object mapping; viewport pointer routing is independent.

Bounds are finite and ordered. They control camera and shadow culling and
transparent-object sorting, including objects whose original mesh is outside the
camera. Each overridden object draws independently; shared source meshes can use
different deformation outputs. Color, shadows, IDs, depth, and normals consume the
same packed geometry. CPU scene geometry and spatial queries remain unchanged.
Use output ID/depth readback for screen-space selection or materialize meshes for
CPU queries at the deformed pose.

`WgpuScene3dRenderer::render_with_geometry` accepts the same draws for a prepared
`Scene3dFrame`. Its input must still contain every overridden object; objects
removed by earlier preparation cannot be recovered. Prepared frames retain packed
resources on their individual objects. `WgpuScene3dRenderer::render` also accepts
frames with those resources attached. An empty override list preserves the frame's
existing geometry.

Rendered geometry memory includes packed vertices, indices, and indirect arguments,
counted once per distinct active packed output. Index buffers shared by multiple
outputs are counted once across those outputs. Source upload buffers and deformation
attributes have separate admission limits. Draw statistics count submissions,
including indirect draws suppressed by invalid GPU attributes.

`Scene3dGpuGeometryMemory::plan` reports 96 bytes per vertex for the static source,
another 96 bytes per packed vertex, four bytes per index, and 32 bytes per draw
argument/validation buffer. The optional byte limit covers one source and one output, not
all retained results or the separate 64-byte deformation attributes. Compute,
indirect execution, five storage bindings, and device buffer/dispatch limits are
required. No global cache retains these objects.

### Geometry validation

`Scene3dGpuGeometry::request_status(max_staging_bytes)` copies a fixed 32-byte
record into owned staging storage. `Scene3dGeometryStatusReadback::try_read()` polls
without waiting and returns `None` while pending. A request may outlive its source
geometry; completion or failure consumes it. Dropping cancels mapping, not queued
work. The optional byte limit applies to each request, and the caller controls the
number of concurrent requests.

`Scene3dGeometryStatus::is_drawable()` reports whether packing admitted the draw,
not whether it is on screen or covered by another object. `issues` combines all
detected categories:

- `INVALID_UV`: A selected coordinate contains a nonfinite lane.
- `INVALID_COLOR`: A color lane is nonfinite or outside `[0, 1]`.
- `DEFORMATION_STATUS`: An input deformation record has nonzero status lanes.
- `NONFINITE_DEFORMATION`: A position, normal, or tangent lane is nonfinite.
- `INVALID_TANGENT`: Tangent handedness, presence, or basis validity is inconsistent.
- `TRIANGLE_TANGENT_SIGN`: A triangle's vertices have different tangent signs.

`first_invalid_vertex` and `first_invalid_triangle` are the lowest base-mesh indices
in each category of location, including unused vertices. They are `None` when no
corresponding issue exists. The vertex index identifies at least one vertex issue,
not necessarily every bit in `issues`. These results belong to the packed geometry
snapshot, independently of camera, material clipping, viewport size, or later
deformation updates. Vertex and index data are not read back.

## Window viewports

`WgpuContext::for_window(window)` shares the window's current device and queue.
Use this context for Morph/Skin sources and packed geometry. A separately created
headless context is not interchangeable, even on the same adapter. Unsupported
backends return `None`. After device recovery, reacquire the context and recreate
device-local sources and outputs.

`viewport3d(id, scene).gpu_geometry(&draws)?` attaches packed outputs before scene
preparation. IDs are scene object indices plus one; conservative local bounds
control culling and transparent sorting. The renderer validates device identity,
source meshes, coordinate sets, and bounds before encoding. Invalid bindings fail
rendering instead of drawing the original mesh.

```rust
# use gpui_3d::{Scene, Scene3dGpuDraw, Viewport3d, viewport3d};
# fn view(scene: Scene, draws: &[Scene3dGpuDraw]) -> anyhow::Result<Viewport3d> {
let viewport = viewport3d("deformation", scene)
    .gpu_geometry(draws)?
    .color_samples(4);
# Ok(viewport)
# }
```

Geometry is retained by each frame object, not a window-wide object-ID table.
Separate viewports may reuse the same IDs with different outputs. Treat buffers
as immutable and replace frame snapshots when geometry changes; unchanged frames
can reuse cached output pixels.

GPU overrides disable CPU object hover/click picking and captured-UI pointer
routing for the entire viewport. This avoids reporting bind-pose hits or selecting
objects through unqueried deformed surfaces. Ordinary surrounding 2D controls and
caller-owned camera gestures remain available. Materialize CPU meshes and use a
viewport without GPU overrides when CPU mesh interactions are required.

### Interactive example

```sh
cargo run -p gpui_3d --example scene --features wgpu
```

Enable **Blend shapes**, **Bend skin**, or both, then switch **CPU deformation**
to **GPU deformation**. Both paths use the same timeline, weights, camera, and
transforms. Pause or step the timeline to compare a fixed pose. **Taper mesh** uses
CPU mesh updates and selects CPU mode. In GPU mode, use the numbered buttons for
assembly selection; orbit, pan, zoom, material edits, and visibility controls remain
available.

GPU sources and packed outputs are retained between unchanged samples. Device
replacement recreates those resources. The example displays preparation failures
in the viewport and pauses playback; it does not substitute CPU deformation.

GPU mode publishes geometry with bounds reduced from the same output. While its
single bounds request is pending, the last complete pair remains visible; playback
samples are coalesced to the latest requested pose. The initial pair uses the
uploaded base mesh. Bounds polling continues when playback is paused, so a seek or
weight change can finish without another input event.

## Resource admission

`Scene3dDeviceCapabilities::query(&context)` reports both `adapter_limits` and
device-enabled `limits`. Check the latter when deciding whether an existing
device can run a pipeline. Adapter support does not enable additional device
limits automatically.

```rust,no_run
# use gpui_3d::{GpuDeformationBounds, GpuMorph, GpuSkin, Scene3dDeviceCapabilities, WgpuContext, WgpuScene3dGeometry};
# fn example(context: &WgpuContext) -> anyhow::Result<()> {
let capabilities = Scene3dDeviceCapabilities::query(context);
GpuMorph::check_support(&capabilities)?;
GpuSkin::check_support(&capabilities)?;
GpuDeformationBounds::check_support(&capabilities)?;
WgpuScene3dGeometry::check_support(&capabilities)?;
# Ok(())
# }
```

Check only the operations used by the application. Morph and Skin require four
storage buffers and one uniform buffer per compute stage. Bounds reduction uses
two storage buffers and workgroup storage. Render vertex packing requires five
storage buffers and indirect execution. These checks create no GPU resources or
submissions. Errors name the missing feature or limit, including required,
device-enabled, and adapter-advertised values for limit failures. Constructors
perform the same checks; payload size, allocation success, device health,
render-target support, and computed vertex validity remain separate concerns.

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
cargo test -p gpui_3d --features wgpu --lib gpu_bounds_match_retained_morph -- --ignored
cargo test -p gpui_wgpu --lib gpu_geometry_preserves_material -- --ignored
```
