# Global Shortcuts

GPUI exposes one global-shortcut API across macOS, Windows, Wayland, and X11. A global shortcut is active even when none of the application's windows has keyboard focus.

## Register shortcuts

Create a batch of `GlobalShortcut` values and register them through `App`:

```rust
use gpui::{App, GlobalShortcut, Keystroke};

let registration = cx.register_global_shortcuts([
    GlobalShortcut::new(
        "show-window",
        "Show the main window",
        Keystroke::parse("ctrl-shift-space")?,
    ),
]);

cx.spawn(async move |cx| {
    let registration = registration.await?;
    // Store `registration` in application or entity state.
    anyhow::Ok(())
}).detach();
```

Registration is asynchronous. Wayland may show a system dialog and may replace the preferred trigger. Use `GlobalShortcutRegistration::shortcuts` to display the effective trigger returned by the operating system.

On Wayland, set a valid reverse-DNS application ID on an application window before registering. GPUI uses it to identify an unsandboxed host application to the portal:

```rust
let options = gpui::WindowOptions {
    app_id: Some("com.example.MyApplication".to_string()),
    ..Default::default()
};
```

For a host application, the same ID must have an installed desktop entry, for example `com.example.MyApplication.desktop`. Sandboxed packages already provide this identity through their package metadata. A bare `cargo run` process has no portal identity unless a matching desktop entry has been installed for the example binary.

The global-shortcuts example includes a matching desktop entry. Install it before
running the example in a Wayland session:

```sh
install -Dm644 \
  crates/gpui/examples/dev.gpui.GlobalShortcutsExample.desktop \
  "$HOME/.local/share/applications/dev.gpui.GlobalShortcutsExample.desktop"
cargo run -p gpui --example global_shortcuts
```

The shortcut ID must be stable and unique within the batch. It is used to route activation events. The description is user-facing and should describe the action rather than repeat the key combination.

## Keep the registration alive

`GlobalShortcutRegistration` owns the native registrations. Store it for as long as the shortcuts should remain active:

```rust
struct ApplicationState {
    global_shortcuts: Option<gpui::GlobalShortcutRegistration>,
}
```

Dropping the value unregisters the entire batch. Call `registration.unregister()` when explicit early removal is more convenient.

## Handle activation

Observe events once at application scope, then route shortcut IDs to application behavior:

```rust
cx.observe_global_shortcuts(|event, cx| {
    if let gpui::GlobalShortcutEvent::Activated {
        shortcut_id,
        activation_token: _,
        ..
    } = event
    {
        match shortcut_id.as_ref() {
            "show-window" => show_main_window(cx),
            "new-note" => create_note(cx),
            _ => {}
        }
    }
})
.detach();
```

Keep the returned `Subscription` alive when the observer belongs to a component. Detaching is appropriate for an observer that should live for the entire application session.

`ShortcutsChanged` reports a complete replacement list when the operating system changes a registration, as Wayland compositors can do from their shortcut settings UI.

## Platform behavior

- macOS uses the system Carbon hot-key service.
- Windows uses `RegisterHotKey` and suppresses key-repeat activation.
- Wayland uses `org.freedesktop.portal.GlobalShortcuts`. The desktop controls approval and the effective trigger.
- X11 uses passive root-window key grabs. Caps Lock and Num Lock do not prevent activation.
- Web and headless platforms report the capability as unavailable.

Use `cx.global_shortcuts_supported()` to detect platforms where the mechanism is definitely unavailable. A `true` result does not guarantee that the active Wayland compositor has a working GlobalShortcuts portal backend. Registration can still fail because the desktop backend is unavailable, a trigger is reserved, conflicts with another application, is unsupported by the current keyboard layout, or is declined by the user. Always handle the asynchronous error.

A runnable implementation is available in `examples/global_shortcuts.rs`.
