# Document resources

`Document::from_slice(bytes, limits)` accepts JSON glTF or GLB 2.0. Parsing owns
the metadata and optional binary chunk without accessing external resources.
`Document::prepare(load_uri)` resolves all declared buffers and encoded images,
returning an immutable `PreparedDocument`. Both operations are synchronous and
can run in caller-managed background work; neither needs a window or GPU.
`prepare_async(load_uri)` accepts a future-returning loader; see
[asynchronous loading](loading.md) for scheduling, cancellation and byte budgets.

```rust
use gpui_3d_gltf::{Document, Limits};

let bytes = br#"{
    "asset": {"version": "2.0"},
    "buffers": [{"uri": "mesh.bin", "byteLength": 4}]
}"#;
let document = Document::from_slice(bytes, Limits::default())?;
let prepared = document.prepare(|uri| {
    anyhow::ensure!(uri == "mesh.bin", "unknown resource");
    Ok(vec![0, 1, 2, 3])
})?;
let data = prepared.buffer(0).unwrap();
# let _ = data;
# Ok::<(), anyhow::Error>(())
```

## Resolution and ownership

Non-data URIs reach the callback unchanged. The extension does not interpret
relative paths, decode percent escapes, access the filesystem, or initiate
network requests. The caller chooses allowed schemes and paths and handles
loading failures. Base64 data URIs with explicit MIME types are decoded
internally; other data-URI encodings or parameters return errors.

Identical URI strings are resolved once per preparation, even across buffers and
images. URI equality is textual; aliases are not canonicalized. A failed
preparation does not return partial resources or modify the document. It can be
retried with a new resolver. Resolver side effects are outside this transaction.

`PreparedDocument::buffer(index)` returns the buffer's declared bytes, excluding
GLB padding or extra resolver bytes. `image(index)` returns an `EncodedImage`
with borrowed encoded bytes and an optional MIME type. Buffer-view images share
their buffer storage; repeated URI images share the same payload. Document and
prepared-resource clones share immutable data, and prepared resources remain
valid after the parsed document is dropped.

Image data is not decoded or validated as an image format. A decoder must enforce
its own dimension, pixel-count, decoded-byte, and format limits. Conflicting
declared and data-URI MIME types are rejected during preparation. A URI image
without a declared type retains `None` for the caller's decoder to identify.

## Admission and validation

`Limits` controls total input bytes, total unique encoded resource bytes, and
buffer/image/accessor/node counts. Defaults are 16 MiB of input, 256 MiB of
resources, 4,096 buffers and images each, and 100,000 accessors and nodes each.
The input-byte limit includes the entire GLB container, not only its JSON chunk.
Count limits apply after bounded-input JSON parsing. Base64 sizes and declared
individual buffer sizes are checked before resource allocation or resolution.
Callback payload sizes are checked on return, so the callback must bound its own
I/O allocation. `resource_bytes()` reports admitted encoded bytes, not total
process or decoded-image memory.

GLB padding counts toward resource admission. Buffer-view images do not count
their shared bytes again; all extra bytes returned for a URI count even when a
buffer exposes only its declared prefix. Budgets and URI caches are per
preparation, not global limits across retained documents.

Validation checks container/chunk lengths, declared buffer-view bounds, accessor
component alignment, element strides and ranges, matrix-column padding, and
sparse storage bounds. Sparse indices must be strictly increasing and within the
accessor count. Byte-range arithmetic is checked for overflow. Errors include the
relevant buffer, image, view, or accessor context. These checks follow the
[glTF accessor layout](https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html#accessors)
and do not replace semantic validation of mesh attributes, animation data,
materials, or extensions.

`gltf()` exposes parsed format metadata with the original array indices. Resource
preparation does not create a `gpui_3d` scene, interpret materials, sample
animation, or claim support for an extension merely because its resources were
resolved. Scene conversion and decoded-image handling consume this prepared data
separately.

## Local inspection

The `inspect` example converts a local asset, decodes active images, instantiates
its selected scene, samples an optional animation, and prepares CPU spatial
indices. It prints resource counts, deformation counts, and world bounds without
opening a window or creating a GPU device.

```sh
cargo run -p gpui_3d_gltf --example inspect -- /path/to/model.glb
cargo run -p gpui_3d_gltf --example inspect -- /path/to/model.gltf \
  --scene 0 --animation 0 --time 0 --time 1.5 --time 3 --time 1.5
```

Without `--scene`, the file must declare a default scene. Animation selection is
explicit; omitting it evaluates authored transforms and Morph weights. Times are
absolute, nonnegative seconds, with zero as the default. Animation tracks outside
the selected scene are skipped and counted. Repeated times are sampled using the
same instance; a changed pose/geometry fingerprint returns an error. Fingerprints
are diagnostic values within a run, not portable asset IDs or visual comparisons.
They cover deformed positions, normals, coordinate sets, tangent bases, vertex
colors, indices, and evaluated node transforms. The summary distinguishes Skin
and Morph bindings from primitives that use both.

`--weights NODE:W0,W1,...` sets a complete Morph weight vector for an original
glTF node index. It overrides that node's animated or default weights at every
sample without changing bone animation. Repeat the option for different nodes;
duplicate nodes, nodes outside the selected scene, missing Morph targets,
non-finite weights, and incorrect weight counts return errors. Signed weights
are neither clamped nor normalized.

```sh
cargo run -p gpui_3d_gltf --example inspect -- /path/to/model.glb \
  --animation 0 --weights 3:0.25,0.75 --time 0 --time 1 --time 0
```

The example uses the default admission limits, including 16 MiB for the complete
document or GLB container. Resource files must be regular files within the asset's
canonical parent directory. Relative paths and percent-encoded filenames are
accepted; network/absolute URIs, query strings, fragments, and resolved directory
escapes are rejected. Use trusted local asset directories; this example does not
provide an operating-system sandbox against concurrent filesystem changes.
