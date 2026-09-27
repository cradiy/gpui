# Glass controls

`GlassSegmentedControl` presents a controlled single selection with a moving
glass indicator. Option layout determines each option's width and remains stable
while its painted contents refract through the lens.

```rust
use uic::components::glass::{GlassSegmentedAppearance, GlassSegmentedControl};
use gpui::{prelude::*, px};

let control = GlassSegmentedControl::new("view", "overview")
    .label("Workspace view")
    .option("overview", "Overview")
    .option("activity", "Recent activity")
    .disabled_option("archive", "Archived")
    .appearance(GlassSegmentedAppearance::light())
    .selection_press_scale(1.35)
    .surface_press_scale(1.05)
    .text_size(px(16.))
    .rounded(px(24.));
```

Use `on_change` to update the selected value in application state. IDs must
remain stable across renders, and option values must be unique. A selected
value absent from the options displays no indicator. Disabled options may
remain selected but cannot be activated.

## Layout

The default is a horizontal row. Use `.flex_col()` for a vertical control, with
`.items_stretch()` to make options share the track width; `.flex_row()` restores
a horizontal row. Dragging and snapping follow the layout axis. Changing the
axis during a drag cancels that drag. Both press-scale settings apply to either
layout, uniformly scaling width and height.

## Interaction

- Press an enabled option to select it immediately; releasing without dragging
  does not fire a second selection change. Options accept arbitrary elements, including
  vertical stacks of icons and labels.
- Drag the lens along the layout direction to preview a position. Release to select the nearest
  enabled option; the lens snaps into place. Dragging beyond the group clamps to
  its first or last enabled option. Release commits a change only if the drag
  target differs from the current selection.
- Window deactivation or disabling the control cancels a drag and
  returns the lens to the current selection, including any selection made on press.
- The component registers no keyboard handlers, creates no focus handle, and adds
  no Tab stop. Pointer selection preserves existing focus. Applications own
  keyboard policy and update the value passed to `new(id, selected)`; `on_change`
  reports pointer selections.
- Selection changes retain the current indicator position and velocity.
- Holding a segment scales the selected lens to 135% and the outer glass to 105%
  by default. Set them independently with `selection_press_scale` and
  `surface_press_scale` (`1.0..=2.0`); `1.0` disables scaling for that layer.
  Each surface scales uniformly about its own center, including corner radii.
  Both share the press spring, which can slightly overshoot the held scale.
  Ancestor clipping still applies. Labels and hit regions retain their layout size.
- `animated(false)` disables motion interpolation.
- `disabled(true)` prevents pointer activation.

## Appearance

`Styled` controls the outer layout, padding, corners, background, shadow, and
inherited typography. `GlassSegmentedAppearance` configures the base and
selected optical surfaces, selected text, and disabled opacity.
Pair `dark()` with a light foreground color using `text_color`.

The track is painted first, followed by the options and finally the selection
lens. The lens samples both the background and the painted options, so icons,
text, emoji, and images participate in edge refraction, dispersion, and material
shading. The center keeps its original geometry. The curved band is capped at
10% of the lens's shorter dimension and scales with the pressed lens. Refraction
is limited relative to this band, including dispersion, to keep interior strokes
from folding into repeated edge images. Content bends as the band passes over
it during dragging; content outside the lens retains its original geometry.

Before refraction, text and monochrome SVGs use `selected_text` within the lens
region. Their source colors remain unchanged outside it. Color and glyph shape
then pass through the same optical sampling as the background. The covered
color's alpha multiplies the original content alpha. Emoji and colored images
participate in refraction without monochrome recoloring.

The material and content coloring share the indicator's animated bounds, corners,
and press scale. Only painted pixels deform; content layout and hit regions stay fixed. Stationary
controls do not request animation frames.

`reduced_transparency(true)` uses opaque surfaces beneath unrefracted options. This fallback also applies
when backdrop blur is unavailable. Map application accessibility preferences
to `animated` and `reduced_transparency` as needed.

```sh
cargo run -p uic --example glass_segmented
```

The example shows independent horizontal and vertical controls over a gray
background with solid shapes. The shapes can be hidden for comparison.
Theme, motion, transparency, and disabled controls apply
to both controls.
