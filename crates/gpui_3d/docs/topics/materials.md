# Materials and textures

[3D viewports](../viewport.md)

## Materials and light

- `Material::color(color)` creates a lit solid surface.
- `Material::image(source)` maps the first decoded image frame onto UVs. Keep the
  image source stable across renders. An object is omitted while its image is unavailable.
- `Material::ui()` samples the viewport's captured UI without lighting.
- `.unlit(true)` disables lighting for any material.
- `.tint(color)` sets an sRGB tint, decoded before multiplication; its alpha multiplies texture alpha.
- `.base_color_texture(texture)` replaces the base image and sampling without
  resetting tint, image encoding, lighting, other maps, alpha mode, or face visibility.
- `.alpha_mode(mode)` selects `AlphaMode::Opaque`, `Mask`, or `Blend`.
- `.alpha_cutoff(value)` selects `Mask` and preserves a finite nonnegative threshold.
  Zero accepts every alpha value; values above one discard the entire surface.
- `.double_sided(false)` discards back faces in color, shadow, depth, normal and
  ID outputs, and in screen/world-ray queries. Materials are double-sided by default.

Front faces use local counterclockwise triangle winding. A negative-determinant
world transform reverses the raster winding convention, preserving the authored
front side under mirrored instances. Shading and queried normals face the visible
side of double-sided surfaces. Bounds and frustum queries remain conservative
and do not apply face visibility.

`Light` supplies a world-space direction toward the light, color, intensity and
ambient strength. Materials use basic diffuse shading unless `.pbr(parameters)`
is selected. Distinct opaque surfaces occlude each
other independently of object submission order. Coplanar surfaces should be
separated to avoid depth conflicts.

## Transparency

The default is `AlphaMode::Mask` with a cutoff of 0.5.

| Mode | Fragment alpha | Depth writes |
| --- | --- | --- |
| `Opaque` | Ignored; the surface is opaque | Yes |
| `Mask` | Below cutoff is discarded; survivors are opaque | Yes |
| `Blend` | Zero is discarded; nonzero values blend continuously | No |

```rust
use gpui::rgba;
use gpui_3d::{AlphaMode, Material};

let material = Material::color(rgba(0x65e0f580))
    .alpha_mode(AlphaMode::Blend);
```

Texture alpha, tint alpha, and interpolated vertex alpha are multiplied and
clamped to `[0, 1]`. Blending
uses premultiplied source-over in the linear HDR target, before exposure and
tone mapping. It applies to both lit and unlit materials.

Opaque and masked objects render first. Blended objects then render from far
to near, ordered by the forward camera depth of each transformed mesh-bounds
center. Equal-depth blended objects retain submission order. All modes depth-test
against opaque and masked surfaces. Sort order does not change object IDs.

Sorting is per object, not per triangle. Intersecting meshes, cyclic overlaps,
and self-overlapping transparent meshes can produce incorrect layer ordering.
Separate independently ordered surfaces into objects. Order-independent
transparency, refraction, and transparent shadows are not provided.

Viewport image picking and headless object IDs select the nearest surviving
surface, even when its blended opacity is small; they do not choose the largest
color contributor. `Opaque` ignores alpha, `Mask` uses the cutoff, and `Blend`
passes through only zero-alpha regions. Captured UI picking respects vertex and
material alpha but does not sample captured pixel alpha.

Vertex RGB multiplies the linear base color with perspective-correct
interpolation. It is not sRGB-decoded, does not tint emission, and applies to
solid, image, and captured-UI materials. Color, shadow, depth, normal, and object-ID
passes use the same base-alpha calculation. In the `materials` example,
`Vertex colors` toggles sphere color gradients and a color/alpha ramp on the
textured strip; `Alpha mode` controls the strip's coverage.

## Metallic-roughness materials

```rust
use gpui::rgb;
use gpui_3d::{Material, PbrMaterial};

let material = Material::color(rgb(0xdca773)).pbr(PbrMaterial {
    metallic: 1.,
    roughness: 0.35,
    emissive: [0.; 3],
});
```

`PbrMaterial` enables a GGX microfacet distribution, height-correlated Smith
visibility, Schlick Fresnel, and Fresnel-weighted Lambert diffuse response.
Dielectrics use normal-incidence reflectance 0.04. Increasing metallic blends
that reflectance toward the linear base color and removes diffuse reflection.
The base color comes from the material's solid color or sampled image and tint.

