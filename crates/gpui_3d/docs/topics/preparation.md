# Preparation caching

`PreparationCache` retains `Arc<PreparedScene>` values. Unchanged `Scene` clones
and scenes created from the same `EvaluatedScene` snapshot reuse validation,
culling, matrices, and identity mapping. Camera values, aspect ratio, UI logical
dimensions, and raster density select separate entries. Content builders,
new scenes, and new evaluated snapshots require preparation even when their
values match an older scene.

`new()` and `default()` retain at most one entry. `with_capacity(entries)` selects
a different limit; `set_capacity(entries)` changes it immediately. Eviction removes
the least recently used entry. Zero disables retention. The limit counts entries,
not bytes; larger scenes can retain more memory per entry. Increasing the limit
does not preallocate preparations.

```rust
use gpui_3d::{Camera, Material, Mesh, Object, PreparationCache, ResolvedTexture, Scene, TextureState};

let scene = Scene::new().object(Object::new(Mesh::cube(), Material::color(gpui::white())));
let side = scene.clone().camera(Camera::orbit(0.5, 0.2, 5.));
let mut cache = PreparationCache::with_capacity(2);
let resolve = |_: gpui_3d::TextureRequest<'_>| Ok(TextureState::Ready(ResolvedTexture::None));
let first = cache.prepare(&scene, 1., None, resolve)?;
cache.prepare(&side, 1., None, resolve)?;
let next = cache.prepare(&scene, 1., None, resolve)?;
assert!(std::sync::Arc::ptr_eq(&first, &next));
# Ok::<(), gpui_3d::PrepareError>(())
```

Scene validation completes before the first resource request. Every call invokes
the resolver once per active input, including cache hits. Pending/ready transitions
and changed atlas tile references rebind the selected output using its retained
geometry plan. Matrices, culling, sort depths, and identity mapping remain shared
or unchanged; objects with pending inputs are omitted until ready.
An error discards the matching entry and returns no preparation; unrelated
entries remain available. Eviction and `clear()` release retained CPU inputs
without invalidating previously returned preparations or clearing renderer caches.

Viewports retain a default-capacity cache under their element ID.
`HeadlessRenderer::set_preparation_capacity(entries)` controls its renderer-local
cache. Keep scenes or evaluated snapshots in application state and clone them
when rendering. Retaining additional CPU preparations does not retain their image
atlas allocations or GPU draw plans.

This is not a rendered-image cache. UI layout, painting, and pick-surface
resolution still run normally; headless rendering still submits GPU work.
Unchanged tile references do not imply unchanged pixels. Resource managers must
resolve current residency, retain allocations through submission, and request
redraws when asynchronous inputs change.

The CPU-only alternating-camera benchmark compares one-entry and two-entry
caches over the same two views:

```sh
cargo bench -p gpui_3d --bench scene -- alternating_cameras
```

The `resource_rebinding` workload compares cache bypass with retained geometry
plans at 1,024 and 16,384 objects. Each iteration performs two preparations:
either pending then ready inputs, or two different ready atlas references.
Texture references are supplied directly; image decoding, atlas allocation, and
GPU uploads are excluded.

```sh
cargo bench -p gpui_3d --bench scene -- resource_rebinding
```
