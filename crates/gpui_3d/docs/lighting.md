# Lighting and environments

[3D viewports](viewport.md)

## Direct lights

```rust
use gpui::rgb;
use gpui_3d::{Light, PunctualLight, Scene};

let scene = Scene::new()
    .light(Light { ambient: 0.05, ..Default::default() })
    .lights([
        PunctualLight::directional([-0.5, 0.8, 0.7]).intensity(0.3),
        PunctualLight::point([1., 1., 2.])
            .color(rgb(0xffbb88)).intensity(4.).range(Some(6.)),
        PunctualLight::spot([-1., 1., 2.], [0., -0.3, -1.])
            .cone_angles(0.2, 0.5).intensity(6.),
    ]);
```

`Scene::lights` replaces the direct-light list, preserving uniform ambient and
diffuse environment illumination. `MAX_PUNCTUAL_LIGHTS` is eight; rendering
rejects larger lists rather than silently truncating them. An empty list disables
direct illumination. `Scene::light` sets the single directional light and ambient
multiplier and clears the explicit list. Neither method changes the environment.

Light positions and directions are world-space values, independent of object
transforms. A directional vector points **toward the source**. A spot vector
points **outward along the beam**. Directions need not be normalized, but must be
finite and nonzero. Point lights ignore direction. All positions must be finite.
The core does not attach lights to scene nodes or load lighting from asset files.

`color` is sRGB RGB in `[0, 1]` with alpha ignored. `intensity` is a finite linear
multiplier in `[0, 65504]`, defaulting to one. Direct illumination is summed in
linear HDR before exposure and tone mapping. Basic lit materials use Lambert
shading; PBR materials use the same GGX/Smith/Schlick response for every source.
The intensity scale follows the material's existing shading model; it is not a
calibrated photometric exposure system.

Directional light has no distance attenuation. Point and spot lights use
`1 / max(distance, minimum_distance)^2`. `minimum_distance` defaults to 0.01
world units and accepts `[0.0001, 65504]`, keeping near-source intensity finite
without creating an area light. At the exact source position the direction is
undefined and the direct contribution is zero.

