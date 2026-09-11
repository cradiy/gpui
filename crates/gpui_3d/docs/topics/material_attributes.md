# Custom vertex inputs

[Material programs](material_programs.md) · [GPU vertex streams](vertex_streams.md)

`Scene3dMaterialProgram::compile_with_attributes()` declares named vertex inputs
and compiles their transport into the renderer's camera and shadow vertex stages.
Material functions read the interpolated values through `input.attributes`.
Compilation and device-limit checks perform no GPU work.

```rust
use gpui_3d::{
    Scene3dMaterialProgram, Scene3dVertexAttribute, Scene3dVertexInterpolation,
};
use gpui_wgpu::wgpu::VertexFormat;

let attributes = [
    Scene3dVertexAttribute::new("weight", VertexFormat::Float32),
    Scene3dVertexAttribute::new("region", VertexFormat::Uint32),
    Scene3dVertexAttribute::new("direction", VertexFormat::Float32x3)
        .interpolation(Scene3dVertexInterpolation::Linear),
];
let program = Scene3dMaterialProgram::compile_with_attributes(material_wgsl, &attributes)?;
```

For example, a surface evaluator may multiply alpha by
`clamp(input.attributes.weight, 0.0, 1.0)`, while a shading evaluator may select a
response using `input.attributes.region`. Surface alpha still follows the material's
`Opaque`, `Mask`, or `Blend` coverage rules; shading returns RGB only.

## Stream snapshots

`Scene3dMaterialSource::bind_vertex_streams(vertex_count, values, max_payload_bytes)`
uploads one value for each declared stream and returns `Scene3dVertexStreams`.
Values are borrowed `(name, bytes)` pairs in arbitrary order. Every stream contains
exactly one tightly packed record per vertex, including unused vertices. Float lanes
must be finite; integer lanes preserve all bit patterns. Unknown, duplicate, missing,
incorrectly sized, and nonfinite inputs are rejected before buffer allocation.

```rust
let source = gpui_3d::Scene3dMaterialSource::new(context, program)?;
let streams = source.bind_vertex_streams(mesh.vertex_count(), &[
    ("weight", bytemuck::cast_slice(&weights)),
    ("region", bytemuck::cast_slice(&regions)),
    ("direction", bytemuck::cast_slice(&directions)),
], 8 * 1024 * 1024)?;
let bindings = source.bind(material_values, Default::default())?
    .with_vertex_streams(streams.clone())?;
let material = gpui_3d::Material::color(gpui::white()).program(bindings);

let updated = streams.with_values(&[
    ("weight", bytemuck::cast_slice(&updated_weights)),
], 8 * 1024 * 1024)?;
```

`with_values()` on streams uploads only the named replacements. Omitted buffers
remain shared; previous snapshots are never overwritten. Empty updates retain the
snapshot after checking the full payload budget and device health. The snapshot owns
private storage buffers; caller input bytes are not retained, and raw external GPU
buffers are not accepted by this upload interface.

`Scene3dMaterialSnapshot::with_vertex_streams()` requires streams from that exact
source shader/layout and preserves uniform/texture bindings. Material `with_values()`
preserves attached vertex streams. The new snapshot must be attached to the submitted
material to publish an update; updating streams does not mutate existing materials.

Draw preparation rejects missing streams or a vertex count different from the mesh.
Values use the mesh's vertex order, including for GPU-deformed geometry. Sharing
streams across meshes with the same count is allowed; the application is responsible
for matching their vertex semantics and remapping values when topology changes.
Color, shadow, ID, depth, and normal passes bind the same snapshot. CPU mesh queries
do not evaluate custom attribute coverage; use GPU ID/depth captures for picking.

## Types and interpolation

Supported formats are `Float32`, `Sint32`, `Uint32`, and their x2/x3/x4 vector
variants. Each stream is tightly packed in mesh vertex order: scalar, two-, three-,
and four-component records occupy 4, 8, 12, and 16 bytes respectively. Names begin
with an ASCII letter and contain at most 64 ASCII letters, digits, or underscores.
They must be unique and valid WGSL member identifiers. Normalized, half-precision,
and packed integer formats are not accepted.

Floating-point inputs default to perspective-correct interpolation. `Linear`
interpolates in screen space; both use pixel-center sampling. `Flat` takes the first
vertex's value and is mandatory for integer inputs. Camera and shadow stages use
the same stream indices and interpolation declarations. Their projected footprints
differ, so smooth values are evaluated at each pass's own rasterized samples.

No standard vertex locations, UV sets, or color components are repurposed. Streams
use vertex-visible, read-only storage bindings in group 2, indexed by declaration
order. One four-byte word stores each component; signed integers and floats retain
their bit representation. Generated varyings begin at location 9. Fragment material
functions receive these values through `SurfaceInput`, not storage-buffer access.
They cannot declare group 2 bindings or access the generated buffers directly.
Surface derivative and camera-input restrictions remain unchanged.

## Admission

`vertex_attributes()` returns the retained declarations in binding order, separately
from group 1 `resources()`. `compile_with_attributes_and_limits()` accepts
`Scene3dMaterialLimits`; the default declaration budget is 16 custom streams.
`max_source_bytes` applies to application WGSL, while generated transport is bounded
by the declaration count and name lengths.

`validate_limits()` includes the renderer's nine standard varyings, vertex storage
bindings, three bind groups, and the standard vertex uniform buffer. The default
WGPU inter-stage limit of 16 variables permits at most seven custom streams; actual
enabled storage and binding limits may be lower. Source construction also requires
vertex-storage support and validates driver shader/layout creation.

Stream binding checks enabled buffer/storage limits and u32 vertex-word addressing.
The byte budget covers all buffers represented by the snapshot, including shared
and unchanged streams; it excludes retained older snapshots, material uniforms,
CPU input storage, and driver overhead. `payload_bytes()` reports this complete
payload. No positions, normals, tangents, or indices are rebuilt or uploaded.
Clones retain device-local resources; recreate the source and streams after device
replacement. `vertex_layout()` exposes the source's group 2 layout and stream
`bind_group()` exposes its compatible bindings for application-owned renderers.
