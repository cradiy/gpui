use std::{borrow::Cow, ops::Range, time::Duration};

use gpui::{
    App, Bounds, CaretAffinity, ClipboardItem, Context, CursorStyle, DispatchPhase,
    EntityInputHandler, FocusHandle, Focusable, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, Pixels, Point, PreeditSelection, Render, ScrollHandle, SharedString, Task,
    TextCaret, UTF16Selection, Window, WrappedLine, div, point, prelude::*, px,
};
use unicode_segmentation::UnicodeSegmentation;

use super::{InputAppearance, InputEvent, InputMode, actions::*, element::TextElement};
use crate::components::scrollbar::ScrollbarState;

pub(super) struct TextLayout {
    pub(super) lines: Vec<WrappedLine>,
    pub(super) line_starts: Vec<usize>,
    pub(super) line_height: Pixels,
    pub(super) shaping_run: Option<gpui::TextRun>,
}

fn preedit_offset_from_utf16(text: &str, offset: usize) -> usize {
    let mut units = 0;
    for (byte, ch) in text.char_indices() {
        if units + ch.len_utf16() > offset {
            return byte;
        }
        units += ch.len_utf16();
    }
    text.len()
}

impl TextLayout {
    pub(super) fn new(
        lines: Vec<WrappedLine>,
        line_starts: Vec<usize>,
        line_height: Pixels,
    ) -> Self {
        Self {
            lines,
            line_starts,
            line_height,
            shaping_run: None,
        }
    }

    pub(super) fn visual_row_count(&self) -> usize {
        self.lines
            .iter()
            .map(|line| line.wrap_boundaries().len() + 1)
            .sum::<usize>()
            .max(1)
    }

    fn line_for_offset(&self, offset: usize) -> (usize, usize) {
        let line_ix = self
            .line_starts
            .partition_point(|start| *start <= offset)
            .saturating_sub(1)
            .min(self.lines.len().saturating_sub(1));
        let line_start = self.line_starts.get(line_ix).copied().unwrap_or(0);
        let local_offset = offset
            .saturating_sub(line_start)
            .min(self.lines.get(line_ix).map_or(0, WrappedLine::len));
        (line_ix, local_offset)
    }

    pub(super) fn position_for_offset(&self, offset: usize) -> Point<Pixels> {
        self.position_for_caret(offset, CaretAffinity::Downstream)
    }

    pub(super) fn position_for_caret(
        &self,
        offset: usize,
        affinity: CaretAffinity,
    ) -> Point<Pixels> {
        let (line_ix, local_offset) = self.line_for_offset(offset);
        let rows_before = self
            .lines
            .iter()
            .take(line_ix)
            .map(|line| line.wrap_boundaries().len() + 1)
            .sum::<usize>();
        let local = self.lines[line_ix]
            .position_for_caret(
                TextCaret {
                    index: local_offset,
                    affinity,
                },
                self.line_height,
            )
            .unwrap_or_default();
        point(local.x, local.y + self.line_height * rows_before as f32)
    }

    pub(super) fn offset_for_position(&self, position: Point<Pixels>) -> usize {
        self.caret_for_position(position).index
    }

    fn caret_for_position(&self, position: Point<Pixels>) -> TextCaret {
        if position.y < px(0.) {
            return 0.into();
        }

        let target_row = (position.y / self.line_height).floor() as usize;
        let mut rows_before = 0;
        for (line_ix, line) in self.lines.iter().enumerate() {
            let rows = line.wrap_boundaries().len() + 1;
            if target_row < rows_before + rows {
                let local_y = self.line_height * (target_row - rows_before) as f32;
                let mut caret =
                    line.closest_caret_for_position(point(position.x, local_y), self.line_height);
                caret.index += self.line_starts[line_ix];
                return caret;
            }
            rows_before += rows;
        }

        (self.line_starts.last().copied().unwrap_or(0)
            + self.lines.last().map_or(0, WrappedLine::len))
        .into()
    }

    fn row_range_for_offset(&self, offset: usize) -> Range<usize> {
        let (line_ix, local) = self.line_for_offset(offset);
        let line = &self.lines[line_ix];
        let position = line
            .position_for_index(local, self.line_height)
            .unwrap_or_default();
        let row = (position.y / self.line_height) as usize;
        let range = line.row_range(row).unwrap_or(0..0);
        let start = self.line_starts[line_ix];
        start + range.start..start + range.end
    }

    fn matches_content(&self, content: &str) -> bool {
        self.line_starts.last().copied().unwrap_or(0)
            + self.lines.last().map_or(0, WrappedLine::len)
            == content.len()
            && self
                .lines
                .iter()
                .zip(&self.line_starts)
                .all(|(line, start)| {
                    content.get(*start..start + line.len()) == Some(line.text.as_ref())
                })
    }

    fn refresh_for_edit(&mut self, content: &SharedString, shaper: &gpui::WindowTextSystem) {
        if self.matches_content(content) {
            return;
        }
        let Some(mut run) = self.shaping_run.clone() else {
            return;
        };
        let Some(first) = self.lines.first() else {
            return;
        };
        run.len = content.len();
        if let Ok(lines) = shaper.shape_text(
            content.clone(),
            first.font_size(),
            &[run],
            first.wrap_width,
            None,
        ) {
            self.lines = lines.into_vec();
            self.line_starts = std::iter::once(0)
                .chain(content.match_indices('\n').map(|(i, _)| i + 1))
                .collect();
        }
    }

    fn bidi_horizontal_target(
        &self,
        content: &str,
        caret: TextCaret,
        right: bool,
        collapse: Option<Range<usize>>,
    ) -> Option<TextCaret> {
        if !self.lines.iter().any(|line| {
            line.runs()
                .iter()
                .any(|run| run.glyphs.iter().any(|glyph| glyph.is_rtl))
        }) || !self.matches_content(content)
        {
            return None;
        }

        let boundaries: Vec<_> = content
            .grapheme_indices(true)
            .map(|(i, _)| i)
            .chain(std::iter::once(content.len()))
            .collect();
        let mut stops = Vec::new();
        let mut selection_edges = Vec::new();
        let mut y = px(0.);
        for (line, start) in self.lines.iter().zip(&self.line_starts) {
            for (mut stop, mut position) in line.visual_carets(self.line_height) {
                stop.index += start;
                position.y += y;
                if boundaries.binary_search(&stop.index).is_ok() {
                    stops.push((stop, position));
                }
            }
            if let Some(range) = &collapse {
                let local = range.start.saturating_sub(*start).min(line.len())
                    ..range.end.saturating_sub(*start).min(line.len());
                for bounds in line.selection_bounds(local, self.line_height) {
                    selection_edges.push(point(
                        if right { bounds.right() } else { bounds.left() },
                        bounds.top() + y,
                    ));
                }
            }
            y += self.line_height * (line.wrap_boundaries().len() + 1);
        }
        let key = |position: Point<Pixels>| (position.y, position.x);
        let current = key(self.position_for_caret(caret.index, caret.affinity));
        let target_position = if collapse.is_some() && !selection_edges.is_empty() {
            selection_edges.into_iter().map(key).reduce(
                |a, b| {
                    if right { a.max(b) } else { a.min(b) }
                },
            )
        } else {
            stops
                .iter()
                .map(|(_, position)| key(*position))
                .filter(|position| {
                    if right {
                        *position > current
                    } else {
                        *position < current
                    }
                })
                .reduce(|a, b| if right { a.min(b) } else { a.max(b) })
        };
        Some(
            target_position
                .and_then(|position| {
                    stops
                        .into_iter()
                        .filter(|(_, p)| key(*p) == position)
                        .min_by_key(|(stop, _)| {
                            (
                                collapse.as_ref().is_some_and(|range| {
                                    stop.index < range.start || stop.index > range.end
                                }),
                                stop.index.abs_diff(caret.index),
                            )
                        })
                        .map(|(stop, _)| stop)
                })
                .unwrap_or(caret),
        )
    }

    fn bidi_deletion_target(
        &self,
        content: &str,
        caret: TextCaret,
        right: bool,
    ) -> Option<(Range<usize>, TextCaret)> {
        let (line_ix, index) = self.line_for_offset(caret.index);
        let line = self.lines.get(line_ix)?;
        let start = self.line_starts[line_ix];
        if !self.matches_content(content)
            || !line
                .runs()
                .iter()
                .any(|run| run.glyphs.iter().any(|glyph| glyph.is_rtl))
        {
            return None;
        }
        let key = |p: Point<Pixels>| (p.y, p.x);
        let position = line.position_for_caret(TextCaret { index, ..caret }, self.line_height)?;
        let edges = line.visual_carets(self.line_height);
        let cluster = edges
            .chunks_exact(2)
            .filter(|pair| {
                let left = key(pair[0].1).min(key(pair[1].1));
                let end = key(pair[0].1).max(key(pair[1].1));
                if right {
                    end > key(position)
                } else {
                    left < key(position)
                }
            })
            .reduce(|a, b| {
                let edge = |pair: &[(TextCaret, Point<Pixels>)]| {
                    if right {
                        key(pair[0].1).min(key(pair[1].1))
                    } else {
                        key(pair[0].1).max(key(pair[1].1))
                    }
                };
                if (right && edge(b) < edge(a)) || (!right && edge(b) > edge(a)) {
                    b
                } else {
                    a
                }
            });
        let Some(cluster) = cluster else {
            // Hard line breaks remain editable at the displayed line edges.
            let range = if !right && start > 0 && content.as_bytes().get(start - 1) == Some(&b'\n')
            {
                start - 1..start
            } else if right && content.as_bytes().get(start + line.len()) == Some(&b'\n') {
                start + line.len()..start + line.len() + 1
            } else {
                caret.index..caret.index
            };
            return Some((range.clone(), range.start.into()));
        };
        let rtl = cluster[0].1.x > cluster[1].1.x;
        let cluster_range = start + cluster[0].0.index..start + cluster[1].0.index;
        let mut graphemes = content.grapheme_indices(true).filter_map(|(index, text)| {
            let range = index..index + text.len();
            (range.start < cluster_range.end && cluster_range.start < range.end).then_some(range)
        });
        let range = if right != rtl {
            graphemes.next()
        } else {
            graphemes.next_back()
        }?;

        // Anchor to a surviving cluster, so deletion at a direction boundary
        // does not move the caret to the other visual representation of an index.
        let anchor = edges
            .chunks_exact(2)
            .filter(|pair| {
                let cluster = start + pair[0].0.index..start + pair[1].0.index;
                cluster.end <= range.start || cluster.start >= range.end
            })
            .flat_map(|pair| pair.iter())
            .min_by_key(|(stop, p)| {
                (
                    (p.y - position.y).abs(),
                    (p.x - position.x).abs(),
                    *stop != TextCaret { index, ..caret },
                )
            })
            .map(|(stop, _)| TextCaret {
                index: start + stop.index,
                ..*stop
            });
        let mut anchor = anchor.unwrap_or_else(|| range.start.into());
        if anchor.index >= range.end {
            anchor.index -= range.len();
        } else if anchor.index > range.start {
            anchor.index = range.start;
        }
        Some((range, anchor))
    }
}

pub struct TextInput {
    pub(super) focus_handle: FocusHandle,
    pub(super) content: SharedString,
    pub(super) committed_content: SharedString,
    pub(super) placeholder: SharedString,
    pub(super) selected_range: Range<usize>,
    pub(super) selection_reversed: bool,
    pub(super) caret_affinity: CaretAffinity,
    horizontal_selection_anchor: Option<TextCaret>,
    pub(super) marked_range: Option<Range<usize>>,
    pub(super) preedit_cursor_hidden: bool,
    pub(super) last_layout: Option<TextLayout>,
    pub(super) last_bounds: Option<Bounds<Pixels>>,
    pub(super) last_viewport_bounds: Option<Bounds<Pixels>>,
    pub(super) is_selecting: bool,
    selection_pointer: Option<Point<Pixels>>,
    selection_line_anchor: Option<Range<usize>>,
    selection_scroll_task: Option<Task<()>>,
    pub(super) disabled: bool,
    pub(super) mode: InputMode,
    pub(super) appearance: InputAppearance,
    pub(super) preferred_x: Option<Pixels>,
    pub(super) scroll_handle: ScrollHandle,
    pub(super) scrollbar_state: ScrollbarState,
    pub(super) scroll_cursor_pending: bool,
    pub(super) single_line_scroll_offset: Pixels,
}

