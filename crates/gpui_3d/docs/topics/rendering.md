# Rendering and resources

[3D viewports](../viewport.md)

## Instanced draws

Reuse a `Mesh` across objects to share GPU geometry and allow automatic WGPU
batching. Each object retains its own transform, base tint, visibility, and
picking identity.

```rust
use gpui_3d::{Material, Mesh, Object, Scene};

let mesh = Mesh::cube();
let mut scene = Scene::new();
for (index, color) in [0x8dd8e8, 0xf4cf89, 0x526a87].into_iter().enumerate() {
    scene = scene.object(
        Object::new(mesh.clone(), Material::color(gpui::rgb(color)))
            .id(format!("cube-{index}"))
            .position([index as f32 * 1.5, 0., 0.]),
    );
}
```

Adjacent opaque or masked objects with the same mesh storage, textures, sampling,
shading factors, and shadow settings share one instanced draw. Transforms, normal
matrices, base tints, and output IDs are per-instance inputs. Separately constructed
meshes are not deduplicated, even when their vertices match. Material changes
split batches. Blended objects retain their back-to-front order and individual
draws; batching does not reorder opaque or masked objects.

Use ordinary object values or `SceneGraph` edits followed by evaluation to supply
new instance data. Instance buffers retain their allocation while capacity fits
and grow within device limits. Unused batch slots are released. Uniform and
instance uploads are encoded before their draws, including shadows and headless
geometry channels, so previously encoded frames retain their inputs. CPU picking
and bounds remain per object.

## Resource preparation

`Scene::prepare(aspect, ui_texture, resolver)` produces a `PreparedScene` without
opening a window or requiring a GPU. It shares validation, culling, transforms,
material selection, and output IDs with viewport and headless rendering.

The resolver receives a `TextureRequest` containing the original object index,
output ID, application ID, node handle, material slot, and borrowed source.
Return `TextureState::Ready` with a matching `ResolvedTexture`, or
`TextureState::Pending` while the input is unavailable. Solid sources use
`ResolvedTexture::None`, images use `ResolvedTexture::Image(tile)`, and UI
captures use `ResolvedTexture::Subtree`.

```rust
use gpui_3d::{Material, Mesh, Object, ResolvedTexture, Scene, TextureSource, TextureState};

let scene = Scene::new().object(
    Object::new(Mesh::plane(), Material::image("cover.png")).id("cover"),
);
let prepared = scene.prepare(16. / 9., None, |request| {
    Ok(match request.source {
        TextureSource::Solid => TextureState::Ready(ResolvedTexture::None),
        TextureSource::Image(_) | TextureSource::Ui => TextureState::Pending,
    })
})?;
assert!(!prepared.is_ready());
let pending = &prepared.pending_textures()[0];
assert_eq!(prepared.object(pending.output_id).unwrap().id, Some("cover".into()));
# Ok::<(), gpui_3d::PrepareError>(())
```

All active inputs of an eligible object are requested even when another input
is pending. An object enters `frame().objects` only when every active input is
ready. Other objects can render while it waits. Disabled maps and meshes outside
both camera and shadow coverage make no requests. Readiness applies to the
current view, not every resource in the scene. Call `prepare` again after resource
completion or scene changes; preparation does not start tasks or schedule redraws.

Resolver errors return `PrepareError::Resource` with the object index, slot, and
underlying error. Incompatible ready texture kinds return `InvalidResolution`;
invalid scene inputs return `InvalidScene`. Errors return no prepared frame, but
uploads or other resolver side effects are not rolled back. There is no implicit
fallback material or retry policy.

`objects()` and `identities()` retain the complete original object mapping,
including pending and culled objects. `object(id)` returns `None` for zero or an
unknown ID. A later preparation does not mutate an earlier frame or its mapping.

Atlas tiles are renderer-local references. Upload decoded images with
`Window::prepare_effect_image` or the atlas exposed by
`WgpuScene3dRenderer::sprite_atlas`, and retain those allocations through render
submission. `PreparedScene` does not own atlas residency and must not be submitted
to a different renderer. Pass `frame()` to `WgpuScene3dRenderer::render`, or use
`into_frame()` with `Window::with_scene3d` and the corresponding UI capture.
Retain `identities()` with custom outputs before consuming the prepared scene.
File resolution, decoding, cache eviction, cancellation, and retries belong to
the resource manager.

## Retained preparation