`range(Some(r))` sets a finite positive cutoff for point/spot lights; `None`
means unlimited range. Attenuation is multiplied by
`(1 - min(distance/r, 1)^4)^2`, reaching zero smoothly at the cutoff.
Spot half-angles satisfy `0 <= inner < outer <= pi/2`. Their cosines must remain
distinct at f32 precision. The default angles are zero and pi/4. Angular weight
is `clamp((cos(theta)-cos(outer))/(cos(inner)-cos(outer)), 0, 1)^2`, where theta
is measured from the outward beam axis. Distance and cone attenuation follow
the conventions of [KHR_lights_punctual](https://github.com/KhronosGroup/glTF/blob/main/extensions/2.0/Khronos/KHR_lights_punctual/README.md).

AO attenuates indirect illumination only, not these direct sources. Unlit
materials bypass every light. Lights do not change emission, alpha, picking,
object IDs, depth, or geometric normals. Point and spot lights do not compute
shadows. Viewport and headless rendering share the same light list.

The `lighting` example switches directional, point, and spot sources in the same
scene. Move the pointer to move the light, adjust its range and cone, or add a
directional fill. Right-drag to orbit and scroll to zoom.

## Directional shadows

```rust
use gpui_3d::{DirectionalShadow, Light, Scene};

let scene = Scene::new()
    .light(Light { direction: [-1., 2., 1.], ..Default::default() })
    .directional_shadow(Some(DirectionalShadow {
        resolution: 2048,
        softness: 1.5,
        depth_bias: 0.0005,
        normal_bias: 0.01,
        ..DirectionalShadow::new([0., 0., 0.], [4., 4., 6.])
    }));
```

Shadows are optional and default to off. `light_index` selects a directional
source in `Scene::lights`; zero selects the source configured by `Scene::light`.
Rendering rejects missing, point, or spot sources. Reordering or replacing the
light list does not change the index. `directional_shadow(None)` disables the map.

`center` is a world-space point. `half_extent` gives half-width, half-height and
half-depth along the light's local axes, with each extent finite and at least
0.0001. Local Z points toward the source. The projection uses world Y as its up
reference, or world Z when the direction is near vertical. Receivers outside
this volume remain lit; casters outside it cannot contribute to its map. Include
both casters and receivers in the covered volume, including casters outside the
view camera. The volume does not automatically follow the camera or fit scene bounds.

`resolution` accepts powers of two from 256 to 4096, subject to device limits.
The default is 2048. A smaller covered volume or larger map gives finer detail.
`softness` accepts `[0, 4]` in shadow texels: zero uses one hard depth comparison;
positive values spread a 3-by-3 PCF kernel with bilinear depth comparisons.
This is filtered shadow mapping, not physical area-light penumbra simulation.

`depth_bias` offsets receiver depth toward the source in normalized light depth;
it accepts `[0, 0.05]`. `normal_bias` is a finite nonnegative world-space offset
along the geometric surface normal, weighted by the light/surface angle. Defaults
are 0.0005 and 0.01. Increase offsets to suppress self-shadowing artifacts; excessive
values detach shadows from their casters. Normal maps do not change this offset.

`Object::cast_shadows` and `Object::receive_shadows` default to true. The same
builders on `Node` control its own mesh, not its descendants, and are retained
by evaluated scenes and subtree copies. Opaque and Mask materials cast shadows;
Mask uses the base texture's alpha, tint alpha, cutoff and UV sampling, including
captured UI textures. Blend materials receive shadows when lit but never cast
them. Unlit meshes can cast shadows but bypass shadow reception.

The shadow affects only its selected light's direct diffuse and specular terms.
Other lights, ambient/environment illumination, emission, output alpha, picking,
object IDs, depth and geometric normal outputs are unchanged. Viewport and
headless color rendering share the depth pass and sampling implementation.
Maps are reused by resolution within a renderer; disabled shadows allocate no
full-size map. There is one shadowed directional source per scene, without
cascades, contact shadows, or colored transparent shadows.

Run `cargo run -p gpui_3d --example lighting`. Move the pointer to steer sunlight,
right-drag to orbit, and scroll to zoom. Controls toggle shadows and soft edges,
cycle map resolution, and lift the objects above the ground.

## Diffuse environment lighting

```rust
use gpui_3d::{DiffuseEnvironment, Light, Scene};

// Row-major, linear HDR RGB radiance, with the top row facing +Y.
let pixels = [
    [0.2, 0.6, 2.0], [0.2, 0.6, 2.0],
    [0.4, 0.1, 0.02], [0.4, 0.1, 0.02],
];
let environment = DiffuseEnvironment::from_equirectangular([2, 2], &pixels)?;
let scene = Scene::new()
    .light(Light { ambient: 0., intensity: 0., ..Default::default() })
    .diffuse_environment(environment.intensity(1.5).rotation_y(0.5));
# Ok::<(), gpui_3d::EnvironmentError>(())
```

`DiffuseEnvironment` accepts decoded equirectangular radiance or nine precomputed
real spherical-harmonic coefficients. Source RGB must be finite and in
`[0, 65504]`; values above one retain their HDR energy. Decode image formats and
convert nonlinear color inputs to linear RGB before projection. The core does
not load environment files or render them as a background.

Projection integrates each piecewise-constant texel over its spherical area
and convolves the first three SH bands with the cosine kernel divided by pi.
Perform this step once when the source changes, then reuse the value across
scenes and frames. The GPU evaluates nine RGB coefficients per shaded pixel;
environment image dimensions do not affect per-frame storage or sampling cost.

Coordinates use `theta = pi*v`, `phi = 2*pi*u - pi`, and direction
`(sin(theta)*cos(phi), cos(theta), sin(theta)*sin(phi))`. Thus the upper pole is
`+Y`, the image center is `+X`, `u=0.75` is `+Z`, and the seam is `-X`.
`.rotation_y(radians)` applies a right-handed environment-to-world rotation
around `+Y` without reprojection. `.intensity(value)` is a finite linear
multiplier in `[0, 65504]`; zero disables the contribution. Neither control
changes the camera, objects, or GPUI background.

`from_coefficients` expects **irradiance divided by pi**, not raw radiance.
The coefficient order and orthonormal real SH basis are:

| Index | Basis |
| --- | --- |
| 0 | `0.28209479` |
| 1 | `0.48860251 * y` |
| 2 | `0.48860251 * z` |
| 3 | `0.48860251 * x` |
| 4 | `1.09254843 * x*y` |
| 5 | `1.09254843 * y*z` |
| 6 | `0.31539157 * (3*z*z - 1)` |
| 7 | `1.09254843 * x*z` |
| 8 | `0.54627422 * (x*x - y*y)` |

Each coefficient component must be finite and in `[-262016, 262016]`.
`coefficients()` returns the projected coefficients before intensity and rotation.
The representation follows the low-order irradiance approximation described in
[An Efficient Representation for Irradiance Environment Maps](https://graphics.stanford.edu/papers/envmap/).
It captures broad directional illumination, not sharp environment features.
Negative reconstructed irradiance from SH ringing is clamped to zero.

Basic lit materials add `base * irradiance/pi` to existing lighting. PBR materials
add `base * (1 - metallic) * (1 - F0) * irradiance/pi`, where
`F0 = mix(0.04, base, metallic)`, evaluated with the shading normal, including an
active normal map. This diffuse approximation is view-independent; it does not
provide specular IBL, roughness-dependent reflections, or geometry-derived occlusion.
Uniform ambient and direct lighting remain additive. Unlit materials bypass
environment lighting. Alpha, picking, object IDs, depth, and geometric normal
outputs are unchanged. Viewport and headless color rendering share the same path.

The `lighting` example toggles a colored HDR environment independently of direct
sources. Rotate the environment with the toolbar to inspect the illumination.

## Environment background

`EnvironmentMap` retains shared, immutable decoded linear RGB radiance. Its
equirectangular orientation matches `DiffuseEnvironment`: the top is +Y,
the middle column faces +X, and increasing U turns toward +Z. Texels must be
finite and in `[0, 65504]`; dimensions and pixel count are validated at construction.
Decoding and file/resource selection belong to the caller.

```rust
use gpui_3d::{DiffuseEnvironment, EnvironmentBackground, EnvironmentMap, Scene};

let map = EnvironmentMap::from_equirectangular([2, 1], vec![
    [0.2, 0.5, 1.5], [4.0, 1.5, 0.3],
])?;
let illumination = DiffuseEnvironment::from_map(&map)?.intensity(0.5);
let background = EnvironmentBackground::new(map)
    .intensity(0.25)
    .rotation_y(0.4);
let scene = Scene::new()
    .diffuse_environment(illumination)
    .background(Some(background));
# Ok::<(), gpui_3d::EnvironmentError>(())
```

`Scene::background(None)` leaves uncovered pixels transparent. A visible
background is opaque, including at zero intensity, which draws black. Background
intensity and world-Y rotation do not change diffuse illumination, direct lights,
shadow maps, or object picking. A single map can feed both background and diffuse
projection, with independent settings.

The background is infinitely distant: camera rotation and perspective field of
view change the sampled directions, but translating the camera and target together
does not introduce parallax. Orthographic rays are parallel, so the background is
a constant direction across the viewport. Background color is linearly filtered,
wraps across the horizontal seam, and clamps at the poles. Maps use RGBA16Float
GPU storage with binary16 precision; dimensions must fit the device texture limit.
Shared maps are uploaded once while in use and evicted from the background cache
when absent from the prepared scenes. Brightness, orientation, and camera changes
reuse the texture.

The background is composited before scene geometry in linear HDR. Transparent
objects blend over it; opaque surfaces cover it. Exposure and tone mapping apply
to the combined display result. Headless `LINEAR_COLOR` includes the background
before display mapping; object ID, linear depth, and world normal remain zero
where no geometry survives. The background neither writes depth nor casts shadows.
Viewport clipping, subtree effects, and group opacity apply to the combined output.

Use the `lighting` example's background visibility, brightness, and rotation
controls independently of the environment illumination controls.

## Specular environment lighting

`SpecularEnvironment` supplies distant reflections for metallic-roughness PBR
materials. Background visibility and diffuse illumination are independent.

```rust,no_run
use gpui_3d::{EnvironmentMap, Scene, SpecularEnvironment, SpecularPrefilter};

let map = EnvironmentMap::from_equirectangular([2, 1], vec![
    [0.1, 0.4, 1.0], [4.0, 1.0, 0.2],
])?;
let reflections = SpecularEnvironment::from_map(&map, SpecularPrefilter {
    resolution: 128,
    samples: 256,
})?.intensity(0.7).rotation_y(0.4);
let scene = Scene::new().specular_environment(Some(reflections));
# Ok::<(), gpui_3d::EnvironmentError>(())
```

Prefiltering is explicit, synchronous CPU work. Run it during asset preparation,
outside the frame loop; applications may use their own loading workers. Clones
share the resulting cube levels. Rotation and intensity changes reuse these
levels. `None` or zero intensity disables reflections. `SpecularPrefilter`
defaults to 128-pixel faces and 256 GGX samples per filtered texel. Face size
must be a power of two in `[16, 512]`, samples must be in `[16, 4096]`, and
`2 * resolution² * samples` must not exceed 64 million. Invalid quality or source
data returns `EnvironmentError`.

Level zero samples the source sharply. Successive cube levels cover evenly spaced
perceptual roughness values through one. Source mip reduction weights texels by
spherical area and handles non-power-of-two dimensions; filtered importance
sampling reduces aliasing from concentrated radiance. Higher sample counts reduce
prefilter noise but increase preparation cost. Higher face resolution retains
more detail in smooth reflections.

The renderer uses the [split-sum IBL approximation](https://google.github.io/filament/Filament.md.html),
combining prefiltered radiance with a 128 × 128 RG16Float BRDF lookup. The lookup
integrates GGX, height-correlated Smith visibility, and Schlick Fresnel using
512 samples per texel and is generated once when first needed by the renderer.
Cube storage is RGBA16Float, with linear filtering across faces and roughness
levels. The PBR path uses the reflected view direction and shading normal,
including normal maps, then applies metallic/F0, roughness maps, intensity, and
ambient occlusion. Direct light, emission, and geometry output channels are
unchanged. Basic diffuse and unlit materials do not receive specular IBL.

This is a single-scattering, infinitely distant environment approximation. It
does not reflect nearby scene objects or add local occlusion, parallax-corrected
probes, or multiple-scattering energy compensation. Ambient occlusion attenuates
indirect reflections as a scalar approximation. Exposure and display mapping
follow the normal color pipeline; `LINEAR_COLOR` retains linear reflected energy.

#### Prefiltered inputs

External preprocessors can supply `SpecularEnvironmentMap::from_prefiltered`
and pass the result to `SpecularEnvironment::from_prefiltered`. The input is a
complete power-of-two mip chain, ending at one texel per face, with base face
size at most 512. Each level is six contiguous square faces in the order below.
RGB is finite linear radiance in `[0, 65504]`. Let `s` and `t` span `[-1, 1]`
from left to right and top to bottom; normalize each listed direction.

| Face | Direction |
| --- | --- |
| +X | `(1, -t, -s)` |
| -X | `(-1, -t, s)` |
| +Y | `(s, 1, t)` |
| -Y | `(s, -1, -t)` |
| +Z | `(s, -t, 1)` |
| -Z | `(-s, -t, -1)` |

For `N` levels, level `i` represents perceptual roughness `i / (N - 1)`.
One-level inputs provide the same radiance at every roughness. The map contains
normalized GGX radiance convolution, without Fresnel or BRDF response baked in.
`map().levels()` exposes the prepared data for inspection and reuse. GPU caches
retain shared maps while used by prepared scenes and release unused cube maps.

The `lighting` example controls reflections, reflection rotation, and surface
roughness independently of the background and diffuse environment.

## Related topics

[Materials and textures](materials.md).
