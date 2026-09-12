# Local lens

`subtree_lens` magnifies or compresses a local region of an element subtree.
Text, artwork and decorations share the same displacement. The surrounding
pixels remain unchanged, with no added border, tint or shadow.

## Usage

```rust,ignore
use gpui::{point, px};
use gpui_effects::{LensOptions, subtree_lens};

let surface = subtree_lens(content, LensOptions {
    center: point(0.5, 0.5),
    radius: px(220.),
    magnification: 1.8,
    ..Default::default()
});
```

`center` is normalized to the capture rectangle: `(0, 0)` is the top-left and
`(1, 1)` the bottom-right. Subtract the capture origin from a pointer position
and divide by the capture size to obtain these coordinates. Include any padding
contributed by the effect chain when calculating that rectangle.

## Configuration

- `radius`: support radius in logical pixels, default `220 px`. Pixels outside
  this radius are unchanged.
- `magnification`: center scale, default `1.8`. Values above one magnify;
  values below one compress. The supported range is `0.5..=3.0`.
- `softness`: falloff shape in `0..=1`, default `0.5`. Higher values concentrate
  the full-strength region near the center and leave a gentler outer transition.
- `edge_fade`: fade distance at capture edges, default `32 px`. It is clamped
  to at least one logical pixel.

Spatial values follow the window scale. Displacement fades near the capture
edges, and sampling is clamped to the input rectangle. The stage adds no padding.
Zero radius, unit magnification or a non-finite center disables the stage.
Non-finite magnification is treated as one; non-finite softness uses `0.5`.

## Pointer tracking and animation

The lens is stateless. Update `center` from pointer events and notify the view.
For smooth following, interpolate the center using elapsed time. On pointer
exit, animate `magnification` back to one to restore the original content.

Request animation frames only while interpolated values are changing; a settled
lens does not need continuous redraws. `.time()` on an effect chain does not drive
the lens. Applications own the motion, pause behavior and input handling.

The example interpolates the center and logarithmic magnification, providing
smooth entry, movement and return to the original image without overshoot.

## Composition

```rust,ignore
use gpui_effects::{BloomOptions, EffectStage, subtree_effect_chain};

let surface = subtree_effect_chain(content, [
    EffectStage::lens(options),
    EffectStage::bloom(BloomOptions::default()),
]);
```

Stages run in order, and lens coordinates refer to their shared capture bounds.
Layout and accessibility retain their original coordinates. Enable
`.map_interaction(true)` to align pointer targets and event positions with the
lens. See [Interaction mapping](interaction_mapping.md) for supported chains.

Linux WGPU and macOS Metal support subtree effects. Unsupported backends draw the input
normally; check `window.supports_subtree_effects()` for availability.

## Example

```sh
cargo run -p gpui_effects --example lens
```

Move the pointer over the text or artwork. Controls switch between magnification
and compression and select the affected radius. Leaving the surface restores it.