See [Preparation caching](preparation.md) for scene reuse, capacity limits,
resource refresh, and viewport/headless ownership.

## CPU preparation benchmarks

```sh
cargo bench -p gpui_3d --bench scene
```

The workloads prepare 1,024 and 16,384 objects with shared geometry, mixed PBR
factors, mostly off-camera placement, and pending image inputs. Measurements
include scene validation, culling, resource callbacks, output construction, and
identity mapping. The `retained_preparation` group measures steady-state cache
hits with the same resource callbacks. Scene construction is outside the timed
region. These CPU-only
measurements do not include GPU uploads, draw encoding, shading, or readback.

## Draw statistics

With the `wgpu` feature, `Scene3dDrawStatistics::plan(frame, channels, limit)`
runs the WGPU mesh planner without an adapter. Pass a prepared frame and a
positive maximum instance count per batch. A renderer exposes its device-specific
limit through `max_instances_per_batch()`; an explicit limit also allows offline
comparisons of batch sizes. Planning does not validate scene inputs, atlas
residency, or device capabilities.

Statistics report camera and shadow draw calls, instance counts, and submitted
triangle counts. `batches`, `instance_upload_bytes`, and `uniform_upload_bytes`
describe per-submission payloads, not allocated buffer capacity. A batch shared
by camera and shadow work uploads once. Color and linear color share one shaded
pass; each selected object-ID, depth, or normal output has its own mesh pass.
Consequently, multi-channel counts include repeated work across outputs.

`RenderedFrame::gpu().draw_statistics()` retains the counts from the actual
submission's prepared plans without a GPU readback. Statistics do not include
fullscreen background/display draws, texture or geometry uploads, GPU timings,
occlusion, or pixel coverage. Frustum-culled meshes contribute no work, while
occluded meshes can still contribute draws and triangles.

```sh
cargo bench -p gpui_3d --features wgpu --bench scene -- draw_planning
```

These CPU workloads compare shared geometry, mixed materials, culled scenes, and
ordered transparency. Each workload verifies its expected instance and draw
counts before timing. Scene preparation is outside the measured region.

## GPU submission benchmarks

The `draw_encoding` group requires an explicit GPU opt-in:

```sh
GPUI_3D_GPU_BENCH=1 cargo bench -p gpui_3d --features wgpu --bench scene -- draw_encoding
```

It renders 1,024 and 16,384 cube instances at 256 × 256 with one sample per pixel,
using shared geometry, mixed PBR materials, mostly culled objects, and fixed-topology
vertex updates. Color-only and color/ID/depth/normal outputs run separately.
`vertex_updates` replaces the shared cube's vertices each iteration while retaining
its index storage; it measures upload and buffer-reuse overhead, not bulk transfer
bandwidth. Throughput counts visible instances across all selected output passes.

Timing covers `WgpuScene3dRenderer::render`: validation, retained draw preparation,
resource preparation, command encoding, output allocation, and queue submission.
Scene construction, vertex generation, initial pipeline warm-up, GPU completion
waits, result validation, and output release are outside the measured interval.
Only one submission is outstanding at a time, with a 30-second completion timeout.
These are CPU submission timings, not GPU timestamps, throughput under a deep
queue, readback latency, or visual validation.

The harness prints the selected adapter and checks expected draw/instance counts
against each submitted output. Unsupported selected channels fail explicitly.
Criterion filters select individual workloads, for example
`draw_encoding/shared_geometry/color/1024`. Without `GPUI_3D_GPU_BENCH=1`, this
group does not create a device or submit GPU work.

## Rendering and support

Linux WGPU supports these viewports. Check `window.supports_scene3d()` before
displaying 3D content; unsupported backends draw no mesh scene. Native Metal and
DirectX backends do not currently implement the mesh pass.

Each viewport has isolated depth visibility and is composited into GPUI's normal
paint order. Ancestor opacity applies once to the final image, and ancestor
clipping still applies. Mesh edges default to four samples when supported,
otherwise one. Captured viewports can be nested in other subtree effects.

## Viewport effects

Wrap a viewport with `gpui_effects::subtree_effect_chain` to apply Bloom and color
adjustment to its composed image. The wrapper shares the viewport's layout;
effect padding expands capture space without changing its camera aspect ratio
or pointer coordinates. Put toolbars outside the wrapper to leave them unaffected.

