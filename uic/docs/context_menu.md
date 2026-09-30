# Context menu

UIC context menus are window-level overlays with one shared session for the root menu and up to two submenu levels. A session owns positioning, keyboard navigation, focus restoration, outside-click dismissal, and surface selection.

Initialize UIC, mount the context-menu layer as the last child of the window root, then attach a menu to any interactive element:

```rust,ignore
use gpui::{FontWeight, prelude::*, px};
use uic::components::context_menu::{self, ContextMenu, ContextMenuExt};

div()
    .child(
        div()
            .context_menu(|_, _| {
                ContextMenu::new()
                    .w(px(260.0))
                    .p_2()
                    .rounded(px(12.0))
                    .font_family("Inter")
                    .font_weight(FontWeight::MEDIUM)
                    .text_size(px(15.0))
                    .action_with_shortcut("Open", "Enter", |_, _| {})
                    .submenu("Open With", |menu| {
                        menu.action("Text Editor", |_, _| {})
                            .submenu("More", |menu| {
                                menu.action("Hex Editor", |_, _| {})
                            })
                    })
            })
            .child("Right-click me"),
    )
    .child(context_menu::layer(cx))
```

The layer should normally be a sibling of the application content rather than a child of the right-click target. `ContextMenu` uses the standard GPUI `Styled` API for its menu surface, including size, padding, background, border, radius, opacity, and typography. These refinements are inherited by every submenu. `ContextMenuAppearance` describes menu-specific states and row details such as selected, muted, danger, item height, and separator color. Custom label elements can override inherited styles locally.

## Surfaces

UIC does not depend on a particular material implementation. A surface callback receives the semantic menu body and returns any GPUI element.

`ContextMenuSurfaceState::depth` is zero based:

- `0` is the root menu.
- `1` is the second-level menu.
- `2` is the third-level menu.

The common root/submenu split can use the convenience methods:

```rust,ignore
ContextMenu::new()
    .root_surface(|state, content, _, _| {
        FrostedGlass::with_appearance(FrostedGlassAppearance::dark())
            .id(("root-menu", state.session_id))
            .child(content)
    })
    .submenu_surface(|state, content, _, _| {
        FrostedGlass::with_appearance(FrostedGlassAppearance::light())
            .id(("submenu", state.session_id * 10 + state.depth as u64))
            .child(content)
    })
```

Use `surface_for_depth` when all three levels need different styling:

```rust,ignore
ContextMenu::new()
    .surface_for_depth(0, dark_frosted_surface)
    .surface_for_depth(1, light_frosted_surface)
    .surface_for_depth(2, plain_div_surface)
```

`surface` applies one callback to all three levels. Set surface callbacks on the root `ContextMenu`; submenu definitions inherit the root session's surfaces and appearance.

## Positioning

`ContextMenuTrigger` anchors a menu to the current displayed bounds of its trigger.
Open menus and submenus follow layout, scrolling and affine transforms while retaining
their normal size and logical-pixel gap. Give triggers created in a loop a stable `.id(...)`.
`show_below` maps the supplied bounds once; use `ContextMenuTrigger` for continuous tracking.
Right-click menus map the click to window coordinates and stay at that position.

## Interaction

- Hovering a submenu entry opens it after a short delay.
- `Up` and `Down` move through enabled items.
- `Right` opens a submenu and `Left` returns to its parent.
- `Enter` or `Space` activates the selected entry.
- `Home`, `End`, and `Escape` follow standard menu behavior.
- A left or middle click outside all open levels closes the session and is consumed.
- A right click outside closes the old session and can open the target's context menu.
- Window deactivation or resize closes the session.

Actions close the session before running. Use `ContextMenuItem::keep_open(true)` for actions such as checkboxes that should leave the menu visible.

Use `action_with` and `submenu_with` when a row label needs custom GPUI content such as an icon, status badge, or check indicator.

Menus deeper than three levels are rejected by `context_menu::show` with `ContextMenuDepthError`.

## Examples

```sh
cargo run -p uic --example context_menu
cargo run -p uic --example transform_overlays
```

The transform example includes Dropdown, Popover and context menus. Open a popup,
then scroll over the background to change zoom.
