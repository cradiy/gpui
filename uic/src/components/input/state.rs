use std::{borrow::Cow, ops::Range};

use gpui::{
    App, Bounds, ClipboardItem, Context, CursorStyle, EntityInputHandler, FocusHandle, Focusable,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point, Render, ScrollHandle,
    SharedString, UTF16Selection, Window, WrappedLine, div, point, prelude::*, px,
};
use unicode_segmentation::UnicodeSegmentation;

use super::{InputAppearance, InputEvent, InputMode, actions::*, element::TextElement};
use crate::components::scrollbar::ScrollbarState;

pub(super) struct TextLayout {
    pub(super) lines: Vec<WrappedLine>,
    pub(super) line_starts: Vec<usize>,
    pub(super) line_height: Pixels,
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
        let (line_ix, local_offset) = self.line_for_offset(offset);
        let rows_before = self
            .lines
            .iter()
            .take(line_ix)
            .map(|line| line.wrap_boundaries().len() + 1)
            .sum::<usize>();
        let local = self.lines[line_ix]
            .position_for_index(local_offset, self.line_height)
            .unwrap_or_default();
        point(local.x, local.y + self.line_height * rows_before as f32)
    }

    pub(super) fn offset_for_position(&self, position: Point<Pixels>) -> usize {
        if position.y < px(0.) {
            return 0;
        }

        let target_row = (position.y / self.line_height).floor() as usize;
        let mut rows_before = 0;
        for (line_ix, line) in self.lines.iter().enumerate() {
            let rows = line.wrap_boundaries().len() + 1;
            if target_row < rows_before + rows {
                let local_y = self.line_height * (target_row - rows_before) as f32;
                let local = line
                    .closest_index_for_position(point(position.x, local_y), self.line_height)
                    .unwrap_or_else(|index| index);
                return self.line_starts[line_ix] + local;
            }
            rows_before += rows;
        }

        self.line_starts.last().copied().unwrap_or(0)
            + self.lines.last().map_or(0, WrappedLine::len)
    }

    fn row_range_for_offset(&self, offset: usize) -> Range<usize> {
        let position = self.position_for_offset(offset);
        let start = self.offset_for_position(point(px(-1_000_000.), position.y));
        let end = self.offset_for_position(point(px(1_000_000.), position.y));
        start..end.max(start)
    }
}

