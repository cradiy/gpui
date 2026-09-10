# Frame depth comparisons

[Headless output](headless.md) · [Spatial queries](queries.md)

With the `wgpu` feature, `ReadFrame::compare_depth(world, tolerance)` compares a
world point with the linear-depth sample at its projected pixel. It uses the
frame's retained camera, lens shift, projection aspect, and physical dimensions.
It does not render, submit GPU commands, allocate, or start another readback.
Request `Scene3dChannels::LINEAR_DEPTH` when rendering the frame.

```rust
use gpui_3d::{DepthComparison, DepthQueryError, ReadFrame};

fn compare_anchor(
    frame: &ReadFrame,
    position: [f32; 3],
) -> Result<Option<DepthComparison>, DepthQueryError> {
    frame.compare_depth(position, 0.01)
}
```

The point is projected into a top-left-origin, half-open output rectangle.
Its containing pixel is selected with floor coordinates; depth is not bilinearly
interpolated across surfaces or background. `Ok(None)` means the point is behind
the eye, clipped by near/far, or outside that rectangle. Near depth is inclusive;
far depth and the right/bottom edges are exclusive.

An in-bounds result contains the pixel, the point's positive camera-forward depth,
the sampled surface depth, and a `DepthRelation`:

| Relation | Meaning |
| --- | --- |
| `Background` | The sample is zero; `surface_depth` is `None`. |
| `InFront` | The point is nearer than the sampled surface by more than the tolerance. |
| `WithinTolerance` | The absolute forward-depth difference is at most the tolerance. |
| `Behind` | The point is farther than the sampled surface by more than the tolerance. |

Tolerance is a finite nonnegative distance in scene units, not pixels, normalized
hardware depth, or ray distance. Choose it for the scene scale and depth precision;
zero requests a strict comparison. Differences are computed in f64 from the f32
point and depth values. This does not recover precision lost during rendering.

These results describe one depth sample, not continuous visibility. A world point
can project away from the pixel center where the surface was sampled, especially
at silhouettes, steep surfaces, and subpixel features. `WithinTolerance` does not
prove object identity, and `Background` does not prove the point is drawable.
Use `object_at(result.pixel[0], result.pixel[1])` separately when an ID channel
was requested. Low-opacity Blend surfaces can own the nearest depth; color MSAA,
alpha-weighted contributions, and post-processing do not change the comparison.

The complete depth-channel length and nonzero dimensions are required even for
clipped points. Missing channels, mismatched lengths, invalid tolerance, invalid
camera/point values, and a selected negative or nonfinite depth return
`DepthQueryError`. Other pixels are not scanned or validated. Queries take O(1)
time and leave the frame unchanged. Compare points from the same scene state as
the retained frame; later graph edits do not update its depth samples.
