# SDF shapes

`SdfScene` draws circles, rounded rectangles and capsules as one distance field.
Boolean operations combine their silhouettes, while smooth operations blend
boundaries and colors. A scene supports up to six independently transformed shapes.

## Compose shapes

```rust,ignore
use gpui::{point, px, rgb, size};
use gpui_effects::{SdfScene, SdfShape};

let circle = SdfShape::circle(point(px(140.), px(120.)), px(64.), rgb(0x61dce6));
let rectangle = SdfShape::rounded_rect(
    point(px(240.), px(120.)),
    size(px(120.), px(100.)),
    px(24.),
    rgb(0xa38aef),
);
let cutter = SdfShape::capsule(
    point(px(190.), px(150.)), px(60.), px(20.), rgb(0xffffff),
);
let scene = SdfScene::new(circle.smooth_union(rectangle).subtract(cutter))?;
```

Coordinates and dimensions use surface-local logical pixels. Capsule length is
the distance between its semicircle centers; its total width also includes both
end radii. Rounded rectangle dimensions describe the full width and height.

| Operation | Result |
| --- | --- |
| `union` | Both shapes |
| `intersect` | Only the overlapping region |
| `subtract` | The left shape with the right shape cut out |
| `smooth_union` | Both shapes with a blended join and blended color |
| `smooth_intersect` | Overlap with a rounded boundary |
| `smooth_subtract` | Cutout with a rounded boundary |

Methods compose in their written order. Build a subtree first to group
operations: `base.subtract(first.union(second))` cuts both shapes out of `base`.
Subtraction preserves the left field's color; a cutter's color does not tint the
cut edge. Color blending respects alpha.

## Render and animate

Keep the scene in the owning view and render it with a styled canvas:

```rust,ignore
use gpui::prelude::*;
use gpui_effects::sdf;

let surface = sdf(&self.scene).size_full();
```

Geometry, colors and composition are fixed when the scene is constructed.
Position, uniform scale, rotation and edge settings can change without rebuilding
the shader:

```rust,ignore
use gpui_effects::SdfTransform;

self.scene.set_transform(0, SdfTransform {
    center: pointer_local,
    scale: 1.2,
    rotation: 0.4,
});
cx.notify();
```

Leaf indices follow left-to-right construction order, including cutters. In the
first example, the circle is `0`, the rectangle is `1`, and the capsule is `2`.
Use `transform(index)` to read a leaf's current transform. Rotation is clockwise
in radians; scale is uniform and limited to `0.001..=1000`.

The canvas owns layout and rectangular hit testing; individual shape interactions
belong to the application. No continuous animation frames are requested by the
field. Notify the view when input or an application animation changes its state.

## Edges and light

```rust,ignore
use gpui_effects::SdfOptions;

self.scene.set_options(SdfOptions {
    smoothing: px(40.),
    outer_glow: px(20.),
    outer_glow_opacity: 0.25,
    inner_glow: px(12.),
    inner_glow_intensity: 0.3,
    ..Default::default()
});
```

The default is an opaque fill, a 32-pixel smooth-operation radius, and no stroke
or glow. Set `smoothing` to zero for hard Boolean operations. `stroke_width` adds
a centered stroke. With `fill_opacity: 0.`, only the stroke and outer glow remain.
Stroke and lighting follow the local shape color.

All edge widths are logical pixels and follow device scale. Shape scaling changes
the geometry, not stroke or glow widths. The final silhouette uses derivative-based
antialiasing. Leave space around shapes for outer glow; drawing stays inside the
canvas and its parent clip. Parent opacity applies to the completed field.

For custom painting, use `scene.paint(bounds, window)`. `shader()` and
`uniforms(scale_factor)` expose the corresponding low-level effect inputs.
The canvas can also feed a subtree effect chain, including Bloom.

## Example

```sh
cargo run -p gpui_effects --example sdf
```

Drag the three shapes. Fusion joins them, Cutout uses the capsule as a cutter,
and Overlap keeps its intersection with the other shapes. A faint capsule guide
remains visible in Cutout and Overlap modes. Controls switch soft/hard blending,
outline and lighting, rotate the capsule, or restore its placement.
