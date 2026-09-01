mod appearance;
mod menu;

use std::{cell::Cell, fmt, rc::Rc, time::Duration};

use gpui::{
    Anchor, AnchoredPositionMode, AnyElement, App, Bounds, Context, Entity, FocusHandle, Global,
    InteractiveElement, IntoElement, KeyDownEvent, MouseButton, MouseDownEvent, Pixels, Point,
    Render, Subscription, Task, Window, WindowId, anchored, canvas, deferred, div, point,
    prelude::*,
};

pub use appearance::ContextMenuAppearance;
pub use menu::{ContextMenu, ContextMenuItem};
use menu::{ContextMenuEntry, ContextMenuItemKind};

pub const MAX_CONTEXT_MENU_DEPTH: usize = 3;
const CONTEXT_MENU_PRIORITY: usize = 1_100;
const SUBMENU_OPEN_DELAY: Duration = Duration::from_millis(150);

type BoundsTracker = Rc<Cell<Option<Bounds<Pixels>>>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContextMenuPlacement {
    Root,
    Left,
    Right,
}

#[derive(Clone, Copy, Debug)]
pub struct ContextMenuSurfaceState {
    pub session_id: u64,
    /// Zero-based menu depth: 0=root, 1=second level, 2=third level.
    pub depth: usize,
    pub is_submenu: bool,
    pub placement: ContextMenuPlacement,
}

type SurfaceRender =
    dyn Fn(ContextMenuSurfaceState, AnyElement, &mut Window, &mut App) -> AnyElement;

#[derive(Clone)]
pub struct ContextMenuSurface(Rc<SurfaceRender>);

impl ContextMenuSurface {
    fn new<E: IntoElement>(
        render: impl Fn(ContextMenuSurfaceState, AnyElement, &mut Window, &mut App) -> E + 'static,
    ) -> Self {
        Self(Rc::new(move |state, content, window, cx| {
            render(state, content, window, cx).into_any_element()
        }))
    }
}

#[derive(Clone, Default)]
pub(crate) struct ContextMenuSurfaces {
    by_depth: [Option<ContextMenuSurface>; MAX_CONTEXT_MENU_DEPTH],
}

impl ContextMenuSurfaces {
    fn all(surface: ContextMenuSurface) -> Self {
        Self {
            by_depth: [Some(surface.clone()), Some(surface.clone()), Some(surface)],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContextMenuDismissReason {
    Action,
    OutsideClick,
    Escape,
    WindowBlur,
    Replaced,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ContextMenuDepthError {
    pub depth: usize,
}

impl fmt::Display for ContextMenuDepthError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "context menus support at most {MAX_CONTEXT_MENU_DEPTH} levels, but level {} was found",
            self.depth
        )
    }
}

impl std::error::Error for ContextMenuDepthError {}

#[derive(Clone)]
struct ContextMenuLevel {
    menu: ContextMenu,
    position: Point<Pixels>,
    anchor: Anchor,
    placement: ContextMenuPlacement,
    selected_index: Option<usize>,
    bounds: BoundsTracker,
    item_bounds: Vec<BoundsTracker>,
}

impl ContextMenuLevel {
    fn new(
        menu: ContextMenu,
        position: Point<Pixels>,
        anchor: Anchor,
        placement: ContextMenuPlacement,
    ) -> Self {
        let selected_index = menu.entries.iter().position(selectable);
        let item_bounds = menu
            .entries
            .iter()
            .map(|_| Rc::new(Cell::new(None)))
            .collect();
        Self {
            menu,
            position,
            anchor,
            placement,
            selected_index,
            bounds: Rc::new(Cell::new(None)),
            item_bounds,
        }
    }
}

struct ActiveContextMenu {
    session_id: u64,
    window_id: WindowId,
    previous_focus: Option<FocusHandle>,
    levels: Vec<ContextMenuLevel>,
    appearance: ContextMenuAppearance,
    style: gpui::StyleRefinement,
    viewport_margin: Pixels,
    submenu_gap: Pixels,
    surfaces: ContextMenuSurfaces,
    viewport_width: Pixels,
}

pub struct ContextMenuLayer {
    active: Option<ActiveContextMenu>,
    focus_handle: FocusHandle,
    appearance: ContextMenuAppearance,
    next_session_id: u64,
    pending_submenu: Option<Task<()>>,
    window_subscriptions: Vec<Subscription>,
}

impl ContextMenuLayer {
    fn new(appearance: ContextMenuAppearance, cx: &mut Context<Self>) -> Self {
        Self {
            active: None,
            focus_handle: cx.focus_handle(),
            appearance,
            next_session_id: 1,
            pending_submenu: None,
            window_subscriptions: Vec::new(),
        }
    }

