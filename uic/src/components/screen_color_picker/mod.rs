//! A styled button that invokes the platform's screen-color sampler.
mod state;
pub use state::{ScreenColorPickerEvent, ScreenColorPickerState};

use gpui::{
    AnyElement, App, Entity, Hsla, IntoElement, RenderOnce, Role, SharedString, StyleRefinement,
    Styled, Window, div, prelude::*, px, rgb, rgba, svg,
};

/// Opens the platform sampler on click and emits results through its shared state.
/// Styling applies to the button; the platform provides the sampling interface.
/// The button does not install keyboard bindings or move application focus.
#[derive(IntoElement)]
pub struct ScreenColorPicker {
    state: Entity<ScreenColorPickerState>,
    style: StyleRefinement,
    label: SharedString,
    busy_label: SharedString,
    content: Option<AnyElement>,
    disabled: bool,
    show_label: bool,
    icon_color: Option<Hsla>,
}

impl ScreenColorPicker {
    pub fn new(state: &Entity<ScreenColorPickerState>) -> Self {
        Self {
            state: state.clone(),
            style: StyleRefinement::default(),
            label: "Pick screen color".into(),
            busy_label: "Picking…".into(),
            content: None,
            disabled: false,
            show_label: false,
            icon_color: None,
        }
    }
    /// Shows text beside the pipette icon. The default is an icon-only button.
    pub fn show_label(mut self, show: bool) -> Self {
        self.show_label = show;
        self
    }

    /// Overrides the default icon color independently of the label.
    pub fn icon_color(mut self, color: impl Into<Hsla>) -> Self {
        self.icon_color = Some(color.into());
        self
    }

    /// Sets the accessible name and optional button text.
    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = label.into();
        self
    }
    /// Sets the text displayed while the sampler is open.
    pub fn busy_label(mut self, label: impl Into<SharedString>) -> Self {
        self.busy_label = label.into();
        self
    }
    /// Supplies custom idle content, such as an icon and text.
    pub fn child(mut self, content: impl IntoElement) -> Self {
        self.content = Some(content.into_any_element());
        self
    }
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

impl Styled for ScreenColorPicker {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl RenderOnce for ScreenColorPicker {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        window.use_keyed_state(
            ("screen-color-picker-observer", self.state.entity_id()),
            cx,
            |_, cx| cx.observe(&self.state, |_, _, cx| cx.notify()),
        );
        let busy = self.state.read(cx).is_busy();
        let disabled = self.disabled || busy;
        let color = self
            .icon_color
            .or(self.style.text.color)
            .unwrap_or_else(|| rgb(0xe6edf5).into());
        let label = if busy {
            self.busy_label.clone()
        } else {
            self.label.clone()
        };
        let default_content = || {
            div()
                .flex()
                .items_center()
                .justify_center()
                .gap(px(8.))
                .child(
                    svg()
                        .path(if busy {
                            crate::assets::LucideIcons::LoaderCircle.path()
                        } else {
                            crate::assets::LucideIcons::Pipette.path()
                        })
                        .size(px(18.))
                        .text_color(color),
                )
                .when(self.show_label, |content| content.child(label))
                .into_any_element()
        };
        let content = if busy {
            default_content()
        } else {
            self.content.unwrap_or_else(default_content)
        };
        let state = self.state;
        let mut button = div()
            .id(("screen-color-picker", state.entity_id()))
            .debug_selector(|| "uic-screen-color-picker".into())
            .role(Role::Button)
            .aria_label(self.label)
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .min_w(px(36.))
            .h(px(36.))
            .px(px(9.))
            .rounded(px(10.))
            .border_1()
            .border_color(rgba(0xffffff18))
            .bg(rgb(0x303339))
            .text_color(rgb(0xe6edf5))
            .text_size(px(14.))
            .child(content)
            .when(!disabled, |button| {
                button
                    .cursor_pointer()
                    .hover(|style| style.bg(rgb(0x41464e)).border_color(rgba(0xffffff38)))
                    .active(|style| style.bg(rgb(0x25282e)).border_color(rgb(0x6ea9ed)))
                    .on_click(move |_, _, cx| {
                        state.update(cx, |state, cx| {
                            state.pick(cx);
                        });
                        cx.stop_propagation();
                    })
            });
        button.style().refine(&self.style);
        button.when(disabled, |button| button.opacity(0.5))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{
        AppContext, Context, FocusHandle, Modifiers, Render, TestAppContext, VisualTestContext,
        point, size,
    };

    struct Host {
        state: Entity<ScreenColorPickerState>,
        disabled: bool,
        focus: FocusHandle,
    }
    impl Render for Host {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().track_focus(&self.focus).p(px(20.)).child(
                ScreenColorPicker::new(&self.state)
                    .disabled(self.disabled)
                    .w(px(180.))
                    .h(px(44.))
                    .label("Sample")
                    .child("Sample"),
            )
        }
    }

    #[gpui::test]
    fn button_respects_disabled_styling_and_preserves_host_focus(cx: &mut TestAppContext) {
        let state = cx.new(ScreenColorPickerState::new);
        let window = cx.open_window(size(px(300.), px(160.)), |window, cx| {
            let focus = cx.focus_handle();
            focus.focus(window, cx);
            Host {
                state: state.clone(),
                disabled: true,
                focus,
            }
        });
        let mut visual = VisualTestContext::from_window(window.into(), cx);
        visual.update(|window, cx| {
            window.activate_window();
            window.draw(cx).clear();
        });
        let bounds = visual.debug_bounds("uic-screen-color-picker").unwrap();
        assert_eq!(bounds.size, size(px(180.), px(44.)));
        visual.simulate_click(bounds.center(), Modifiers::default());
        visual.run_until_parked();
        visual.update(|_, cx| assert!(state.read(cx).error().is_none()));
        window
            .update(&mut visual.cx, |this, _, cx| {
                this.disabled = false;
                cx.notify();
            })
            .unwrap();
        visual.update(|window, cx| window.draw(cx).clear());
        visual.simulate_click(
            point(bounds.center().x, bounds.center().y),
            Modifiers::default(),
        );
        visual.run_until_parked();
        window
            .update(&mut visual.cx, |this, window, cx| {
                assert!(this.focus.is_focused(window));
                assert!(!state.read(cx).is_busy());
                assert!(state.read(cx).error().is_some());
            })
            .unwrap();
    }
}