Metallic and perceptual roughness must be finite and in `[0, 1]`. Defaults are
metallic 0 and roughness 0.5. Shading limits perceptual roughness to at least
0.045 for finite highlights. Emissive is additive linear RGB radiance, defaults
to zero, and accepts finite components in `[0, 65504]`. It is independent of base
color, tint and scene lighting, but receives scene exposure and tone mapping.
Invalid parameters cause rendering to fail. `.unlit(true)` bypasses all PBR
terms, including emissive, and displays the sampled base color.

Specular response follows the world-space viewing direction. Perspective cameras
use the eye-to-surface vector; orthographic cameras use a constant direction.
Both viewport and headless rendering use these conventions. PBR does not alter
alpha cutout, depth writes, object IDs, or picking.

Direct lighting supports a single `Light` or an explicit `PunctualLight` list,
alongside uniform ambient and optional diffuse environment illumination. Pure
metals receive no diffuse ambient or environment light. Specular environment
reflections use [prefiltered environment maps](lighting.md#specular-environment-lighting).
One directional source can use a shadow map.
Emission does not illuminate other objects or add a glow outside the surface.

## Material textures

```rust,no_run
use gpui::rgb;
use gpui_3d::{Material, MaterialTexture, PbrMaterial, TextureAddressMode, TextureSampling};

let material = Material::color(rgb(0xdca773))
    .pbr(PbrMaterial { metallic: 1., roughness: 1., emissive: [2.; 3] })
    .metallic_roughness_texture(MaterialTexture::new("surface.png").sampling(TextureSampling {
        address_u: TextureAddressMode::Repeat,
        address_v: TextureAddressMode::Repeat,
        ..Default::default()
    }))
    .emissive_texture(MaterialTexture::new("emission.png"));
```

`MaterialTexture` accepts the same image sources as `Material::image` and uses
the first decoded frame. Each map has its own `TextureSampling`, including UV
transform, addressing and filtering. All maps use mesh UVs; base-color sampling
does not affect other maps.

| Input | Channels | Color interpretation | Applied to |
| --- | --- | --- | --- |
| Metallic-roughness | G: roughness, B: metallic | Linear data | Corresponding PBR factors |
| Emissive | RGB | sRGB decoded before filtering | Linear emissive factor |

Map values multiply the material factors. A zero factor remains zero; an absent
map uses a multiplier of one. R in the metallic-roughness map and alpha in both
maps are ignored. Alpha cutout and picking visibility depend only on base-color
alpha and tint. These maps do not add occlusion, surface displacement, or normals.

Metallic-roughness and emissive maps are loaded only for lit PBR materials.
Until all required images are ready, the viewport omits the object from rendering
and visible picking. Direct
headless rendering requires decoded `ImageSource::Render` inputs for every map
and returns an error for unresolved inputs. `.unlit(true)` and diffuse materials
ignore these maps without requesting their resources.

## Ambient occlusion maps

```rust,no_run
use gpui::rgb;
use gpui_3d::{Material, MaterialTexture};

let material = Material::color(rgb(0xd6c3a5))
    .occlusion_texture(MaterialTexture::new("occlusion.png"))
    .occlusion_strength(0.8);
```

Occlusion maps use linear R, where zero blocks indirect light and one leaves it
unchanged. G, B and alpha are ignored. `occlusion_strength` defaults to one and
must be finite and in `[0, 1]`; rendering rejects invalid values. The multiplier
is `1 + strength * (R - 1)`, following the
[glTF occlusion convention](https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html#materialocclusiontextureinfo).
An absent map or zero strength leaves indirect lighting unchanged.

Both basic lit and PBR materials apply this multiplier to uniform ambient and
diffuse environment illumination. Direct diffuse/specular light and PBR
emission are unaffected. Unlit materials bypass occlusion. The texture does not
alter base color, transparency, geometry, picking, depth or geometric normals.
It represents authored or baked occlusion, not dynamic shadows or screen-space AO.

Use `MaterialTexture::sampling` for independent UV transforms, addressing and
filtering. An ORM image can be shared between `occlusion_texture` (R) and
`metallic_roughness_texture` (G/B). No mesh tangents are required for occlusion.
Disabled maps request no resources. Active maps use the same readiness and
decoded-image requirements as other material maps in viewport/headless rendering.

The `materials` example toggles occlusion independently of normal, metallic-roughness,
and emission maps. Occlusion uses the R channel of the shared ORM image.

## Coordinate selection

Each image slot selects mesh coordinates with `MaterialTexture::uv_set(set)`.
`Material::image_uv_set(set)` selects coordinates for a base-color image created
with `Material::image`. The default is set zero. `base_color_texture(texture)`
copies both the sampling configuration and coordinate selection.

```rust
use gpui_3d::{Material, MaterialTexture, PbrMaterial};

let material = Material::image("base.png")
    .image_uv_set(0)
    .pbr(PbrMaterial::default())
    .normal_texture(MaterialTexture::new("normal.png").uv_set(2))
    .occlusion_texture(MaterialTexture::new("occlusion.png").uv_set(7));
```

The mesh must contain each active slot's coordinate set. A normal map requires
a tangent basis generated or supplied for its selected set. Missing sets or
mismatched bases fail scene preparation before resolving that object's images. Inactive maps
do not require coordinates. Captured UI always uses set zero, independent of
image settings.

UV transforms and sampling gradients operate on each slot's selected set.
Color, depth, normal, object-ID and shadow passes use the same base-color
coordinates for alpha coverage. Viewport image-alpha picking uses that set at
level zero; `Hit::uv` remains the untransformed set-zero coordinate.

## Normal maps and tangents

```rust,no_run
use gpui::rgb;
use gpui_3d::{Material, MaterialTexture, Mesh, PbrMaterial};

let mesh = Mesh::plane();
let material = Material::color(rgb(0x79a6b0))
    .pbr(PbrMaterial { roughness: 0.35, ..Default::default() })
    .normal_texture(MaterialTexture::new("normal.png"))
    .normal_scale(0.8);
```

Normal-map RGB is linear vector data, decoded from `[0, 1]` to `[-1, 1]` and
normalized after filtering. R points along the tangent, G along the bitangent,
and B along the surface normal; alpha is ignored. `normal_scale` multiplies XY
before normalization. It defaults to 1 and accepts finite nonnegative values.
Zero disables the normal map and its resource requests. A zero decoded vector
uses the interpolated mesh normal.

Normal maps affect lit PBR shading only. They do not move vertices, alter
silhouettes or depth, or change object IDs, ray intersections, and picking normals.
They use independent `MaterialTexture` sampling. UV transforms and addressing
change sampled locations, not the tangent frame or the decoded vector axes.

`Mesh::plane()` and `Mesh::cube()` provide tangents. Custom meshes attach one
`[f32; 4]` tangent per vertex using `mesh.with_tangents(data)`, returning a new
mesh with shared vertex/index storage and unchanged geometry queries. XYZ is
orthogonalized against the vertex normal and normalized; W is exactly -1 or +1.
The bitangent is `cross(normal, tangent.xyz) * tangent.w`. Read the resulting
data with `mesh.tangents()`. `with_tangents(data)` associates the basis with set
zero. Use `with_tangents_for_uv_set(set, data)` to supply frames for another
existing coordinate set. `tangent_uv_set()` returns the associated identifier,
or `None` when no tangents exist. Morph, Skin and fixed-topology vertex updates
preserve this association when supplying replacement tangents.

Invalid counts, non-finite or undefined bases, zero vertex normals, and mixed W
signs within a triangle return `TangentError`. Split vertices at tangent-space
seams before supplying data. Rendering does not generate missing tangent bases.
Rendering an active normal map requires a tangent basis for its selected UV set.

`Mesh::generate_tangents()` generates MikkTSpace frames synchronously and returns
`GeneratedTangents`. It uses normalized copies of indexed normals and preserves
the stored positions, normals, and UVs. Shared vertices split when face-corner
tangent frames differ, including mirrored UV seams. Triangle order and winding
are unchanged, preserving triangle IDs for queries and per-triangle metadata.
Existing tangents are replaced without modifying the source mesh.

`generate_tangents_for_uv_set(set, mode)` uses the requested coordinate set for
validation, MikkTSpace and any repairs. It preserves all coordinate sets and
records the selected identifier in `tangent_uv_set()`. Missing sets return
`TangentGenerationError::MissingUvSet`. The default generation methods use set
zero. A mesh holds one tangent basis at a time.

```rust
use gpui_3d::Mesh;

let source = Mesh::new(Mesh::plane().vertices().to_vec(), Mesh::plane().indices().to_vec());
let generated = source.generate_tangents()?;
let source_weights = vec![0.5_f32; source.vertex_count()];
let weights: Vec<_> = generated.source_vertices().iter()
    .map(|&source| source_weights[source as usize])
    .collect();
let (mesh, source_vertices) = generated.into_parts();
# Ok::<(), gpui_3d::TangentGenerationError>(())
```

Output vertices follow first use and omit unreferenced vertices; output bounds
therefore exclude unused positions. `source_vertices()[output_index]` identifies
the original vertex. Use this mapping for external vertex attributes, morph
deltas, and skin influences before constructing deformation inputs. Distinct
source vertices are not merged even when all their mesh attributes match.

`TangentGenerationError` identifies zero indexed normals, zero geometric or UV
area, unrepresentable f32 intermediates, and undefined output frames. The default
`generate_tangents()` uses `TangentGenerationMode::Strict`. Select another mode
with `generate_tangents_with_mode(mode)`:

- `Strict` rejects zero-area triangles and undefined frames.
- `Inherit` lets MikkTSpace inherit frames from neighboring primitives. A corner
  without a usable inherited frame remains an error.
- `Repair` keeps usable MikkTSpace frames and repairs undefined corners with the
  triangle's position/UV derivative projected against the vertex normal. If no
  usable derivative exists, it projects the least-aligned coordinate axis to
  create a deterministic orthonormal basis. Handedness follows a usable corner
  of the same triangle, otherwise the UV orientation, otherwise positive one.

`GeneratedTangents::repairs()` reports repaired triangle/corner positions and
`TangentRepairKind`. Neighbor inheritance is not counted as a repair. An
`OrthonormalBasis` is a convention for undefined UV directions, not a recovery
of authored normal-map orientation. Callers can reject such repairs when asset
fidelity requires a defined UV basis. Conflicting triangle handedness, invalid
normals, and unrepresentable calculations remain errors in every mode. No mode
removes triangles or changes positions, normals, UVs, or source correspondence.

Finite UVs outside `[0, 1]` are supported. Generate during asset preparation and
share the result across objects; generation is not part of per-frame rendering.

Tangents follow the model transform; normals use the inverse transpose. Shading
reorthogonalizes the world-space frame, adjusts handedness for reflected transforms,
and reverses the mapped normal on back faces. Vertex-normal shading uses the same
reflection-aware face orientation.

## Color and exposure

```rust
use gpui_3d::{ColorOutput, Material, Scene, TextureColorSpace, ToneMapping};

let material = Material::image("albedo.png")
    .image_color_space(TextureColorSpace::Srgb);
let scene = Scene::new().color_output(ColorOutput {
    exposure: -1.,
    tone_mapping: ToneMapping::Reinhard,
});
```

Solid colors, material tints and light colors use sRGB RGB values in `[0, 1]`;
values outside that range are clamped. Image RGB defaults to `TextureColorSpace::Srgb`
and is decoded before Nearest or Linear filtering. `TextureColorSpace::Linear`
treats image RGB as linear values without transfer-function conversion. Alpha is
always linear and is unaffected by the color-space setting. The setting does not
interpret texture channels as normals, roughness, or other material parameters.

Lighting and image filtering operate in linear space. Light intensity and ambient
strength are linear multipliers and may exceed one. Shading uses an `Rgba16Float`
intermediate, retaining values up to 65504 per channel. Exposure and display
mapping are applied to each MSAA sample before averaging premultiplied display
colors. Captured UI colors are unpremultiplied and decoded
for shading; ordinary GPUI elements outside the viewport are not processed.

`ColorOutput` defaults to zero exposure and `ToneMapping::None`. Exposure is in
stops: +1 doubles intensity and -1 halves it. Rendering rejects non-finite exposure
or values outside `[-16, 16]`. `None` clamps the exposed result to the display
range. `Reinhard` compresses each channel with `x / (1 + x)` before sRGB encoding.
Both operate on straight color, then restore premultiplied coverage for GPUI
composition. Unlit materials bypass lighting, but still receive exposure and
tone mapping. Default output preserves unlit source colors up to numeric precision.

Viewport and headless rendering use the same color pipeline. Exposure and tone
mapping do not affect alpha cutout, depth, object IDs, or picking. Display outputs
are sRGB-encoded SDR. Headless `LINEAR_COLOR` exports premultiplied linear HDR
before exposure and tone mapping.

## Image sampling

```rust
use gpui_3d::{Material, TextureSampling, TextureAddressMode, TextureFilter, TextureMipFilter, UvTransform};

# fn main() -> Result<(), gpui_3d::UvTransformError> {
let sampling = TextureSampling {
    transform: UvTransform::from_scale_rotation_translation([2., 2.], 0.2, [-0.25, 0.])?,
    address_u: TextureAddressMode::Repeat,
    address_v: TextureAddressMode::Mirror,
    filter: TextureFilter::Linear,
    mip_filter: TextureMipFilter::Linear,
    max_anisotropy: 8,
    mag_filter: None,
};
let material = Material::image("tile.png").image_sampling(sampling);
# Ok(())
# }
```

`TextureSampling` defaults to identity UVs, Clamp on both axes, Linear texel filtering,
no mipmaps, and isotropic sampling (`max_anisotropy: 1`).
`UvTransform` applies scale, rotation about UV origin, then translation. Rotation
is in radians, positive clockwise in top-left-origin coordinates. `from_rows`
accepts two affine rows `[u, v, offset]`, including shear, reflection and zero
scale. Constructors reject non-finite coefficients. `transform(uv)` returns the
unaddressed coordinates, or `None` for non-finite input or overflow.

Clamp extends edge texels. Repeat tiles every unit interval, with linear
interpolation across the first/last texel seam. Mirror alternates forward and
reflected copies; negative coordinates follow the same period. U and V modes
are independent. Image UVs describe texel edges: texel `i` is centered at
`(i + 0.5) / extent`. Nearest selects one texel and Linear interpolates four
neighbors. Sampling remains inside the image, without accessing neighboring atlas tiles.

`filter` selects minification filtering within a level. `mag_filter` optionally
selects a different magnification filter; `None` uses `filter` for both. GPU
sampling chooses between them from the transformed UV footprint, including when
mipmaps are disabled.

`mip_filter` selects `None`, `Nearest`, or `Linear`: the original image only,
the nearest mip level, or interpolation between adjacent levels. With mipmaps
enabled, `max_anisotropy` controls the maximum sampling ratio for oblique surfaces,
from 1 through 16. Values above 1 require linear minification, magnification, and
mip filtering. Invalid combinations return a scene-preparation error before resource
resolution. Each material-map slot has its own sampling configuration.
Actual anisotropic filtering is backend-dependent; WGPU uses isotropic sampling
on devices without anisotropic-filtering support.

The WGPU renderer generates independent RGBA16Float mip chains on first use and
reuses them while referenced by prepared scenes. Images are decoded to linear
RGB before reduction; normal, metallic-roughness, and occlusion maps retain their
linear channel values. Alpha is averaged independently. Area-weighted reduction
includes odd image edges and supports one-pixel axes. Image identity, atlas
allocation generation, and color interpretation distinguish cached chains;
sampling changes reuse a chain while mipmapping remains enabled. Chain storage
uses eight bytes per texel summed across all levels, in addition to atlas storage.
One-sample and four-sample views in the same WGPU viewport renderer share mip
chains and samplers. Chains remain cached while any prepared view needs them;
separate windows and nested UI-capture renderers have independent caches.
Samplers unused by all prepared views are released, including after sampling
changes, material-map deactivation, and removal of the last viewport. Images
without mipmaps retain their active samplers independently of mip-chain storage.

GPU level selection uses derivatives of transformed, unwrapped UVs. Color,
object-ID, depth, and normal outputs share the same image-alpha sampling at a
given output resolution. Shadow maps select levels using their own projected
footprint. Mip generation does not preserve alpha-test coverage or sharpen normal
maps; use an appropriate cutoff and texture content for distant masked surfaces.

These settings apply only to image materials. Captured UI textures retain their
identity UV mapping and linear edge-clamped sampling, including pointer routing.
`Hit::uv` always contains the original mesh UVs. Viewport image-alpha picking
applies the material's UV transform, addressing, and magnification filter at level zero
before evaluating its alpha mode. CPU ray queries do not have a screen-space
sampling footprint, so these alpha queries can differ from GPU visibility
during minification. Interpolation near a cutoff can also differ at floating-point
precision boundaries between CPU and GPU.

## Related topics

[Lighting and environments](lighting.md).