```no_run
use gpui::{prelude::*, px};
use gpui_3d::{Scene, viewport3d};
use gpui_effects::{BloomOptions, EffectStage, SubtreeColorOptions, subtree_effect_chain};

let view = subtree_effect_chain(
    viewport3d("scene", Scene::new()).size_full(),
    [
        EffectStage::bloom(BloomOptions {
            threshold: 0.7,
            radius: px(32.),
            ..Default::default()
        }),
        EffectStage::color_adjust(SubtreeColorOptions {
            saturation: 0.8,
            ..Default::default()
        }),
    ],
)
.map_interaction(true);
```

Stages run in order: color adjustment after Bloom also changes the halo's color.
Both stages preserve geometry and support identity pointer mapping; the halo does
not create additional interactive surfaces. Ancestor clipping also clips the
halo. Use stage-level `enabled(false)` to remove a pass, or disable the wrapper
to paint the viewport directly. The wrapper does not request animation frames
for these static stages.

The input is the viewport's display-encoded image after scene exposure and tone
mapping, including its environment background. These stages do not read the
linear HDR, depth, normal, or object-ID outputs, and they do not change geometric
picking. For HDR processing before display mapping or depth-dependent effects,
use the headless GPU output textures in a same-device rendering pipeline.

Check `window.supports_subtree_effects()` in addition to 3D support. On a backend
without subtree effects, the wrapper paints its content directly. The `lighting`
example exposes Bloom and Natural/Monochrome/Vivid color controls on the viewport.

## Raster quality

`resolution_scale` sets mesh raster density relative to physical render-surface pixels;
`color_samples` requests one or four samples per mesh pixel. Defaults are `1.0`
and `4`. These controls do not change logical layout, camera aspect ratio,
geometric picking, pointer routing, or UI capture density.

```no_run
use gpui_3d::{Scene, viewport3d};

let viewport = viewport3d("preview", Scene::new())
    .resolution_scale(0.5)
    .color_samples(1);
```

Scale must be finite and positive; invalid scale or sample count panics at the
builder call. Dimensions are scaled uniformly to fit the device texture limit,
rounded up, and kept at least one pixel on each axis. Lower scales reduce mesh
attachment memory and raster work; higher scales increase them. Non-native
resolutions use bilinear reconstruction of premultiplied display color. This is
not an area-filtered downsampling chain for large scale factors. Device limits
are not memory budgets; applications remain responsible for total GPU memory use.

Four samples fall back to one when unavailable. Use
`window.scene3d_support().capabilities()` and
`capabilities.color_samples_for(ViewportQuality::new(scale, samples))` to query
the effective count. Viewports in one window may use different configurations.
Low-level frames carry `Scene3dFrame::viewport_quality`; direct headless outputs
use `Scene3dOutputConfig` instead. Use `ui_texture_scale` independently to adjust
the raster density of captured UI.

## Allocation and visibility

Rendering conservatively rejects indexed mesh bounds outside the camera frustum
before allocating geometry buffers or uploading instance data. Bounds touching
a clip plane or crossing the camera plane remain eligible. Local bounds follow
vertex snapshots and are transformed with the object's full matrix, including
shear and reflections. Numerically uncertain cases remain eligible for GPU
clipping. Unreferenced vertices do not enlarge render-culling bounds.

Directional shadows use their own light-space clip volume. A mesh outside the
camera can still cast a visible shadow; disabling shadow casting or using Blend
removes that shadow-only work. Scene preparation resolves material images only
for meshes eligible for the camera or shadow volume. Moving a camera, changing
geometry, or changing shadow coverage reevaluates visibility on the next render.
This does not hide scene nodes, alter bounds or world-ray queries, or renumber
output IDs. It is not occlusion culling: geometry behind other objects still
participates in depth testing.

Geometry buffers are reused for shared meshes. Intermediate HDR color and depth
targets cover the viewport's pixel bounds intersected with the render surface,
rounded outward to whole pixels. Fractional layout positions keep their pixel
alignment. Viewports with the same target dimensions and sample count share
temporary attachments; other configurations have separate attachments retained
only while used by the current scene. A window resize preserves attachments whose viewport dimensions remain
unchanged. Fully off-surface viewports do not allocate mesh attachments.

Instance buffers grow in power-of-two steps capped by the device batch limit.
An active batch reuses its capacity while demand stays above one quarter of it;
at or below that threshold, the next preparation allocates a smaller buffer.
Removed batches release their buffers. The same policy applies to viewport and
direct-output passes, without changing draw order or object identities. Queued
commands retain any replaced resources they still reference.

