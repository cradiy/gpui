# Semantic inspection

`gpui_automation` provides in-process window discovery and exact-match semantic
queries. It enables GPUI's `automation` Cargo feature, but collection remains
off until explicitly enabled for a window. No network listener or external
control endpoint is started.

For semantic activation, value updates and runnable verification commands, see
[Actions and waiting](actions.md).

## Identify elements

```rust
use gpui::{div, prelude::*, Role};

let button = div()
    .automation_id("save")
    .role(Role::Button)
    .aria_label("Save changes");
```

`automation_id` supplies a stable element ID and Group role if these were not
already assigned. Repeated identifiers are allowed under distinct GPUI ID
paths. Use a parent scope to disambiguate them. Custom elements can provide an
AccessKit author ID through `Element::write_a11y_info`.

## Collect and query

Call `Session::windows(cx)` to list windows or `Session::window_by_title(title,
cx)` to require one exact title match. Titles need not be unique; ambiguous
lookups return an error.

```rust,ignore
Session::enable(handle, cx)?;
// Read after the application has completed its next draw.
let snapshot = Session::snapshot(handle, cx)?;
let button = snapshot.get_by_role(Role::Button).named("Save changes").one()?;
let panel = snapshot.get_by_id("preferences").one()?;
let scoped = snapshot.get_by_id("save").within(panel.id).one()?;
```

`one()` rejects zero or multiple matches. `all()` returns every match in
semantic tree order. Names match authored labels exactly; visible text and
label relationships are not used to infer names. `within` searches descendants
and excludes the scope node itself. Node IDs belong to a window, and scope IDs
must be taken from the queried snapshot.

Snapshots retain their contents when the interface changes. Each includes the
window ID, snapshot generation, viewport size and scale factor. The generation
identifies a completed draw, not a GPU completion or presentation fence.
Obtain a new snapshot to inspect current state. Enabling collection schedules
a draw; querying before that draw returns a not-ready error.

## Semantics and privacy

The tree contains elements that provide semantic roles and stable IDs, including
synthetic accessibility children. It is not a complete layout tree. Bounds are
reported in window-local logical pixels and do not prove visibility, clipping,
or pointer hit-testability. Hidden and disabled flags reflect the semantic tree
and its ancestors; applications must report them accurately. `aria_disabled`
reports state but does not disable input handlers.

Password values and content beneath PasswordInput nodes are redacted. The
password field's own authored label is retained. Other sensitive content must
not be exposed through semantic labels or values. An application created with
accessibility forcibly disabled cannot enable semantic collection.

Collection rebuilds semantic information inside cached views while enabled.
`Session::disable` stops collection and drops the window's retained snapshot;
snapshots already held by the caller remain readable.
