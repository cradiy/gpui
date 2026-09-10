# Headless rendering

Enable the `wgpu` feature to render 3D scenes without a native window, `App`,
or UI layout. `HeadlessRenderer` uses the same scene preparation, mesh pass,
lighting, and alpha-mode shaders as GPUI viewports.

Material face visibility applies to every requested channel and directional
shadow casting. Single-sided materials preserve local front faces under reflected
transforms. Mask thresholds are finite and nonnegative: zero keeps all samples,
including zero-alpha samples, and values above one discard all samples. Opaque
and Blend ignore the cutoff.

```toml
gpui_3d = { path = "../gpui/crates/gpui_3d", features = ["wgpu"] }
```

```no_run
use gpui::rgb;
use gpui_3d::{Camera, HeadlessRenderer, Material, Mesh, Node, SceneGraph, Scene3dOutputConfig};

# fn main() -> anyhow::Result<()> {
let mut graph = SceneGraph::new();
let node = graph.insert(None, Node::new().id("model")
    .mesh(Mesh::cube(), Material::color(rgb(0x8dd8e8))))?;
let evaluated = graph.evaluate()?;
let camera = Camera::orbit(0.4, 0.3, 6.)
    .frame_bounds(evaluated.bounds().unwrap(), 16. / 9., 1.2)?;

let mut renderer = HeadlessRenderer::new()?;
let frame = renderer.render(
    &evaluated.scene(camera),
    Scene3dOutputConfig::new([1280, 720]),
)?;
assert_eq!(frame.object(1).unwrap().node, Some(node));

let color_texture = frame.gpu().color().unwrap();
let id_texture = frame.gpu().object_ids().unwrap();
let mut pending = frame.readback()?;
// Call again from the application's polling or task loop while the result is None.
if let Some(result) = pending.try_read()? {
    let rgba = result.pixels.rgba.as_ref().unwrap();
    let center_object = result.object_at(640, 360);
}
# Ok(())
# }
```

## Inputs and identity

Both `Scene` and `EvaluatedScene::scene(camera)` can be rendered. The caller
provides the complete scene state and camera for each call; output does not
depend on a window's size, DPI, last rendered frame, or camera controller.

Solid materials and `Material::image(Arc<RenderImage>)` are supported, including
tint, basic lighting, unlit shading, alpha cutout, and alpha blending. Images use their first
decoded BGRA frame. Decode resources before rendering. Resource paths, URLs,
encoded `Image` values, custom UI image loaders, and captured UI textures return
errors identifying the object; they are not silently omitted or loaded on the
rendering thread. Image atlas entries unused by the next preparation are released.

Numeric output IDs are local to a returned frame, not persistent scene IDs.
Each submitted mesh receives a nonzero `u32`; `RenderedFrame::objects()` maps it
to the source object index, optional application `ObjectId`, and optional stable
`NodeHandle`. Unnamed meshes also receive IDs. Hidden evaluated nodes are absent.
Picking modes do not affect render visibility or output IDs.

Use `frame.object(id)` or `ReadFrame::object_at(x, y)` to resolve IDs. Background,
unknown IDs, out-of-bounds coordinates, or an unrequested ID channel return
`None`. Keep each result's own mapping: IDs may be reassigned in later frames.
Returned mappings remain unchanged after graph edits; a retained node handle
can still be stale when queried against a modified graph.

## Output contract

`Scene3dOutputConfig` takes physical pixel dimensions, requested channels, and
one or four color samples. Direct rendering ignores `Scene3dFrame::viewport_quality`.
Combine `Scene3dChannels::COLOR`, `OBJECT_ID`,
`LINEAR_COLOR`, `LINEAR_DEPTH`, and `WORLD_NORMAL` with `|`, or use `Scene3dChannels::all()`.
The default selects color and object IDs with four color samples. Empty or
unknown channel selections are rejected. Unrequested channels are not allocated
or read back.

| Channel | GPU format | CPU layout | Background and coverage |
| --- | --- | --- | --- |
| Color | `Rgba8Unorm` | RGBA bytes, width × 4 bytes per row | Transparent black or configured environment; premultiplied alpha |
| Linear color | `Rgba16Float` | `[f32; 4]` values in `linear_rgba`, width values per row | Transparent black or configured environment; premultiplied linear HDR |
| Object ID | `R32Uint` | `u32` values, width values per row | Zero background; nearest surviving surface at the pixel center |
| Linear depth | `R32Float` | `f32` values, width values per row | Zero background; positive camera-forward depth in scene units |
| World normal | `Rgba32Float` | `[f32; 4]` values, width values per row | Zero background; world XYZ normal and validity W |