    pub fn is_open(&self) -> bool {
        self.active.is_some()
    }

    fn show(
        &mut self,
        menu: ContextMenu,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.active.is_some() {
            self.dismiss(ContextMenuDismissReason::Replaced, false, window, cx);
        }

        let viewport_width = window.viewport_size().width;
        let appearance = menu.appearance.unwrap_or(self.appearance);
        let style = menu.style.clone();
        let anchor = root_menu_anchor();
        let surfaces = menu.surfaces.clone();
        let viewport_margin = menu.viewport_margin;
        let submenu_gap = menu.submenu_gap;
        let level = ContextMenuLevel::new(menu, position, anchor, ContextMenuPlacement::Root);
        let session_id = self.next_session_id;
        self.next_session_id += 1;
        self.active = Some(ActiveContextMenu {
            session_id,
            window_id: window.window_handle().window_id(),
            previous_focus: window.focused(cx),
            levels: vec![level],
            appearance,
            style,
            viewport_margin,
            submenu_gap,
            surfaces,
            viewport_width,
        });
        self.window_subscriptions.clear();
        self.window_subscriptions.push(cx.observe_window_activation(
            window,
            |layer, window, cx| {
                if !window.is_window_active() {
                    layer.dismiss(ContextMenuDismissReason::WindowBlur, false, window, cx);
                }
            },
        ));
        self.window_subscriptions
            .push(cx.observe_window_bounds(window, |layer, window, cx| {
                if layer.active.is_some() {
                    layer.dismiss(ContextMenuDismissReason::Replaced, false, window, cx);
                }
            }));
        self.focus_handle.focus(window, cx);
        window.refresh();
        cx.notify();
    }

    fn dismiss(
        &mut self,
        _reason: ContextMenuDismissReason,
        restore_focus: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.pending_submenu = None;
        let Some(active) = self.active.take() else {
            return;
        };
        if active.window_id != window.window_handle().window_id() {
            self.active = Some(active);
            return;
        }
        self.window_subscriptions.clear();
        if restore_focus && let Some(previous_focus) = active.previous_focus {
            previous_focus.focus(window, cx);
        }
        window.refresh();
        cx.notify();
    }

    fn point_inside_menu(&self, position: Point<Pixels>) -> bool {
        self.active.as_ref().is_some_and(|active| {
            active.levels.iter().any(|level| {
                level
                    .bounds
                    .get()
                    .is_some_and(|bounds| bounds.contains(&position))
            })
        })
    }

    fn hover_item(
        &mut self,
        depth: usize,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(active) = self.active.as_mut() else {
            return;
        };
        let Some(level) = active.levels.get_mut(depth) else {
            return;
        };
        if !selectable(&level.menu.entries[index]) {
            return;
        }
        level.selected_index = Some(index);
        self.pending_submenu = None;

        if matches!(
            level.menu.entries[index],
            ContextMenuEntry::Item(ContextMenuItem {
                kind: ContextMenuItemKind::Submenu(_),
                ..
            })
        ) {
            let task = cx.spawn(async move |this, cx| {
                cx.background_executor().timer(SUBMENU_OPEN_DELAY).await;
                if let Some(this) = this.upgrade() {
                    this.update(cx, |layer, cx| {
                        layer.open_submenu(depth, index, cx);
                    });
                }
            });
            self.pending_submenu = Some(task);
        } else if active.levels.len() > depth + 1 {
            active.levels.truncate(depth + 1);
        }
        window.refresh();
        cx.notify();
    }