Mesh output is placed back into surface coordinates for subtree composition and
enclosing effects. Generic subtree-composition textures remain surface-sized, so many
nested captures can still consume substantial GPU memory. UI capture and composition
run when GPUI repaints; there is no autonomous background render loop. UI layout,
texture sampling coordinates, picking, and pointer routing
are independent of mesh attachment dimensions.
UI texture targets and their rendering resources are reused while attached;
pixel-size changes resize the capture targets independently of the window.

## Submitted viewport outputs

WGPU retains visibility, transparent ordering, and instance-batch plans for
unchanged object snapshots, camera/shadow clip matrices, output mode, and batch
limits. Lighting or display settings that do not affect those inputs preserve
the plan. Unused plans are evicted on preparation. This CPU reuse does not depend
on command submission and does not skip resource checks or request UI frames.

WGPU window rendering reuses submitted mesh pixels when the immutable
`Scene3dFrame`, raster region, and referenced atlas generations are unchanged.
`Viewport3d` preserves frame identity across CPU preparation cache hits with the
same raster quality. Camera, material, lighting, geometry, or quality changes
produce a new frame and redraw the mesh.

Meshes sampling UI also compare captured paint content and image-pass inputs.
Equivalent freshly painted scenes can reuse mesh output. Changes to geometry,
style, clipping, draw order, shader parameters, nested frames, or referenced atlas
generations invalidate it. Particle, fluid, feedback, particle-transition, and
external-surface inputs bypass output reuse.

Independent UI textures retain submitted pixels separately from mesh outputs.
An unchanged capture does not redraw when the camera moves or unrelated UI
repaints. Text, images, paths, background blur, and stateless subtree effects are
eligible; animation time and effect parameters participate in content comparison.
Capture-size changes replace the texture. Layout, paint callbacks, resource
resolution, hit testing, and focus/event handling remain active; reuse skips only
GPU capture rendering. No application-managed dirty flag is required.

The mesh-output cache holds only viewport-covered display pixels. Each WGPU
window or external renderer has a shared 64 MiB budget by default, including
mesh outputs nested inside its independent UI captures. Other windows have
independent budgets even when they share a device.
Inputs must repeat before a pixel texture is allocated; continuously changing
snapshots do not allocate output-cache textures. Surface-size, transparency-mode,
and subpixel-layout changes invalidate retained outputs.
Entries are retained only for the current visible viewport list; oversized or
uncacheable inputs render normally. This limit excludes intermediate attachments,
UI capture textures, atlas resources, geometry, and outputs still held only by
submitted GPU commands. Abandoned
encodings never make an output reusable.

`Window::set_scene3d_output_cache_budget(bytes)` changes the shared limit; zero
disables mesh pixel reuse. Changing the limit releases all existing mesh output
entries immediately without clearing UI pixels, atlas images, or mesh buffers.
Setting the same limit preserves entries. The setting survives WGPU device
recovery and does not request a frame or wait for GPU completion.
`Window::scene3d_output_cache_stats()` returns the budget, retained bytes, and
texture count, or `None` when the backend does not expose this cache. Counts include
allocated cache textures awaiting submission, not total physical GPU memory.
`WgpuRenderer` and `WgpuOffscreenRenderer` expose the same controls and statistics.

`WgpuRenderer::draw_external` clears, renders, and submits an external target,
enabling the same output reuse without a native window. `WgpuOffscreenRenderer`
uses this path before readback. `encode_external` leaves submission to its caller
and does not reuse mesh or UI capture pixels. Subsequent captures use a different
texture after caller-owned encoding, isolating cached pixels from late submission
of older commands. Direct `HeadlessRenderer` outputs are
independent per-call textures, not viewport pixel-cache entries.

`Window::clear_scene3d_caches()` releases mesh buffers, intermediate mesh targets,
shadow maps, environment uploads, image mip chains, retained mesh output pixels,
and mesh pipelines for all viewports, including those inside UI captures.
Shared 2D atlas entries, UI capture textures, and generic
subtree-effect resources remain intact. Capture contents are invalidated, and the
next mesh draw rebuilds its caches.
The call does not request a repaint or wait for GPU completion, and unsupported
backends do nothing. In-flight commands retain the resources they use, so release
does not guarantee an immediate reduction in physical GPU memory use.

## Related topics

[Headless output](headless.md).
