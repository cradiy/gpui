use super::{InputAppearance, InputMode};
use gpui::{
    AnyElement, CursorStyle, ElementId, InteractiveText, IntoElement, Refineable as _, RenderOnce,
    SharedString, StyleRefinement, Styled, StyledText, div, prelude::*,
};
/// An input-shaped value display, optionally selectable for plain-text copying.
#[derive(IntoElement)]
pub struct ReadOnlyInput {
    value: SharedString,
    prefix: Option<AnyElement>,
    suffix: Option<AnyElement>,
    mode: InputMode,
    appearance: InputAppearance,
    rows: Option<usize>,
    style: StyleRefinement,
    selection_id: Option<ElementId>,
}

input_appearance!(ReadOnlyInput);

impl ReadOnlyInput {
    pub fn new(value: impl Into<SharedString>) -> Self {
        Self {
            value: value.into(),
            prefix: None,
            suffix: None,
            mode: InputMode::Text,
            appearance: InputAppearance::default(),
            rows: None,
            style: StyleRefinement::default(),
            selection_id: None,
        }
    }

    pub fn prefix(mut self, prefix: impl IntoElement) -> Self {
        self.prefix = Some(prefix.into_any_element());
        self
    }

    pub fn suffix(mut self, suffix: impl IntoElement) -> Self {
        self.suffix = Some(suffix.into_any_element());
        self
    }

    pub fn mode(mut self, mode: InputMode) -> Self {
        self.mode = mode;
        self
    }

    pub fn text(mut self) -> Self {
        self.mode = InputMode::Text;
        self
    }

    pub fn password(mut self) -> Self {
        self.mode = InputMode::Password;
        self
    }

    pub fn multiline(mut self) -> Self {
        self.mode = InputMode::Multiline;
        self
    }

    pub fn appearance(mut self, appearance: InputAppearance) -> Self {
        self.appearance = appearance;
        self
    }

    /// Allow selecting and copying the displayed text using a stable element ID.
    /// Password displays remain non-selectable.
    pub fn selectable(mut self, id: impl Into<ElementId>) -> Self {
        self.selection_id = Some(id.into());
        self
    }
}

impl RenderOnce for ReadOnlyInput {
    fn render(self, _window: &mut gpui::Window, _cx: &mut gpui::App) -> impl IntoElement {
        let multiline = self.mode == InputMode::Multiline;
        let display_value = match self.mode {
            InputMode::Text | InputMode::Multiline => self.value,
            InputMode::Password => "•".repeat(self.value.chars().count()).into(),
        };
        let row_height = self
            .rows
            .map(|rows| super::row_height(&self.style, rows, _window.rem_size()));
        let content = if let Some(id) = self
            .selection_id
            .filter(|_| self.mode != InputMode::Password)
        {
            InteractiveText::new(id, StyledText::new(display_value))
                .selectable(self.appearance.selection)
                .into_any_element()
        } else {
            display_value.into_any_element()
        };

        let mut element = div()
            .flex()
            .when(multiline, |this| this.items_start())
            .when(!multiline, |this| this.items_center())
            .w_full()
            .h(gpui::px(44.))
            .when_some(row_height.filter(|_| multiline), |this, height| {
                this.h(height)
            })
            .px(gpui::px(14.))
            .when(multiline, |this| this.py(gpui::px(10.)))
            .gap(gpui::px(10.))
            .rounded(gpui::px(10.))
            .border(gpui::px(1.))
            .border_color(gpui::hsla(0., 0., 0.75, 1.))
            .bg(gpui::hsla(0., 0., 1., 1.))
            .text_size(gpui::px(16.))
            .line_height(gpui::px(24.))
            .text_color(gpui::hsla(0., 0., 0.08, 1.))
            .cursor(CursorStyle::Arrow)
            .children(self.prefix)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .when(multiline, |this| this.h_full().whitespace_normal())
                    .overflow_hidden()
                    .child(content),
            )
            .children(self.suffix);
        element.style().refine(&self.style);
        element
    }
}

impl Styled for ReadOnlyInput {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

#[cfg(test)]
mod tests {
    use gpui::px;

    use super::*;

    struct SelectableReadOnly {
        password: bool,
    }

    impl gpui::Render for SelectableReadOnly {
        fn render(
            &mut self,
            _: &mut gpui::Window,
            _: &mut gpui::Context<Self>,
        ) -> impl IntoElement {
            ReadOnlyInput::new("Read only 中文😀 text")
                .mode(if self.password {
                    InputMode::Password
                } else {
                    InputMode::Text
                })
                .selectable("value")
                .w(px(240.))
        }
    }

    #[gpui::test]
    fn selectable_read_only_copies_text_but_not_passwords(cx: &mut gpui::TestAppContext) {
        for password in [false, true] {
            let window = cx.open_window(gpui::size(px(300.), px(100.)), move |_, _| {
                SelectableReadOnly { password }
            });
            let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
            visual.update(|window, cx| {
                window.draw(cx).clear();
                cx.write_to_clipboard(gpui::ClipboardItem::new_string("unchanged".into()));
            });
            visual.simulate_click(gpui::point(px(50.), px(22.)), Default::default());
            visual.simulate_keystrokes(if cfg!(target_os = "macos") {
                "cmd-a cmd-c"
            } else {
                "ctrl-a ctrl-c"
            });
            visual.update(|_, cx| {
                assert_eq!(
                    cx.read_from_clipboard()
                        .and_then(|item| item.text())
                        .as_deref(),
                    Some(if password {
                        "unchanged"
                    } else {
                        "Read only 中文😀 text"
                    })
                );
            });
        }
    }

    #[test]
    fn rows_are_independent_from_the_semantic_appearance() {
        let automatic = ReadOnlyInput::new("").rows(3).text_size(px(20.));
        assert_eq!(automatic.rows, Some(3));
        assert!(automatic.style.text.font_size.is_some());
    }

    #[test]
    fn explicit_height_is_stored_in_styled() {
        let input = ReadOnlyInput::new("").rows(4).h(px(200.));
        assert_eq!(input.rows, Some(4));
        assert!(input.style.size.height.is_some());
    }
}
