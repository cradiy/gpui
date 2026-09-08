mod appearance;
mod modal;

use gpui::{
    AnyElement, App, Context, Entity, FocusHandle, Global, KeyDownEvent, MouseButton,
    Refineable as _, Render, Window, WindowId, deferred, div, prelude::*,
};

pub use appearance::{ModalAppearance, ModalButtonAppearance};
use modal::ModalFooter;
pub use modal::{Modal, ModalPlacement};

use super::input::Submit;

const MODAL_PRIORITY: usize = 1_000;

struct ActiveModal {
    modal: Modal,
    window_id: WindowId,
    previous_focus: Option<FocusHandle>,
}

pub struct ModalLayer {
    active: Option<ActiveModal>,
    focus_handle: FocusHandle,
    appearance: ModalAppearance,
}

impl ModalLayer {
    fn new(appearance: ModalAppearance, cx: &mut Context<Self>) -> Self {
        Self {
            active: None,
            focus_handle: cx.focus_handle(),
            appearance,
        }
    }

    pub fn is_open(&self) -> bool {
        self.active.is_some()
    }

    fn show(&mut self, modal: Modal, window: &mut Window, cx: &mut Context<Self>) {
        self.active = Some(ActiveModal {
            modal,
            window_id: window.window_handle().window_id(),
            previous_focus: window.focused(cx),
        });
        window.focus(&self.focus_handle, cx);
        cx.notify();
    }

    fn dismiss(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(active) = self.active.take() else {
            return;
        };

        if active.window_id != window.window_handle().window_id() {
            self.active = Some(active);
            return;
        }
        if let Some(previous_focus) = active.previous_focus {
            window.focus(&previous_focus, cx);
        }
        cx.notify();
    }

    fn execute_ok(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let should_close = self
            .active
            .as_ref()
            .and_then(|active| active.modal.on_ok.as_ref())
            .is_none_or(|callback| callback(window, cx));
        if should_close {
            self.dismiss(window, cx);
        }
    }

    fn default_button(
        id: &'static str,
        label: AnyElement,
        appearance: ModalButtonAppearance,
    ) -> AnyElement {
        div()
            .id(id)
            .h(appearance.height)
            .px(appearance.padding_x)
            .flex()
            .items_center()
            .justify_center()
            .rounded(appearance.radius)
            .border(appearance.border_width)
            .border_color(appearance.border)
            .bg(appearance.background)
            .text_color(appearance.foreground)
            .cursor_pointer()
            .hover(move |style| style.bg(appearance.hover_background))
            .child(label)
            .into_any_element()
    }
}

impl Render for ModalLayer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(active) = self.active.as_ref() else {
            return div().into_any_element();
        };
        if active.window_id != window.window_handle().window_id() {
            return div().into_any_element();
        }

        let modal = &active.modal;
        let appearance = modal.appearance.unwrap_or(self.appearance);
        let close_on_escape = modal.close_on_escape;
        let close_on_backdrop = modal.close_on_backdrop;
        let ok_on_enter = modal.ok_on_enter && matches!(&modal.footer, ModalFooter::Default);
        let styled = modal.styled;
        let placement = modal.placement;
        let panel_style = modal.style.clone();
        let title = modal.title.as_ref().map(|slot| slot(window, cx));
        let close_button = modal.close_button.as_ref().map(|slot| slot(window, cx));
        let content = (modal.content)(window, cx);

        let mut panel = div()
            .id("global-modal-content")
            .relative()
            .flex()
            .flex_col()
            .overflow_hidden()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation());

        if styled {
            panel.style().refine(&panel_style);
        }

        if title.is_some() || close_button.is_some() {
            let header = div()
                .relative()
                .flex()
                .items_center()
                .justify_between()
                .px(appearance.header_padding_x)
                .py(appearance.header_padding_y)
                .when(styled && appearance.section_borders, |this| {
                    this.border_b_1().border_color(appearance.section_border)
                })
                .children(title)
                .when_some(close_button, |this, button| {
                    this.child(
                        div()
                            .id("global-modal-close")
                            .cursor_pointer()
                            .on_click(cx.listener(|layer, _, window, cx| {
                                layer.dismiss(window, cx);
                                cx.stop_propagation();
                            }))
                            .child(button),
                    )
                });
            panel = panel.child(header);
        }

        let body = div()
            .id("global-modal-body")
            .flex_1()
            .overflow_y_scroll()
            .when(styled, |this| {
                this.px(appearance.body_padding_x)
                    .py(appearance.body_padding_y)
            })
            .child(content);
        panel = panel.child(body);

        panel = match &modal.footer {
            ModalFooter::Hidden => panel,
            ModalFooter::Custom(footer) => panel.child(footer(window, cx)),
            ModalFooter::Default => {
                let cancel = match modal.cancel_button.as_ref() {
                    Some(button) => button(window, cx),
                    None => Self::default_button(
                        "global-modal-cancel-default",
                        (modal.cancel_text)(window, cx),
                        appearance.cancel_button,
                    ),
                };
                let ok = match modal.ok_button.as_ref() {
                    Some(button) => button(window, cx),
                    None => Self::default_button(
                        "global-modal-ok-default",
                        (modal.ok_text)(window, cx),
                        appearance.ok_button,
                    ),
                };
                let on_cancel = modal.on_cancel.clone();

                let footer = div()
                    .flex()
                    .items_center()
                    .justify_end()
                    .gap(appearance.footer_gap)
                    .px(appearance.footer_padding_x)
                    .py(appearance.footer_padding_y)
                    .when(styled && appearance.section_borders, |this| {
                        this.border_t_1().border_color(appearance.section_border)
                    })
                    .child(
                        div()
                            .id("global-modal-cancel")
                            .on_click(cx.listener(move |layer, _, window, cx| {
                                let should_close = on_cancel
                                    .as_ref()
                                    .is_none_or(|callback| callback(window, cx));
                                if should_close {
                                    layer.dismiss(window, cx);
                                }
                                cx.stop_propagation();
                            }))
                            .child(cancel),
                    )
                    .child(
                        div()
                            .id("global-modal-ok")
                            .on_click(cx.listener(move |layer, _, window, cx| {
                                layer.execute_ok(window, cx);
                                cx.stop_propagation();
                            }))
                            .child(ok),
                    );
                panel.child(footer)
            }
        };

        let backdrop = div()
            .id("global-modal-backdrop")
            .absolute()
            .inset_0()
            .flex()
            .justify_center()
            .track_focus(&self.focus_handle)
            .bg(appearance.backdrop)
            .occlude()
            .capture_action(cx.listener(move |layer, _: &Submit, window, cx| {
                if ok_on_enter {
                    layer.execute_ok(window, cx);
                    cx.stop_propagation();
                }
            }))
            .capture_key_down(cx.listener(move |layer, event: &KeyDownEvent, window, cx| {
                if close_on_escape && event.keystroke.key == "escape" {
                    layer.dismiss(window, cx);
                    cx.stop_propagation();
                } else if ok_on_enter
                    && event.keystroke.key == "enter"
                    && !window
                        .context_stack()
                        .iter()
                        .any(|context| context.contains("multiline"))
                {
                    layer.execute_ok(window, cx);
                    cx.stop_propagation();
                }
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |layer, _, window, cx| {
                    if close_on_backdrop {
                        layer.dismiss(window, cx);
                    }
                    cx.stop_propagation();
                }),
            );

        let backdrop = match placement {
            ModalPlacement::Center => backdrop.items_center(),
            ModalPlacement::Top { offset } => backdrop.items_start().pt(offset),
        };

        deferred(backdrop.child(panel))
            .with_priority(MODAL_PRIORITY)
            .into_any_element()
    }
}

