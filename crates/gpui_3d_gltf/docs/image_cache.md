# Decoded image cache

`ImageCache` shares decoded PNG/JPEG pixels across materials, scenes and workers.
Clones share entries and capacity limits. The cache does not perform I/O, upload
GPU textures or schedule work.

## Usage

```rust
use gpui_3d_gltf::{DecodedScene, ImageCache, ImageDecodeLimits, SceneDefinition};

fn prepare_pixels(
    definition: &SceneDefinition,
    cache: &ImageCache,
) -> anyhow::Result<DecodedScene> {
    definition.decode_resources_cached(cache, ImageDecodeLimits::default())
}
```

Keep the cache across loads. Run `decode_resources_cached` on a caller-managed
worker, then call `DecodedScene::resolve()` on the destination thread. The decoded
scene is transferable; the resolved `SceneAsset` is not. Use
[load slots](load_slots.md) to reject superseded results before resolution.

`ImageCache::decode(encoded, limits)` returns one `Arc<gpui::RenderImage>`.
`MaterialDefinition::decode_images_cached(cache, limits)` resolves active material
maps on the calling thread. Output follows the [image pixel contract](images.md#pixel-contract).

## Identity and admission

Keys combine the SHA-256 digest of the encoded image slice with its declared MIME
type. Matching content and MIME can share pixels across documents and image
indices. An absent MIME is distinct from a declared MIME. A conflicting declaration
does not reuse an entry admitted under another MIME. Hashing occurs on each lookup;
neither hashing nor decoding holds the cache lock.

Every hit enforces the current `ImageDecodeLimits`: dimensions, pixel count,
aggregate output bytes and conservative per-image working admission. Working
admission counts encoded bytes, native pixels and BGRA conversion storage even
on hits; no codec work is performed on a hit.

Material and scene calls charge each active original image index once. Separate
indices sharing one cache entry still count separately toward the call's output
budget. A failed call returns no partial material or scene, but images successfully
cached before the failure remain reusable. Errors are not cached.

## Retention and release

`ImageCacheLimits` defaults to 256 MiB of BGRA payloads and 256 entries. Successful
hits promote recency. Insertion evicts least-recently-used entries until both limits
fit. Either zero limit disables retention. An image larger than cache capacity,
but allowed by its decode limits, is returned without retention or unrelated eviction.

`set_limits()` applies capacity changes immediately. `invalidate(encoded)` removes
the matching content/MIME entry; `clear()` releases all retained entries. These
operations leave previously returned images valid. Invalidation, clearing and a
limit change prevent all already-started cache misses from inserting on completion,
but do not interrupt decoding or retract a caller's result.

Concurrent misses decode independently. Only the first insertion for a retained
key is cached; each caller receives its own decoded result. Applications control
concurrency and cancellation of worker tasks.

`cached_bytes()` counts retained BGRA payloads, not total live memory. Metadata,
codec scratch allocations, consumer-held images and GPU copies are excluded.
Entries retain pixels and admission metadata, not encoded buffers. Prepared
documents and decoded scene definitions may independently retain encoded inputs.
Encoded-byte reuse is configured separately through [ResourceCache](cache.md).
