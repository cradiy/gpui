# Additional mesh passes

[Materials](materials.md) · [Material programs](material_programs.md)

`Material::mesh_passes()` attaches up to eight additional color draws to an object.
Each `MeshPass` uses its own device-local material snapshot and custom vertex
streams. Passes reuse the object's mesh, transform, standard material inputs, and
CPU or GPU-deformed geometry; they do not upload another copy of the mesh.

```rust
use gpui_3d::{Material, MeshPass, MeshPassBlend, MeshPassDepth, MeshPassState};

let pass = MeshPass::new(bindings).state(MeshPassState {
    depth_compare: MeshPassDepth::LessEqual,
    depth_write: false,
    blend: MeshPassBlend::SourceOver,
    ..Default::default()
});
let material = Material::color(gpui::rgb(0x718cb8))
    .mesh_passes([pass]);
```

Create `bindings` using [material bindings](material_bindings.md). The shader uses
the same `material_surface` and `material_shading` functions as primary materials.
It receives the owner's standard textures, UV selections, tint, lights, camera and
shadow inputs. Group 1 resources and group 2 custom streams come from the pass's
snapshot. For independent color or textures, declare them in that snapshot.

## State and ordering

`MeshPassState` defaults to source-over blending, Blend alpha, LessEqual depth
testing, no depth writes, zero depth bias, and both local faces visible.

- `cull` selects None, Front, or Back independently of the primary material's
  `double_sided` setting. Front is local counterclockwise winding; reflected object
  transforms preserve that local convention. Face rejection runs in the fragment
  stage, without hardware face culling.
- `alpha_mode` selects Opaque, Mask or Blend surface coverage. `alpha_cutoff` is
  finite and in `[0, 1]`. These controls do not select depth writes or blending.
- `depth_compare`, `depth_write`, `depth_bias`, `depth_slope_bias`, and
  `depth_bias_clamp` control the shared camera depth attachment. Bias is a raster
  depth adjustment, not a vertex displacement or a world-space outline width.
  The finite clamp is signed: a positive value caps positive bias, a negative value
  bounds negative bias, and zero disables clamping. Bias with the opposite sign is
  unchanged.
- `blend` selects Replace, premultiplied SourceOver, or Additive. Additive sums
  RGB while accumulating alpha with source-over. Composition occurs in linear HDR
  before scene exposure, tone mapping and output encoding.

The color schedule is:

1. Primary opaque and masked surfaces.
2. Passes with `stage: MeshPassStage::AfterOpaque` (the default).
3. Primary blended surfaces, sorted back to front.
4. Passes with `stage: MeshPassStage::AfterTransparent`.

Within each pass stage, objects use scene submission order and passes use their
declaration order. They are not depth-sorted. Enabling depth writes affects later
color draws, including primary blended surfaces after an AfterOpaque pass.

## Normal expansion

`MeshPass::expansion()` displaces vertices along their geometric normals after CPU
or GPU deformation. The primary surface and its query geometry remain unchanged.

```rust
use gpui_3d::{MeshPassCull, MeshPassExpansion, MeshPassSpace};

let pass = MeshPass::new(bindings)
    .state(MeshPassState {
        cull: MeshPassCull::Front,
        ..Default::default()
    })
    .expansion(MeshPassExpansion::new(MeshPassSpace::Pixels, 4.)
        .weight("width", 1.));
```

`World` measures displacement in world units along the normalized transformed
normal, independent of object scale. It updates the pass's world position for
shading. `Pixels` measures render-target pixels along the projected normal,
accounting for perspective and viewport aspect ratio. It preserves clip Z/W and
the original world position. A normal with zero projected length produces no
pixel displacement. Pixel widths are raster pixels, not logical UI points;
resampling the viewport's output texture also resamples its outline.

The amount is signed and finite; negative values move inward. Without `.weight()`,
all vertices use the same amount. A weight names a declared `Float32` custom
stream in the pass's material program. Bind it through
[custom vertex inputs](material_attributes.md). The vertex stage clamps values to
`[0, weight_limit]` before multiplication. Negative weights produce zero expansion;
the maximum product must fit finite `f32` arithmetic. GPU-provided weights follow
the stream contract requiring finite float lanes.

World expansion enlarges conservative camera-plane tests by the maximum world
displacement without modifying stored mesh bounds. Pixel expansion keeps candidates
across side planes until raster clipping, because target dimensions are not known
during scene preparation; unchanged near/far bounds still apply. Primary data and
shadow passes retain their original bounds. Additional color passes are not
included in CPU mesh queries or reported GPU deformation bounds.

## Geometry, coverage and lifetime

Additional passes retain the original vertex normals and tangents; expansion does
not rebuild surface directions. GPU deformation and external custom streams follow
their existing ownership and queue contracts.
Stream counts must match the owner's mesh. Objects with additional passes draw
individually, while sharing geometry allocations and compatible shader pipelines.
Draw statistics include their camera draws, instances and triangles without
counting another geometry or instance upload.

Additional passes do not cast shadows or contribute to object ID, linear depth,
or normal output channels. Those outputs and CPU picking retain the primary
surface's coverage. A decorative pass may therefore color pixels without making
them selectable, even when it writes to the camera depth attachment. This does
not change captured data-channel depth. Passes may sample existing scene shadows.

Replacing a material or its pass list creates a new scene submission. Retained
snapshots keep their resources alive; shared external buffers must remain immutable
for their documented lifetime. Recreate device-local snapshots after device
replacement. Invalid pass counts, floating-point state, backend types, devices,
vertex counts and shader pipelines fail explicitly. Native WGPU is required.

## Material comparison

```sh
cargo run -p gpui_3d --features wgpu --example materials
```

Select **PBR / Custom**. **Outline** cycles pixel width, no outline and world width;
**Width** switches uniform and vertex-weighted expansion. Orbit and zoom to compare
world-unit and raster-pixel behavior. The outline shader belongs to the example.