struct GlobalModal(Entity<ModalLayer>);

impl Global for GlobalModal {}

pub fn init(cx: &mut App) {
    init_with_appearance(ModalAppearance::default(), cx);
}

pub fn init_with_appearance(appearance: ModalAppearance, cx: &mut App) {
    if cx.has_global::<GlobalModal>() {
        set_appearance(appearance, cx);
    } else {
        let layer = cx.new(|cx| ModalLayer::new(appearance, cx));
        cx.set_global(GlobalModal(layer));
    }
}

pub fn set_appearance(appearance: ModalAppearance, cx: &mut App) {
    layer(cx).update(cx, |layer, cx| {
        layer.appearance = appearance;
        cx.notify();
    });
}

/// Mount this once as the last child of each window root.
pub fn layer(cx: &App) -> Entity<ModalLayer> {
    cx.global::<GlobalModal>().0.clone()
}

pub fn show(modal: Modal, window: &mut Window, cx: &mut App) {
    layer(cx).update(cx, |layer, cx| layer.show(modal, window, cx));
}

pub fn dismiss(window: &mut Window, cx: &mut App) {
    layer(cx).update(cx, |layer, cx| layer.dismiss(window, cx));
}

pub fn is_open(cx: &App) -> bool {
    layer(cx).read(cx).is_open()
}

#[cfg(test)]
mod tests {
    use gpui::{
        Context, Entity, Focusable, IntoElement, Keystroke, Render, TestAppContext,
        VisualTestContext, Window, div, px, size,
    };

    use super::*;
    use crate::components::input::{Input, TextInput};

    struct ModalInputTest {
        input: Entity<TextInput>,
        modal_layer: Entity<ModalLayer>,
    }

    impl Render for ModalInputTest {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().size_full().child(self.modal_layer.clone())
        }
    }

    #[gpui::test]
    fn multiline_enter_does_not_accept_the_modal(cx: &mut TestAppContext) {
        cx.update(|cx| {
            crate::components::input::init(cx);
            init(cx);
        });
        let window = cx.open_window(size(px(420.), px(320.)), |_, cx| ModalInputTest {
            input: cx.new(|cx| TextInput::new(cx).multiline()),
            modal_layer: layer(cx),
        });
        window
            .update(cx, |view, window, cx| {
                let input = view.input.clone();
                show(Modal::new(move |_, _| Input::new(&input)), window, cx);
            })
            .unwrap();

        let mut visual = VisualTestContext::from_window(window.into(), cx);
        visual.update(|window, cx| {
            window.draw(cx).clear();
        });
        window
            .update(&mut visual.cx, |view, window, cx| {
                let focus_handle = view.input.read(cx).focus_handle(cx);
                window.focus(&focus_handle, cx);
            })
            .unwrap();
        visual.update(|window, cx| {
            window.draw(cx).clear();
        });
        visual.update(|window, cx| {
            window.dispatch_keystroke(Keystroke::parse("enter").unwrap(), cx);
        });

        window
            .update(&mut visual.cx, |view, _, cx| {
                assert!(is_open(cx));
                assert_eq!(view.input.read(cx).value().as_ref(), "\n");
            })
            .unwrap();
    }
}