    fn open_submenu(&mut self, depth: usize, index: usize, cx: &mut Context<Self>) {
        self.pending_submenu = None;
        let Some(active) = self.active.as_mut() else {
            return;
        };
        if depth + 1 >= MAX_CONTEXT_MENU_DEPTH {
            return;
        }
        let Some(parent_level) = active.levels.get(depth) else {
            return;
        };
        let Some(ContextMenuEntry::Item(ContextMenuItem {
            kind: ContextMenuItemKind::Submenu(submenu),
            disabled: false,
            ..
        })) = parent_level.menu.entries.get(index)
        else {
            return;
        };
        let Some(item_bounds) = parent_level.item_bounds[index].get() else {
            return;
        };

        let right_position = item_bounds.right() + active.submenu_gap;
        let estimated_width = parent_level
            .bounds
            .get()
            .map(|bounds| bounds.size.width)
            .unwrap_or(item_bounds.size.width);
        let room_on_right =
            right_position + estimated_width + active.viewport_margin <= active.viewport_width;
        let (position, anchor, placement) = if room_on_right {
            (
                point(right_position, item_bounds.top()),
                Anchor::TopLeft,
                ContextMenuPlacement::Right,
            )
        } else {
            (
                point(item_bounds.left() - active.submenu_gap, item_bounds.top()),
                Anchor::TopRight,
                ContextMenuPlacement::Left,
            )
        };
        let level = ContextMenuLevel::new((**submenu).clone(), position, anchor, placement);
        active.levels.truncate(depth + 1);
        active.levels.push(level);
        cx.notify();
    }

    fn activate_item(
        &mut self,
        depth: usize,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let kind = self.active.as_ref().and_then(|active| {
            let item = active.levels.get(depth)?.menu.entries.get(index)?;
            match item {
                ContextMenuEntry::Item(item) if !item.disabled => {
                    Some((item.kind.clone(), item.keep_open))
                }
                _ => None,
            }
        });
        let Some((kind, keep_open)) = kind else {
            return;
        };

        match kind {
            ContextMenuItemKind::Submenu(_) => {
                self.pending_submenu = None;
                self.open_submenu(depth, index, cx);
            }
            ContextMenuItemKind::Action(action) => {
                if !keep_open {
                    self.dismiss(ContextMenuDismissReason::Action, true, window, cx);
                }
                action(window, cx);
            }
        }
    }

    fn move_selection(&mut self, direction: isize, cx: &mut Context<Self>) {
        let Some(level) = self
            .active
            .as_mut()
            .and_then(|active| active.levels.last_mut())
        else {
            return;
        };
        let len = level.menu.entries.len();
        if len == 0 {
            return;
        }
        let mut index = level
            .selected_index
            .unwrap_or(if direction > 0 { len - 1 } else { 0 });
        for _ in 0..len {
            index = if direction > 0 {
                (index + 1) % len
            } else {
                (index + len - 1) % len
            };
            if selectable(&level.menu.entries[index]) {
                level.selected_index = Some(index);
                cx.notify();
                return;
            }
        }
    }

    fn handle_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let depth = self
            .active
            .as_ref()
            .map_or(0, |active| active.levels.len().saturating_sub(1));
        match event.keystroke.key.as_ref() {
            "escape" => self.dismiss(ContextMenuDismissReason::Escape, true, window, cx),
            "up" => self.move_selection(-1, cx),
            "down" => self.move_selection(1, cx),
            "right" => {
                if let Some(index) = self
                    .active
                    .as_ref()
                    .and_then(|active| active.levels.get(depth))
                    .and_then(|level| level.selected_index)
                {
                    self.open_submenu(depth, index, cx);
                }
            }
            "left" if depth > 0 => {
                if let Some(active) = self.active.as_mut() {
                    active.levels.pop();
                    cx.notify();
                }
            }
            "enter" | "space" => {
                if let Some(index) = self
                    .active
                    .as_ref()
                    .and_then(|active| active.levels.get(depth))
                    .and_then(|level| level.selected_index)
                {
                    self.activate_item(depth, index, window, cx);
                }
            }
            "home" => self.select_edge(false, cx),
            "end" => self.select_edge(true, cx),
            _ => return,
        }
        cx.stop_propagation();
    }

