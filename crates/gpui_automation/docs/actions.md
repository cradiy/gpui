# Actions and waiting

`Session` operates inside the application on GPUI's foreground context. Selectors
are resolved against the window's latest completed semantic snapshot. No desktop
input permissions, socket or external driver are involved.

## Select and operate

```rust,ignore
let save = Selector::role(Role::Button)
    .named("Save changes")
    .within(Selector::id("preferences"));
let generation = cx.update(|cx| Session::click(handle, &save, cx))?;
Session::wait_for_draw(handle, generation, Duration::from_secs(2), cx).await?;
```

`Selector` owns its query and can be reused across draws. Every parent scope must
resolve uniquely. Actions reject ambiguous matches, hidden or disabled nodes,
pending redraws, and unsupported operations. The lower-level
`Window::perform_automation_action` additionally requires the current snapshot
generation and node identity. Do not mutate a view and act on its old snapshot
inside the same application update; wait for the resulting draw first.

| Method | Behavior |
| --- | --- |
| `click` | Invoke semantic activation. Ordinary `on_click` handlers receive a keyboard-style click; an explicit accessible Click handler takes precedence. |
| `focus` | Invoke an accessible Focus handler or focus the node's registered focus target. |
| `set_value` | Pass text to a registered accessible SetValue handler; reject read-only nodes. |

These are semantic operations, not physical input simulation. They do not test
pointer occlusion, trigger hover/press animations, bubble pointer events, type
keys, operate the clipboard, or compose IME text. A successful call means a
handler was dispatched, not that a requested application state was reached.
Handlers remain responsible for validation and application state changes.

## Writable controls

Use normal `on_click` handlers for buttons. A writable control must expose its
current `aria_value` and implement SetValue using its normal value-update logic:

```rust,ignore
let entity = cx.entity().downgrade();
div()
    .automation_id("query")
    .role(Role::TextInput)
    .aria_value(self.value.clone())
    .on_a11y_action(AccessibleAction::SetValue, move |data, _, cx| {
        if let Some(gpui::accesskit::ActionData::Value(value)) = data {
            entity.update(cx, |this, cx| {
                this.value = value.to_string();
                cx.notify();
            }).ok();
        }
    })
```

Role and label annotations alone do not make a control writable. `set_value`
replaces the semantic value; it does not emulate selection or text entry.

## Verify state

```rust,ignore
let field = Selector::id("query");
let generation = cx.update(|cx| Session::set_value(handle, &field, "Hello", cx))?;
let snapshot = Session::wait_for(
    handle,
    Duration::from_secs(2),
    |snapshot| Ok(
        snapshot.data().generation > generation
            && snapshot.find(&field).one()?.value.as_deref() == Some("Hello")
    ),
    cx,
).await?;
```

Run asynchronous code in `cx.spawn(async move |cx| { ... })` from an App, or
`cx.spawn(async move |this, cx| { ... })` from an entity context. Awaiting does not
block the foreground event loop. Keep or detach the returned task.

`wait_for` polls completed snapshots every 16 milliseconds using GPUI's scheduler
clock. It does not force draws or repeat actions. Predicate errors, closed
windows and disabled collection fail immediately. A missing initial snapshot
waits until one is available or the timeout expires. Dropping the future cancels
the wait. To wait for an element that is not present yet, return `Ok(false)` on
`LookupError::NotFound`; propagate ambiguity and other errors.

`wait_for_draw` only requires a newer snapshot. It does not mean animations have
settled, asynchronous work has finished, or a GPU frame has been presented. Use
a predicate over the actual result for those application-specific conditions.

## Run verification

From the workspace root:

```sh
cargo test -p gpui_automation --all-targets
ITERATIONS=20 cargo test -p gpui_automation --test actions
cargo clippy -p gpui_automation --all-targets -- -D warnings
```

Run the interactive example on Linux with Wayland:

```sh
cargo run -p gpui_automation --example interaction --features gpui_platform/wayland
```

For X11, use `--features gpui_platform/x11`. Click **Run automation**. The example
sets the semantic value to `Ready to automate`, focuses the field, activates
Increment, and checks Count is 1. The status reports Passed or the concrete
failure. Run it again to reset and repeat. The value panel demonstrates a
semantic SetValue handler, not a keyboard text editor.

The automated tests use GPUI's test platform; the example exercises the real
window event loop. Neither is a pixel-level or physical-input test.
