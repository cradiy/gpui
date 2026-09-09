# Headless rendering

Enable the `wgpu` feature to render 3D scenes without a native window, `App`,
or UI layout. `HeadlessRenderer` uses the same scene preparation, mesh pass,
lighting, and alpha-mode shaders as GPUI viewports.

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
one or four color samples. Combine `Scene3dChannels::COLOR`, `OBJECT_ID`,
`LINEAR_DEPTH`, and `WORLD_NORMAL` with `|`, or use `Scene3dChannels::all()`.
The default selects color and object IDs with four color samples. Empty or
unknown channel selections are rejected. Unrequested channels are not allocated
or read back.

| Channel | GPU format | CPU layout | Background and coverage |
| --- | --- | --- | --- |
| Color | `Rgba8Unorm` | RGBA bytes, width × 4 bytes per row | Transparent black; premultiplied alpha |
| Object ID | `R32Uint` | `u32` values, width values per row | Zero background; nearest surviving surface at the pixel center |
| Linear depth | `R32Float` | `f32` values, width values per row | Zero background; positive camera-forward depth in scene units |
| World normal | `Rgba32Float` | `[f32; 4]` values, width values per row | Zero background; world XYZ normal and validity W |

All images have a top-left origin. Readback strips GPU row padding. Color uses
sRGB-encoded RGB after linear lighting, `Rgba16Float` intermediate storage,
the scene's `ColorOutput` exposure and tone mapping per sample, and display-color MSAA resolve.
Color conversion preserves premultiplied coverage at MSAA edges. The returned
`Rgba8Unorm` texture stores encoded values; GPU consumers must decode RGB when
using it in linear calculations. The internal HDR texture is not an exported
channel. Materials support diffuse or metallic-roughness shading through
`Material::pbr`, including linear emissive radiance and view-dependent highlights.
Metallic-roughness and emissive maps accept decoded `ImageSource::Render` inputs
with independent sampling. Their channels multiply the material factors; map
alpha does not affect coverage or object IDs.
Normal maps use linear tangent-space vectors and require mesh tangent data.
`normal_scale` controls XY strength; zero bypasses the map. Reflected transforms
and back-face orientation follow the viewport conventions. Picking and depth
remain geometric rather than normal-map perturbed.

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
Raw HDR output is not available.

### Geometry channels

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
requires 16 bytes per pixel, while the other channels require four.

```sh
cargo run -p gpui_3d --features wgpu --example headless -- /tmp/gpui-3d-outputs
```

The example writes aligned `color.png`, `ids.png`, `depth.png`, and `normals.png` previews.
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

## Limits and errors

`capabilities()` reports the device dimension limit, the 16,777,216-pixel output
budget, and four-sample color support. Dimensions must be positive and fit both
limits. Unsupported sampling, invalid camera/light/object parameters, unavailable
images, excessive readback buffer dimensions, and a busy readback return errors.
Device loss is reported when observed by the supplied GPU context; recovery
requires replacing the renderer/context. Unrecoverable backend allocation or
validation failures remain subject to WGPU's device error handling.

## Example

```sh
cargo run -p gpui_3d --features wgpu --example headless -- /tmp/gpui-3d-outputs
```

Creates a scene and writes color, object-ID, depth and normal previews without
opening a window. It prints each object's identity and visible pixel count.
The output directory defaults to `render-output`. ID colors are a display mapping,
not the exact integer channel. The example uses a bounded polling loop for readback.
