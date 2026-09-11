# WGSL material programs

`Scene3dMaterialProgram` compiles material functions and reflects their resource
bindings. It is available with the native `wgpu` feature. Compilation does not
create an adapter, allocate GPU resources, or execute shaders.

Compilation prepares source and layouts without uploading data or resolving images.
Use [material bindings](material_bindings.md) to prepare a device-local snapshot
and attach it with `Material::program()`.

```rust
use gpui_3d::Scene3dMaterialProgram;

let program = Scene3dMaterialProgram::compile(r#"
struct Controls { tint: vec4<f32> }
@group(1) @binding(0) var<uniform> controls: Controls;

fn material_surface(input: SurfaceInput, gradients: mat2x2<f32>) -> vec4<f32> {
    return builtin_surface(input, gradients) * controls.tint;
}

fn material_shading(base: vec3<f32>, input: SurfaceInput,
    gradients: SurfaceGradients, face_sign: f32) -> vec3<f32> {
    return builtin_shading(base, input, gradients, face_sign);
}
"#)?;
let entries: Vec<_> = program.resources().iter()
    .map(|resource| resource.layout_entry()).collect();
# Ok::<(), anyhow::Error>(())
```

## Functions

Both function signatures in the example are required. Additional constants, types,
and helper functions are allowed. Surface evaluation returns unpremultiplied linear
RGBA; shading returns linear HDR RGB. Renderer entry points own face/alpha clipping,
HDR clamping, alpha premultiplication, and data-channel output.

`SurfaceInput` contains world position, the interpolated world normal and tangent,
packed material UV sets, and linear vertex color including the object tint. It does
not expose raster position or output IDs. `face_sign` orients shading normals for
the visible face, including reflected instances.

Coverage may call `builtin_surface`, `unit_vector`, and helpers following the same
restrictions. It cannot access camera/lighting helpers, private renderer globals,
or derivatives. Texture sampling uses supplied gradients or explicit mip levels;
implicit and bias-based sampling are rejected, including through nested helpers.
Supplied gradients still reflect the current rasterization view and sample density.

Shading may also call `builtin_shading`, `surface_normal`, `diffuse_environment`,
`material_view_direction(world)`, `material_ambient(normal)`,
`material_light_count()`, and
`material_light(index, world, geometric_normal, shadow_depth)`. Light samples contain
direction and linear energy with distance/cone attenuation and shadow visibility.
Use `gradients.shadow_depth` for the shadow-depth gradient. Out-of-range light
indices return zero energy.

Material code cannot declare shader entry points, overrides, or private globals,
call private renderer helpers, or discard fragments. The restrictions apply to
helper functions as well as the two required functions. Source validation is not
a sandbox for untrusted GPU programs and does not prove termination or finite output.

## Resources

Group 0 belongs to the renderer. Group 1 supports fixed-layout uniform structs,
non-multisampled `texture_2d<f32>` and `texture_cube<f32>` views, and non-comparison
samplers. Storage resources, texture arrays, depth/integer textures, and binding
arrays are unsupported. Reflected texture layouts require filterable views.

`resources()` returns declarations sorted by binding index, including unused
declarations. Each record includes its name, type, minimum uniform byte size, and
transitive coverage/shading usage. `layout_entry()` produces a fragment-visible
WGPU layout entry without dynamic offsets; every output pass uses the same group 1
layout. Uniform data must follow WGSL alignment and padding, not packed Rust layout.
Texture RGB encoding depends on the bound view format, not its variable name.

`compile_with_limits()` accepts `Scene3dMaterialLimits`. Defaults admit 64 KiB of
source, 16 resources, and 64 KiB of aggregate minimum uniform storage. These limits
exclude retained compiled-source storage, compiler working memory, and GPU storage.

`validate_limits(&device.limits())` checks the enabled device limits, including the
renderer's standard bindings, the highest custom binding index, and uniform sizes.
It does not create bind groups or verify actual resource handles, texture formats,
or driver compilation. Program clones retain the same compiled source and metadata.
