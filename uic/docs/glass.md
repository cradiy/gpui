# Glass controls

`GlassSegmentedControl` presents a controlled single selection with a moving
glass indicator. Labels remain stationary and determine each option's width.

```rust
use uic::components::glass::{GlassSegmentedAppearance, GlassSegmentedControl};
use gpui::{prelude::*, px};

let control = GlassSegmentedControl::new("view", "overview")
    .label("Workspace view")
    .option("overview", "Overview")
    .option("activity", "Recent activity")
    .disabled_option("archive", "Archived")
    .appearance(GlassSegmentedAppearance::light())
    .text_size(px(16.))
    .rounded(px(24.));
```

Use `on_change` to update the selected value in application state. IDs must
remain stable across renders, and option values must be unique. A selected
value absent from the options displays no indicator. Disabled options may
remain selected but cannot be activated.

## Interaction

- Click an enabled option to select it.
- Tab focuses the group. Arrow keys cycle through enabled options; Home and End
  select the first and last enabled option.
- Selection changes retain the current indicator position and velocity.
- `animated(false)` disables motion interpolation.
- `disabled(true)` prevents pointer and keyboard activation.

## Appearance

`Styled` controls the outer layout, padding, corners, background, shadow, and
inherited typography. `GlassSegmentedAppearance` configures the base and
selected optical surfaces, selected text, focus ring, and disabled opacity.
Pair `dark()` with a light foreground color using `text_color`.

Glass samples previously painted content in the same window. Foreground labels
are painted afterward and remain sharp. The indicator's material follows its
animated bounds; content and hit regions do not deform. Stationary controls do
not request animation frames.

`reduced_transparency(true)` uses opaque surfaces. This fallback also applies
when backdrop blur is unavailable. Map application accessibility preferences
to `animated` and `reduced_transparency` as needed.

```sh
cargo run -p uic --example glass_segmented
```