impl gpui::EventEmitter<InputEvent> for TextInput {}

impl TextInput {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
            content: "".into(),
            committed_content: "".into(),
            placeholder: "".into(),
            selected_range: 0..0,
            selection_reversed: false,
            caret_affinity: CaretAffinity::Downstream,
            horizontal_selection_anchor: None,
            marked_range: None,
            preedit_cursor_hidden: false,
            last_layout: None,
            last_bounds: None,
            last_viewport_bounds: None,
            is_selecting: false,
            selection_pointer: None,
            selection_line_anchor: None,
            selection_scroll_task: None,
            disabled: false,
            mode: InputMode::Text,
            appearance: InputAppearance::default(),
            preferred_x: None,
            scroll_handle: ScrollHandle::new(),
            scrollbar_state: ScrollbarState::new(),
            scroll_cursor_pending: true,
            single_line_scroll_offset: px(0.),
        }
    }

    pub fn text(mut self) -> Self {
        self.mode = InputMode::Text;
        self
    }

    pub fn password(mut self) -> Self {
        self.mode = InputMode::Password;
        self
    }

    /// Enables multi-line editing with soft wrapping and newline insertion on Enter.
    pub fn multiline(mut self) -> Self {
        self.mode = InputMode::Multiline;
        self
    }

    pub fn mode(mut self, mode: InputMode) -> Self {
        self.mode = mode;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn set_disabled(&mut self, disabled: bool, cx: &mut Context<Self>) {
        self.disabled = disabled;
        self.stop_selection();
        self.marked_range = None;
        cx.notify();
    }

    pub fn is_disabled(&self) -> bool {
        self.disabled
    }

    pub fn set_mode(&mut self, mode: InputMode) {
        self.stop_selection();
        self.mode = mode;
        self.preferred_x = None;
        self.scroll_cursor_pending = true;
        self.single_line_scroll_offset = px(0.);
    }

    pub fn set_placeholder(&mut self, placeholder: impl Into<SharedString>) {
        self.placeholder = placeholder.into();
    }

    pub fn placeholder(mut self, placeholder: impl Into<SharedString>) -> Self {
        self.placeholder = placeholder.into();
        self
    }

    pub fn initial_value(mut self, value: impl Into<SharedString>) -> Self {
        self.content = value.into();
        self.committed_content = self.content.clone();
        self.selected_range = self.content.len()..self.content.len();
        self.selection_reversed = false;
        self.caret_affinity = CaretAffinity::Downstream;
        self.horizontal_selection_anchor = None;
        self
    }

    pub fn appearance(mut self, appearance: InputAppearance) -> Self {
        self.appearance = appearance;
        self
    }

    pub fn value(&self) -> SharedString {
        self.content.clone()
    }

    pub fn set_value(&mut self, value: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.stop_selection();
        self.content = value.into();
        self.committed_content = self.content.clone();
        self.selected_range = self.content.len()..self.content.len();
        self.selection_reversed = false;
        self.caret_affinity = CaretAffinity::Downstream;
        self.horizontal_selection_anchor = None;
        self.marked_range = None;
        self.scroll_cursor_pending = true;
        cx.emit(InputEvent::Change(self.content.clone()));
        cx.notify();
    }

    pub fn clear(&mut self, cx: &mut Context<Self>) {
        self.set_value("", cx);
    }

    pub fn set_appearance(&mut self, appearance: InputAppearance, cx: &mut Context<Self>) {
        self.appearance = appearance;
        cx.notify();
    }

    fn submit(&mut self, _: &Submit, _: &mut Window, cx: &mut Context<Self>) {
        if self.disabled {
            return;
        }
        cx.emit(InputEvent::Submit(self.content.clone()));
    }

    fn left(&mut self, _: &Left, _: &mut Window, cx: &mut Context<Self>) {
        self.move_horizontal(false, false, cx);
    }

    fn right(&mut self, _: &Right, _: &mut Window, cx: &mut Context<Self>) {
        self.move_horizontal(true, false, cx);
    }

    fn up(&mut self, _: &Up, _: &mut Window, cx: &mut Context<Self>) {
        self.move_vertical(-1., false, cx);
    }

    fn down(&mut self, _: &Down, _: &mut Window, cx: &mut Context<Self>) {
        self.move_vertical(1., false, cx);
    }

    fn select_left(&mut self, _: &SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.move_horizontal(false, true, cx);
    }

    fn select_right(&mut self, _: &SelectRight, _: &mut Window, cx: &mut Context<Self>) {
        self.move_horizontal(true, true, cx);
    }

    fn select_up(&mut self, _: &SelectUp, _: &mut Window, cx: &mut Context<Self>) {
        self.move_vertical(-1., true, cx);
    }

    fn select_down(&mut self, _: &SelectDown, _: &mut Window, cx: &mut Context<Self>) {
        self.move_vertical(1., true, cx);
    }

    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        if self.disabled {
            return;
        }
        self.move_to(0, cx);
        self.select_to(self.content.len(), cx)
    }

    fn home(&mut self, _: &Home, _: &mut Window, cx: &mut Context<Self>) {
        if self.disabled {
            return;
        }
        let offset = if self.mode == InputMode::Multiline {
            self.last_layout
                .as_ref()
                .map(|layout| layout.row_range_for_offset(self.cursor_offset()).start)
                .unwrap_or(0)
        } else {
            0
        };
        self.move_to(offset, cx);
    }

    fn end(&mut self, _: &End, _: &mut Window, cx: &mut Context<Self>) {
        if self.disabled {
            return;
        }
        let offset = if self.mode == InputMode::Multiline {
            self.last_layout
                .as_ref()
                .map(|layout| layout.row_range_for_offset(self.cursor_offset()).end)
                .unwrap_or(self.content.len())
        } else {
            self.content.len()
        };
        self.move_to(offset, cx);
    }

    fn backspace(&mut self, _: &Backspace, window: &mut Window, cx: &mut Context<Self>) {
        self.delete_horizontal(false, window, cx);
    }

    fn delete(&mut self, _: &Delete, window: &mut Window, cx: &mut Context<Self>) {
        self.delete_horizontal(true, window, cx);
    }

    fn delete_horizontal(&mut self, right: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.disabled {
            return;
        }
        let mut anchor = None;
        if self.selected_range.is_empty() {
            if self.marked_range.is_none()
                && self.mode != InputMode::Password
                && let Some(layout) = self.last_layout.as_mut()
                && layout.lines.iter().any(|line| {
                    line.runs()
                        .iter()
                        .any(|run| run.glyphs.iter().any(|glyph| glyph.is_rtl))
                })
            {
                layout.refresh_for_edit(&self.content, window.text_system());
            }
            let caret = TextCaret {
                index: self.cursor_offset(),
                affinity: self.caret_affinity,
            };
            let visual = self
                .marked_range
                .is_none()
                .then(|| {
                    self.last_layout
                        .as_ref()?
                        .bidi_deletion_target(&self.content, caret, right)
                })
                .flatten();
            let range = if let Some((range, target)) = visual {
                anchor = Some(target);
                range
            } else if right {
                caret.index..self.next_boundary(caret.index)
            } else {
                self.previous_boundary(caret.index)..caret.index
            };
            if range.is_empty() {
                window.play_system_bell();
                return;
            }
            self.selected_range = range;
            self.selection_reversed = false;
        }
        self.replace_text_in_range(None, "", window, cx);
        if let Some(anchor) = anchor {
            self.selected_range = anchor.index..anchor.index;
            self.caret_affinity = anchor.affinity;
        }
    }

    fn insert_newline(&mut self, _: &InsertNewline, window: &mut Window, cx: &mut Context<Self>) {
        if self.disabled || self.mode != InputMode::Multiline {
            return;
        }
        self.replace_text_in_range(None, "\n", window, cx);
        cx.stop_propagation();
    }

    fn on_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.disabled {
            return;
        }
        window.focus(&self.focus_handle, cx);
        self.stop_selection();
        self.is_selecting = true;

        let caret = self.caret_for_mouse_position(event.position);
        let offset = caret.index;
        if event.click_count >= 2 {
            let range = self.line_range_at(offset);
            self.selection_line_anchor = Some(range.clone());
            self.preferred_x = None;
            self.selected_range = range;
            self.selection_reversed = false;
            self.caret_affinity = CaretAffinity::Downstream;
            self.horizontal_selection_anchor = None;
            self.scroll_cursor_pending = true;
            cx.notify();
        } else if event.modifiers.shift {
            self.preferred_x = None;
            self.select_to(offset, cx);
        } else {
            self.move_to(offset, cx)
        }
        if event.click_count < 2 {
            self.caret_affinity = caret.affinity;
        }
    }

    fn stop_selection(&mut self) {
        self.is_selecting = false;
        self.selection_pointer = None;
        self.selection_line_anchor = None;
        self.selection_scroll_task = None;
    }

    fn on_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.disabled || !event.dragging() || !self.focus_handle.is_focused(window) {
            self.stop_selection();
            return;
        }
        if !self.is_selecting {
            return;
        }
        self.preferred_x = None;
        self.selection_pointer = Some(event.position);
        self.select_at_drag_pointer(cx);
        if self.selection_scroll_delta() == px(0.) {
            self.selection_scroll_task = None;
        } else if self.selection_scroll_task.is_none() {
            self.selection_scroll_task = Some(cx.spawn_in(window, async move |input, cx| {
                loop {
                    cx.background_executor()
                        .timer(Duration::from_millis(16))
                        .await;
                    let keep_scrolling = input
                        .update_in(cx, |input, window, cx| {
                            if input.disabled
                                || !input.is_selecting
                                || !input.focus_handle.is_focused(window)
                            {
                                input.stop_selection();
                                return false;
                            }
                            if !input.scroll_selection(cx) {
                                input.selection_scroll_task = None;
                                return false;
                            }
                            true
                        })
                        .unwrap_or(false);
                    if !keep_scrolling {
                        break;
                    }
                }
            }));
        }
    }

    fn select_at_drag_pointer(&mut self, cx: &mut Context<Self>) {
        let Some(mut position) = self.selection_pointer else {
            return;
        };
        if self.mode != InputMode::Multiline
            && let Some(viewport) = self.last_viewport_bounds
        {
            position.x = position.x.clamp(viewport.left(), viewport.right());
        }
        let caret = self.caret_for_mouse_position(position);
        let offset = caret.index;
        if let Some(anchor) = &self.selection_line_anchor {
            let range = self.line_range_at(offset);
            self.selection_reversed = range.start < anchor.start;
            self.selected_range = range.start.min(anchor.start)..range.end.max(anchor.end);
            self.scroll_cursor_pending = true;
            cx.notify();
        } else {
            self.select_to(offset, cx);
            self.caret_affinity = caret.affinity;
        }
    }

    fn line_range_at(&self, offset: usize) -> Range<usize> {
        if self.mode != InputMode::Multiline {
            return 0..self.content.len();
        }
        let mut offset = offset.min(self.content.len());
        while !self.content.is_char_boundary(offset) {
            offset -= 1;
        }
        let start = self.content[..offset]
            .rfind('\n')
            .map_or(0, |index| index + 1);
        let end = self.content[offset..]
            .find('\n')
            .map_or(self.content.len(), |index| offset + index);
        start..end
    }

    fn selection_scroll_delta(&self) -> Pixels {
        if self.mode == InputMode::Multiline {
            return px(0.);
        }
        let (Some(position), Some(viewport)) = (self.selection_pointer, self.last_viewport_bounds)
        else {
            return px(0.);
        };
        // A small edge zone also scrolls when the pointer is held just inside the field.
        let edge = px(8.).min(viewport.size.width / 2.);
        let distance = if position.x < viewport.left() + edge {
            viewport.left() + edge - position.x
        } else if position.x > viewport.right() - edge {
            viewport.right() - edge - position.x
        } else {
            return px(0.);
        };
        (distance * 0.35).clamp(px(-24.), px(24.))
    }

    fn scroll_selection(&mut self, cx: &mut Context<Self>) -> bool {
        let (Some(bounds), Some(viewport)) = (self.last_bounds, self.last_viewport_bounds) else {
            return false;
        };
        let max_scroll = (bounds.size.width - viewport.size.width).max(px(0.));
        let offset = (self.single_line_scroll_offset + self.selection_scroll_delta())
            .clamp(-max_scroll, px(0.));
        let change = offset - self.single_line_scroll_offset;
        if change == px(0.) {
            return false;
        }
        self.single_line_scroll_offset = offset;
        // Keep hit testing synchronized even if multiple timer ticks precede paint.
        self.last_bounds.as_mut().unwrap().origin.x += change;
        self.select_at_drag_pointer(cx);
        self.scroll_cursor_pending = false;
        true
    }

    fn show_character_palette(
        &mut self,
        _: &ShowCharacterPalette,
        window: &mut Window,
        _: &mut Context<Self>,
    ) {
        if self.disabled {
            return;
        }
        window.show_character_palette();
    }

    fn paste(&mut self, _: &Paste, window: &mut Window, cx: &mut Context<Self>) {
        if self.disabled {
            return;
        }
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            self.replace_text_in_range(None, &text, window, cx);
        }
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        if self.disabled || self.mode == InputMode::Password {
            return;
        }
        if !self.selected_range.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(
                self.content[self.selected_range.clone()].to_string(),
            ));
        }
    }
    fn cut(&mut self, _: &Cut, window: &mut Window, cx: &mut Context<Self>) {
        if self.disabled || self.mode == InputMode::Password {
            return;
        }
        if !self.selected_range.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(
                self.content[self.selected_range.clone()].to_string(),
            ));
            self.replace_text_in_range(None, "", window, cx)
        }
    }

    fn move_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.selected_range = offset..offset;
        self.selection_reversed = false;
        self.caret_affinity = CaretAffinity::Downstream;
        self.horizontal_selection_anchor = None;
        self.preferred_x = None;
        self.scroll_cursor_pending = true;
        cx.notify()
    }

    fn move_horizontal(&mut self, right: bool, selecting: bool, cx: &mut Context<Self>) {
        if self.disabled {
            return;
        }
        let caret = TextCaret {
            index: self.cursor_offset(),
            affinity: self.caret_affinity,
        };
        let anchor = self.horizontal_selection_anchor.unwrap_or_else(|| {
            if self.selected_range.is_empty() {
                caret
            } else {
                (if self.selection_reversed {
                    self.selected_range.end
                } else {
                    self.selected_range.start
                })
                .into()
            }
        });
        let collapse =
            (!selecting && !self.selected_range.is_empty()).then(|| self.selected_range.clone());
        let visual_target = self.last_layout.as_ref().and_then(|layout| {
            layout.bidi_horizontal_target(&self.content, caret, right, collapse.clone())
        });
        let mut target = visual_target.unwrap_or_else(|| {
            if let Some(range) = collapse {
                (if right { range.end } else { range.start }).into()
            } else if right {
                self.next_boundary(caret.index).into()
            } else {
                self.previous_boundary(caret.index).into()
            }
        });
        if selecting {
            if visual_target.is_some()
                && let Some(layout) = &self.last_layout
                && layout.position_for_caret(target.index, target.affinity)
                    == layout.position_for_caret(anchor.index, anchor.affinity)
            {
                target = anchor;
            }
            self.select_to(target.index, cx);
            self.horizontal_selection_anchor = Some(anchor);
        } else {
            self.move_to(target.index, cx);
        }
        self.caret_affinity = target.affinity;
        self.preferred_x = None;
    }

    fn move_vertical(&mut self, rows: f32, selecting: bool, cx: &mut Context<Self>) {
        self.horizontal_selection_anchor = None;
        if self.disabled || self.mode != InputMode::Multiline {
            return;
        }
        let Some(layout) = self.last_layout.as_ref() else {
            return;
        };
        let cursor = self.cursor_offset();
        let position = layout.position_for_caret(cursor, self.caret_affinity);
        let preferred_x = self.preferred_x.unwrap_or(position.x);
        let target =
            layout.caret_for_position(point(preferred_x, position.y + layout.line_height * rows));
        self.preferred_x = Some(preferred_x);
        if selecting {
            self.select_to(target.index, cx);
        } else {
            self.selected_range = target.index..target.index;
            self.selection_reversed = false;
            self.caret_affinity = CaretAffinity::Downstream;
            self.horizontal_selection_anchor = None;
            self.scroll_cursor_pending = true;
            cx.notify();
        }
        self.caret_affinity = target.affinity;
    }

    pub(super) fn cursor_offset(&self) -> usize {
        if self.selection_reversed {
            self.selected_range.start
        } else {
            self.selected_range.end
        }
    }

    #[cfg(test)]
    fn index_for_mouse_position(&self, position: Point<Pixels>) -> usize {
        self.caret_for_mouse_position(position).index
    }

    fn caret_for_mouse_position(&self, position: Point<Pixels>) -> TextCaret {
        if self.content.is_empty() {
            return 0.into();
        }

        let (Some(bounds), Some(line)) = (self.last_bounds.as_ref(), self.last_layout.as_ref())
        else {
            return 0.into();
        };
        if self.mode != InputMode::Multiline {
            return line.caret_for_position(point(position.x - bounds.left(), px(0.)));
        }
        if position.y < bounds.top() {
            return 0.into();
        }
        if position.y > bounds.bottom() {
            return self.content.len().into();
        }
        line.caret_for_position(point(position.x - bounds.left(), position.y - bounds.top()))
    }

    fn select_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.caret_affinity = CaretAffinity::Downstream;
        self.horizontal_selection_anchor = None;
        if self.selection_reversed {
            self.selected_range.start = offset
        } else {
            self.selected_range.end = offset
        };
        if self.selected_range.end < self.selected_range.start {
            self.selection_reversed = !self.selection_reversed;
            self.selected_range = self.selected_range.end..self.selected_range.start;
        }
        self.scroll_cursor_pending = true;
        cx.notify()
    }

    fn offset_from_utf16(&self, offset: usize) -> usize {
        let mut utf8_offset = 0;
        let mut utf16_count = 0;

        for ch in self.content.chars() {
            if utf16_count >= offset {
                break;
            }
            utf16_count += ch.len_utf16();
            utf8_offset += ch.len_utf8();
        }

        utf8_offset
    }

    fn offset_to_utf16(&self, offset: usize) -> usize {
        let mut utf16_offset = 0;
        let mut utf8_count = 0;

        for ch in self.content.chars() {
            if utf8_count >= offset {
                break;
            }
            utf8_count += ch.len_utf8();
            utf16_offset += ch.len_utf16();
        }

        utf16_offset
    }

    fn range_to_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_to_utf16(range.start)..self.offset_to_utf16(range.end)
    }

    fn range_from_utf16(&self, range_utf16: &Range<usize>) -> Range<usize> {
        self.offset_from_utf16(range_utf16.start)..self.offset_from_utf16(range_utf16.end)
    }

    fn previous_boundary(&self, offset: usize) -> usize {
        self.content
            .grapheme_indices(true)
            .rev()
            .find_map(|(idx, _)| (idx < offset).then_some(idx))
            .unwrap_or(0)
    }

    fn next_boundary(&self, offset: usize) -> usize {
        self.content
            .grapheme_indices(true)
            .find_map(|(idx, _)| (idx > offset).then_some(idx))
            .unwrap_or(self.content.len())
    }

    fn normalize_inserted_text<'a>(&self, text: &'a str) -> Cow<'a, str> {
        if !text.contains(['\r', '\n']) {
            return Cow::Borrowed(text);
        }
        let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
        if self.mode == InputMode::Multiline {
            Cow::Owned(normalized)
        } else {
            Cow::Owned(normalized.replace('\n', " "))
        }
    }

    fn emit_committed_change(&mut self, cx: &mut Context<Self>) {
        if self.content == self.committed_content {
            return;
        }
        self.committed_content = self.content.clone();
        cx.emit(InputEvent::Change(self.content.clone()));
    }

    pub fn reset(&mut self, cx: &mut Context<Self>) {
        self.content = "".into();
        self.committed_content = "".into();
        self.selected_range = 0..0;
        self.selection_reversed = false;
        self.caret_affinity = CaretAffinity::Downstream;
        self.horizontal_selection_anchor = None;
        self.marked_range = None;
        self.last_layout = None;
        self.last_bounds = None;
        self.last_viewport_bounds = None;
        self.stop_selection();
        self.preferred_x = None;
        self.scroll_cursor_pending = true;
        self.single_line_scroll_offset = px(0.);
        cx.emit(InputEvent::Change(self.content.clone()));
        cx.notify();
    }
}

