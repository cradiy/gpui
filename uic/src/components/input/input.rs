use gpui::{
    AnyElement, App, CursorStyle, Entity, IntoElement, MouseButton, Refineable as _, RenderOnce,
    StyleRefinement, Styled, Window, div, prelude::*, px,
};

use super::{InputAppearance, TextInput};
use crate::components::scrollbar::Scrollbar;

#[derive(IntoElement)]
pub struct Input {
    state: Entity<TextInput>,
    prefix: Option<AnyElement>,
    suffix: Option<AnyElement>,
    appearance: InputAppearance,
    configure_scrollbar: Option<Box<dyn FnOnce(Scrollbar) -> Scrollbar>>,
    rows: Option<usize>,
    style: StyleRefinement,
}

input_appearance!(Input);

impl Input {
    pub fn new(state: &Entity<TextInput>) -> Self {
        Self {
            state: state.clone(),
            prefix: None,
            suffix: None,
            appearance: InputAppearance::default(),
            configure_scrollbar: None,
            rows: None,
            style: StyleRefinement::default(),
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

    pub fn appearance(mut self, appearance: InputAppearance) -> Self {
        self.appearance = appearance;
        self
    }

    /// Configures the multi-line input's scrollbar after its defaults are applied.
    ///
    /// Use Styled methods for the cursor, track size, position, and background;
    /// use `appearance` for thumb states and `auto_hide` for visibility behavior.
    /// Increase the input's right padding when widening the overlaid scrollbar.
    /// Repeated calls apply in order.
    pub fn scrollbar(mut self, configure: impl FnOnce(Scrollbar) -> Scrollbar + 'static) -> Self {
        let previous = self.configure_scrollbar.take();
        self.configure_scrollbar = Some(Box::new(move |scrollbar| {
            configure(match previous {
                Some(previous) => previous(scrollbar),
                None => scrollbar,
            })
        }));
        self
    }
}

impl RenderOnce for Input {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let appearance = self.appearance;
        self.state
            .update(cx, |state, _cx| state.appearance = appearance);

        let (focused, focus_handle, disabled, multiline, scroll_handle, scrollbar_state) = {
            let state = self.state.read(cx);
            (
                state.focus_handle.is_focused(window),
                state.focus_handle.clone(),
                state.disabled,
                state.mode == super::InputMode::Multiline,
                state.scroll_handle.clone(),
                state.scrollbar_state.clone(),
            )
        };
        let scrollbar_id = ("uic-input-scrollbar", self.state.entity_id());

        let row_height = self
            .rows
            .map(|rows| super::row_height(&self.style, rows, window.rem_size()));

        let mut element = div()
            .relative()
            .flex()
            .when(multiline, |this| this.items_start())
            .when(!multiline, |this| this.items_center())
            .w_full()
            .h(px(44.))
            .when_some(row_height.filter(|_| multiline), |this, height| {
                this.h(height)
            })
            .px(px(14.))
            .when(multiline, |this| this.py(px(10.)))
            .gap(px(10.))
            .text_size(px(16.))
            .line_height(px(24.))
            .text_color(gpui::hsla(0., 0., 0.08, 1.))
            .rounded(px(10.))
            .border(px(1.))
            .border_color(if focused && !disabled {
                appearance.focus_border
            } else {
                gpui::hsla(0., 0., 0.75, 1.)
            })
            .bg(gpui::hsla(0., 0., 1., 1.))
            .opacity(if disabled { 0.6 } else { 1.0 })
            .cursor(if disabled {
                CursorStyle::Arrow
            } else {
                CursorStyle::IBeam
            })
            .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                if !disabled {
                    window.focus(&focus_handle, cx);
                }
            })
            .children(self.prefix)
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_w_0()
                    .when(multiline, |this| this.h_full())
                    .child(self.state),
            )
            .children(self.suffix)
            .when(multiline, |this| {
                this.child(
                    Scrollbar::vertical(scrollbar_id, &scrollbar_state, &scroll_handle)
                        .auto_hide(false)
                        .absolute()
                        .right(px(2.))
                        .top(px(4.))
                        .bottom(px(4.))
                        .h_auto()
                        .when_some(self.configure_scrollbar, |scrollbar, configure| {
                            configure(scrollbar)
                        }),
                )
            });
        element.style().refine(&self.style);
        if focused && !disabled {
            element = element.border_color(appearance.focus_border);
        }
        element
    }
}

impl Styled for Input {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}
