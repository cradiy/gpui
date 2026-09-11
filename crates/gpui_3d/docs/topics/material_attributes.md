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

`Scene3dMaterialSource` accepts programs without custom vertex streams. A program
with nonempty `vertex_attributes()` cannot be attached through material snapshots;
source construction returns an explicit unsupported-binding error.

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
enabled storage and binding limits may be lower. This does not validate stream
payloads, vertex counts, device downlevel flags, or driver shader compilation.
