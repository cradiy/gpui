use std::ops::Range;

use gpui::{
    App, Bounds, Element, ElementId, ElementInputHandler, Entity, GlobalElementId, LayoutId,
    PaintQuad, Pixels, Style, TextRun, UnderlineStyle, Window, fill, point, prelude::*, px,
    relative, size,
};

use super::{InputMode, TextInput, state::TextLayout};

pub(super) struct TextElement {
    pub(super) input: Entity<TextInput>,
}

pub(super) struct PrepaintState {
    layout: Option<TextLayout>,
    cursor: Option<PaintQuad>,
    cursor_bounds: Option<Bounds<Pixels>>,
    selection: Vec<PaintQuad>,
    text_bounds: Bounds<Pixels>,
    viewport_bounds: Bounds<Pixels>,
    horizontal_offset: Pixels,
}

impl IntoElement for TextElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for TextElement {
    type RequestLayoutState = ();
    type PrepaintState = PrepaintState;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let input = self.input.read(cx);
        let multiline = input.mode == InputMode::Multiline;
        let row_count = if multiline {
            input
                .last_layout
                .as_ref()
                .map(TextLayout::visual_row_count)
                .unwrap_or_else(|| input.content.split('\n').count().max(1))
        } else {
            1
        };
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = (window.line_height() * row_count as f32).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let input = self.input.read(cx);
        let disabled = input.disabled;
        let content = input.content.clone();
        let selected_range = input.selected_range.clone();
        let cursor_offset = input.cursor_offset();
        let appearance = input.appearance;
        let multiline = input.mode == InputMode::Multiline;
        let focus_handle = input.focus_handle.clone();
        let scroll_cursor_pending = input.scroll_cursor_pending;
        let current_scroll_offset = input.single_line_scroll_offset;
        let style = window.text_style();

        let (display_text, text_color) = if content.is_empty() {
            (input.placeholder.clone(), appearance.placeholder)
        } else if input.mode == InputMode::Password {
            ("*".repeat(content.len()).into(), style.color)
        } else if multiline {
            (content.clone(), style.color)
        } else {
            (content.replace(['\r', '\n'], " ").into(), style.color)
        };

        let run = TextRun {
            len: display_text.len(),
            font: style.font(),
            color: text_color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let runs = if !content.is_empty()
            && let Some(marked_range) = input.marked_range.as_ref()
        {
            vec![
                TextRun {
                    len: marked_range.start,
                    ..run.clone()
                },
                TextRun {
                    len: marked_range.end - marked_range.start,
                    underline: Some(UnderlineStyle {
                        color: Some(run.color),
                        thickness: px(1.0),
                        wavy: false,
                    }),
                    ..run.clone()
                },
                TextRun {
                    len: display_text.len() - marked_range.end,
                    ..run
                },
            ]
            .into_iter()
            .filter(|run| run.len > 0)
            .collect::<Vec<_>>()
        } else {
            vec![run]
        };

        let font_size = style.font_size.to_pixels(window.rem_size());
        let wrap_width = multiline.then_some(bounds.size.width);
        let lines = window
            .text_system()
            .shape_text(display_text.clone(), font_size, &runs, wrap_width, None)
            .expect("failed to shape input text")
            .into_vec();
        let mut line_starts = vec![0];
        line_starts.extend(
            display_text
                .match_indices('\n')
                .map(|(offset, _)| offset + 1),
        );
        let layout = TextLayout::new(lines, line_starts, window.line_height());

        let cursor_position = layout.position_for_offset(cursor_offset);
        let mut horizontal_offset = px(0.);
        let mut text_bounds = bounds;
        if !multiline {
            let content_width = layout
                .lines
                .first()
                .map(|line| line.width())
                .unwrap_or_default()
                + appearance.caret_width
                + px(1.);
            let max_scroll = (content_width - bounds.size.width).max(px(0.));
            horizontal_offset = current_scroll_offset.clamp(-max_scroll, px(0.));

            if focus_handle.is_focused(window) && scroll_cursor_pending {
                let cursor_left = cursor_position.x + horizontal_offset;
                let cursor_right = cursor_left + appearance.caret_width + px(1.);
                if cursor_left < px(0.) {
                    horizontal_offset -= cursor_left;
                } else if cursor_right > bounds.size.width {
                    horizontal_offset -= cursor_right - bounds.size.width;
                }
                horizontal_offset = horizontal_offset.clamp(-max_scroll, px(0.));
            }

            text_bounds = Bounds::new(
                point(bounds.left() + horizontal_offset, bounds.top()),
                size(content_width.max(bounds.size.width), bounds.size.height),
            );
        }
        let indicator_top = if multiline {
            bounds.top() + cursor_position.y + (layout.line_height - appearance.caret_height) / 2.
        } else {
            bounds.top() + (bounds.size.height - appearance.caret_height) / 2.
        };
        let cursor_bounds = Bounds::new(
            point(text_bounds.left() + cursor_position.x, indicator_top),
            size(appearance.caret_width, appearance.caret_height),
        );
        let cursor_row_bounds = Bounds::new(
            point(
                text_bounds.left() + cursor_position.x,
                bounds.top() + cursor_position.y,
            ),
            size(appearance.caret_width, layout.line_height),
        );
        let (selection, cursor) = if disabled {
            (Vec::new(), None)
        } else if selected_range.is_empty() {
            (Vec::new(), Some(fill(cursor_bounds, appearance.caret)))
        } else {
            (
                selection_quads(&layout, selected_range, text_bounds, appearance.selection),
                None,
            )
        };

        PrepaintState {
            layout: Some(layout),
            cursor,
            cursor_bounds: Some(cursor_row_bounds),
            selection,
            text_bounds,
            viewport_bounds: bounds,
            horizontal_offset,
        }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let (focus_handle, disabled, multiline, scroll_handle, scroll_cursor_pending) = {
            let input = self.input.read(cx);
            (
                input.focus_handle.clone(),
                input.disabled,
                input.mode == InputMode::Multiline,
                input.scroll_handle.clone(),
                input.scroll_cursor_pending,
            )
        };
        if !disabled {
            window.handle_input(
                &focus_handle,
                ElementInputHandler::new(bounds, self.input.clone()),
                cx,
            );
        }
        for selection in prepaint.selection.drain(..) {
            window.paint_quad(selection);
        }

        let layout = prepaint.layout.take().expect("input layout must exist");
        let mut rows_before = 0;
        for line in &layout.lines {
            let origin = point(
                prepaint.text_bounds.left(),
                bounds.top() + layout.line_height * rows_before as f32,
            );
            line.paint(
                origin,
                layout.line_height,
                gpui::TextAlign::Left,
                Some(prepaint.viewport_bounds),
                window,
                cx,
            )
            .expect("failed to paint input text");
            rows_before += line.wrap_boundaries().len() + 1;
        }

        if !disabled
            && focus_handle.is_focused(window)
            && let Some(cursor) = prepaint.cursor.take()
        {
            window.paint_quad(cursor);
        }

        let old_rows = self
            .input
            .read(cx)
            .last_layout
            .as_ref()
            .map(TextLayout::visual_row_count);
        let new_rows = layout.visual_row_count();
        let rows_changed = old_rows != Some(new_rows);
        self.input.update(cx, |input, cx| {
            input.last_layout = Some(layout);
            input.last_bounds = Some(prepaint.text_bounds);
            input.last_viewport_bounds = Some(prepaint.viewport_bounds);
            if !multiline {
                input.single_line_scroll_offset = prepaint.horizontal_offset;
                if focus_handle.is_focused(window) && scroll_cursor_pending {
                    input.scroll_cursor_pending = false;
                }
            }
            if rows_changed {
                cx.notify();
            }
        });

        if multiline && focus_handle.is_focused(window) && scroll_cursor_pending && !rows_changed {
            let scroll_changed = prepaint
                .cursor_bounds
                .is_some_and(|cursor_bounds| keep_cursor_visible(cursor_bounds, &scroll_handle));
            self.input.update(cx, |input, _| {
                input.scroll_cursor_pending = false;
            });
            if scroll_changed {
                cx.notify(self.input.entity_id());
            }
        }
    }
}

