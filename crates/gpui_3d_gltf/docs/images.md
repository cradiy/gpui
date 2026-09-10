# Image decoding

The built-in decoder accepts PNG and JPEG from retained encoded resources.

- `EncodedImage::decode(limits)` returns one `Arc<gpui::RenderImage>`.
- `MaterialDefinition::decode_images(limits)` resolves all active material maps.
- `SceneDefinition::decode_images(limits)` resolves all active scene materials.
- `SceneDefinition::decode_resources(limits)` returns a transferable `DecodedScene`.

The material and scene methods share decoding by original image index. An image
used by several slots or materials is decoded and charged once per call. Separate
indices are charged separately even when they reference identical encoded bytes.
There is no persistent cache or filesystem/network access. Calls are synchronous.

```rust
use gpui_3d_gltf::{ImageDecodeLimits, PreparedDocument, SceneAsset, SceneOptions};

fn load_scene(document: &PreparedDocument) -> anyhow::Result<SceneAsset> {
    let definition = document.scene(None, SceneOptions::default())?;
    definition.decode_images(ImageDecodeLimits {
        max_dimension: 8192,
        output_bytes: 128 * 1024 * 1024,
        ..Default::default()
    })
}
```

## Pixel contract

Decoded output is row-major, top-left-origin, straight-alpha BGRA8. Grayscale and
RGB inputs expand to four channels; absent alpha becomes 255. Sixteen-bit PNG
channels are reduced to eight bits. The PNG default image is used without playing
animation. EXIF orientation and pixel-aspect metadata do not alter pixel layout.

Color profiles and gamma metadata do not apply transfer conversions. Image
channels retain their encoded values at the output bit depth; the material slot
determines sRGB versus linear sampling. Transparent pixels retain RGB values,
and normal-map channels are not inverted. These conventions follow the
[glTF image contract](https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html#images).

Content signatures select the decoder. When a MIME type is present it must match
`image/png` or `image/jpeg` for the detected format. Unsupported formats, MIME
conflicts, damaged data and exceeded limits return errors. Material/scene errors
retain the original image and binding context. Failures return no partial result
and may be retried without changing the definition.

## Limits and scheduling

`ImageDecodeLimits` controls admission:

- `max_dimension`: maximum width and height per image, checked by the codec.
- `max_pixels`: maximum width times height, checked before pixel decoding.
- `output_bytes`: strict aggregate BGRA payload limit for one call, checked before
  decoding each image's pixels. Existing retained images from other calls do not
  count toward this limit.
- `working_bytes`: per-image admission for encoded bytes plus native decoded
  pixels plus BGRA conversion storage. It is also passed to the codec as its
  best-effort internal allocation limit. The admission conservatively counts both
  pixel buffers even when conversion can reuse one.

The defaults are 16,384 pixels per dimension, 16,777,216 pixels per image, 256 MiB
of aggregate BGRA output and 256 MiB for per-image working admission. Encoded
resource limits remain separate. Header parsing occurs before pixel-count
admission; codec scratch space, allocator overhead, other retained outputs and
GPU copies are not a strict process-memory quota. Some codec allocation limits
are best-effort, so hostile-input isolation requires application-level controls.

Use `EncodedImage::decode` on caller-managed workers to prepare shareable decoded
images. Then use the existing material/scene `resolve_images` callback to supply
those images when constructing core materials and subtrees on the owning thread.
Custom formats, background queues and cross-asset caches also use `resolve_images`;
they do not inherit the built-in decoder's limits automatically.

For a complete scene, `decode_resources` performs the active-image decoding under
one aggregate budget and returns a `Send + Sync` result. `definition()` exposes
its source definition; `image(index)` returns an active decoded image by original
glTF index. Clones share the definition and pixel storage. The definition retains
encoded inputs as well as geometry; decoded output limits do not include those
retained inputs.

Call `DecodedScene::resolve()` on the destination thread to construct core materials
and a subtree with authored initial deformation. It reuses the decoded pixels
without decoding again. The resulting `SceneAsset` is not transferable between
threads. `decode_images` combines these two operations on the calling thread.
[`SceneLoadSlot::accept_with`](load_slots.md) can reject superseded worker results
before resolution.
