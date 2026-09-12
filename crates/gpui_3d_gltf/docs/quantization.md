# Quantized geometry

`KHR_mesh_quantization` enables integer geometry inputs to
`PreparedDocument::geometry` and `PreparedDocument::scene`. The extension must
appear in both `extensionsUsed` and `extensionsRequired`. Without it, the base
glTF attribute formats apply.

| Attribute | Additional input formats |
| --- | --- |
| POSITION | Signed or unsigned 8/16-bit VEC3, normalized or unnormalized |
| NORMAL | Normalized signed 8/16-bit VEC3 |
| TANGENT | Normalized signed 8/16-bit VEC4 |
| TEXCOORD_n | Signed 8/16-bit VEC2, normalized or unnormalized; unnormalized unsigned 8/16-bit VEC2 |
| Morph POSITION | Signed 8/16-bit VEC3, normalized or unnormalized |
| Morph NORMAL / TANGENT | Normalized signed 8/16-bit VEC3 |

Float attributes remain supported. Base glTF normalized unsigned UVs, colors,
joint indices and weights retain their existing interpretation. UV Morph targets
and custom attributes are unsupported and return errors.

## Decoding and transforms

Conversion produces the same floating-point core mesh representation as ordinary
geometry. Integer storage reduces encoded asset size, not core vertex allocation.
Unnormalized components retain their numeric value. Normalized signed components
use the signed positive maximum as divisor and clamp the negative endpoint to -1;
unsigned components divide by their full positive range. Normals are normalized,
and authored tangent bases use the core's orthogonalization and handedness rules.

Dequantization uses authored transforms: node transforms for ordinary positions,
inverse bind matrices for skin bindings, and `KHR_texture_transform` for UVs.
No additional bounds-derived scale, UV clamp, axis conversion or transform is
inserted. Mesh bounds come from decoded vertices rather than accessor metadata.

Interleaved and sparse inputs are supported, including sparse accessors without
base buffer views. Omitted base elements start at zero. Quantized vertex buffer
views require four-byte-aligned offsets and element strides; packed VEC3 values
need an explicit padded stride. Sparse replacement values retain the accessor's
packed component layout.

Morph deltas decode before vertex remapping and evaluation. Generated vertex
splits preserve correspondence with all UV sets, Morph inputs and skin influences.
Scene conversion applies default Morph weights before skinning.

Existing [geometry admission](geometry.md) and [scene limits](scenes.md) apply to
decoded element counts and generated output, regardless of encoded component size.
Malformed declarations, unsupported formats and invalid layouts return errors;
conversion never interprets an integer accessor as float bytes.
