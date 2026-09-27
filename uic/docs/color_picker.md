# Color Picker

`color_picker` provides composable controls for editing one shared color value.

## Imports

```rust
use uic::components::color_picker::{
    AlphaSlider, AlphaSliderAppearance, ColorPicker, ColorPickerAppearance,
    ColorPickerEvent, ColorPickerState, Hsva,
};
```

## Quick start

Create one `ColorPickerState` and pass the same entity to every control that
should edit that color.

```rust
use gpui::{
    AppContext, Context, Entity, IntoElement, Render, Subscription, Window, div, prelude::*,
    rgba,
};
use uic::components::color_picker::{
    AlphaSlider, ColorPicker, ColorPickerEvent, ColorPickerState,
};

struct ColorEditor {
    picker: Entity<ColorPickerState>,
    _subscription: Subscription,
}

impl ColorEditor {
    fn new(cx: &mut Context<Self>) -> Self {
        let picker = cx.new(|cx| ColorPickerState::new(rgba(0x2563ebff), cx));
        let subscription = cx.subscribe(
            &picker,
            |_, _, event: &ColorPickerEvent, _| match event {
                ColorPickerEvent::Preview(color) => {
                    println!("Preview: {color:?}");
                }
                ColorPickerEvent::Commit(color) => {
                    println!("Commit: {color:?}");
                }
            },
        );

        Self {
            picker,
            _subscription: subscription,
        }
    }
}

impl Render for ColorEditor {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let color = self.picker.read(cx).value();

        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(
                ColorPicker::new(&self.picker)
                    .sv_aria_label("Saturation and brightness")
                    .hue_aria_label("Hue"),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(format!("Opacity · {:.0}%", color.a * 100.0))
                    .child(div().flex_1().child(AlphaSlider::new(&self.picker)))
                    .child(div().size_10().rounded_md().bg(color)),
            )
    }
}
```

Keep the returned `Subscription` alive for as long as the editor needs to
receive events.

## Composing the controls

### Color surface and hue

`ColorPicker` renders the saturation/value surface and the hue track. Style its
outer surface with the normal GPUI `Styled` methods. Track dimensions and
marker geometry remain picker-specific appearance values.

```rust
div()
    .w_full()
    .child(
        ColorPicker::new(&picker)
            .p_3()
            .rounded_lg()
            .bg(rgb(0x111827)),
    )
```

Accessibility names are supplied by the caller when needed:

```rust
ColorPicker::new(&picker)
    .sv_aria_label("Saturation and brightness")
    .hue_aria_label("Hue")
```

### Opacity

`AlphaSlider` renders only the opacity track. Place any label, formatted value,
or preview beside it in the surrounding layout.

```rust
let color = picker.read(cx).value();

div()
    .flex()
    .items_center()
    .gap_3()
    .child(format!("{:.0}%", color.a * 100.0))
    .child(div().flex_1().child(AlphaSlider::new(&picker)))
    .child(div().size_10().rounded_md().bg(color))
```

### Palette swatches

A swatch sets the shared state directly. Preserve the current alpha when a
palette stores RGB colors only.

```rust
let mut swatch = rgba(0xef4444ff);
swatch.a = picker.read(cx).value().a;

picker.update(cx, |picker, cx| {
    picker.set_value(swatch, cx);
});
```

### Screen sampling

Compose a [`ScreenColorPicker`](screen_color_picker.md) with the same color state
to invoke the platform screen-color sampler. Handle its `Picked` event with
`ColorPickerState::set_value`; preserve the current alpha if desired.

### Text inputs

Parse the input into `Rgba`, then update the same state:

```rust
picker.update(cx, |picker, cx| {
    picker.set_value(parsed_color, cx);
});
```

When formatted inputs should follow pointer interaction, update them on
`ColorPickerEvent::Commit`. Use `Preview` only for UI that must change while the
pointer is moving.

## Reading and updating the value

Read the current color as `Rgba` or `Hsva`:

```rust
let rgba = picker.read(cx).value();
let hsva = picker.read(cx).hsva();
```

Replace the value programmatically with `set_value`:

```rust
picker.update(cx, |picker, cx| {
    picker.set_value(rgba(0xef4444cc), cx);
});
```

`set_value` redraws the controls but does not emit `Preview` or `Commit`. The
caller remains responsible for any side effects associated with a
programmatic update.

Use `Hsva` when a consumer needs explicit HSV conversion:

```rust
let hsva = Hsva::from(rgba);
let rgba = hsva.to_rgba();
```

All `Hsva` channels use normalized values from `0.0` to `1.0`.

## Events

Subscribe to `ColorPickerState` with `Context::subscribe`:

```rust
let subscription = cx.subscribe(
    &picker,
    |_, _, event: &ColorPickerEvent, cx| match event {
        ColorPickerEvent::Preview(color) => {
            update_preview(*color);
            cx.notify();
        }
        ColorPickerEvent::Commit(color) => {
            save_color(*color);
        }
    },
);
```

| Event | Sent when | Typical use |
| --- | --- | --- |
| `Preview(Rgba)` | A pointer presses or moves on a control | Live color preview |
| `Commit(Rgba)` | The pointer is released or a keyboard adjustment is applied | Persisting the value or synchronizing formatted inputs |

Both `ColorPicker` and `AlphaSlider` emit through the same state entity.

## Appearance

Start from the default appearance and chain the fields that the consumer needs
to customize.

```rust
use gpui::px;

let picker_appearance = ColorPickerAppearance::default()
    .area_height(px(240.0))
    .hue_width(px(28.0));

let alpha_appearance = AlphaSliderAppearance::default()
    .marker(rgb(0xffffff).into());

div()
    .child(
        ColorPicker::new(&picker)
            .appearance(picker_appearance)
            .p_3()
            .rounded_lg(),
    )
    .child(
        AlphaSlider::new(&picker)
            .appearance(alpha_appearance)
            .h(px(20.0))
            .rounded(px(10.0)),
    )
```

### `ColorPickerAppearance`

| Field | Controls |
| --- | --- |
| `accent` | Keyboard focus indicator |
| `marker` | SV and hue markers |
| `area_height` | SV surface and hue track height |
| `hue_width` | Hue track width |
| `marker_size` | SV marker diameter |

### `AlphaSliderAppearance`

| Field | Controls |
| --- | --- |
| `checker` | Checkerboard color under the alpha gradient |
| `marker` | Slider marker color |
| `focus_border` | Keyboard focus indicator |

`ColorPicker` and `AlphaSlider` both implement `Styled`. Their outer width,
height, margin, padding, gap, background, border, radius, opacity, and text
style use the same API as a GPUI `div`.

## Interaction

| Control | Pointer | Keyboard |
| --- | --- | --- |
| Saturation/value surface | Drag horizontally for saturation and vertically for value | Arrow keys adjust by 1%; hold Shift for 10% |
| Hue track | Drag vertically | Arrow keys adjust by 1 degree; hold Shift for 10 degrees |
| Alpha slider | Drag horizontally | Arrow keys adjust by 1%; hold Shift for 10% |

Pointer capture remains active after the pointer leaves a control, and values
are clamped to their valid range.

## Complete example

The complete example also composes a material palette, formatted color inputs,
copy actions, and a color preview:

```sh
cargo run -p uic --example color_picker
```

Source: `uic/examples/color_picker.rs`.