    fn select_edge(&mut self, from_end: bool, cx: &mut Context<Self>) {
        let Some(level) = self
            .active
            .as_mut()
            .and_then(|active| active.levels.last_mut())
        else {
            return;
        };
        level.selected_index = if from_end {
            level.menu.entries.iter().rposition(selectable)
        } else {
            level.menu.entries.iter().position(selectable)
        };
        cx.notify();
    }

    fn render_level(
        &self,
        level: &ContextMenuLevel,
        depth: usize,
        active: &ActiveContextMenu,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let appearance = active.appearance;
        let items = level
            .menu
            .entries
            .iter()
            .enumerate()
            .map(|(index, entry)| match entry {
                ContextMenuEntry::Separator => div()
                    .h(gpui::px(1.))
                    .my(appearance.separator_margin)
                    .bg(appearance.separator)
                    .into_any_element(),
                ContextMenuEntry::Item(item) => {
                    let disabled = item.disabled;
                    let selected = level.selected_index == Some(index);
                    let has_submenu = matches!(item.kind, ContextMenuItemKind::Submenu(_));
                    let foreground = if disabled {
                        Some(appearance.muted_foreground)
                    } else if selected {
                        Some(appearance.selected_foreground)
                    } else if item.danger {
                        Some(appearance.danger_foreground)
                    } else {
                        None
                    };
                    let tracker = level.item_bounds[index].clone();
                    let label = (item.label)(window, cx);
                    let shortcut = item.shortcut.clone();
                    let row = div()
                        .id((
                            "context-menu-item",
                            active.session_id as usize * 1_000 + depth * 100 + index,
                        ))
                        .relative()
                        .h(appearance.item_height)
                        .px(appearance.item_padding_x)
                        .flex()
                        .items_center()
                        .gap_3()
                        .rounded(appearance.item_radius)
                        .when_some(foreground, |this, foreground| this.text_color(foreground));
                    row.when(selected && !disabled, |this| {
                        this.bg(appearance.selected_background)
                    })
                    .when(!disabled, |this| {
                        this.cursor_pointer()
                            .on_hover(cx.listener(move |layer, hovered, window, cx| {
                                if *hovered {
                                    layer.hover_item(depth, index, window, cx);
                                }
                            }))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |layer, _, window, cx| {
                                    layer.activate_item(depth, index, window, cx);
                                    cx.stop_propagation();
                                }),
                            )
                    })
                    .child(div().flex_1().min_w_0().child(label))
                    .when_some(shortcut, |this, shortcut| {
                        this.child(
                            div()
                                .flex_none()
                                .text_color(appearance.muted_foreground)
                                .child(shortcut),
                        )
                    })
                    .when(has_submenu, |this| {
                        this.child(
                            div()
                                .flex_none()
                                .text_color(appearance.muted_foreground)
                                .child("›"),
                        )
                    })
                    .child(bounds_tracker(tracker))
                    .into_any_element()
                }
            });

        let mut content = div()
            .id((
                "context-menu-scroll",
                active.session_id as usize * MAX_CONTEXT_MENU_DEPTH + depth,
            ))
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .children(items);
        gpui::Refineable::refine(content.style(), &active.style);
        let content = content.into_any_element();
        let state = ContextMenuSurfaceState {
            session_id: active.session_id,
            depth,
            is_submenu: depth > 0,
            placement: level.placement,
        };
        let surface = match active.surfaces.by_depth[depth].as_ref() {
            Some(surface) => (surface.0)(state, content, window, cx),
            None => div().shadow_lg().child(content).into_any_element(),
        };
        let bounds = level.bounds.clone();
        let mut wrapper = div()
            .id((
                "context-menu-level",
                active.session_id as usize * MAX_CONTEXT_MENU_DEPTH + depth,
            ))
            .relative()
            .occlude()
            .on_any_mouse_down(|_, _, cx| cx.stop_propagation())
            .child(surface)
            .child(bounds_tracker(bounds));
        if depth == 0 {
            wrapper = wrapper.on_mouse_down_out(cx.listener(
                |layer, event: &MouseDownEvent, window, cx| {
                    if !layer.point_inside_menu(event.position) {
                        let pass_through = event.button == MouseButton::Right;
                        layer.dismiss(ContextMenuDismissReason::OutsideClick, true, window, cx);
                        if !pass_through {
                            cx.stop_propagation();
                        }
                    }
                },
            ));
        }

        deferred(
            anchored()
                .anchor(level.anchor)
                .position(level.position)
                .position_mode(AnchoredPositionMode::Window)
                .snap_to_window_with_margin(active.viewport_margin)
                .child(wrapper),
        )
        .with_priority(CONTEXT_MENU_PRIORITY + depth)
        .into_any_element()
    }
}

