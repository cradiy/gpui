# Material bindings

`Scene3dMaterialSource` prepares a compiled [material program](material_programs.md)
on a caller-supplied `WgpuContext`. It creates the shader module and group 1 layout
once. The native `wgpu` feature is required.

`bind()` uploads uniform values and binds existing texture views and samplers.
It returns a `Scene3dMaterialSnapshot` retaining the source and all bound resources.
`Material::program(snapshot)` attaches these bindings to a scene material.

Programs declaring [custom vertex inputs](material_attributes.md) also require
`with_vertex_streams()` before drawing. Uniform/texture updates retain attached
streams; attribute updates preserve the material's group 1 bindings.

```rust
use gpui_3d::{
    Scene3dMaterialBindingLimits, Scene3dMaterialSource, Scene3dMaterialValue,
};

let source = Scene3dMaterialSource::new(context, program)?;
let image = source.context().create_texture(&image_descriptor);
let image_view = image.create_view(&Default::default());
let image_sampler = source.context().create_sampler(&sampler_descriptor);
let limits = Scene3dMaterialBindingLimits::default();
let bindings = source.bind([
    (0, Scene3dMaterialValue::Uniform(parameter_bytes.into())),
    (1, Scene3dMaterialValue::Texture(image_view)),
    (2, Scene3dMaterialValue::Sampler(image_sampler)),
], limits)?;

let updated = bindings.with_values([
    (0, Scene3dMaterialValue::Uniform(updated_parameter_bytes.into())),
], limits)?;

let material = gpui_3d::Material::color(gpui::white()).program(updated);
```

The binding numbers and resource kinds must match the program's declarations.
Initial binding requires every declaration exactly once, including unused ones.
Input order is arbitrary. Uniform bytes must exactly match the reflected struct
size, including WGSL padding. Oversized buffers are not treated as subranges.

## Scene rendering

Viewport and headless renderers use the same snapshot for color, directional
shadow, object ID, linear depth, and geometric normal output. GPU-deformed meshes
use the same material path. Surface alpha is evaluated before renderer-owned face
and alpha clipping; shading cannot override coverage. Camera and shadow passes
have different pixel footprints, so explicit texture gradients can select different
mip levels. Blend materials retain back-to-front color compositing, nearest surviving
ID/depth selection, and no shadow casting.

Programs return linear HDR RGB. Depth testing, alpha premultiplication, tone mapping,
and output encoding remain renderer-owned. Built-in tint, textures, lights and
material parameters remain available through the program helpers. `builtin_program()`
restores built-in evaluation without changing those parameters.

Pipeline variants are reused by shader identity within each renderer's output format
and sample count. Parameter updates do not recompile the program. Unreferenced
variants are released during preparation. Objects sharing a material snapshot can
batch when their other draw state is compatible; distinct snapshots cannot batch.
Wrong backend types, foreign/lost devices, and pipeline compilation failures reject
rendering rather than selecting a different material.

Custom programs disable viewport CPU object hover/click and captured-UI pointer
routing. CPU mesh queries do not evaluate custom WGSL coverage. Use retained GPU
ID/depth captures for visible-surface selection; camera controls and surrounding
ordinary UI remain available.

## Updates and ownership

`with_values()` replaces only the specified bindings. It reuses the source shader
and layout, and shares unmodified buffers, views, and samplers. Changed uniform
blocks receive new private buffers; existing snapshots are never overwritten.
An empty update retains all resources. Invalid updates leave the original snapshot
unchanged. Cloning a snapshot shares its complete binding state.

Uniform input bytes are copied during binding and are not retained by the snapshot.
Texture views retain their underlying textures without copying pixels. External
texture content is not immutable: callers must preserve its content for retained
frames and must not destroy textures while those frames can still use them. A
different image or immutable texture revision can be supplied through a new view.

`bind_group()` exposes the snapshot's group 1 bindings; `source()` retains its
matching shader and layout. Resources belong to the source device. Recreate the
source and bindings after device replacement.

Texture and sampler values use `WgpuResource` handles created by
`WgpuContext::create_texture()` and `create_sampler()`. Texture views inherit the
creating device. These handles retain that identity across clones and expose
`raw()` or dereferencing for application uploads and commands. Arbitrary raw WGPU
handles cannot be relabeled with an application-supplied device identity.

## Validation and budgets

Unknown, missing, duplicate, mistyped, and incorrectly sized values are rejected
before parameter allocation. Resource device identities are checked before backend
binding operations. WGPU validation checks actual view dimensions, filterability,
usage, and sampler compatibility when creating the bind group; validation failures
are returned as errors.

`Scene3dMaterialBindingLimits` defaults to 64 KiB of uniform payload per complete
snapshot. `uniform_bytes()` reports that payload, counting shared buffers in every
snapshot. The limit also applies to partial and empty updates. It excludes external
texture memory, driver overhead, and simultaneous retained snapshots; callers bound
their lifetimes and memory separately. Unchanged uniforms are not uploaded again.
