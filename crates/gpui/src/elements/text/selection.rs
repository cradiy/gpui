use super::{InteractiveTextClickEvent, TextLayout};
use crate::{
    App, Bounds, ClipboardItem, CursorStyle, DispatchPhase, FocusHandle, Hitbox, Hsla,
    KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PaintQuad, Pixels,
    Point, SharedString, TextAlign, Window, fill, point, px, size,
};
use std::{
    cell::{Cell, RefCell},
    ops::Range,
    rc::Rc,
};

pub(super) struct TextSelection {
    pub focus: FocusHandle,
    text: SharedString,
    anchor: usize,
    head: usize,
    dragging: bool,
    dragged: bool,
}

impl TextSelection {
    pub fn new(cx: &mut App) -> Self {
        Self {
            focus: cx.focus_handle(),
            text: "".into(),
            anchor: 0,
            head: 0,
            dragging: false,
            dragged: false,
        }
    }

    pub fn sync_text(&mut self, text: SharedString) -> bool {
        if self.text != text {
            self.text = text;
            self.anchor = 0;
            self.head = 0;
            self.dragging = false;
            self.dragged = false;
            return true;
        }
        false
    }

    pub fn range(&self) -> Range<usize> {
        self.anchor.min(self.head)..self.anchor.max(self.head)
    }
}

struct Row {
    range: Range<usize>,
    line_start: usize,
    layout: std::sync::Arc<crate::LineLayout>,
    origin: Point<Pixels>,
    base_x: Pixels,
    width: Pixels,
    height: Pixels,
    newline: bool,
}

fn rows(layout: &TextLayout, align: TextAlign) -> Vec<Row> {
    let inner = layout.0.borrow();
    let inner = inner.as_ref().expect("text is measured before selection");
    let bounds = inner.bounds.expect("text is prepainted before selection");
    let mut result = Vec::new();
    let mut line_start = 0;
    let mut y = bounds.top();
    for line in &inner.lines {
        let starts = std::iter::once(0)
            .chain(
                line.wrap_boundaries()
                    .iter()
                    .map(|b| line.runs()[b.run_ix].glyphs[b.glyph_ix].index),
            )
            .collect::<Vec<_>>();
        for (index, start) in starts.iter().copied().enumerate() {
            let end = starts.get(index + 1).copied().unwrap_or(line.len());
            let unwrapped = &line.layout.unwrapped_layout;
            let base_x = unwrapped.x_for_index(start);
            let width = unwrapped.x_for_index(end) - base_x;
            let offset = match align {
                TextAlign::Left => px(0.),
                TextAlign::Center => (bounds.size.width - width) / 2.,
                TextAlign::Right => bounds.size.width - width,
            };
            result.push(Row {
                range: line_start + start..line_start + end,
                line_start,
                layout: unwrapped.clone(),
                origin: point(bounds.left() + offset, y),
                base_x,
                width,
                height: inner.line_height,
                newline: index + 1 == starts.len() && line_start + end < inner.len,
            });
            y += inner.line_height;
        }
        line_start += line.len() + 1;
    }
    result
}

fn index_at(rows: &[Row], position: Point<Pixels>) -> usize {
    let Some(first) = rows.first() else {
        return 0;
    };
    if position.y < first.origin.y {
        return 0;
    }
    for row in rows {
        if position.y < row.origin.y + row.height {
            return (row.line_start
                + row
                    .layout
                    .closest_index_for_x(position.x - row.origin.x + row.base_x))
            .clamp(row.range.start, row.range.end);
        }
    }
    rows.last().unwrap().range.end
}

fn character_at(rows: &[Row], position: Point<Pixels>) -> Option<usize> {
    let row = rows.iter().find(|row| {
        position.y >= row.origin.y
            && position.y < row.origin.y + row.height
            && position.x >= row.origin.x
            && position.x < row.origin.x + row.width
    })?;
    row.layout
        .index_for_x(position.x - row.origin.x + row.base_x)
        .map(|index| row.line_start + index)
}

