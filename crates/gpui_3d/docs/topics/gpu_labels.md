# GPU labels

[Headless output](headless.md) · [CPU label images](labels.md)

With the `wgpu` feature, `RenderedFrame::label_texture` maps an Object ID texture
to caller-defined `u32` labels without CPU pixel readback. Create a reusable
`WgpuIdRemapper` on the renderer's context:

```rust
use std::collections::HashMap;
use gpui_3d::{
    HeadlessRenderer, IdRemapConfig, NodeHandle, RenderedFrame,
    RenderedLabels, WgpuIdRemapper,
};

fn label_frame(
    frame: &RenderedFrame,
    mapper: &WgpuIdRemapper,
    groups: &HashMap<NodeHandle, u32>,
) -> anyhow::Result<RenderedLabels> {
    frame.label_texture(mapper, |object| {
        object.node.and_then(|node| groups.get(&node).copied()).unwrap_or(0)
    })
}

fn create_mapper(renderer: &HeadlessRenderer) -> anyhow::Result<WgpuIdRemapper> {
    WgpuIdRemapper::new(renderer.context().clone(), IdRemapConfig::default())
}
```

The callback runs once per frame object, including objects with no visible
pixels. Repeated labels merge objects; zero excludes them. Background stays
zero. Every `u32` label is preserved exactly, including values above the exact
integer range of `f32`. Excluding a foreground object does not reveal surfaces
behind it. Coverage comes from the original ID channel, not color blending.

`RenderedLabels::texture()` exposes a same-size, single-sampled, single-mip
`R32Uint` texture with `TEXTURE_BINDING`, `RENDER_ATTACHMENT`, and `COPY_SRC`
usages. Read it in shaders through `texture_2d<u32>` and `textureLoad`, without
filtering or color conversion. Each call owns a fresh output, independent of
later remaps and the source frame's lifetime. The output retains no source
textures or geometry. Consumer writes to the exposed texture are caller-owned.

`size()` reports physical dimensions and `camera()` retains the original camera.
`label_for_object(output_id)` returns the assigned label, or `None` for zero and
unknown source IDs. `objects(label)` lists original source identities, including
zero-coverage objects; label zero lists excluded objects, not background.

## Submission and limits

`label_texture` submits its work but does not wait for GPU completion. Subsequent
consumers must use the same device and respect queue ordering. Use
`encode_label_texture(mapper, encoder, assign)` to append the pass to a
caller-owned command encoder and encode consumers after it. The caller finishes
and submits that encoder and handles validation; discard it if recording fails.
An encoded texture is not ready for GPU consumption until its work is submitted.

Only the label table is uploaded: four bytes per frame object, with a four-byte
zero table for an empty frame. Output payload is four bytes per pixel. Defaults
admit up to 64 MiB of output and 16 MiB of table data per call.
`IdRemapConfig` can change either budget or disable it with `None`; enabled
device limits still apply. These budgets exclude retained outputs, source
frames, driver overhead, source-identity storage, and callback allocations.

Missing ID channels, unsupported input metadata, budget failures, observed
device loss, and captured WGPU validation errors return errors. Admission checks
precede callbacks; callback side effects are not rolled back by later failures.
Metadata admission does not establish device ownership: inputs and encoders
must belong to the mapper's device.

For integer textures outside a `RenderedFrame`, use `WgpuIdRemapper::render` or
`encode` directly. Inputs must be nonempty, single-layer 2D, single-sampled
`R32Uint` textures with `TEXTURE_BINDING`. Only mip zero is read. Table entry zero
maps source ID one; source zero and out-of-table IDs produce zero. An empty
table produces an all-zero image. This low-level interface retains no object
identities.