pub struct TextInput {
    pub(super) focus_handle: FocusHandle,
    pub(super) content: SharedString,
    pub(super) committed_content: SharedString,
    pub(super) placeholder: SharedString,
    pub(super) selected_range: Range<usize>,
    pub(super) selection_reversed: bool,
    pub(super) marked_range: Option<Range<usize>>,
    pub(super) last_layout: Option<TextLayout>,
    pub(super) last_bounds: Option<Bounds<Pixels>>,
    pub(super) is_selecting: bool,
    pub(super) disabled: bool,
    pub(super) mode: InputMode,
    pub(super) appearance: InputAppearance,
    pub(super) preferred_x: Option<Pixels>,
    pub(super) scroll_handle: ScrollHandle,
    pub(super) scrollbar_state: ScrollbarState,
    pub(super) scroll_cursor_pending: bool,
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
            marked_range: None,
            last_layout: None,
            last_bounds: None,
            is_selecting: false,
            disabled: false,
            mode: InputMode::Text,
            appearance: InputAppearance::default(),
            preferred_x: None,
            scroll_handle: ScrollHandle::new(),
            scrollbar_state: ScrollbarState::new(),
            scroll_cursor_pending: true,
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
        self.is_selecting = false;
        self.marked_range = None;
        cx.notify();
    }

    pub fn is_disabled(&self) -> bool {
        self.disabled
    }

    pub fn set_mode(&mut self, mode: InputMode) {
        self.mode = mode;
        self.preferred_x = None;
        self.scroll_cursor_pending = true;
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
        self.content = value.into();
        self.committed_content = self.content.clone();
        self.selected_range = self.content.len()..self.content.len();
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
        if self.disabled {
            return;
        }
        if self.selected_range.is_empty() {
            self.move_to(self.previous_boundary(self.cursor_offset()), cx);
        } else {
            self.move_to(self.selected_range.start, cx)
        }
    }

    fn right(&mut self, _: &Right, _: &mut Window, cx: &mut Context<Self>) {
        if self.disabled {
            return;
        }
        if self.selected_range.is_empty() {
            self.move_to(self.next_boundary(self.selected_range.end), cx);
        } else {
            self.move_to(self.selected_range.end, cx)
        }
    }

    fn up(&mut self, _: &Up, _: &mut Window, cx: &mut Context<Self>) {
        self.move_vertical(-1., false, cx);
    }

    fn down(&mut self, _: &Down, _: &mut Window, cx: &mut Context<Self>) {
        self.move_vertical(1., false, cx);
    }

    fn select_left(&mut self, _: &SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        if self.disabled {
            return;
        }
        self.preferred_x = None;
        self.select_to(self.previous_boundary(self.cursor_offset()), cx);
    }

    fn select_right(&mut self, _: &SelectRight, _: &mut Window, cx: &mut Context<Self>) {
        if self.disabled {
            return;
        }
        self.preferred_x = None;
        self.select_to(self.next_boundary(self.cursor_offset()), cx);
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
        if self.disabled {
            return;
        }
        if self.selected_range.is_empty() {
            let prev = self.previous_boundary(self.cursor_offset());
            if self.cursor_offset() == prev {
                window.play_system_bell();
                return;
            }
            self.select_to(prev, cx)
        }
        self.replace_text_in_range(None, "", window, cx)
    }

    fn delete(&mut self, _: &Delete, window: &mut Window, cx: &mut Context<Self>) {
        if self.disabled {
            return;
        }
        if self.selected_range.is_empty() {
            let next = self.next_boundary(self.cursor_offset());
            if self.cursor_offset() == next {
                window.play_system_bell();
                return;
            }
            self.select_to(next, cx)
        }
        self.replace_text_in_range(None, "", window, cx)
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
        self.is_selecting = true;

        if event.modifiers.shift {
            self.preferred_x = None;
            self.select_to(self.index_for_mouse_position(event.position), cx);
        } else {
            self.move_to(self.index_for_mouse_position(event.position), cx)
        }
    }

    fn on_mouse_up(&mut self, _: &MouseUpEvent, _window: &mut Window, _: &mut Context<Self>) {
        self.is_selecting = false;
    }

    fn on_mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.disabled {
            return;
        }
        if self.is_selecting {
            self.preferred_x = None;
            self.select_to(self.index_for_mouse_position(event.position), cx);
        }
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
        self.preferred_x = None;
        self.scroll_cursor_pending = true;
        cx.notify()
    }

    fn move_vertical(&mut self, rows: f32, selecting: bool, cx: &mut Context<Self>) {
        if self.disabled || self.mode != InputMode::Multiline {
            return;
        }
        let Some(layout) = self.last_layout.as_ref() else {
            return;
        };
        let cursor = self.cursor_offset();
        let position = layout.position_for_offset(cursor);
        let preferred_x = self.preferred_x.unwrap_or(position.x);
        let target =
            layout.offset_for_position(point(preferred_x, position.y + layout.line_height * rows));
        self.preferred_x = Some(preferred_x);
        if selecting {
            self.select_to(target, cx);
        } else {
            self.selected_range = target..target;
            self.selection_reversed = false;
            self.scroll_cursor_pending = true;
            cx.notify();
        }
    }

    pub(super) fn cursor_offset(&self) -> usize {
        if self.selection_reversed {
            self.selected_range.start
        } else {
            self.selected_range.end
        }
    }

    fn index_for_mouse_position(&self, position: Point<Pixels>) -> usize {
        if self.content.is_empty() {
            return 0;
        }

        let (Some(bounds), Some(line)) = (self.last_bounds.as_ref(), self.last_layout.as_ref())
        else {
            return 0;
        };
        if position.y < bounds.top() {
            return 0;
        }
        if position.y > bounds.bottom() {
            return self.content.len();
        }
        line.offset_for_position(point(position.x - bounds.left(), position.y - bounds.top()))
    }

    fn select_to(&mut self, offset: usize, cx: &mut Context<Self>) {
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
        self.marked_range = None;
        self.last_layout = None;
        self.last_bounds = None;
        self.is_selecting = false;
        self.preferred_x = None;
        self.scroll_cursor_pending = true;
        cx.emit(InputEvent::Change(self.content.clone()));
        cx.notify();
    }
}

impl EntityInputHandler for TextInput {
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
        self.selected_range = new_selected_range_utf16
            .as_ref()
            .map(|range_utf16| self.range_from_utf16(range_utf16))
            .map(|new_range| new_range.start + range.start..new_range.end + range.start)
            .unwrap_or_else(|| range.start + new_text.len()..range.start + new_text.len());
        self.preferred_x = None;
        self.scroll_cursor_pending = true;

        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let last_layout = self.last_layout.as_ref()?;
        let range = self.range_from_utf16(&range_utf16);
        let start = last_layout.position_for_offset(range.start);
        let end = last_layout.position_for_offset(range.end);
        let top_left = if start.y == end.y {
            point(bounds.left() + start.x, bounds.top() + start.y)
        } else {
            point(bounds.left(), bounds.top() + start.y)
        };
        let bottom_right = if start.y == end.y {
            point(
                bounds.left() + end.x,
                bounds.top() + end.y + last_layout.line_height,
            )
        } else {
            point(
                bounds.right(),
                bounds.top() + end.y + last_layout.line_height,
            )
        };
        Some(Bounds::from_corners(top_left, bottom_right))
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
        div()
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
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
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
