# Headless rendering

Enable the `wgpu` feature to render 3D scenes without a native window, `App`,
or UI layout. `HeadlessRenderer` uses the same scene preparation, mesh pass,
lighting, and alpha-cutout shader as GPUI viewports.

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
tint, basic lighting, unlit shading, and alpha cutout. Images use their first
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
one or four color samples. `Scene3dChannels` selects Color, ObjectId, or both.
Only requested output attachments are created. Defaults request both channels
with four color samples.

| Channel | GPU format | CPU layout | Background and coverage |
| --- | --- | --- | --- |
| Color | `Rgba8Unorm` | RGBA bytes, width × 4 bytes per row | Transparent black; premultiplied alpha at MSAA edges |
| Object ID | `R32Uint` | `u32` values, width values per row | Zero background; nearest surviving surface at the pixel center |

Both images have a top-left origin. Readback strips GPU row padding. Color uses
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

Surviving alpha-cutout fragments are opaque. Both channels use the same mesh
visibility, transforms, clip planes, texture sampling, and alpha threshold.
IDs are written as integers, without color conversion, filtering, or MSAA
averaging. With four color samples, an edge pixel may have partial color coverage
but a zero ID when its center is outside the mesh. Use one color sample for
matching pixel-center coverage. Equal-depth overlaps follow submission order.
Blended transparency, depth/normal exports, and raw HDR output are not available.

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
cargo run -p gpui_3d --features wgpu --example headless -- /tmp/scene.png
```

Creates a scene, writes its color image, and prints the visible pixel count for
each object ID without opening a window. The output path defaults to `scene.png`.
The example uses a bounded polling loop for readback.