pub(super) fn selection_quads(
    layout: &TextLayout,
    selected: Range<usize>,
    bounds: Bounds<Pixels>,
    color: gpui::Hsla,
) -> Vec<PaintQuad> {
    let mut quads = Vec::new();
    let mut rows_before = 0;
    for (line_ix, line) in layout.lines.iter().enumerate() {
        let line_start = layout.line_starts[line_ix];
        let row_starts = std::iter::once(0)
            .chain(
                line.wrap_boundaries()
                    .iter()
                    .map(|boundary| line.runs()[boundary.run_ix].glyphs[boundary.glyph_ix].index),
            )
            .collect::<Vec<_>>();
        let row_ends = row_starts
            .iter()
            .copied()
            .skip(1)
            .chain(std::iter::once(line.len()))
            .collect::<Vec<_>>();

        for (row_ix, (row_start, row_end)) in row_starts.into_iter().zip(row_ends).enumerate() {
            let start = selected.start.max(line_start + row_start);
            let end = selected.end.min(line_start + row_end);
            if start >= end {
                continue;
            }
            let local_start = start - line_start;
            let local_end = end - line_start;
            let start_position = line
                .position_for_index(local_start, layout.line_height)
                .unwrap_or_default();
            let end_position = line
                .position_for_index(local_end, layout.line_height)
                .unwrap_or_default();
            let x_start = if local_start == row_start {
                px(0.)
            } else {
                start_position.x
            };
            let width = (end_position.x - x_start).max(px(1.));
            quads.push(fill(
                Bounds::new(
                    point(
                        bounds.left() + x_start,
                        bounds.top() + layout.line_height * (rows_before + row_ix) as f32,
                    ),
                    size(width, layout.line_height),
                ),
                color,
            ));
        }
        rows_before += line.wrap_boundaries().len() + 1;
    }
    quads
}

fn keep_cursor_visible(cursor: Bounds<Pixels>, scroll: &gpui::ScrollHandle) -> bool {
    let viewport = scroll.bounds();
    if viewport.size.height <= px(0.) {
        return false;
    }
    let mut offset = scroll.offset();
    if cursor.top() < viewport.top() {
        offset.y += viewport.top() - cursor.top();
    } else if cursor.bottom() > viewport.bottom() {
        offset.y -= cursor.bottom() - viewport.bottom();
    }
    let max_offset = scroll.max_offset();
    offset.y = offset.y.clamp(-max_offset.y, px(0.));
    if offset != scroll.offset() {
        scroll.set_offset(offset);
        true
    } else {
        false
    }
}
