# WGSL material programs

`Scene3dMaterialProgram` compiles material functions and reflects their resource
bindings. It is available with the native `wgpu` feature. Compilation does not
create an adapter, allocate GPU resources, or execute shaders.

Compilation prepares source and layouts without uploading data or resolving images.
Use [material bindings](material_bindings.md) to prepare a device-local snapshot
and attach it with `Material::program()`.

[Custom vertex inputs](material_attributes.md) define typed attributes and
interpolation for program compilation.

[Additional mesh passes](mesh_passes.md) use these programs for independent color
draws while preserving primary shadow and data-channel ownership.

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

`material_factors(input, gradients) -> MaterialFactors` samples standard material
maps using their selected UV sets and supplied gradients. Its fields are
`metallic`, `roughness`, `emission` (linear RGB), and `occlusion`. Metallic and
roughness include their B/G texture multipliers; emission includes its decoded RGB
multiplier; occlusion includes the material's strength. Missing maps use unit
multipliers. Missing PBR parameters use metallic 0, roughness 0.5, and zero emission.
The helper does not impose the built-in PBR roughness floor of 0.045 or apply a
lighting response. It is shading-only.

Custom primary programs and additional mesh passes activate all configured standard
material maps, even without PBR or with `.unlit(true)`. These settings select only
the built-in lighting response. `surface_normal` uses the configured normal map;
active maps require matching UV sets and normal maps require matching mesh tangents.
Zero normal scale or occlusion strength disables that map and its resource requests.
Additional passes share these standard inputs with the primary material; their
group 1 resources remain independent.

`material_view_position(world)` transforms a world position into camera space.
`material_view_vector(world_vector)` applies the camera's linear transform without
translation or normalization. These use the submitted `world_to_view` matrix,
not the camera projection or the shadow camera. GPUI cameras look along negative Z
with positive X right and positive Y up; their view matrices are rigid, so the vector
helper also rotates world-space normals. For a custom non-rigid view transform,
normal conversion requires the inverse-transpose transform supplied by the application.
Both helpers are shading-only. `material_view_direction(world)` instead returns the
normalized world-space direction toward the viewer, including orthographic cameras.

`material_environment_radiance(direction, roughness) -> vec3<f32>` samples the
scene's prefiltered specular environment in a world-space direction. It normalizes
the direction, applies the environment's Y rotation and intensity, and returns
linear HDR radiance. Roughness is clamped to `[0, 1]` and maps linearly across the
prefiltered mip levels. A zero direction or inactive environment returns zero.
The result excludes Fresnel, BRDF weighting, base color, and ambient occlusion.

`material_environment_brdf(n_dot_v, roughness) -> vec2<f32>` samples the renderer's
GGX split-sum lookup table. Both inputs are clamped to `[0, 1]`; the result contains
the Fresnel scale and bias. An inactive specular environment returns zero. For a
reflectance `f0`, an application can combine the helpers as
`radiance * (f0 * brdf.x + vec3<f32>(brdf.y))` or use its own response. Both helpers
are shading-only and require no application texture or sampler bindings.

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
source, 16 resources, 16 custom vertex declarations, and 64 KiB of aggregate minimum uniform storage. These limits
exclude retained compiled-source storage, compiler working memory, and GPU storage.

`validate_limits(&device.limits())` checks the enabled device limits, including the
renderer's standard bindings, the highest custom binding index, and uniform sizes.
Material programs require two bind groups even without custom resources; custom
vertex streams require three. Per-stage buffer limits include the standard
uniform block alongside the material's declared resources, including unused ones.
It does not create bind groups or verify actual resource handles, texture formats,
or driver compilation. Program clones retain the same compiled source and metadata.

## Material comparison

```sh
cargo run -p gpui_3d --features wgpu --example materials
```

Select **PBR / Custom** to compare built-in PBR, a stepped direct-light response,
and a camera-space sphere-map reflection. Right-drag rotates the camera; the
projection control switches perspective and orthographic views. **Toon bands**
and **Sphere brightness** update uniform snapshots without recompiling shaders.
The sphere map uses an sRGB texture view, decoded to linear RGB during sampling.
The example's WGSL defines the shading styles; the renderer supplies lights,
coordinate conversion, texture bindings, coverage, and output processing.
