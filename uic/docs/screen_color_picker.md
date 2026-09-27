# Screen color picker

`ScreenColorPicker` is a styled pipette icon button that opens the platform's
screen-color sampler. A retained `ScreenColorPickerState` manages pending state
and emits the result. No screen-capture feature or global keyboard binding is
required.

On supported Wayland compositors, sampling freezes the desktop and displays a
pointer-following 10× pixel magnifier with a center-pixel marker, color swatch,
and hexadecimal readout. Left-click selects; Escape or right-click cancels.
The magnifier stays inside the active output. Sampling uses physical pixels and
accounts for output scale, rotation, and capture row orientation.

## Composition

```rust
use uic::components::screen_color_picker::{
    ScreenColorPicker, ScreenColorPickerEvent, ScreenColorPickerState,
};

let screen = cx.new(ScreenColorPickerState::new);
let subscription = cx.subscribe(&screen, |this, _, event, cx| {
    if let ScreenColorPickerEvent::Picked(mut color) = event.clone() {
        color.a = this.color.read(cx).value().a;
        this.color.update(cx, |picker, cx| picker.set_value(color, cx));
    }
});
```

Retain the state and subscription in the host. Samples are opaque sRGB colors;
preserving an existing alpha value is a consumer choice. Programmatic
`ColorPickerState::set_value` updates its controls without emitting its editing
events.

```rust
ScreenColorPicker::new(&screen)
    .label("Pick screen color")
    .show_label(true)
    .busy_label("Picking…")
    .px_3()
    .py_2()
    .rounded_lg()
    .bg(rgb(0x34373c))
    .text_size(px(14.))
```

The default is a compact icon-only button. Use `show_label(true)` to display
text beside the icon. Register `uic::assets::LucideAssets` through
`Application::with_assets`, or serve its paths from the application's asset
source. `text_color` also colors the default icon explicitly; `icon_color`
overrides only the icon color.

`Styled` applies to the button's actual surface. `child` replaces idle content
with an icon, text, or another composed element; `label` still supplies its
accessible name. `disabled(true)` prevents pointer activation. A pending request
also disables the button and displays a loading icon; `busy_label` is shown when
text labels are enabled.

The button observes its state for redraws. A host displaying errors or other
status can observe the same state and use `is_busy()` and `error()`.

## Actions and results

Call `state.pick(cx)` from a custom button or application-owned keyboard action
to use the same state without the built-in button. Repeated calls return `false`
while the sampler is open. Keyboard interaction belongs to the active sampling
session; no global key binding is installed by the component.

| Event | Meaning |
| --- | --- |
| `Picked(Rgba)` | The system returned a color |
| `Cancelled` | The user cancelled; keep the previous color |
| `Failed(SharedString)` | The system sampler failed or is unavailable |

Every completed request clears the busy state. Failures remain available in
`error()` until the next request. Dropping the state suppresses delivery of a
late result. Dropping the low-level response receiver closes the Wayland
magnifier; system-owned interfaces may remain open until cancelled by the user.

For low-level consumers, `App::pick_screen_color()` returns an asynchronous
receiver of `Result<Option<Rgba>>`: `Some` is selection, `None` is cancellation,
and `Err` is failure.

## Platform backends

- Wayland uses an integrated desktop magnifier when the compositor supports
  `wlr-screencopy`, `wlr-layer-shell`, and `wp-viewporter`, including Niri.
  It takes one snapshot per output before opening overlays. Only the small loupe
  is repainted during pointer movement; no video stream or extra GPU renderer is
  created. Screenshots stay in temporary, unlinked buffers and are released when
  sampling ends. Output removal or reconfiguration cancels the session.
- Other Wayland compositors and X11 use the XDG Screenshot portal's
  [`PickColor`](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.Screenshot.html#org-freedesktop-portal-screenshot-pickcolor)
  interface. Its interface and magnification are controlled by the desktop
  backend. Capture failures on supported Wayland compositors are reported as
  errors rather than starting a second picker.
- macOS uses `NSColorSampler` and converts its result to sRGB.
- Other platform implementations currently return an unsupported error.

The built-in Wayland sampler supports 8-bit RGB/BGR and 10-bit RGB/BGR SHM
formats. Other capture formats produce an explicit error. Multiple simultaneous
Wayland sampling sessions in one process are rejected.

## Example

```sh
cargo run -p uic --example screen_color_picker
```

The example composes the button with a `ColorPicker` and `AlphaSlider`, preserving
alpha when a screen color is selected.
