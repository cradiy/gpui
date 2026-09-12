# Geometry memory

`Scene3dGeometryMemory::plan(frame, channels)` reports the vertex and index payload
required by a valid prepared frame without creating an adapter or GPU resources.
It uses the renderer's camera and shadow eligibility rules. Shadow-only geometry
counts when a shaded output is requested; geometry outside both relevant clip
volumes does not count.

The report counts each mesh allocation and active material-coordinate combination
once across instances and output channels. Two separately allocated meshes count
separately even when their contents match. Different active UV combinations need
separate packed vertex/index buffers. Stored but inactive coordinate sets do not
increase the packed vertex stride. Unreferenced stored vertices still occupy
space in their mesh's vertex buffer.

Fields report geometry entries, vertex bytes, index bytes, their total, and the
largest individual buffer with a referencing frame-local object ID. The report
excludes instance and uniform buffers, vertex staging copies, textures, previous
frames, transient overlap, and driver overhead. It is request payload, not current
GPU allocation or total process memory. Cache hits do not change its values.

```rust
use gpui_3d::{Material, Mesh, Object, ResolvedTexture, Scene, Scene3dChannels,
    Scene3dGeometryMemory, TextureState};

let scene = Scene::new().object(Object::new(Mesh::cube(), Material::color(gpui::white())));
let prepared = scene.prepare(1., None, |_| Ok(TextureState::Ready(ResolvedTexture::None)))?;
let memory = Scene3dGeometryMemory::plan(prepared.frame(), Scene3dChannels::all())?;
println!("{} meshes, {} geometry bytes", memory.meshes, memory.total_bytes);
# Ok::<(), anyhow::Error>(())
```

The optional `wgpu` feature exposes these APIs. `WgpuScene3dRenderer` checks each
request against the device's individual-buffer limit before creating geometry
buffers or recording mesh uploads. `set_geometry_byte_limit(Some(bytes))` also
limits the total vertex/index payload. `None` disables the optional total limit;
zero accepts only requests with no active geometry. The device limit always applies.

`validate_geometry_memory(frame, channels)` performs the same CPU-only admission
on a prepared frame. The report's `validate(max_buffer_bytes, max_total_bytes)`
also permits offline checks against explicit limits. Neither entry point validates
all scene parameters, atlas residency, or output-format support.

`HeadlessRenderer` forwards `geometry_byte_limit()` and
`set_geometry_byte_limit(...)`. It resolves decoded images before geometry
admission; a geometry rejection does not undo those atlas changes. Previously
returned output textures remain valid. Changing the limit does not evict caches,
and `clear_caches()` preserves it. Successful outputs retain their report through
`RenderedFrame::gpu().geometry_memory()`.

For per-submission instance and uniform payloads, use
[draw statistics](rendering.md#draw-statistics).
See [Headless output](headless.md) for separate target and readback limits.