impl EntityInputHandler for TextInput {
    fn surrounding_text(
        &mut self,
        max_bytes: usize,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<gpui::SurroundingText> {
        if self.disabled || self.mode == InputMode::Password {
            return None;
        }
        let anchor = if self.selection_reversed {
            self.selected_range.end
        } else {
            self.selected_range.start
        };
        gpui::SurroundingText::from_utf8(
            &self.content,
            self.cursor_offset(),
            anchor,
            self.marked_range.clone(),
            max_bytes,
        )
    }

    fn delete_surrounding_text(
        &mut self,
        before_utf16: usize,
        after_utf16: usize,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.disabled || self.mode == InputMode::Password || self.marked_range.is_some() {
            return false;
        }
        let selection = self.range_to_utf16(&self.selected_range);
        let Some(start) = selection.start.checked_sub(before_utf16) else {
            return false;
        };
        let Some(end) = selection.end.checked_add(after_utf16) else {
            return false;
        };
        let start_byte = self.offset_from_utf16(start);
        let end_byte = self.offset_from_utf16(end);
        if self.offset_to_utf16(start_byte) != start || self.offset_to_utf16(end_byte) != end {
            return false;
        }
        let selected_len = self.selected_range.len();
        self.stop_selection();
        self.content = format!(
            "{}{}{}",
            &self.content[..start_byte],
            &self.content[self.selected_range.clone()],
            &self.content[end_byte..]
        )
        .into();
        self.selected_range = start_byte..start_byte + selected_len;
        self.preferred_x = None;
        self.scroll_cursor_pending = true;
        self.emit_committed_change(cx);
        cx.notify();
        true
    }

    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        actual_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let range = self.range_from_utf16(&range_utf16);
        actual_range.replace(self.range_to_utf16(&range));
        Some(self.content[range].to_string())
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.range_to_utf16(&self.selected_range),
            reversed: self.selection_reversed,
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        self.marked_range
            .as_ref()
            .map(|range| self.range_to_utf16(range))
    }

