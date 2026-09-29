# Native file dragging

Start a typed drag with `on_drag` and promote it when the pointer approaches the
window edge:

```rust,ignore
window.promote_active_file_drag_to_system(
    paths, // Arc<[PathBuf]> containing absolute paths
    SystemFileDragOptions::default(),
    cx,
)?;
```

Ordinary in-window dragging stays internal. Promotion preserves the payload,
preview, and `on_drop::<T>` / `on_drag_end::<T>` handlers. Windows in the same
GPUI application receive the typed payload; other applications receive file URIs.
Paths are not canonicalized and no file contents or metadata are read.

The default options allow Copy and Move and prefer Move. X11 uses Ctrl for Copy
and Shift for Move. On Wayland, the compositor and destination determine the
preferred action and modifier behavior; the source cannot force a preference.
Link is available on X11 and rejected on Wayland.

Handle completion through `on_drag_end::<T>`:

- `Dropped` means a typed GPUI target accepted the payload.
- `ExternalDropped { action }` reports the native target's completed operation.
- `Unaccepted` means the drop was not accepted.
- `Cancelled` means the application or native protocol cancelled the session.
  Wayland also uses cancellation when no destination accepts the offer.
- `Failed` reports a transfer, timeout, or protocol failure without confirmed success.

GPUI never deletes or moves source files. After an external Move, refresh the
source directory; do not delete the original paths in the completion handler.
Failure or timeout does not guarantee that the destination made no filesystem
changes, so do not automatically retry a Move.

Run the two-window example with disposable files:

```sh
cargo run -p gpui --example file_drag -- /absolute/path/to/file
```

Drop on the other window's target or on another application's directory.
You can also drag files from a file manager onto the example's target. It displays
the received file count and prints the paths without copying or moving files.
Applications receive these incoming paths through `on_drop::<ExternalPaths>`;
drags within the same GPUI application keep their original typed payload.
