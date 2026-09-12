# Headless image residency

`HeadlessRenderer` maintains a private atlas of decoded images. Images needed by
the latest successful CPU preparation remain active until another preparation
replaces that set. Repeated references to the same `RenderImage::id` share one
atlas entry, including references from different material slots.

Idle images are released by default. `set_image_cache_limits` allows unused
images to remain resident for later scenes or camera views:

```rust
use gpui_3d::{HeadlessRenderer, ImageCacheLimits};

fn configure(renderer: &mut HeadlessRenderer) {
    renderer.set_preparation_capacity(2);
    renderer.set_image_byte_limit(Some(256 * 1024 * 1024));
    renderer.set_image_cache_limits(ImageCacheLimits {
        max_idle_images: 32,
        max_idle_bytes: 64 * 1024 * 1024,
    });
}
```

Both idle limits apply. When either limit is exceeded, the least recently used
images are removed; images used in the same preparation have equal recency.
Lowering a limit applies immediately without removing active images. A zero
image limit disables idle retention. A zero byte limit releases every idle image
with a nonempty pixel payload.

`image_cache_limits()` returns the configured idle limits. `image_cache_usage()`
reports active and idle image counts and pixel bytes separately. Bytes count the
first decoded BGRA frame once per image identity. They exclude atlas padding and
unused space, derived mipmaps, and caller-owned decoded pixel storage.

`set_image_byte_limit(Some(bytes))` limits the active image payload requested by
each preparation, including resident images and CPU cache hits. Shared identities
count once across objects and material slots; distinct identities count separately
even when their pixels match. Culled objects and inactive material slots do not
count. The default is `None`; `Some(0)` admits only preparations without image
inputs. `image_byte_limit()` returns this setting.

Admission is checked before the atlas lookup or allocation of the image that would
exceed the limit. A failure releases earlier new allocations from that preparation
and preserves previous residency. Lowering the limit does not evict existing
images; it applies to subsequent preparations. Idle retention remains controlled
separately by `ImageCacheLimits`. Neither limit is a total GPU memory quota.

Every preparation resolves current atlas references, including
[CPU preparation cache](preparation.md) hits. A reactivated idle image no longer
counts toward the idle limits. Prepared scenes do not own atlas allocations;
keeping a preparation alive alone does not keep its images resident.

On a CPU preparation error, newly allocated image entries are released and the
previous active and idle sets are preserved. Output admission errors occurring
before preparation leave residency unchanged. A later GPU rendering error does
not roll back a completed CPU preparation.

`clear_caches()` clears the private atlas and its residency records while
preserving the configured limits. Returned output frames and pending readbacks
remain independently owned. Idle residency is renderer-local, including when
renderers share a `WgpuContext`; it does not retain source `RenderImage` objects.

See [Headless output](headless.md) for rendering and output ownership.
