# Frame labels

[Headless output](headless.md) · [GPU labels](gpu_labels.md)

With the `wgpu` feature, `ReadFrame::label_image(pixel_limit, assign)` remaps an
available Object ID image to caller-defined `u32` labels. Multiple source objects
can share a label, allowing a model's primitive nodes to form one instance or
segmentation group. Mapping returns an owned `FrameLabels` image without changing
the frame, rendering again, or submitting GPU work.

```rust
use std::collections::HashMap;
use gpui_3d::{FrameLabels, LabelError, NodeHandle, ReadFrame};

fn segmentation(
    frame: &ReadFrame,
    groups: &HashMap<NodeHandle, u32>,
) -> Result<FrameLabels, LabelError> {
    frame.label_image(16_777_216, |object| {
        object.node.and_then(|node| groups.get(&node).copied()).unwrap_or(0)
    })
}
```

The callback receives each `RenderObject` once in frame order, including mapped
objects with zero visible pixels. It can use node handles, application IDs, or
an external instance mapping. Labels are not restricted to dense indices: every
nonzero value through `u32::MAX` is preserved exactly. Background stays zero;
returning zero also excludes an object from the label image. Classification,
label persistence, and instance ownership remain caller-defined.

`size()` reports physical width and height. `pixels()` contains one row-major
`u32` per pixel, with top-left origin and no row padding. `label_at(x, y)` returns
`Some(0)` for background/excluded samples and `None` outside the image.
`into_pixels()` moves the buffer without copying; retain `size()` separately.
Do not pass labels through f32 or display-color conversions when serializing
exact integer data.

`camera()` and source identities belong to the original frame and remain usable
after it is dropped. No GPU resources, scene geometry, or original pixel buffers
are retained. `label_for_object(output_id)` resolves a source frame ID to its
assigned label; zero and unknown source IDs return `None`. `objects(label)` lists
all mapped objects with that label, including zero-coverage objects. For label
zero it lists excluded objects, not the background.

Combine the label mapping with coverage from the same frame for group counts:

```rust
use gpui_3d::{FrameCoverage, FrameLabels};

fn group_pixels(coverage: &FrameCoverage, labels: &FrameLabels, label: u32) -> u64 {
    coverage.objects()
        .filter(|entry| labels.label_for_object(entry.object.output_id) == Some(label))
        .map(|entry| entry.pixels)
        .sum()
}
```

For label zero this counts excluded-object samples only; add
`coverage.background_pixels()` to include original background samples.
Use the same source frame for both results. Numeric output IDs from a different
frame do not establish correspondence.

Label coverage follows the existing ID channel's nearest surviving surface,
including Mask cutouts and Blend surfaces. Excluding a foreground object turns
its pixels into zero; it does not reveal hidden surfaces behind it. Labels are
not alpha-weighted color contributions or MSAA-averaged values. This is a CPU
conversion after readback, not a separate GPU output channel or texture upload.

The pixel limit is checked before allocation and callbacks. Output pixel payload
is four bytes per pixel, plus four bytes per mapped object for the label table
and shared source-identity storage. The limit does not cover existing frames,
other retained images, allocator overhead, or callback allocations. Missing ID
channels, zero dimensions, incorrect pixel counts, unknown source IDs, admission
failure, and allocation failure return `LabelError` without a partial image.
Callback side effects are not rolled back. Conversion takes O(pixels + objects)
time; reverse object lookup takes O(objects) time.
