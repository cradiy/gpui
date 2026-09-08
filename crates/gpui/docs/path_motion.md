# Path motion

`gpui::MeasuredPath` retains vector geometry and cached arc-length measurements.
It supports partial strokes, dash phase and distance-based sampling on lines,
quadratic curves, cubic curves and arcs created through `PathBuilder`.

## Measure a path

```rust,ignore
use gpui::{PathBuilder, point, px};

let mut builder = PathBuilder::stroke(px(3.));
builder.move_to(point(px(20.), px(100.)));
builder.cubic_bezier_to(
    point(px(420.), px(100.)),
    point(px(140.), px(10.)),
    point(px(280.), px(190.)),
);
let path = builder.measure();
```

Keep the measured path in the owning view and rebuild it when its geometry
changes. Clones share geometry and measurements. Builder transforms are applied
before measurement; fill/stroke style and dash settings are not retained.

Coordinates and measured distances use logical pixels. `length()` includes
closing edges but excludes gaps between subpaths.

## Reveal a stroke

```rust,ignore
use gpui::{LineCap, LineJoin, StrokeOptions};

let style = StrokeOptions::default()
    .with_line_width(3.)
    .with_line_cap(LineCap::Round)
    .with_line_join(LineJoin::Round);

let stroke = path.stroke_range(0.0..progress, &style)?;
window.paint_path(stroke, gpui::rgb(0x82e8f5));
```

Call paint methods inside a canvas paint callback. `stroke_range` uses normalized
arc length, not a Bézier parameter. Endpoints are clamped to `0..=1`; reversed or
empty ranges draw nothing, and non-finite endpoints return an error. Ranges do
not wrap across the path's end.

`stroke(&style)` draws the complete path. A full `0.0..1.0` range preserves closed
contour joins. Partial ranges use the selected caps. Trimming across subpaths
preserves gaps rather than connecting them.

## Move along a path

```rust,ignore
let length = f32::from(path.length());
if length > 0. {
    let distance = gpui::px((elapsed_seconds * 120.).rem_euclid(length));
    if let Some(sample) = path.sample_at(distance) {
        let position = sample.position;
        let angle = sample.tangent.y.atan2(sample.tangent.x);
        // Position and rotate the application element using this sample.
    }
}
```

Guard against zero length before computing a wrapped distance. `sample(progress)`
accepts a normalized distance; `sample_at(distance)` accepts logical pixels.
Both clamp to the endpoints. Empty and zero-length paths or non-finite input
return `None`.

The returned tangent follows increasing path distance. Negate it for reverse
travel. At corners and subpath boundaries the direction can change abruptly;
subpath gaps contribute no travel distance. A tangent that cannot be determined
is zero. Sampling uses cached measurements with a 0.01-pixel curve tolerance.

## Flowing dashes

```rust,ignore
let stroke = path.stroke_dashed(
    0.0..1.0,
    &[gpui::px(14.), gpui::px(11.)],
    gpui::px(-elapsed_seconds * 35.),
    &style,
)?;
window.paint_path(stroke, gpui::rgb(0xf1ba91));
```

Positive offsets advance into the pattern; a decreasing offset moves dashes
forward along the path. Phase stays anchored to the complete path when its
visible range changes and continues across subpaths. Odd-length patterns repeat
twice. An empty pattern draws a solid stroke.

Dash lengths must be finite and positive, and offset must be finite. Patterns
that require more than 16,384 dash/gap intervals in the visible range, or cannot
advance at the available precision, return an error.

For a directly built stroke, `PathBuilder::dash_array` and `dash_offset` provide
the same pattern semantics:

```rust,ignore
let mut builder = PathBuilder::stroke(gpui::px(3.))
    .dash_array(&[gpui::px(14.), gpui::px(11.)])
    .dash_offset(gpui::px(-phase));
```

## Playback and rendering

The path does not own a clock or request animation frames. Applications control
elapsed time, speed, direction and pause. Reuse measurements while animating;
each stroke call tessellates its current visible geometry. Cache the finished
`Path` as well when the stroke is static.

Normal GPUI path rendering applies device scale, parent clipping and opacity.

## Example

```sh
cargo run -p gpui --example path_motion
```

Reveal draws a growing stroke, Travel moves a tangent-aligned marker by distance,
and Dashes scrolls a repeating pattern. Controls provide pause/resume, speed,
reverse and replay.