pub(super) fn selection_quads(
    layout: &TextLayout,
    selected: Range<usize>,
    align: TextAlign,
    color: Hsla,
) -> Vec<PaintQuad> {
    if selected.is_empty() {
        return Vec::new();
    }
    rows(layout, align)
        .into_iter()
        .filter_map(|row| {
            let start = selected.start.max(row.range.start);
            let end = selected.end.min(row.range.end);
            let newline =
                row.newline && selected.start <= row.range.end && selected.end > row.range.end;
            if start >= end && !newline {
                return None;
            }
            let x = if start < end {
                row.layout.x_for_index(start - row.line_start) - row.base_x
            } else {
                row.width
            };
            let right = if newline {
                row.width + px(3.)
            } else {
                row.layout.x_for_index(end - row.line_start) - row.base_x
            };
            Some(fill(
                Bounds::new(
                    row.origin + point(x, px(0.)),
                    size((right - x).max(px(1.)), row.height),
                ),
                color,
            ))
        })
        .collect()
}

type ClickListener = Box<dyn Fn(&[Range<usize>], InteractiveTextClickEvent, &mut Window, &mut App)>;

pub(super) fn register_handlers(
    selection: Rc<RefCell<TextSelection>>,
    layout: TextLayout,
    hitbox: Hitbox,
    mouse_down: Rc<Cell<Option<usize>>>,
    click: Option<ClickListener>,
    ranges: Vec<Range<usize>>,
    window: &mut Window,
) {
    let rows = Rc::new(rows(&layout, window.text_style().text_align));
    let hovered = character_at(&rows, window.mouse_position());
    window.set_cursor_style(
        if hovered.is_some_and(|index| ranges.iter().any(|range| range.contains(&index))) {
            CursorStyle::PointingHand
        } else {
            CursorStyle::IBeam
        },
        &hitbox,
    );
    window.on_mouse_event({
        let state = selection.clone();
        let rows = rows.clone();
        let hitbox = hitbox.clone();
        let down = mouse_down.clone();
        move |event: &MouseDownEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble || event.button != MouseButton::Left {
                return;
            }
            if !hitbox.is_hovered(window) {
                let mut state = state.borrow_mut();
                state.anchor = 0;
                state.head = 0;
                state.dragging = false;
                if state.focus.is_focused(window) {
                    window.blur();
                }
                down.set(None);
                window.refresh();
                return;
            }
            let index = index_at(&rows, event.position);
            let focus = {
                let mut state = state.borrow_mut();
                if !event.modifiers.shift {
                    state.anchor = index;
                }
                state.head = index;
                state.dragging = true;
                state.dragged = event.modifiers.shift;
                state.focus.clone()
            };
            down.set(character_at(&rows, event.position));
            window.focus(&focus, cx);
            window.refresh();
            cx.stop_propagation();
        }
    });
    window.on_mouse_event({
        let state = selection.clone();
        let rows = rows.clone();
        move |event: &MouseMoveEvent, phase, window, cx| {
            if phase != DispatchPhase::Capture {
                return;
            }
            let mut state = state.borrow_mut();
            if !state.dragging {
                return;
            }
            if event.pressed_button != Some(MouseButton::Left) || !state.focus.is_focused(window) {
                state.dragging = false;
                return;
            }
            let head = index_at(&rows, event.position);
            state.dragged |= head != state.head;
            state.head = head;
            drop(state);
            window.refresh();
            cx.stop_propagation();
        }
    });
    window.on_mouse_event({
        let state = selection.clone();
        move |event: &MouseUpEvent, phase, window, cx| {
            if phase != DispatchPhase::Capture || event.button != MouseButton::Left {
                return;
            }
            let should_click = {
                let mut state = state.borrow_mut();
                if !state.dragging {
                    return;
                }
                state.dragging = false;
                let head = index_at(&rows, event.position);
                state.dragged |= head != state.head;
                state.head = head;
                !state.dragged && state.range().is_empty() && hitbox.is_hovered(window)
            };
            let start = mouse_down.take();
            if should_click
                && let (Some(click), Some(start), Some(end)) =
                    (&click, start, character_at(&rows, event.position))
            {
                click(
                    &ranges,
                    InteractiveTextClickEvent {
                        mouse_down_index: start,
                        mouse_up_index: end,
                    },
                    window,
                    cx,
                );
            }
            window.refresh();
            cx.stop_propagation();
        }
    });
    window.on_key_event(move |event: &KeyDownEvent, phase, window, cx| {
        if phase != DispatchPhase::Bubble {
            return;
        }
        let mut state = selection.borrow_mut();
        if !state.focus.is_focused(window) {
            return;
        }
        let modifiers = event.keystroke.modifiers;
        let command = if cfg!(target_family = "wasm") {
            modifiers.platform != modifiers.control
        } else if cfg!(target_os = "macos") {
            modifiers.platform && !modifiers.control
        } else {
            modifiers.control && !modifiers.platform
        };
        if !command || modifiers.alt || modifiers.shift {
            return;
        }
        match event.keystroke.key.as_str() {
            "a" => {
                state.anchor = 0;
                state.head = state.text.len();
            }
            "c" => {
                if state.range().is_empty() {
                    return;
                }
                cx.write_to_clipboard(ClipboardItem::new_string(
                    state.text[state.range()].to_owned(),
                ));
            }
            _ => return,
        }
        drop(state);
        window.refresh();
        cx.stop_propagation();
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Context, FontWeight, InteractiveText, IntoElement, Render, StyledText, TestAppContext,
        VisualTestContext, WindowHandle, div, prelude::*,
    };

    struct Demo {
        text: SharedString,
        layout: TextLayout,
        clicks: usize,
        align: TextAlign,
        truncate: bool,
    }
    impl Render for Demo {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let text = StyledText::new(self.text.clone())
                .with_highlights([(0..4, FontWeight::BOLD.into())]);
            self.layout = text.layout().clone();
            let clicked = cx.listener(|this, _: &usize, _, _| this.clicks += 1);
            div().size_full().p_4().child(
                div()
                    .w(px(180.))
                    .text_align(self.align)
                    .when(self.truncate, |this| this.truncate())
                    .child(
                        InteractiveText::new("selectable", text)
                            .selectable(crate::rgba(0x4488ff55))
                            .on_click(vec![0..4], move |index, window, cx| {
                                clicked(&index, window, cx)
                            }),
                    ),
            )
        }
    }

    fn setup(cx: &mut TestAppContext, align: TextAlign) -> (WindowHandle<Demo>, VisualTestContext) {
        let window = cx.open_window(size(px(400.), px(400.)), move |_, _| Demo {
            text: "link 中文😀\nA long styled line that wraps across several rows.\n\nLast".into(),
            layout: TextLayout::default(),
            clicks: 0,
            align,
            truncate: false,
        });
        let mut visual = VisualTestContext::from_window(window.into(), cx);
        visual.update(|window, cx| {
            window.draw(cx).clear();
        });
        (window, visual)
    }

    fn position(
        window: WindowHandle<Demo>,
        visual: &mut VisualTestContext,
        index: usize,
    ) -> Point<Pixels> {
        window
            .update(&mut visual.cx, |view, _, _| {
                let rows = rows(&view.layout, view.align);
                let row = rows
                    .iter()
                    .find(|row| row.range.start <= index && index <= row.range.end)
                    .unwrap();
                row.origin
                    + point(
                        row.layout.x_for_index(index - row.line_start) - row.base_x,
                        row.height / 2.,
                    )
            })
            .unwrap()
    }

    fn copy(visual: &mut VisualTestContext) -> Option<String> {
        visual.simulate_keystrokes(if cfg!(target_os = "macos") {
            "cmd-c"
        } else {
            "ctrl-c"
        });
        visual.update(|_, cx| cx.read_from_clipboard().and_then(|item| item.text()))
    }

    #[crate::test]
    fn selectable_text_drag_copies_unicode_across_wrapped_rows(cx: &mut TestAppContext) {
        let (window, mut visual) = setup(cx, TextAlign::Center);
        let start = position(window, &mut visual, 5);
        let end = position(window, &mut visual, 35);
        visual.simulate_mouse_down(end, MouseButton::Left, Default::default());
        visual.simulate_mouse_move(start, MouseButton::Left, Default::default());
        visual.simulate_mouse_up(start, MouseButton::Left, Default::default());
        let expected = window
            .update(&mut visual.cx, |view, _, _| view.text[5..35].to_owned())
            .unwrap();
        assert_eq!(copy(&mut visual), Some(expected));
        visual.simulate_keystrokes(if cfg!(target_os = "macos") {
            "cmd-a"
        } else {
            "ctrl-a"
        });
        let text = window
            .update(&mut visual.cx, |view, _, _| view.text.to_string())
            .unwrap();
        assert_eq!(copy(&mut visual), Some(text));
    }

    #[crate::test]
    fn selectable_text_drag_does_not_activate_links_and_resets_on_content_change(
        cx: &mut TestAppContext,
    ) {
        let (window, mut visual) = setup(cx, TextAlign::Left);
        let start = position(window, &mut visual, 0);
        let end = position(window, &mut visual, 3);
        visual.simulate_mouse_down(start, MouseButton::Left, Default::default());
        visual.simulate_mouse_move(end, MouseButton::Left, Default::default());
        visual.simulate_mouse_up(end, MouseButton::Left, Default::default());
        assert_eq!(copy(&mut visual).as_deref(), Some("lin"));
        window
            .update(&mut visual.cx, |view, _, _| assert_eq!(view.clicks, 0))
            .unwrap();
        let link_end = position(window, &mut visual, 4);
        visual.simulate_click(
            point(end.x + (link_end.x - end.x) * 0.8, end.y),
            Default::default(),
        );
        visual.simulate_keystrokes(if cfg!(target_os = "macos") {
            "cmd-a"
        } else {
            "ctrl-a"
        });
        window
            .update(&mut visual.cx, |view, _, cx| {
                assert_eq!(view.clicks, 1);
                view.text = "link".into();
                cx.notify();
            })
            .unwrap();
        visual.update(|window, cx| {
            window.draw(cx).clear();
        });
        assert_eq!(copy(&mut visual).as_deref(), Some("lin"));
        visual.simulate_keystrokes(if cfg!(target_os = "macos") {
            "cmd-a"
        } else {
            "ctrl-a"
        });
        assert_eq!(copy(&mut visual).as_deref(), Some("link"));
    }

    #[crate::test]
    fn selectable_text_highlights_wrapped_and_empty_lines(cx: &mut TestAppContext) {
        let (window, mut visual) = setup(cx, TextAlign::Right);
        window
            .update(&mut visual.cx, |view, _, _| {
                let rows = rows(&view.layout, view.align);
                assert!(rows.len() > 4);
                let quads =
                    selection_quads(&view.layout, 0..view.text.len(), view.align, crate::blue());
                assert_eq!(quads.len(), rows.len());
                for (quad, row) in quads.iter().zip(&rows) {
                    assert_eq!(quad.bounds.origin, row.origin);
                    assert!(quad.bounds.size.width > px(0.));
                    assert_eq!(quad.bounds.size.height, row.height);
                }
            })
            .unwrap();
    }

    #[crate::test]
    fn selectable_text_copies_displayed_ellipsis_and_clears_on_outside_click(
        cx: &mut TestAppContext,
    ) {
        let (window, mut visual) = setup(cx, TextAlign::Left);
        window
            .update(&mut visual.cx, |view, _, cx| {
                view.text = "link 中文😀 repeated text that is wider than the block".into();
                view.truncate = true;
                cx.notify();
            })
            .unwrap();
        visual.update(|window, cx| {
            window.draw(cx).clear();
        });
        let (displayed, pos) = window
            .update(&mut visual.cx, |view, _, _| {
                let layout = view.layout.0.borrow();
                let layout = layout.as_ref().unwrap();
                assert_ne!(layout.text, view.text);
                (layout.text.to_string(), layout.bounds.unwrap().center())
            })
            .unwrap();
        visual.simulate_click(pos, Default::default());
        visual.simulate_keystrokes(if cfg!(target_os = "macos") {
            "cmd-a"
        } else {
            "ctrl-a"
        });
        assert_eq!(copy(&mut visual), Some(displayed));
        visual.simulate_click(point(px(380.), px(380.)), Default::default());
        visual.simulate_keystrokes(if cfg!(target_os = "macos") {
            "cmd-a"
        } else {
            "ctrl-a"
        });
        visual.update(|_, cx| cx.write_to_clipboard(ClipboardItem::new_string("unchanged".into())));
        assert_eq!(copy(&mut visual).as_deref(), Some("unchanged"));
    }
}