impl Render for ContextMenuLayer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(active) = self.active.as_ref() else {
            return div().into_any_element();
        };
        if active.window_id != window.window_handle().window_id() {
            return div().into_any_element();
        }
        let levels = active
            .levels
            .iter()
            .enumerate()
            .map(|(depth, level)| self.render_level(level, depth, active, window, cx))
            .collect::<Vec<_>>();

        div()
            .absolute()
            .inset_0()
            .track_focus(&self.focus_handle)
            .capture_key_down(cx.listener(|layer, event, window, cx| {
                layer.handle_key(event, window, cx);
            }))
            .children(levels)
            .into_any_element()
    }
}

fn bounds_tracker(tracker: BoundsTracker) -> impl IntoElement {
    canvas(
        move |bounds, _, _| tracker.set(Some(bounds)),
        |_, _, _, _| {},
    )
    .absolute()
    .inset_0()
}

fn selectable(entry: &ContextMenuEntry) -> bool {
    matches!(entry, ContextMenuEntry::Item(item) if !item.disabled)
}

fn root_menu_anchor() -> Anchor {
    // Root menus should follow the pointer and only move when their measured bounds
    // would overflow the viewport. Anchoring every click in the right half to the
    // top-right corner makes the menu jump left at the window midpoint, even when
    // there is enough room to open to the right.
    Anchor::TopLeft
}

struct GlobalContextMenu(Entity<ContextMenuLayer>);

impl Global for GlobalContextMenu {}

pub fn init(cx: &mut App) {
    init_with_appearance(ContextMenuAppearance::default(), cx);
}

pub fn init_with_appearance(appearance: ContextMenuAppearance, cx: &mut App) {
    if cx.has_global::<GlobalContextMenu>() {
        layer(cx).update(cx, |layer, cx| {
            layer.appearance = appearance;
            cx.notify();
        });
    } else {
        let layer = cx.new(|cx| ContextMenuLayer::new(appearance, cx));
        cx.set_global(GlobalContextMenu(layer));
    }
}

/// Returns the global layer. Mount it as the last child of each window root.
pub fn layer(cx: &App) -> Entity<ContextMenuLayer> {
    cx.global::<GlobalContextMenu>().0.clone()
}

pub fn show(
    menu: ContextMenu,
    position: Point<Pixels>,
    window: &mut Window,
    cx: &mut App,
) -> Result<(), ContextMenuDepthError> {
    menu.validate_depth(0)
        .map_err(|depth| ContextMenuDepthError { depth })?;
    layer(cx).update(cx, |layer, cx| layer.show(menu, position, window, cx));
    Ok(())
}

pub fn dismiss(window: &mut Window, cx: &mut App) {
    layer(cx).update(cx, |layer, cx| {
        layer.dismiss(ContextMenuDismissReason::OutsideClick, true, window, cx)
    });
}

pub fn is_open(cx: &App) -> bool {
    layer(cx).read(cx).is_open()
}

/// Adds a right-click context-menu trigger without adding a layout wrapper.
pub trait ContextMenuExt: InteractiveElement + Sized {
    fn context_menu(self, build: impl Fn(&mut Window, &mut App) -> ContextMenu + 'static) -> Self {
        self.on_mouse_down(MouseButton::Right, move |event, window, cx| {
            let result = show(build(window, cx), event.position, window, cx);
            debug_assert!(result.is_ok(), "{result:?}");
            cx.stop_propagation();
        })
    }
}

impl<T: InteractiveElement> ContextMenuExt for T {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_menu_starts_at_the_pointer_without_a_midpoint_flip() {
        assert_eq!(root_menu_anchor(), Anchor::TopLeft);
    }
}