    fn unmark_text(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let had_marked_text = self.marked_range.take().is_some();
        if had_marked_text {
            self.emit_committed_change(cx);
            cx.notify();
        }
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.disabled {
            return;
        }
        self.stop_selection();
        let new_text = self.normalize_inserted_text(new_text);
        let new_text = new_text.as_ref();
        let range = range_utf16
            .as_ref()
            .map(|range_utf16| self.range_from_utf16(range_utf16))
            .or(self.marked_range.clone())
            .unwrap_or(self.selected_range.clone());

        self.content =
            (self.content[0..range.start].to_owned() + new_text + &self.content[range.end..])
                .into();
        self.selected_range = range.start + new_text.len()..range.start + new_text.len();
        self.selection_reversed = false;
        self.caret_affinity = CaretAffinity::Downstream;
        self.horizontal_selection_anchor = None;
        self.marked_range.take();
        self.preferred_x = None;
        self.scroll_cursor_pending = true;
        self.emit_committed_change(cx);
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.disabled {
            return;
        }
        self.stop_selection();
        let selected_offsets = new_selected_range_utf16.map(|range| {
            let offset = |utf16| {
                let end = preedit_offset_from_utf16(new_text, utf16);
                self.normalize_inserted_text(&new_text[..end]).len()
            };
            offset(range.start)..offset(range.end)
        });
        let new_text = self.normalize_inserted_text(new_text);
        let new_text = new_text.as_ref();
        let range = range_utf16
            .as_ref()
            .map(|range_utf16| self.range_from_utf16(range_utf16))
            .or(self.marked_range.clone())
            .unwrap_or(self.selected_range.clone());

        self.content =
            (self.content[0..range.start].to_owned() + new_text + &self.content[range.end..])
                .into();
        if !new_text.is_empty() {
            self.marked_range = Some(range.start..range.start + new_text.len());
        } else {
            self.marked_range = None;
        }
        self.selected_range = selected_offsets
            .map(|new_range| {
                new_range.start.min(new_range.end) + range.start
                    ..new_range.end.max(new_range.start) + range.start
            })
            .unwrap_or_else(|| range.start + new_text.len()..range.start + new_text.len());
        self.selection_reversed = false;
        self.caret_affinity = CaretAffinity::Downstream;
        self.horizontal_selection_anchor = None;
        self.preedit_cursor_hidden = false;
        self.preferred_x = None;
        self.scroll_cursor_pending = true;

        cx.notify();
    }

    fn replace_and_mark_text_with_selection(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        selection: PreeditSelection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.disabled {
            return;
        }
        self.replace_and_mark_text_in_range(range_utf16, new_text, selection.range(), window, cx);
        self.selection_reversed =
            matches!(selection, PreeditSelection::Range { anchor, head } if head < anchor);
        self.preedit_cursor_hidden = selection == PreeditSelection::Hidden;
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let last_layout = self.last_layout.as_ref()?;
        let bounds = if self.mode == InputMode::Multiline {
            bounds
        } else {
            self.last_bounds.unwrap_or(bounds)
        };
        let range = self.range_from_utf16(&range_utf16);
        if range.is_empty() {
            let affinity = if range.start == self.cursor_offset() {
                self.caret_affinity
            } else {
                CaretAffinity::Downstream
            };
            let position = last_layout.position_for_caret(range.start, affinity);
            return Some(Bounds::new(
                bounds.origin + position,
                gpui::size(px(0.), last_layout.line_height),
            ));
        }
        let mut selected: Option<Bounds<Pixels>> = None;
        let mut rows_before = 0;
        for (ix, line) in last_layout.lines.iter().enumerate() {
            let start = last_layout.line_starts[ix];
            let local =
                range.start.saturating_sub(start)..range.end.saturating_sub(start).min(line.len());
            for mut rect in line.selection_bounds(local, last_layout.line_height) {
                rect.origin += bounds.origin + point(px(0.), last_layout.line_height * rows_before);
                selected = Some(selected.map_or(rect, |old| old.union(&rect)));
            }
            rows_before += line.wrap_boundaries().len() + 1;
        }
        selected.or_else(|| {
            Some(Bounds::new(
                bounds.origin + last_layout.position_for_offset(range.start),
                gpui::size(px(0.), last_layout.line_height),
            ))
        })
    }

    fn character_index_for_point(
        &mut self,
        point: gpui::Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        let bounds = self.last_bounds?;
        let line_point = bounds.localize(&point)?;
        let last_layout = self.last_layout.as_ref()?;

        let utf8_index = last_layout.offset_for_position(line_point);
        Some(self.offset_to_utf16(utf8_index))
    }
}

impl Render for TextInput {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let multiline = self.mode == InputMode::Multiline;
        let scroll_handle = self.scroll_handle.clone();
        let input = cx.weak_entity();
        div()
            .on_paint_before_children(move |_, _, window, _| {
                let moving_input = input.clone();
                window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
                    if phase == DispatchPhase::Capture
                        && let Some(input) = moving_input.upgrade()
                        && input.read(cx).is_selecting
                    {
                        input.update(cx, |input, cx| input.on_mouse_move(event, window, cx));
                        cx.stop_propagation();
                    }
                });
                let input = input.clone();
                window.on_mouse_event(move |event: &MouseUpEvent, phase, _, cx| {
                    if phase == DispatchPhase::Capture
                        && event.button == MouseButton::Left
                        && let Some(input) = input.upgrade()
                        && input.read(cx).is_selecting
                    {
                        input.update(cx, |input, _| input.stop_selection());
                    }
                });
            })
            .id(("uic-text-input", cx.entity_id()))
            .flex()
            .w_full()
            .min_w_0()
            .when(multiline, |this| {
                this.h_full()
                    .flex_col()
                    .overflow_scroll()
                    .track_scroll(&scroll_handle)
            })
            .when(!multiline, |this| this.overflow_hidden())
            .key_context(if multiline {
                "TextInput multiline"
            } else {
                "TextInput"
            })
            .track_focus(&self.focus_handle(cx))
            .cursor(if self.disabled {
                CursorStyle::Arrow
            } else {
                CursorStyle::IBeam
            })
            .on_action(cx.listener(Self::backspace))
            .on_action(cx.listener(Self::delete))
            .on_action(cx.listener(Self::left))
            .on_action(cx.listener(Self::right))
            .on_action(cx.listener(Self::up))
            .on_action(cx.listener(Self::down))
            .on_action(cx.listener(Self::select_left))
            .on_action(cx.listener(Self::select_right))
            .on_action(cx.listener(Self::select_up))
            .on_action(cx.listener(Self::select_down))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::home))
            .on_action(cx.listener(Self::end))
            .on_action(cx.listener(Self::show_character_palette))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::insert_newline))
            .on_action(cx.listener(Self::submit))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .child(
                div()
                    .w_full()
                    .min_w_0()
                    .when(multiline, |this| this.flex_none())
                    .child(TextElement { input: cx.entity() }),
            )
    }
}