All images have a top-left origin. Readback strips GPU row padding. Color uses
sRGB-encoded RGB after linear lighting, `Rgba16Float` intermediate storage,
the scene's `ColorOutput` exposure and tone mapping per sample, and display-color MSAA resolve.
Color conversion preserves premultiplied coverage at MSAA edges. The returned
`Rgba8Unorm` texture stores encoded values; GPU consumers must decode RGB when
using it in linear calculations. Materials support diffuse or metallic-roughness shading through
`Material::pbr`, including linear emissive radiance and view-dependent highlights.
Metallic-roughness and emissive maps accept decoded `ImageSource::Render` inputs
with independent sampling. Their channels multiply the material factors; map
alpha does not affect coverage or object IDs.
Normal maps use linear tangent-space vectors and require mesh tangent data.
`normal_scale` controls XY strength; zero bypasses the map. Reflected transforms
and back-face orientation follow the viewport conventions. Picking and depth
remain geometric rather than normal-map perturbed.

`Scene::specular_environment` accepts shared GGX-prefiltered radiance for PBR
reflections. Viewport and headless rendering share cube upload, BRDF lookup,
roughness sampling, and normal-map handling. Prefilter on the caller's resource
preparation path, not in the render loop. Reflections affect both display and
linear HDR color, but not geometry channels or background visibility. See
[Specular environment lighting](lighting.md#specular-environment-lighting).

`Opaque` ignores alpha, `Mask` discards values below its cutoff and makes survivors
opaque, and `Blend` blends nonzero alpha using linear premultiplied source-over.
Color draws depth-writing surfaces first, then blended objects from far to near
by transformed bounds-center depth, without depth writes. Sorting is per object;
intersecting and self-overlapping transparent surfaces are not resolved.

Color and object IDs use the same mesh visibility, transforms, clip planes, texture
sampling, and alpha-mode discard rules. Object IDs select the nearest surviving
surface, including low-opacity blended surfaces, rather than the largest color contributor.
IDs are written as integers, without color conversion, filtering, or MSAA
averaging. With four color samples, an edge pixel may have partial color coverage
but a zero ID when its center is outside the mesh. Use one color sample for
matching pixel-center coverage. Equal-depth ID overlaps keep the first submitted
surface; equal-depth blended color layers compose in submission order.

### Object coverage

`ReadFrame::coverage()` summarizes an available object-ID channel without further
rendering, GPU submission, or readback. It scans the tightly packed image once
and uses one record per mapped object: O(pixels + objects) time and O(objects)
additional storage. Request `Scene3dChannels::OBJECT_ID` when rendering; missing
IDs return `CoverageError::MissingObjectIds`, not an empty summary.

```rust
use gpui_3d::{CoverageError, ReadFrame};

fn inspect(frame: &ReadFrame) -> Result<(), CoverageError> {
    let coverage = frame.coverage()?;
    let size = coverage.size();
    let camera = coverage.camera();
    let background = coverage.background_pixels();
    for entry in coverage.objects() {
        let identity = (entry.object.node, &entry.object.id);
        let pixel_count = entry.pixels;
        let fraction_of_frame = entry.screen_fraction;
        let pixel_bounds = entry.bounds;
    }
    Ok(())
}
```

`FrameCoverage` owns its summary and shares the immutable object mapping. It
retains the frame's camera and physical dimensions, but no GPU resources or pixel
buffers, and survives dropping or replacing the read frame. Callers associate
their scene revision, task, and animation time with that output. Modifying the
public CPU pixel data affects subsequent calls to `coverage()`, not earlier
summaries. Malformed dimensions, pixel counts, or IDs outside the frame mapping
return structured errors instead of partial statistics.

`objects()` includes all mapped objects in frame order, even when their count is
zero. `object(output_id)` returns `None` for background ID zero or unknown IDs;
an existing object with zero samples remains distinguishable from a missing
object. Graph nodes omitted from the evaluated scene have no output mapping.
Retain node handles or application IDs when comparing different frames, rather
than assuming numeric output IDs remain unchanged.

Counts use the ID channel's nearest surviving surface at each pixel center.
They are not alpha-weighted color contributions: a low-opacity `Blend` fragment
can own a sample ahead of an opaque object. `Mask` cutouts and clip planes follow
the rendered ID channel. Color MSAA and post-processing do not change these
statistics. Background samples have ID zero, including environment pixels.

`screen_fraction` is `pixels / (width * height)`, not the visible fraction of an
object's full projected surface. The per-object counts plus background count
equal the frame's `pixel_count()`. Pixel bounds have a top-left origin and
exclusive right/bottom endpoints, enclose every matching sample, and may contain
holes or samples belonging to other objects. Zero-count objects have no bounds.
Zero coverage alone does not distinguish occlusion, clipping, discarded alpha,
or subpixel geometry. These results describe this output's resolution, not
continuous geometric visibility or a different camera view.

### Linear HDR color

`LINEAR_COLOR` exports linear shaded radiance before exposure, tone mapping,
clamping to the display range, or sRGB encoding. It can be requested independently
of `COLOR`; requesting both shares the same scene shading pass.

```no_run
use gpui_3d::{HeadlessRenderer, Scene, Scene3dChannels, Scene3dOutputConfig};
# fn capture(renderer: &mut HeadlessRenderer, scene: &Scene) -> anyhow::Result<()> {
let frame = renderer.render(scene, Scene3dOutputConfig {
    size: [800, 600],
    channels: Scene3dChannels::LINEAR_COLOR,
    color_samples: 1,
})?;
let hdr_texture = frame.gpu().linear_color().unwrap();
let mut pending = frame.readback()?;
if let Some(result) = pending.try_read()? {
    let linear_rgba = result.pixels.linear_rgba.as_ref().unwrap();
}
# Ok(())
# }
```

The GPU texture is single-sampled `Rgba16Float`, retaining values above one up
to 65504 with binary16 precision. Readback widens each channel to `f32` without
color conversion; widening does not restore precision lost in half-float storage.
RGB is premultiplied by alpha, including transparent layers and edge coverage.
Divide RGB by nonzero alpha when a consumer requires straight color.

Four-sample HDR output averages linear samples. Display output instead applies
exposure, tone mapping, and sRGB encoding to each sample before averaging.
Converting resolved HDR to SDR therefore need not reproduce `COLOR` at edges.
Use `capabilities().linear_color_msaa4` to query four-sample HDR resolve support;
`color_msaa4` describes display output. Both outputs are independent of geometry
channel selection, and retained HDR frames survive later renders and resizing.

### GPU texture effects

`gpui_wgpu::WgpuTextureEffect` processes output textures with an `EffectShader`
without CPU readback, UI capture, or atlas upload. Construct processors once and
reuse them for successive frames. `render` submits one pass on the supplied
context's queue. `encode` appends a pass to a caller-owned command encoder, so
multiple effects can share one submission. Both return new owned textures;
later calls cannot overwrite an earlier output.

```no_run
use gpui::{EffectTextureOptions, EffectUniforms};
use gpui_3d::{HeadlessRenderer, Scene, Scene3dChannels, Scene3dOutputConfig};
use gpui_effects::{depth_fog_shader, hdr_tone_map_shader};
use gpui_wgpu::{TextureEffectConfig, WgpuTextureEffect, wgpu};

# fn process(renderer: &mut HeadlessRenderer, scene: &Scene) -> anyhow::Result<()> {
let context = renderer.context().clone();
let fog = WgpuTextureEffect::new(context.clone(), &depth_fog_shader(), TextureEffectConfig {
    inputs: vec![
        EffectTextureOptions { premultiplied_alpha: true, nearest: false },
        EffectTextureOptions { premultiplied_alpha: false, nearest: true },
    ],
    ..Default::default()
})?;
let display = WgpuTextureEffect::new(context.clone(), &hdr_tone_map_shader(), TextureEffectConfig {
    output_format: wgpu::TextureFormat::Rgba8Unorm,
    ..Default::default()
})?;
let size = [1280, 720];
let frame = renderer.render(scene, Scene3dOutputConfig {
    size,
    channels: Scene3dChannels::LINEAR_COLOR | Scene3dChannels::LINEAR_DEPTH,
    color_samples: 1,
})?;
let mut encoder = context.device.create_command_encoder(&Default::default());
let fogged = fog.encode(
    &mut encoder,
    &[frame.gpu().linear_color().unwrap(), frame.gpu().linear_depth().unwrap()],
    size,
    EffectUniforms::new()
        .with_slot(0, [3., 12., 0., 0.])
        .with_slot(1, [0.12, 0.18, 0.25, 1.]),
    0.,
)?;
let color = display.encode(
    &mut encoder, &[&fogged], size,
    EffectUniforms::new().with_slot(0, [0., 1., 0., 0.]), 0.,
)?;
context.queue.submit(Some(encoder.finish()));
let view = color.create_view(&Default::default());
// Bind this view in a subsequent same-device render pass.
# Ok(())
# }
```

The fog shader reads linear color and positive camera-forward depth, leaves zero
depth unchanged, and preserves color alpha. Slot 0.xy defines start/end distances
in scene units; slot 1 contains linear fog RGB and its strength in alpha. An equal
or reversed distance interval gives a hard transition at the start distance.
Geometry depth identifies only the nearest surviving surface, so fog is not a
volumetric integration through multiple transparent layers. Single-sample color
matches geometry coverage; MSAA-resolved color can differ at silhouette pixels.

The display shader applies exposure in stops from slot 0.x and optional Reinhard
mapping when slot 0.y exceeds 0.5, then encodes sRGB. Scene exposure and tone
mapping have not been applied to `LINEAR_COLOR`. Store this display result in
`Rgba8Unorm`, not an sRGB-encoding attachment. Both processors use premultiplied
output by default.

Inputs must be single-sample, single-layer 2D textures with `TEXTURE_BINDING`
usage from the same device. One, two, and four inputs may have different sizes.
Float-sampled formats include `R32Float` depth and `Rgba32Float` normals without
requiring float32 filtering features. Integer object IDs and native depth/stencil
formats are not accepted. Input sRGB formats use hardware decoding; other formats
are sampled as stored. The processor inserts no exposure or color-space conversion.

Each input independently chooses nearest or pixel-center bilinear sampling,
clamped at its edges. Premultiplied color is interpolated before unpremultiplication;
numeric data should set `premultiplied_alpha: false`. Effect functions receive and
return straight-alpha color. `load_effect_image` and the corresponding
`load_effect_second_image`, `load_effect_third_image`, and `load_effect_fourth_image`
helpers return raw clamped texels without alpha conversion. Uniform pixel values
are physical pixels and are not adjusted for window DPI.

Output formats are `Rgba8Unorm`, `Bgra8Unorm`, `Rgba16Float`, `Rgba32Float`,
`R32Float`, and `Rg32Float`, subject to device support. Outputs support sampling
and copying as well as render attachments. Floating formats retain HDR or signed
numeric RGB; alpha is clamped to `[0, 1]`. Set output `premultiplied_alpha: false`
when writing scalar/vector data. `max_output_bytes` defaults to 64 MiB per output;
`output_bytes(size)` checks its unpadded payload without a GPU. This excludes held
inputs, previous outputs, temporary buffers, and driver overhead. Zero rejects all
output allocations; `None` retains only device dimension/format limits.

The processor retains its pipeline, not rendered images or their readbacks.
Shader, input, encoding and submission validation errors are returned to the
caller by `render`. With `encode`, the caller owns finish/submission validation.
Create the encoder from `processor.context().device` or the same device passed
at construction. Inputs may be written earlier in the encoder, or by command
buffers submitted first on the same queue. An encoded output is not ready before
its producing commands are submitted. Dropping an unfinished encoder cancels its
work; output allocation alone does not execute an effect.

Metadata and input-binding failures occur before recording the pass. After an
encoding failure, discard the encoder: earlier commands cannot be rolled back.
WGPU does not expose an encoder's owning device for inspection; wrong-device
encoders follow WGPU's validation behavior rather than a guaranteed returned error.
Keep different-device command streams separate.

Retain and release output textures according to the consumer's lifetime;
do not call `destroy()` while queued work or another consumer uses them. Native
WGPU contexts are supported; this API does not import outputs into a `Window`
or automatically execute compound `EffectStage` pipelines.

### Geometry channels

`Scene::background` fills uncovered color pixels with a decoded HDR environment.
The background is opaque, including at zero intensity. It participates in linear
composition beneath transparent objects and appears in both color outputs;
exposure and tone mapping apply only to `COLOR`. Background visibility, brightness,
and rotation are independent of illumination. Geometry channels retain zero
background values and nearest-surface coverage. See
[Environment background](lighting.md#environment-background) for map orientation,
camera projection, filtering, and cache ownership.

```no_run
use gpui_3d::{HeadlessRenderer, Scene, Scene3dChannels, Scene3dOutputConfig};
# fn capture(renderer: &mut HeadlessRenderer, scene: &Scene) -> anyhow::Result<()> {
let frame = renderer.render(scene, Scene3dOutputConfig {
    size: [800, 600],
    channels: Scene3dChannels::OBJECT_ID
        | Scene3dChannels::LINEAR_DEPTH
        | Scene3dChannels::WORLD_NORMAL,
    color_samples: 1,
})?;
let depth_texture = frame.gpu().linear_depth().unwrap();
let normal_texture = frame.gpu().world_normals().unwrap();
let mut pending = frame.readback()?;
if let Some(result) = pending.try_read()? {
    let depths = result.pixels.linear_depth.as_ref().unwrap();
    let normals = result.pixels.world_normals.as_ref().unwrap();
}
# Ok(())
# }
```

Linear depth is the negated view-space Z coordinate, not hardware depth in
`[0, 1]` and not radial distance from the camera. It uses the same scene units
as object positions, for both perspective and orthographic cameras. Zero means
background for a valid camera whose near plane is positive.

`RenderedFrame::camera()` and `ReadFrame::camera()` retain the camera that
produced each output. Camera edits, resizing, and renderer destruction do not
change this snapshot. `ReadFrame::world_position_at(x, y)` reconstructs the
nearest surface from the depth sample at the physical pixel center. It requires
only `LINEAR_DEPTH`, not an ID or normal channel. Background, missing samples,
and coordinates outside the image return `Ok(None)`; invalid nonzero samples
or unrepresentable coordinates return a `CameraError`.

```no_run
# use gpui_3d::ReadFrame;
# fn position(result: &ReadFrame) -> Result<(), gpui_3d::CameraError> {
if let Some(world) = result.world_position_at(400, 300)? {
    let camera = result.camera();
    let view = camera.world_to_view(world)?;
}
# Ok(())
# }
```

GPU consumers can load the depth texture directly with `textureLoad`; no CPU
readback is needed. Use texel centers with top-left UV coordinates
`u = (x + 0.5) / width`, `v = (y + 0.5) / height`. Do not interpolate depth
across surface/background boundaries. For depth `d > 0`, the matching camera's
`projection_matrix(width / height)` and `axes()` give:

- `a = (2*u - 1 + lens_shift.x) / projection[0][0]`
- `b = (1 - 2*v + lens_shift.y) / projection[1][1]`
- `s = d` for perspective, or `s = 1` for orthographic
- `world = eye + s * (a * right + b * up) - d * backward`

This uses the unnormalized camera-plane direction, not a normalized picking ray.
`Camera::screen_to_world` provides the same reconstruction for screen coordinates
with an arbitrary viewport origin. Geometry-channel coverage is at pixel centers;
reconstructed positions do not represent partially covered color MSAA samples.

Normals are perspective-correctly interpolated vertex normals, transformed by
the inverse transpose, normalized, and oriented to the visible side using the
same double-sided/reflection convention as lighting. They are world-space
vectors in `[-1, 1]`, not display colors. Normal maps do not perturb this output.
W is one for a surface and zero for background; a surface with zero-length
vertex normals can have zero XYZ with W still one.

Both channels use single-sample pixel-center coverage and depth writes,
independent of color MSAA. Their nearest surface matches the ID pass, including
low-opacity blended surfaces and alpha cutouts. Values are not blended between
transparent layers, color converted, tone mapped, or averaged at edges.
Lighting changes do not affect these geometric outputs.

Use `capabilities().geometry_outputs` to check geometry-format support. All
outputs remain subject to dimension/pixel budgets and the per-buffer
`max_readback_buffer_bytes` limit, including row padding. The normal channel
requires 16 GPU/readback bytes per pixel, linear color requires eight, and the
other channels require four. CPU linear-color storage uses 16 bytes per pixel.

```sh
cargo run -p gpui_3d --features wgpu --example headless -- /tmp/gpui-3d-outputs
```

The example writes aligned `color.png`, `ids.png`, `depth.png`, and `normals.png` previews
and prints the maximum linear RGB radiance.
Depth is mapped from the visible range to grayscale with nearer surfaces brighter;
normal XYZ is mapped from `[-1, 1]` to RGB `[0, 1]`. Preview mappings do not change
the raw floating-point outputs. The translucent plane contributes blended color
but has a single surface depth and normal.

## GPU ownership and readback

`HeadlessRenderer::with_context` accepts an existing `WgpuContext`; `context()`
exposes the device and queue for GPU consumers. Output textures support sampling
and copying. Treat them as immutable. They belong to that device and must be
consumed with ordering after the submitted render commands. Do not transfer
them to another device or destroy them while another consumer holds the frame.

Frames own their output textures and identity maps. Subsequent renders and
dimension changes cannot overwrite an earlier frame. Geometry buffers are
cached by shared mesh allocation and reused between color and ID passes.
Unused geometry is evicted on preparation. Holding many output frames retains
their GPU memory; applications should drop frames they no longer need.

`readback()` starts GPU copies and asynchronous mapping without waiting. At most
one readback per renderer may remain unfinished, including frames retained from
earlier renders. A concurrent request returns an error. `try_read()` pumps GPU
callbacks without waiting and returns `None` until all requested channels are
ready. It returns a complete result once; polling after completion or failure
returns an error. Dropping the pending readback cancels mapping and releases
its permit after the cancellation callbacks finish. Later readback calls also
pump callbacks, so a busy result may require a later retry after cancellation.

The renderer does not spawn threads, run an executor, or schedule UI frames.
Use an application timer or worker loop to poll; avoid busy-spinning. Packing
CPU pixels takes place in `try_read()`, so large readbacks belong on a worker.
Image decoding and image-file encoding are caller responsibilities. GPUI image
sources with UI callbacks are not required to be transferable between threads;
construct the ready scene on the worker when needed.

### Cache release

Each renderer retains one CPU scene preparation. Unchanged scene clones reuse
validation, matrices, culling, and identity mapping while decoded-image atlas
references are refreshed on every render. Scene content, camera, aspect, and
resolved tile changes invalidate this cache. Output size changes that preserve
aspect reuse CPU preparation but still render into the requested output targets.
Every `render` call produces GPU output; this cache does not retain rendered pixels.

WGPU draw plans separately retain visibility, transparent ordering, and instance
batches for immutable object snapshots. Camera/shadow clip matrices, output mode,
and batch limits participate in the key. ID, depth, and normal channels share
compatible plans. Inactive plans are evicted during preparation, and resource
validation still runs on every render.

`HeadlessRenderer::clear_caches()` releases its cached mesh buffers, intermediate
targets, shadow maps, environment uploads, image mip chains, pipelines, and
private image atlas, along with retained CPU preparation and draw plans. The next
render rebuilds resources from the supplied scene.
It can be called repeatedly, including before the first render. Renderers sharing
a `WgpuContext` retain independent caches.

Returned frames and pending readbacks remain valid. Clearing caches does not
cancel a readback or release its concurrency permit. It does not wait for GPU
completion, destroy the device, or change its capability report. Resources held
by frames, submitted commands, or other GPU consumers remain allocated until
those owners release them; immediate physical memory reclamation is not guaranteed.

The lower-level `WgpuScene3dRenderer::clear_caches()` preserves its public image
atlas and existing tile references. Callers managing that atlas retain ownership
of its allocation lifetime. Window renderers expose
`Window::clear_scene3d_caches()` without clearing shared 2D resources.

## Limits and errors

### Target memory

`Scene3dOutputConfig::target_memory(shadow_resolution)` computes unpadded texture
payload without a device or allocation. `Scene3dTargetMemory` separates returned
outputs, intermediate attachments, and the optional directional shadow map,
with their sum in `total_bytes`. Supply the scene's shadow resolution or `None`.
Color and linear color share HDR/depth attachments; each geometry channel has
its own depth attachment. MSAA applies to the shaded attachments, and a shadow
map contributes only when a shaded channel is selected.

```no_run
use gpui_3d::{HeadlessRenderer, Scene, Scene3dOutputConfig};

# fn main() -> anyhow::Result<()> {
let config = Scene3dOutputConfig::new([1280, 720]);
let memory = config.target_memory(None)?;
let mut renderer = HeadlessRenderer::new()?;
renderer.set_target_byte_limit(Some(128 * 1024 * 1024));
let frame = renderer.render(&Scene::new(), config)?;
assert_eq!(frame.gpu().target_memory(), memory);
# Ok(())
# }
```

`set_target_byte_limit` sets a per-request admission limit. The default is `None`;
`Some(0)` rejects every render. Over-budget requests fail before image uploads,
target allocation, or submission, leaving previous frames and caches unchanged.
The lower-level `WgpuScene3dRenderer` exposes the same limit and
`validate_target_memory(config, shadow_resolution)` for preflight checks against
device limits and the configured budget. Changing the limit does not release
resources; `clear_caches()` preserves the limit.

The report counts the requested layout even when cached attachments are reused.
It excludes retained older frames, transient overlap between submissions, GPU
allocation alignment, readback/staging buffers, meshes, material/environment
textures, and pipeline resources. It is neither current nor peak physical GPU
memory, and the per-request limit is not a device-wide residency quota.

### Output limits

`capabilities()` reports the device dimension limit, the 16,777,216-pixel output
budget, and separate four-sample display/HDR color support. `channels()` returns
the available output mask; `color_sample_counts(channels)` returns the supported
color sample counts for that selection, or an empty slice for unsupported
channels. Geometry outputs retain pixel-center sampling even when four color
samples are selected. Dimensions must be positive and fit both limits.
Unsupported sampling, invalid camera/light/object parameters, unavailable
images, excessive readback buffer dimensions, and a busy readback return errors.
Device loss is reported when observed by the supplied GPU context; recovery
requires replacing the renderer/context. Unrecoverable backend allocation or
validation failures remain subject to WGPU's device error handling.

### Device capabilities

`device_capabilities()` exposes a snapshot of the current WGPU context: adapter
name, backend and device type, advertised and enabled features, enabled limits,
downlevel flags, atlas format, effective image anisotropy, and per-format usage
and feature flags. Each format distinguishes adapter support from features
enabled on the device. Inspect the device flags for filtering, blending, sample
counts, and resolve support; raw format support does not expand the renderer's
one/four-sample output contract.

Query an existing context before creating a renderer when diagnostics are needed
even on a device that cannot run the mesh pipeline:

```rust,no_run
use gpui_3d::{HeadlessRenderer, Scene3dDeviceCapabilities, WgpuContext};

# fn main() -> anyhow::Result<()> {
let context = WgpuContext::new_headless()?;
let device = Scene3dDeviceCapabilities::query(&context);
let outputs = device.rendering()?;
println!("Backend: {:?}; outputs: {:?}", device.adapter_info.backend, outputs.channels());
let renderer = HeadlessRenderer::with_context(context)?;
# Ok(())
# }
```

The query does not allocate targets, create pipelines, submit commands, or poll
the GPU. `rendering()` checks output/image/environment/depth formats, comparison
samplers, uniform and vertex layouts, binding counts, and resource limits. A
failure identifies the unavailable format feature or insufficient enabled
limit. Renderer construction uses the same check. Depth and normal availability
is reported together through `geometry_outputs`; individual raw format flags
remain available in the device snapshot.

These reports describe the WGPU mesh path, not native-window presentation or
other GPUI renderers. They do not guarantee available memory, device health, or
successful rendering on an unvalidated platform. Query a new context after
device replacement.

`Scene3dDeviceCapabilities::query_with_formats(context, formats)` includes
additional target formats. `viewport(target_format)` checks a queried format
for the composited mesh path without requiring direct geometry outputs. Use
`Window::scene3d_support()` for a live platform window's actual support state;
the device snapshot alone does not indicate whether its platform renderer
implements mesh viewports.

## Example

```sh
cargo run -p gpui_3d --features wgpu --example headless -- /tmp/gpui-3d-outputs
```

Creates a scene and writes color, object-ID, depth and normal previews without
opening a window. It prints the adapter and output capabilities, then each
object's identity and visible pixel count.
The output directory defaults to `render-output`. ID colors are a display mapping,
not the exact integer channel. The example uses a bounded polling loop for readback.
