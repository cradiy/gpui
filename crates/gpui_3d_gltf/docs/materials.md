
# Materials

`PreparedDocument::material(index)` creates a `MaterialDefinition` from an
original glTF material index. Pass `None` for the implicit glTF material: white,
metallic 1, roughness 1, no emission, opaque and single-sided. Definition creation
validates parameters and active texture bindings without decoding images or
accessing a GPU.

`resolve_images(decode)` produces a core `gpui_3d::Material`. The callback receives
the original image index and its retained `EncodedImage`. It returns an
`Arc<gpui::RenderImage>` containing straight-alpha BGRA pixels. Preserve encoded
RGB values: do not linearize colors, premultiply alpha, flip UVs, or invert the
normal map's green channel during decoding. Sampling interprets each image's
channels according to its slot.

```rust
use std::sync::Arc;
use anyhow::Context;
use gpui::RenderImage;
use gpui_3d::Material;
use gpui_3d_gltf::{EncodedImage, GeometryOptions, PreparedDocument, PrimitiveGeometry};

fn convert(
    prepared: &PreparedDocument,
    mesh_index: usize,
    primitive_index: usize,
    decode: impl FnMut(usize, &EncodedImage) -> anyhow::Result<Arc<RenderImage>>,
) -> anyhow::Result<(PrimitiveGeometry, Material)> {
    let mesh = prepared.gltf().meshes().nth(mesh_index).context("missing mesh")?;
    let primitive = mesh.primitives().nth(primitive_index).context("missing primitive")?;
    let definition = prepared.material(primitive.material().index())?;
    let generate_tangents = definition.requires_tangents()
        && (primitive.get(&gltf::Semantic::Tangents).is_none()
            || primitive.get(&gltf::Semantic::Normals).is_none());
    let geometry = prepared.geometry(mesh_index, primitive_index, GeometryOptions {
        tex_coord_set: definition.tex_coord_set().unwrap_or(0),
        generate_tangents,
        ..Default::default()
    })?;
    definition.validate_geometry(&geometry)?;
    let material = definition.resolve_images(decode)?;
    Ok((geometry, material))
}
```

## Factors and texture semantics

Base-color factors are linear in glTF. Conversion encodes their RGB values into
the core's sRGB tint representation; rendering decodes that tint before
multiplication. Alpha remains linear. Metallic, roughness and emissive factors
retain their numeric values. Factors outside their glTF ranges return errors.

Opaque, Mask and Blend map directly to core alpha modes. Mask cutoffs preserve
zero and values above one. `doubleSided` controls face visibility, including
reflected transforms. `KHR_materials_unlit` bypasses lighting and emission.

| Slot | RGB interpretation | Sampled channels |
| --- | --- | --- |
| Base color | sRGB | RGB color, linear alpha |
| Metallic-roughness | Linear | G roughness, B metallic |
| Emissive | sRGB | RGB emission multiplier |
| Normal | Linear | RGB tangent-space direction |
| Occlusion | Linear | R indirect-light attenuation |

`textures()` lists active bindings with original texture/image indices, encoded
data, UV set, sampling configuration and color space. Unlit definitions request
only base color. Zero normal scale, zero occlusion strength, and zero emissive
factors suppress their corresponding bindings. Active negative normal scales
are unsupported and return an error.

## Sampling and geometry requirements

Clamp, Repeat and Mirror addressing and all six glTF minification modes are
mapped explicitly. With no specified filters, conversion uses linear texel and
trilinear mip filtering. If only magnification is specified, its texel filter is
also used for minification with linear mip interpolation. If only minification
is specified, magnification uses the same texel filter. Explicitly different
magnification/minification texel filters are unsupported by the core sampler and
return an error. Anisotropy is one.

`KHR_texture_transform` applies scale, rotation and offset to each slot's sampling
coordinates. Its `texCoord` overrides the texture's original set, including for
normal and occlusion maps. The core mesh has one UV set, so all active bindings
must use the same set. Different per-slot affine transforms are supported;
different active UV sets return an error.

`tex_coord_set()` and `requires_tangents()` expose geometry requirements.
`validate_geometry()` verifies those requirements against a converted primitive.
It allows material overrides and does not require matching source material
indices. Tangents use the selected mesh UVs; image decoding does not generate or
modify them. Required extensions other than `KHR_materials_unlit` and
`KHR_texture_transform` return errors; unknown optional extensions use core glTF
fallback behavior and are not interpreted as supported features.

## Resource ownership

Definitions retain encoded image payloads independently of the prepared document.
Definitions are `Send + Sync`; core materials are created during image resolution.
Each image index is decoded at most once per `resolve_images` call, even when used
by several slots with different color-space or sampling semantics. The resulting
material retains shared decoded images. Only the first frame is used; empty first
frames are rejected.

`decode_images(limits)` provides built-in PNG/JPEG decoding. Definitions own no
filesystem policy, global image cache, or GPU upload.
The callback must enforce decoded dimensions, pixel counts, allocation budgets,
and supported formats before allocating. It may use caller-managed background
work and caches. A failed resolution returns no partial material and can be
retried; external callback side effects are not rolled back. Errors include
material, slot, texture and image context where applicable.