impl Focusable for TextInput {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, rc::Rc};

    use gpui::{
        Context, Entity, EntityInputHandler, IntoElement, Render, SharedString, Subscription,
        TestAppContext, VisualTestContext, Window, px, size,
    };

    use super::*;
    use crate::components::input::Input;

    struct BidiText;

    impl gpui::PlatformTextSystem for BidiText {
        fn add_fonts(&self, fonts: Vec<Cow<'static, [u8]>>) -> gpui::Result<()> {
            gpui::NoopTextSystem.add_fonts(fonts)
        }
        fn all_font_names(&self) -> Vec<String> {
            gpui::NoopTextSystem.all_font_names()
        }
        fn font_id(&self, font: &gpui::Font) -> gpui::Result<gpui::FontId> {
            gpui::NoopTextSystem.font_id(font)
        }
        fn font_metrics(&self, id: gpui::FontId) -> gpui::FontMetrics {
            gpui::NoopTextSystem.font_metrics(id)
        }
        fn typographic_bounds(
            &self,
            font: gpui::FontId,
            glyph: gpui::GlyphId,
        ) -> gpui::Result<Bounds<f32>> {
            gpui::NoopTextSystem.typographic_bounds(font, glyph)
        }
        fn advance(
            &self,
            font: gpui::FontId,
            glyph: gpui::GlyphId,
        ) -> gpui::Result<gpui::Size<f32>> {
            gpui::NoopTextSystem.advance(font, glyph)
        }
        fn glyph_for_char(&self, font: gpui::FontId, ch: char) -> Option<gpui::GlyphId> {
            gpui::NoopTextSystem.glyph_for_char(font, ch)
        }
        fn glyph_raster_bounds(
            &self,
            params: &gpui::RenderGlyphParams,
        ) -> gpui::Result<Bounds<gpui::DevicePixels>> {
            gpui::NoopTextSystem.glyph_raster_bounds(params)
        }
        fn rasterize_glyph(
            &self,
            params: &gpui::RenderGlyphParams,
            bounds: Bounds<gpui::DevicePixels>,
        ) -> gpui::Result<(gpui::Size<gpui::DevicePixels>, Vec<u8>)> {
            gpui::NoopTextSystem.rasterize_glyph(params, bounds)
        }
        fn recommended_rendering_mode(
            &self,
            _: gpui::FontId,
            _: Pixels,
        ) -> gpui::TextRenderingMode {
            gpui::TextRenderingMode::Grayscale
        }
        fn layout_line(
            &self,
            text: &str,
            font_size: Pixels,
            _: &[gpui::FontRun],
        ) -> gpui::LineLayout {
            let glyphs: &[(usize, usize, bool)] = match text {
                "aאב12" => &[
                    (0, 1, false),
                    (5, 6, false),
                    (6, 7, false),
                    (3, 5, true),
                    (1, 3, true),
                ],
                "אב" => &[(2, 4, true), (0, 2, true)],
                "ב" => &[(0, 2, true)],
                "א\u{5b0}ב" => &[(4, 6, true), (0, 4, true)],
                "אב😀" => &[(4, 8, true), (2, 4, true), (0, 2, true)],
                "" => &[],
                _ => panic!("unexpected test text: {text}"),
            };
            gpui::LineLayout {
                font_size,
                width: px(glyphs.len() as f32 * 10.),
                ascent: px(12.),
                descent: px(4.),
                len: text.len(),
                runs: vec![gpui::ShapedRun {
                    font_id: gpui::FontId(0),
                    glyphs: glyphs
                        .iter()
                        .copied()
                        .enumerate()
                        .map(|(visual, (index, cluster_end, is_rtl))| gpui::ShapedGlyph {
                            id: gpui::GlyphId(0),
                            position: point(px(visual as f32 * 10.), px(0.)),
                            index,
                            cluster_end,
                            advance: px(10.),
                            is_rtl,
                            is_emoji: false,
                        })
                        .collect(),
                }],
            }
        }
    }

    fn bidi_text_layout(text: &str, wrap_width: Option<Pixels>) -> TextLayout {
        use std::sync::Arc;
        let shaper =
            gpui::WindowTextSystem::new(Arc::new(gpui::TextSystem::new(Arc::new(BidiText))));
        let lines = shaper
            .shape_text(
                text.to_owned().into(),
                px(16.),
                &[gpui::TextRun {
                    len: text.len(),
                    ..Default::default()
                }],
                wrap_width,
                None,
            )
            .unwrap()
            .into_vec();
        let starts = std::iter::once(0)
            .chain(text.match_indices('\n').map(|(i, _)| i + 1))
            .collect();
        TextLayout::new(lines, starts, px(20.))
    }

    #[gpui::test]
    fn bidi_deletion_removes_only_the_visual_neighbor(cx: &mut TestAppContext) {
        let window = open_input(cx, TextInput::new);
        let mut visual = draw_and_focus(&window, cx);
        window
            .update(&mut visual.cx, |view, window, cx| {
                view.state.update(cx, |input, cx| {
                    for (text, index, affinity, right, expected) in [
                        ("אב", 0, CaretAffinity::Downstream, false, "ב"),
                        ("אב", 4, CaretAffinity::Upstream, true, "א"),
                        ("aאב12", 1, CaretAffinity::Downstream, false, "aב12"),
                        ("aאב12", 1, CaretAffinity::Upstream, true, "aאב2"),
                        ("aאב12", 7, CaretAffinity::Upstream, false, "aאב1"),
                        ("aאב12", 5, CaretAffinity::Upstream, true, "aא12"),
                        ("aאב12", 1, CaretAffinity::Upstream, false, "אב12"),
                        ("א\u{5b0}ב", 0, CaretAffinity::Downstream, false, "ב"),
                        ("אב😀", 4, CaretAffinity::Upstream, false, "אב"),
                    ] {
                        input.set_value(text, cx);
                        input.last_layout = Some(bidi_text_layout(text, None));
                        input.move_to(index, cx);
                        input.caret_affinity = affinity;
                        if right {
                            input.delete(&Delete, window, cx);
                        } else {
                            input.backspace(&Backspace, window, cx);
                        }
                        assert_eq!(input.content.as_ref(), expected, "{text}, {index}, {right}");
                        assert!(input.selected_range.is_empty());
                        assert!(input.content.is_char_boundary(input.cursor_offset()));
                        if text == "aאב12"
                            && index == 1
                            && affinity == CaretAffinity::Upstream
                            && !right
                        {
                            assert_eq!(input.cursor_offset(), 4);
                            assert_eq!(input.caret_affinity, CaretAffinity::Downstream);
                        }
                    }
                    input.set_value("aאב12", cx);
                    input.last_layout = Some(bidi_text_layout("aאב12", None));
                    input.move_to(1, cx);
                    input.select_to(5, cx);
                    input.backspace(&Backspace, window, cx);
                    assert_eq!(input.content.as_ref(), "a12");
                    for (right, index) in [(false, 0), (true, 1)] {
                        input.set_value("aאב12", cx);
                        input.last_layout = Some(bidi_text_layout("aאב12", None));
                        input.move_to(index, cx);
                        input.delete_horizontal(right, window, cx);
                        assert_eq!(input.content.as_ref(), "aאב12");
                    }
                    input.disabled = true;
                    input.move_to(1, cx);
                    input.backspace(&Backspace, window, cx);
                    assert_eq!(input.content.as_ref(), "aאב12");
                });
            })
            .unwrap();
    }

    #[test]
    fn bidi_repeated_backspace_refreshes_changed_text_before_next_frame() {
        use std::sync::Arc;
        let shaper =
            gpui::WindowTextSystem::new(Arc::new(gpui::TextSystem::new(Arc::new(BidiText))));
        let mut content: SharedString = "אב".into();
        let mut layout = bidi_text_layout(&content, None);
        layout.shaping_run = Some(gpui::TextRun {
            len: content.len(),
            ..Default::default()
        });
        let mut caret = TextCaret::default();
        for expected in ["ב", ""] {
            layout.refresh_for_edit(&content, &shaper);
            let (range, next) = layout.bidi_deletion_target(&content, caret, false).unwrap();
            let mut text = content.to_string();
            text.replace_range(range, "");
            content = text.into();
            caret = next;
            assert_eq!(content.as_ref(), expected);
            assert_eq!(caret.index, 0);
        }
    }

    #[test]
    fn bidi_continuous_backspace_with_real_shaping() {
        use gpui::PlatformTextSystem;
        use std::sync::Arc;
        let system = gpui_wgpu::CosmicTextSystem::new_without_system_fonts("Lilex");
        system
            .add_fonts(vec![Cow::Borrowed(include_bytes!(
                "../../../../assets/fonts/lilex/Lilex-Regular.ttf"
            ))])
            .unwrap();
        let shaper = gpui::WindowTextSystem::new(Arc::new(gpui::TextSystem::new(Arc::new(system))));
        let mut content: SharedString = "English אבג 123 العربية 中文".into();
        let run = gpui::TextRun {
            len: content.len(),
            font: gpui::font("Lilex"),
            ..Default::default()
        };
        let lines = shaper
            .shape_text(content.clone(), px(20.), &[run.clone()], None, None)
            .unwrap()
            .into_vec();
        let mut layout = TextLayout::new(lines, vec![0], px(24.));
        layout.shaping_run = Some(run);
        let mut caret: TextCaret = content.find("中文").unwrap().into();
        let mut deleted = String::new();
        let mut reordered = false;
        for _ in 0..30 {
            layout.refresh_for_edit(&content, &shaper);
            if content.contains("العربية") {
                let left = |word: &str| {
                    let start = content.find(word).unwrap();
                    layout.lines[0]
                        .selection_bounds(start..start + word.len(), layout.line_height)
                        .iter()
                        .map(|bounds| bounds.left())
                        .min()
                        .unwrap()
                };
                if content.contains(['א', 'ב', 'ג']) {
                    assert!(left("العربية") < left("123"));
                } else {
                    assert!(left("123") < left("العربية"));
                    reordered = true;
                }
            }
            let (range, next) = layout
                .bidi_deletion_target(&content, caret, false)
                .unwrap_or_else(|| {
                    let prev = content
                        .grapheme_indices(true)
                        .rev()
                        .find_map(|(i, _)| (i < caret.index).then_some(i))
                        .unwrap_or(0);
                    (prev..caret.index, prev.into())
                });
            if range.is_empty() {
                break;
            }
            assert_eq!(content[range.clone()].graphemes(true).count(), 1);
            deleted.push_str(&content[range.clone()]);
            let mut text = content.to_string();
            text.replace_range(range, "");
            content = text.into();
            caret = next;
            assert_eq!(&content[caret.index..], "中文");
        }
        assert!(reordered);
        assert_eq!(deleted, " אבגالعربية 321  hsilgnE");
        assert_eq!(content.as_ref(), "中文");
    }

    #[test]
    fn bidi_arrows_follow_visual_rows_and_stop_at_edges() {
        for (text, wrap_width) in [
            ("אב", None),
            ("aאב12", None),
            ("aאב12", Some(px(30.))),
            ("aאב12\naאב12", None),
        ] {
            let layout = bidi_text_layout(text, wrap_width);
            let mut caret = layout.caret_for_position(Point::default());
            let mut positions = vec![layout.position_for_caret(caret.index, caret.affinity)];
            for _ in 0..30 {
                let next = layout
                    .bidi_horizontal_target(text, caret, true, None)
                    .unwrap();
                if next == caret {
                    break;
                }
                let position = layout.position_for_caret(next.index, next.affinity);
                let previous = positions.last().unwrap();
                assert!((position.y, position.x) > (previous.y, previous.x));
                positions.push(position);
                caret = next;
            }
            assert!(positions.len() >= if text == "אב" { 3 } else { 6 });
            if wrap_width.is_some() || text.contains('\n') {
                assert!(positions.last().unwrap().y > px(0.));
            }
            assert_eq!(
                layout.bidi_horizontal_target(text, caret, true, None),
                Some(caret)
            );
            for expected in positions.iter().rev().skip(1) {
                caret = layout
                    .bidi_horizontal_target(text, caret, false, None)
                    .unwrap();
                assert_eq!(
                    layout.position_for_caret(caret.index, caret.affinity),
                    *expected
                );
            }
            assert_eq!(
                layout.bidi_horizontal_target(text, caret, false, None),
                Some(caret)
            );
            assert!(
                layout
                    .bidi_horizontal_target("changed", caret, true, None)
                    .is_none()
            );
            assert!(
                layout
                    .bidi_horizontal_target(&format!("{text}z"), caret, true, None)
                    .is_none()
            );
        }
    }

    #[gpui::test]
    fn horizontal_arrows_preserve_ltr_grapheme_steps(cx: &mut TestAppContext) {
        let window = open_input(cx, |cx| TextInput::new(cx).initial_value("a\u{301}😀中文"));
        let mut visual = draw_and_focus(&window, cx);
        window
            .update(&mut visual.cx, |view, window, cx| {
                view.state.update(cx, |input, cx| {
                    input.move_to(0, cx);
                    for end in [3, 7, 10, 13, 13] {
                        input.right(&Right, window, cx);
                        assert_eq!(input.selected_range, end..end);
                    }
                    for start in [10, 7, 3, 0, 0] {
                        input.select_left(&SelectLeft, window, cx);
                        assert_eq!(input.selected_range, start..13);
                    }
                    for start in [3, 7, 10, 13] {
                        input.select_right(&SelectRight, window, cx);
                        assert_eq!(input.selected_range, start..13);
                    }
                });
            })
            .unwrap();
    }

    #[gpui::test]
    fn bidi_keyboard_selection_preserves_anchor_and_collapses_visually(cx: &mut TestAppContext) {
        let window = open_input(cx, |cx| TextInput::new(cx).initial_value("aאב12"));
        let mut visual = draw_and_focus(&window, cx);
        window
            .update(&mut visual.cx, |view, window, cx| {
                view.state.update(cx, |input, cx| {
                    input.last_layout = Some(bidi_text_layout("aאב12", None));
                    let caret_x = |input: &TextInput| {
                        input
                            .last_layout
                            .as_ref()
                            .unwrap()
                            .position_for_caret(input.cursor_offset(), input.caret_affinity)
                            .x
                    };
                    for (start, affinity, xs) in [
                        (1, CaretAffinity::Downstream, [40., 30., 40., 50.]),
                        (1, CaretAffinity::Upstream, [0., 0., 10., 20.]),
                    ] {
                        input.move_to(start, cx);
                        input.caret_affinity = affinity;
                        input.select_left(&SelectLeft, window, cx);
                        assert_eq!(caret_x(input), px(xs[0]));
                        input.select_left(&SelectLeft, window, cx);
                        assert_eq!(caret_x(input), px(xs[1]));
                        input.select_right(&SelectRight, window, cx);
                        assert_eq!(caret_x(input), px(xs[2]));
                        input.select_right(&SelectRight, window, cx);
                        assert_eq!(caret_x(input), px(xs[3]));
                    }
                    // Both visual representations of the same logical anchor must
                    // survive extending and retracting through a direction boundary.
                    for affinity in [CaretAffinity::Upstream, CaretAffinity::Downstream] {
                        input.move_to(1, cx);
                        input.caret_affinity = affinity;
                        let right = affinity == CaretAffinity::Upstream;
                        input.move_horizontal(right, true, cx);
                        input.move_horizontal(!right, true, cx);
                        assert_eq!(input.selected_range, 1..1);
                        assert_eq!(input.caret_affinity, affinity);
                    }
                    for right in [false, true] {
                        input.move_to(1, cx);
                        input.select_to(5, cx);
                        input.move_horizontal(right, false, cx);
                        assert_eq!(caret_x(input), px(if right { 50. } else { 30. }));
                        assert!(input.selected_range.is_empty());
                    }
                    input.move_to(1, cx);
                    input.left(&Left, window, cx);
                    assert_eq!(caret_x(input), px(40.));
                    input.right(&Right, window, cx);
                    assert_eq!(caret_x(input), px(50.));
                    input.disabled = true;
                    input.left(&Left, window, cx);
                    assert_eq!(caret_x(input), px(50.));
                });
            })
            .unwrap();
    }

    #[gpui::test]
    fn bidi_mouse_caret_selection_and_ime_bounds_agree(cx: &mut TestAppContext) {
        let window = open_input(cx, |cx| TextInput::new(cx).initial_value("aאב12"));
        let mut visual = draw_and_focus(&window, cx);
        window
            .update(&mut visual.cx, |view, window, cx| {
                view.state.update(cx, |input, cx| {
                    input.last_layout = Some(bidi_text_layout("aאב12", None));
                    let bounds = Bounds::new(point(px(10.), px(10.)), size(px(100.), px(20.)));
                    input.last_bounds = Some(bounds);
                    for (x, affinity, caret_x) in [
                        (9., CaretAffinity::Upstream, 10.),
                        (49., CaretAffinity::Downstream, 50.),
                    ] {
                        input.on_mouse_down(
                            &MouseDownEvent {
                                position: bounds.origin + point(px(x), px(5.)),
                                button: MouseButton::Left,
                                click_count: 1,
                                modifiers: Default::default(),
                                first_mouse: false,
                            },
                            window,
                            cx,
                        );
                        assert_eq!(input.selected_range, 1..1);
                        assert_eq!(input.caret_affinity, affinity);
                        let rect = input.bounds_for_range(1..1, bounds, window, cx).unwrap();
                        assert_eq!(rect.origin.x, bounds.left() + px(caret_x));
                    }
                    input.select_to(3, cx);
                    let quads = super::super::element::selection_quads(
                        input.last_layout.as_ref().unwrap(),
                        0..3,
                        bounds,
                        gpui::black(),
                    );
                    assert_eq!(quads.len(), 2);
                    assert_eq!(quads[0].bounds.origin.x, bounds.left());
                    assert_eq!(quads[1].bounds.origin.x, bounds.left() + px(40.));
                    input.on_mouse_down(
                        &MouseDownEvent {
                            position: bounds.origin + point(px(45.), px(5.)),
                            button: MouseButton::Left,
                            click_count: 2,
                            modifiers: Default::default(),
                            first_mouse: false,
                        },
                        window,
                        cx,
                    );
                    assert_eq!(input.selected_range, 0..7);
                    input.stop_selection();
                });
            })
            .unwrap();
    }

    struct TestInput {
        state: Entity<TextInput>,
        rows: Option<usize>,
        changes: Rc<RefCell<Vec<SharedString>>>,
        _subscription: Subscription,
    }

    impl Render for TestInput {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            Input::new(&self.state).when_some(self.rows, Input::rows)
        }
    }

    fn open_input(
        cx: &mut TestAppContext,
        build: impl FnOnce(&mut Context<TextInput>) -> TextInput + 'static,
    ) -> gpui::WindowHandle<TestInput> {
        cx.update(crate::components::input::init);
        cx.open_window(size(px(220.), px(180.)), move |_, cx| {
            let state = cx.new(build);
            let changes = Rc::new(RefCell::new(Vec::new()));
            let changes_for_subscription = changes.clone();
            let subscription = cx.subscribe(&state, move |_, _, event, _| {
                if let InputEvent::Change(value) = event {
                    changes_for_subscription.borrow_mut().push(value.clone());
                }
            });
            TestInput {
                state,
                rows: None,
                changes,
                _subscription: subscription,
            }
        })
    }

    fn open_input_with_rows(
        cx: &mut TestAppContext,
        rows: usize,
        build: impl FnOnce(&mut Context<TextInput>) -> TextInput + 'static,
    ) -> gpui::WindowHandle<TestInput> {
        cx.update(crate::components::input::init);
        cx.open_window(size(px(220.), px(180.)), move |_, cx| {
            let state = cx.new(build);
            let changes = Rc::new(RefCell::new(Vec::new()));
            let changes_for_subscription = changes.clone();
            let subscription = cx.subscribe(&state, move |_, _, event, _| {
                if let InputEvent::Change(value) = event {
                    changes_for_subscription.borrow_mut().push(value.clone());
                }
            });
            TestInput {
                state,
                rows: Some(rows),
                changes,
                _subscription: subscription,
            }
        })
    }

    fn draw_and_focus(
        window: &gpui::WindowHandle<TestInput>,
        cx: &mut TestAppContext,
    ) -> VisualTestContext {
        let mut visual = VisualTestContext::from_window((*window).into(), cx);
        visual.update(|window, cx| {
            window.draw(cx).clear();
        });
        window
            .update(&mut visual.cx, |view, window, cx| {
                let focus_handle = view.state.read(cx).focus_handle.clone();
                window.focus(&focus_handle, cx);
            })
            .unwrap();
        visual
    }

    #[gpui::test]
    fn double_click_selects_the_complete_scrolled_single_line(cx: &mut TestAppContext) {
        let value = "前😀 A deliberately long single-line value extending beyond the viewport";
        let window = open_input(cx, move |cx| TextInput::new(cx).initial_value(value));
        let mut visual = draw_and_focus(&window, cx);
        visual.update(|window, cx| {
            window.draw(cx).clear();
        });
        let position = window
            .update(&mut visual.cx, |view, _, cx| {
                let input = view.state.read(cx);
                assert!(input.single_line_scroll_offset < px(0.));
                input.last_viewport_bounds.unwrap().center()
            })
            .unwrap();
        visual.simulate_event(MouseDownEvent {
            position,
            button: MouseButton::Left,
            click_count: 2,
            modifiers: Default::default(),
            first_mouse: false,
        });
        visual.simulate_mouse_move(position, MouseButton::Left, Default::default());
        visual.simulate_mouse_up(position, MouseButton::Left, Default::default());
        window
            .update(&mut visual.cx, |view, _, cx| {
                let input = view.state.read(cx);
                assert_eq!(input.selected_range, 0..value.len());
                assert!(!input.is_selecting);
                assert!(input.selection_line_anchor.is_none());
            })
            .unwrap();
        visual.simulate_keystrokes("x");
        window
            .update(&mut visual.cx, |view, _, cx| {
                assert_eq!(view.state.read(cx).value().as_ref(), "x");
            })
            .unwrap();
    }

    #[gpui::test]
    fn double_click_drag_extends_and_reverses_by_whole_lines(cx: &mut TestAppContext) {
        let window = open_input_with_rows(cx, 4, |cx| {
            TextInput::new(cx)
                .multiline()
                .initial_value("前😀\n中间\n末尾")
        });
        let mut visual = draw_and_focus(&window, cx);
        visual.update(|window, cx| {
            window.draw(cx).clear();
        });
        let positions = window
            .update(&mut visual.cx, |view, _, cx| {
                let input = view.state.read(cx);
                let layout = input.last_layout.as_ref().unwrap();
                [0, 8, 15].map(|offset| {
                    input.last_bounds.unwrap().origin
                        + layout.position_for_offset(offset)
                        + point(px(1.), layout.line_height / 2.)
                })
            })
            .unwrap();
        visual.simulate_event(MouseDownEvent {
            position: positions[1],
            button: MouseButton::Left,
            click_count: 2,
            modifiers: Default::default(),
            first_mouse: false,
        });
        window
            .update(&mut visual.cx, |view, _, cx| {
                assert_eq!(view.state.read(cx).selected_range, 8..14);
            })
            .unwrap();
        visual.simulate_mouse_move(positions[2], MouseButton::Left, Default::default());
        window
            .update(&mut visual.cx, |view, _, cx| {
                let input = view.state.read(cx);
                assert_eq!(input.selected_range, 8..21);
                assert!(!input.selection_reversed);
            })
            .unwrap();
        visual.simulate_mouse_move(positions[0], MouseButton::Left, Default::default());
        window
            .update(&mut visual.cx, |view, _, cx| {
                let input = view.state.read(cx);
                assert_eq!(input.selected_range, 0..14);
                assert!(input.selection_reversed);
            })
            .unwrap();
        visual.simulate_mouse_up(positions[0], MouseButton::Left, Default::default());
        visual.simulate_mouse_down(positions[1], MouseButton::Left, Default::default());
        window
            .update(&mut visual.cx, |view, _, cx| {
                let input = view.state.read(cx);
                assert!(input.selected_range.is_empty());
                assert!(input.selection_line_anchor.is_none());
            })
            .unwrap();
        visual.simulate_mouse_up(positions[1], MouseButton::Left, Default::default());
    }

    #[gpui::test]
    fn double_click_selects_the_logical_line_across_soft_wraps(cx: &mut TestAppContext) {
        let value = "A long line with 中文 and 😀 that wraps across several visual rows.";
        let window = open_input_with_rows(cx, 8, move |cx| {
            TextInput::new(cx)
                .multiline()
                .initial_value(format!("{value}\nnext"))
        });
        let mut visual = draw_and_focus(&window, cx);
        visual.update(|window, cx| {
            window.draw(cx).clear();
        });
        let position = window
            .update(&mut visual.cx, |view, _, cx| {
                let input = view.state.read(cx);
                let layout = input.last_layout.as_ref().unwrap();
                assert!(!layout.lines[0].wrap_boundaries().is_empty());
                input.last_bounds.unwrap().origin + point(px(10.), layout.line_height * 1.5)
            })
            .unwrap();
        visual.simulate_event(MouseDownEvent {
            position,
            button: MouseButton::Left,
            click_count: 2,
            modifiers: Default::default(),
            first_mouse: false,
        });
        visual.simulate_mouse_up(position, MouseButton::Left, Default::default());
        window
            .update(&mut visual.cx, |view, _, cx| {
                assert_eq!(view.state.read(cx).selected_range, 0..value.len());
            })
            .unwrap();
    }

    #[gpui::test]
    fn double_click_drag_stops_when_the_value_changes(cx: &mut TestAppContext) {
        let window = open_input(cx, |cx| TextInput::new(cx).initial_value("前😀hello"));
        let mut visual = draw_and_focus(&window, cx);
        visual.update(|window, cx| {
            window.draw(cx).clear();
        });
        let position = window
            .update(&mut visual.cx, |view, _, cx| {
                view.state.read(cx).last_viewport_bounds.unwrap().center()
            })
            .unwrap();
        for external_update in [false, true] {
            visual.simulate_event(MouseDownEvent {
                position,
                button: MouseButton::Left,
                click_count: 2,
                modifiers: Default::default(),
                first_mouse: false,
            });
            if external_update {
                window
                    .update(&mut visual.cx, |view, _, cx| {
                        view.state.update(cx, |input, cx| input.set_value("短", cx));
                    })
                    .unwrap();
            } else {
                visual.simulate_keystrokes("x");
            }
            visual.simulate_mouse_move(position, MouseButton::Left, Default::default());
            window
                .update(&mut visual.cx, |view, _, cx| {
                    let input = view.state.read(cx);
                    assert_eq!(
                        input.value().as_ref(),
                        if external_update { "短" } else { "x" }
                    );
                    assert_eq!(
                        input.selected_range,
                        input.content.len()..input.content.len()
                    );
                    assert!(!input.is_selecting);
                    assert!(input.selection_line_anchor.is_none());
                })
                .unwrap();
            visual.simulate_mouse_up(position, MouseButton::Left, Default::default());
        }
    }

    #[gpui::test]
    fn multiline_enter_inserts_newline(cx: &mut TestAppContext) {
        let window = open_input(cx, |cx| {
            TextInput::new(cx).multiline().initial_value("first")
        });
        let mut visual = draw_and_focus(&window, cx);

        visual.simulate_keystrokes("enter");

        window
            .update(&mut visual.cx, |view, _, cx| {
                assert_eq!(view.state.read(cx).value().as_ref(), "first\n");
            })
            .unwrap();
    }

    #[gpui::test]
    fn multiline_vertical_navigation_uses_visual_rows(cx: &mut TestAppContext) {
        let window = open_input(cx, |cx| {
            TextInput::new(cx).multiline().initial_value("abc\ndef")
        });
        let mut visual = draw_and_focus(&window, cx);

        visual.simulate_keystrokes("up enter");

        window
            .update(&mut visual.cx, |view, _, cx| {
                assert_eq!(view.state.read(cx).value().as_ref(), "abc\n\ndef");
            })
            .unwrap();
    }

    #[gpui::test]
    fn multiline_soft_wraps_to_the_available_width(cx: &mut TestAppContext) {
        let window = open_input(cx, |cx| {
            TextInput::new(cx)
                .multiline()
                .initial_value("A deliberately long line that must wrap inside the input.")
        });
        let mut visual = draw_and_focus(&window, cx);
        visual.update(|window, cx| {
            window.draw(cx).clear();
        });

        assert!(
            visual.debug_bounds("uic-scrollbar").is_some(),
            "a multiline input should render its scrollbar"
        );

        window
            .update(&mut visual.cx, |view, _, cx| {
                assert!(
                    view.state
                        .read(cx)
                        .last_layout
                        .as_ref()
                        .unwrap()
                        .visual_row_count()
                        > 1
                );
            })
            .unwrap();
    }

    #[gpui::test]
    fn multiline_content_taller_than_the_viewport_is_scrollable(cx: &mut TestAppContext) {
        let window = open_input(cx, |cx| {
            TextInput::new(cx)
                .multiline()
                .initial_value("one\ntwo\nthree\nfour\nfive\nsix\nseven\neight")
        });
        let mut visual = draw_and_focus(&window, cx);
        visual.update(|window, cx| {
            window.draw(cx).clear();
        });

        window
            .update(&mut visual.cx, |view, _, cx| {
                let scroll_handle = &view.state.read(cx).scroll_handle;
                assert!(scroll_handle.max_offset().y > px(0.));
                assert!(scroll_handle.offset().y < px(0.));
            })
            .unwrap();

        window
            .update(&mut visual.cx, |view, _, cx| {
                view.state
                    .read(cx)
                    .scroll_handle
                    .set_offset(point(px(0.), px(0.)));
                cx.notify();
            })
            .unwrap();
        visual.update(|window, cx| {
            window.draw(cx).clear();
        });
        window
            .update(&mut visual.cx, |view, _, cx| {
                assert_eq!(view.state.read(cx).scroll_handle.offset().y, px(0.));
            })
            .unwrap();
    }

    #[gpui::test]
    fn single_line_keeps_a_long_value_and_its_cursor_visible(cx: &mut TestAppContext) {
        let window = open_input(cx, |cx| {
            TextInput::new(cx)
                .initial_value("A deliberately long single-line value that exceeds the input width")
        });
        let mut visual = draw_and_focus(&window, cx);
        visual.update(|window, cx| {
            window.draw(cx).clear();
        });

        window
            .update(&mut visual.cx, |view, _, cx| {
                let input = view.state.read(cx);
                assert!(input.single_line_scroll_offset < px(0.));

                let layout = input.last_layout.as_ref().unwrap();
                let bounds = input.last_bounds.unwrap();
                let viewport = input.last_viewport_bounds.unwrap();
                let cursor = layout.position_for_offset(input.cursor_offset());
                let cursor_right = bounds.left() + cursor.x + input.appearance.caret_width;
                assert!(
                    cursor_right <= viewport.right(),
                    "cursor_right={cursor_right:?}, viewport={viewport:?}, offset={:?}, bounds={bounds:?}, cursor={cursor:?}",
                    input.single_line_scroll_offset,
                );
                assert!(
                    input.index_for_mouse_position(point(
                        viewport.left() + px(1.),
                        viewport.top() + viewport.size.height / 2.,
                    )) > 0,
                    "mouse hit testing must account for the shifted text origin",
                );
            })
            .unwrap();

        visual.simulate_keystrokes("home");
        visual.update(|window, cx| {
            window.draw(cx).clear();
        });
        window
            .update(&mut visual.cx, |view, _, cx| {
                let input = view.state.read(cx);
                assert_eq!(input.cursor_offset(), 0);
                assert_eq!(input.single_line_scroll_offset, px(0.));
            })
            .unwrap();
    }

    #[gpui::test]
    fn single_line_drag_selection_scrolls_outside_both_edges_and_stops_on_release(
        cx: &mut TestAppContext,
    ) {
        let window = open_input(cx, |cx| {
            TextInput::new(cx)
            .initial_value("长文本 Unicode selection stays anchored while dragging beyond either edge of this narrow input")
        });
        let mut visual = draw_and_focus(&window, cx);
        let snapshot = |visual: &mut VisualTestContext| {
            window
                .update(&mut visual.cx, |view, _, cx| {
                    let input = view.state.read(cx);
                    (
                        input.single_line_scroll_offset,
                        input.selected_range.clone(),
                    )
                })
                .unwrap()
        };
        for left in [true, false] {
            visual.simulate_keystrokes(if left { "end" } else { "home" });
            visual.update(|window, cx| window.draw(cx).clear());
            let viewport = window
                .update(&mut visual.cx, |view, _, cx| {
                    view.state.read(cx).last_viewport_bounds.unwrap()
                })
                .unwrap();
            let down = viewport.center();
            visual.simulate_mouse_down(down, MouseButton::Left, gpui::Modifiers::default());
            let anchor = snapshot(&mut visual).1.start;
            let outside = point(
                if left {
                    viewport.left() - px(30.)
                } else {
                    viewport.right() + px(30.)
                },
                viewport.top() - px(12.),
            );
            visual.simulate_mouse_move(outside, MouseButton::Left, gpui::Modifiers::default());
            visual.update(|window, cx| window.draw(cx).clear());
            let before = snapshot(&mut visual);
            // No further mouse events: holding at the edge must keep extending the selection.
            for _ in 0..4 {
                visual
                    .cx
                    .executor()
                    .advance_clock(Duration::from_millis(16));
                visual.run_until_parked();
                visual.update(|window, cx| window.draw(cx).clear());
            }
            let after = snapshot(&mut visual);
            if left {
                assert!(after.0 > before.0, "left drag must reveal earlier text");
                assert!(after.1.start < before.1.start);
                assert_eq!(after.1.end, anchor);
            } else {
                assert!(after.0 < before.0, "right drag must reveal later text");
                assert!(after.1.end > before.1.end);
                assert_eq!(after.1.start, anchor);
            }
            visual.simulate_mouse_up(outside, MouseButton::Left, gpui::Modifiers::default());
            visual
                .cx
                .executor()
                .advance_clock(Duration::from_millis(100));
            visual.run_until_parked();
            visual.update(|window, cx| window.draw(cx).clear());
            assert_eq!(snapshot(&mut visual), after);
            window
                .update(&mut visual.cx, |view, _, cx| {
                    let input = view.state.read(cx);
                    assert!(!input.is_selecting);
                    assert!(input.selection_scroll_task.is_none());
                    assert!(input.content.is_char_boundary(input.selected_range.start));
                    assert!(input.content.is_char_boundary(input.selected_range.end));
                })
                .unwrap();
        }
    }

    #[gpui::test]
    fn single_line_drag_scroll_stops_inside_at_content_start_and_when_disabled(
        cx: &mut TestAppContext,
    ) {
        let window = open_input(cx, |cx| {
            TextInput::new(cx)
            .initial_value("A long single line which must scroll left until its very first character is selected")
        });
        let mut visual = draw_and_focus(&window, cx);
        visual.update(|window, cx| window.draw(cx).clear());
        let viewport = window
            .update(&mut visual.cx, |view, _, cx| {
                view.state.read(cx).last_viewport_bounds.unwrap()
            })
            .unwrap();
        let outside = point(viewport.left() - px(60.), viewport.center().y);
        visual.simulate_mouse_down(
            viewport.center(),
            MouseButton::Left,
            gpui::Modifiers::default(),
        );
        visual.simulate_mouse_move(outside, MouseButton::Left, gpui::Modifiers::default());
        visual.simulate_mouse_move(
            viewport.center(),
            MouseButton::Left,
            gpui::Modifiers::default(),
        );
        window
            .update(&mut visual.cx, |view, _, cx| {
                assert!(view.state.read(cx).selection_scroll_task.is_none());
            })
            .unwrap();
        visual.simulate_mouse_move(outside, MouseButton::Left, gpui::Modifiers::default());
        for _ in 0..100 {
            visual
                .cx
                .executor()
                .advance_clock(Duration::from_millis(16));
            visual.run_until_parked();
            visual.update(|window, cx| window.draw(cx).clear());
        }
        window
            .update(&mut visual.cx, |view, _, cx| {
                let input = view.state.read(cx);
                assert_eq!(input.single_line_scroll_offset, px(0.));
                assert_eq!(input.selected_range.start, 0);
                assert!(input.selection_scroll_task.is_none());
            })
            .unwrap();
        visual.simulate_mouse_move(
            point(viewport.right() + px(30.), viewport.center().y),
            MouseButton::Left,
            gpui::Modifiers::default(),
        );
        window
            .update(&mut visual.cx, |view, _, cx| {
                view.state.update(cx, |input, cx| {
                    assert!(input.selection_scroll_task.is_some());
                    input.set_disabled(true, cx);
                    assert!(input.selection_scroll_task.is_none());
                    assert!(!input.is_selecting);
                });
            })
            .unwrap();
    }

    #[gpui::test]
    fn multiline_scrollbar_stays_at_the_edge_and_dragging_preserves_selection(
        cx: &mut TestAppContext,
    ) {
        let window = open_input_with_rows(cx, 3, |cx| {
            TextInput::new(cx)
                .multiline()
                .initial_value("one\ntwo\nthree\nfour\nfive\nsix\nseven\neight")
        });
        let mut visual = draw_and_focus(&window, cx);
        window
            .update(&mut visual.cx, |view, _, cx| {
                view.state.update(cx, |input, cx| input.move_to(0, cx));
            })
            .unwrap();
        for _ in 0..3 {
            visual.update(|window, cx| {
                window.draw(cx).clear();
            });
        }
        let track = visual.debug_bounds("uic-scrollbar").unwrap();
        window
            .update(&mut visual.cx, |view, _, cx| {
                let viewport = view.state.read(cx).scroll_handle.bounds();
                assert!(track.left() >= viewport.right());
                assert!(track.top() < viewport.top());
                assert!(track.bottom() > viewport.bottom());
            })
            .unwrap();
        let start = point(track.center().x, track.top() + px(8.));
        let end = point(track.center().x, track.bottom() - px(8.));
        visual.simulate_mouse_down(start, MouseButton::Left, gpui::Modifiers::default());
        visual.simulate_mouse_move(end, MouseButton::Left, gpui::Modifiers::default());
        visual.simulate_mouse_up(end, MouseButton::Left, gpui::Modifiers::default());
        visual.update(|window, cx| {
            window.draw(cx).clear();
        });
        window
            .update(&mut visual.cx, |view, _, cx| {
                let input = view.state.read(cx);
                assert!(input.scroll_handle.offset().y < px(0.));
                assert_eq!(input.selected_range, 0..0);
                assert!(!input.is_selecting);
                assert!(!input.scrollbar_state.is_dragging());
            })
            .unwrap();
    }

    #[gpui::test]
    fn multiline_enter_scrolls_as_soon_as_the_cursor_adds_a_row(cx: &mut TestAppContext) {
        let window = open_input_with_rows(cx, 3, |cx| {
            TextInput::new(cx)
                .multiline()
                .initial_value("one\ntwo\nthree")
        });
        let mut visual = draw_and_focus(&window, cx);

        visual.simulate_keystrokes("enter");
        visual.update(|window, cx| {
            window.draw(cx).clear();
        });
        visual.update(|window, cx| {
            window.draw(cx).clear();
        });

        window
            .update(&mut visual.cx, |view, _, cx| {
                let input = view.state.read(cx);
                assert_eq!(input.value().as_ref(), "one\ntwo\nthree\n");
                assert!(input.scroll_handle.max_offset().y > px(0.));
                assert!(input.scroll_handle.offset().y < px(0.));
            })
            .unwrap();

        visual.simulate_keystrokes("enter");
        for _ in 0..3 {
            visual.update(|window, cx| {
                window.draw(cx).clear();
            });
        }
        window
            .update(&mut visual.cx, |view, _, cx| {
                let input = view.state.read(cx);
                let layout = input.last_layout.as_ref().unwrap();
                let bounds = input.last_bounds.unwrap();
                let cursor = layout.position_for_offset(input.cursor_offset());
                let cursor_bottom = bounds.top() + cursor.y + layout.line_height;
                assert_eq!(input.value().as_ref(), "one\ntwo\nthree\n\n");
                assert!(
                    cursor_bottom <= input.scroll_handle.bounds().bottom(),
                    "cursor_bottom={cursor_bottom:?}, viewport={:?}, offset={:?}, max_offset={:?}, bounds={bounds:?}, cursor={cursor:?}",
                    input.scroll_handle.bounds(),
                    input.scroll_handle.offset(),
                    input.scroll_handle.max_offset(),
                );
            })
            .unwrap();
    }

    #[gpui::test]
    fn repeated_select_all_keeps_every_multiline_row_selected(cx: &mut TestAppContext) {
        let window = open_input_with_rows(cx, 3, |cx| {
            TextInput::new(cx)
                .multiline()
                .initial_value("one\ntwo\nthree")
        });
        let mut visual = draw_and_focus(&window, cx);

        let select_all = if cfg!(target_os = "macos") {
            "cmd-a"
        } else {
            "ctrl-a"
        };
        visual.simulate_keystrokes(select_all);
        window
            .update(&mut visual.cx, |view, _, cx| {
                let input = view.state.read(cx);
                assert_eq!(input.selected_range, 0..input.content.len());
            })
            .unwrap();
        visual.simulate_keystrokes(select_all);
        visual.update(|window, cx| {
            window.draw(cx).clear();
        });

        window
            .update(&mut visual.cx, |view, _, cx| {
                let input = view.state.read(cx);
                assert_eq!(input.selected_range, 0..input.content.len());
                let quads = super::super::element::selection_quads(
                    input.last_layout.as_ref().unwrap(),
                    input.selected_range.clone(),
                    input.last_bounds.unwrap(),
                    input.appearance.selection,
                );
                assert_eq!(quads.len(), 3);
            })
            .unwrap();
    }

    #[gpui::test]
    fn single_line_enter_does_not_insert_newline(cx: &mut TestAppContext) {
        let window = open_input(cx, |cx| TextInput::new(cx).initial_value("first"));
        let mut visual = draw_and_focus(&window, cx);

        visual.simulate_keystrokes("enter");

        window
            .update(&mut visual.cx, |view, _, cx| {
                assert_eq!(view.state.read(cx).value().as_ref(), "first");
            })
            .unwrap();
    }

    #[gpui::test]
    fn surrounding_deletion_preserves_selected_text_and_direction(cx: &mut TestAppContext) {
        let window = open_input(cx, |cx| TextInput::new(cx).initial_value("前😀选中🌍后"));
        let mut visual = draw_and_focus(&window, cx);
        window
            .update(&mut visual.cx, |view, window, cx| {
                view.state.update(cx, |input, _| {
                    input.selected_range = 7..13;
                    input.selection_reversed = true;
                });
                let mut handler =
                    gpui::ElementInputHandler::new(Bounds::default(), view.state.clone());
                let snapshot =
                    gpui::InputHandler::surrounding_text(&mut handler, 4000, window, cx).unwrap();
                assert_eq!((snapshot.cursor, snapshot.anchor), (7, 13));
                let (before, after) = snapshot.deletion_utf16(4, 4).unwrap();
                assert!(gpui::InputHandler::delete_surrounding_text(
                    &mut handler,
                    before,
                    after,
                    window,
                    cx
                ));
                let input = view.state.read(cx);
                assert_eq!(input.value().as_ref(), "前选中后");
                assert_eq!(input.selected_range, 3..9);
                assert_eq!(input.cursor_offset(), 3);
                assert!(input.selection_reversed);
            })
            .unwrap();
    }

    #[gpui::test]
    fn surrounding_delete_commit_and_preedit_keep_the_insertion_point(cx: &mut TestAppContext) {
        let window = open_input(cx, |cx| TextInput::new(cx).initial_value("前😀后"));
        let mut visual = draw_and_focus(&window, cx);
        window
            .update(&mut visual.cx, |view, window, cx| {
                view.state.update(cx, |input, cx| {
                    input.selected_range = 7..7;
                    input.replace_and_mark_text_in_range(None, "ni", Some(1..1), window, cx);
                    let snapshot = input.surrounding_text(4000, window, cx).unwrap();
                    assert_eq!(snapshot.text, "前😀后");
                    assert_eq!(snapshot.cursor, 7);
                    let deletion = snapshot.deletion_utf16(4, 3).unwrap();
                    let marked = input.marked_text_range(window, cx).unwrap();
                    input.replace_and_mark_text_in_range(Some(marked), "", None, window, cx);
                    assert!(input.delete_surrounding_text(deletion.0, deletion.1, window, cx));
                    input.replace_text_in_range(None, "你", window, cx);
                    let committed = input.surrounding_text(4000, window, cx).unwrap();
                    assert_eq!(committed.text, "前你");
                    assert_eq!(committed.cursor, 6);
                    input.replace_and_mark_text_in_range(None, "hao", Some(1..1), window, cx);
                    assert_eq!(input.value().as_ref(), "前你hao");
                    assert_eq!(input.cursor_offset(), 7);
                    assert_eq!(input.surrounding_text(4000, window, cx), Some(committed));
                });
            })
            .unwrap();
    }

    #[gpui::test]
    fn surrounding_rejects_invalid_deletions_and_private_fields(cx: &mut TestAppContext) {
        let window = open_input(cx, |cx| TextInput::new(cx).initial_value("前😀"));
        let mut visual = draw_and_focus(&window, cx);
        window
            .update(&mut visual.cx, |view, window, cx| {
                view.state.update(cx, |input, cx| {
                    assert!(!input.delete_surrounding_text(1, 0, window, cx));
                    assert!(!input.delete_surrounding_text(0, 1, window, cx));
                    assert!(!input.delete_surrounding_text(4, 0, window, cx));
                    assert_eq!(input.value().as_ref(), "前😀");
                    assert_eq!(input.selected_range, 7..7);
                    input.mode = InputMode::Password;
                    assert!(input.surrounding_text(4000, window, cx).is_none());
                    assert!(!input.delete_surrounding_text(2, 0, window, cx));
                    input.mode = InputMode::Text;
                    input.disabled = true;
                    assert!(input.surrounding_text(4000, window, cx).is_none());
                    assert!(!input.delete_surrounding_text(2, 0, window, cx));
                    assert_eq!(input.value().as_ref(), "前😀");
                });
            })
            .unwrap();
        visual.run_until_parked();
        window
            .update(&mut visual.cx, |view, _, _| {
                assert!(view.changes.borrow().is_empty());
            })
            .unwrap();
    }

    #[gpui::test]
    fn ime_preedit_selection_uses_inserted_text_offsets(cx: &mut TestAppContext) {
        let window = open_input(cx, |cx| TextInput::new(cx).initial_value("前😀ab "));
        let mut visual = draw_and_focus(&window, cx);
        window
            .update(&mut visual.cx, |view, window, cx| {
                view.state.update(cx, |input, cx| {
                    input.replace_and_mark_text_in_range(None, "ab😀你", Some(2..4), window, cx);
                    assert_eq!(input.value().as_ref(), "前😀ab ab😀你");
                    assert_eq!(input.selected_range, 12..16);
                    assert_eq!(input.marked_range, Some(10..19));
                    assert!(!input.selection_reversed);
                    input.replace_and_mark_text_in_range(None, "a\r\n😀", Some(3..5), window, cx);
                    assert_eq!(input.value().as_ref(), "前😀ab a 😀");
                    assert_eq!(input.selected_range, 12..16);
                });
            })
            .unwrap();
        visual.run_until_parked();
        window
            .update(&mut visual.cx, |view, _, _| {
                assert!(view.changes.borrow().is_empty());
            })
            .unwrap();
    }

    #[gpui::test]
    fn ime_directed_and_hidden_selection_reaches_the_input(cx: &mut TestAppContext) {
        let window = open_input(cx, |cx| TextInput::new(cx).initial_value("前😀ab "));
        let mut visual = draw_and_focus(&window, cx);
        window
            .update(&mut visual.cx, |view, window, cx| {
                let mut handler =
                    gpui::ElementInputHandler::new(Bounds::default(), view.state.clone());
                gpui::InputHandler::replace_and_mark_text_with_selection(
                    &mut handler,
                    None,
                    "ab😀你",
                    PreeditSelection::Range { anchor: 4, head: 2 },
                    window,
                    cx,
                );
                let input = view.state.read(cx);
                assert_eq!(input.selected_range, 12..16);
                assert_eq!(input.cursor_offset(), 12);
                assert!(input.selection_reversed);
                gpui::InputHandler::replace_and_mark_text_with_selection(
                    &mut handler,
                    None,
                    "ab😀你",
                    PreeditSelection::Hidden,
                    window,
                    cx,
                );
                let input = view.state.read(cx);
                assert!(input.preedit_cursor_hidden);
                assert!(input.selected_range.is_empty());
                gpui::InputHandler::replace_text_in_range(&mut handler, None, "选", window, cx);
                let input = view.state.read(cx);
                assert_eq!(input.value().as_ref(), "前😀ab 选");
                assert_eq!(input.marked_range, None);
                assert!(!input.selection_reversed);
            })
            .unwrap();
        visual.run_until_parked();
        window
            .update(&mut visual.cx, |view, _, _| {
                assert_eq!(
                    view.changes.borrow().as_slice(),
                    &[SharedString::from("前😀ab 选")]
                );
            })
            .unwrap();
    }

    #[gpui::test]
    fn ime_preedit_is_visible_without_emitting_change(cx: &mut TestAppContext) {
        let window = open_input(cx, |cx| TextInput::new(cx).initial_value("prefix "));
        let mut visual = draw_and_focus(&window, cx);

        window
            .update(&mut visual.cx, |view, window, cx| {
                view.state.update(cx, |input, cx| {
                    input.replace_and_mark_text_in_range(None, "ni", None, window, cx);
                    input.replace_and_mark_text_in_range(None, "你", None, window, cx);
                });
            })
            .unwrap();
        visual.update(|window, cx| {
            window.draw(cx).clear();
        });

        window
            .update(&mut visual.cx, |view, _, cx| {
                let input = view.state.read(cx);
                assert_eq!(input.value().as_ref(), "prefix 你");
                assert_eq!(input.marked_range, Some(7..10));
                assert!(view.changes.borrow().is_empty());
            })
            .unwrap();
    }

    #[gpui::test]
    fn ime_candidate_commit_emits_one_change(cx: &mut TestAppContext) {
        let window = open_input(cx, |cx| TextInput::new(cx).initial_value("prefix "));
        let mut visual = draw_and_focus(&window, cx);

        window
            .update(&mut visual.cx, |view, window, cx| {
                view.state.update(cx, |input, cx| {
                    input.replace_and_mark_text_in_range(None, "ni", None, window, cx);
                    input.replace_and_mark_text_in_range(None, "你", None, window, cx);
                    input.replace_text_in_range(None, "你", window, cx);
                    input.unmark_text(window, cx);
                });
            })
            .unwrap();
        visual.update(|window, cx| {
            window.draw(cx).clear();
        });

        window
            .update(&mut visual.cx, |view, _, _| {
                assert_eq!(
                    view.changes.borrow().as_slice(),
                    &[SharedString::from("prefix 你")]
                );
            })
            .unwrap();
    }

    #[gpui::test]
    fn ime_unmark_commits_when_the_platform_does_not_insert_again(cx: &mut TestAppContext) {
        let window = open_input(cx, |cx| TextInput::new(cx).initial_value("prefix "));
        let mut visual = draw_and_focus(&window, cx);

        window
            .update(&mut visual.cx, |view, window, cx| {
                view.state.update(cx, |input, cx| {
                    input.replace_and_mark_text_in_range(None, "かな", None, window, cx);
                    input.unmark_text(window, cx);
                });
            })
            .unwrap();
        visual.update(|window, cx| {
            window.draw(cx).clear();
        });

        window
            .update(&mut visual.cx, |view, _, _| {
                assert_eq!(
                    view.changes.borrow().as_slice(),
                    &[SharedString::from("prefix かな")]
                );
            })
            .unwrap();
    }

    #[gpui::test]
    fn cancelling_ime_preedit_does_not_emit_a_change(cx: &mut TestAppContext) {
        let window = open_input(cx, |cx| TextInput::new(cx).initial_value("prefix "));
        let mut visual = draw_and_focus(&window, cx);

        window
            .update(&mut visual.cx, |view, window, cx| {
                view.state.update(cx, |input, cx| {
                    input.replace_and_mark_text_in_range(None, "ni", None, window, cx);
                    input.replace_text_in_range(None, "", window, cx);
                });
            })
            .unwrap();
        visual.update(|window, cx| {
            window.draw(cx).clear();
        });

        window
            .update(&mut visual.cx, |view, _, cx| {
                assert_eq!(view.state.read(cx).value().as_ref(), "prefix ");
                assert!(view.changes.borrow().is_empty());
            })
            .unwrap();
    }
}
