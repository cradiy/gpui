use gpui::{
    AppContext, Context, Entity, IntoElement, Render, ScrollHandle, Window, WindowOptions, div,
    prelude::*, px, rgb,
};
use uic::components::{
    badge::{Badge, BadgeState},
    popover::{Popover, PopoverPlacement, PopoverState},
    scrollbar::{Scrollbar, ScrollbarState},
    selection::{Checkbox, RadioGroup, Switch},
};

#[derive(Clone, Copy, PartialEq)]
enum Density {
    Compact,
    Comfortable,
    Spacious,
}

struct ControlsExample {
    popover: Entity<PopoverState>,
    remember: bool,
    notifications: bool,
    badge: Entity<BadgeState>,
    badge_dismissals: usize,
    density: Density,
    scroll: ScrollHandle,
    scrollbar: ScrollbarState,
}

impl ControlsExample {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            popover: cx.new(|cx| PopoverState::new(window, cx)),
            remember: true,
            notifications: false,
            badge: cx.new(|_| BadgeState::new()),
            badge_dismissals: 0,
            density: Density::Comfortable,
            scroll: ScrollHandle::new(),
            scrollbar: ScrollbarState::new(),
        }
    }
}

impl Render for ControlsExample {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let remember_entity = cx.entity();
        let switch_entity = cx.entity();
        let badge_entity = cx.entity();
        let reset_badge = self.badge.clone();
        let radio_entity = cx.entity();
        let popover_state = self.popover.clone();

        div()
            .size_full()
            .bg(rgb(0xf4f6f8))
            .text_color(rgb(0x172033))
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .w(px(720.))
                    .p_8()
                    .rounded(px(20.))
                    .bg(rgb(0xffffff))
                    .shadow_lg()
                    .flex()
                    .flex_col()
                    .gap_6()
                    .child(div().text_2xl().child("Selection, popover, and scrollbar"))
                    .child(
                        Checkbox::new("remember", self.remember)
                            .label_text("Remember this workspace")
                            .on_change(move |checked, _, cx| {
                                remember_entity.update(cx, |this, cx| {
                                    this.remember = checked;
                                    cx.notify();
                                });
                            }),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child("Desktop notifications")
                            .child(
                                Switch::new("notifications", self.notifications)
                                    .label("Desktop notifications")
                                    .checked_content(
                                        div().flex().items_center().gap_1().child("✓").child("On"),
                                    )
                                    .unchecked_content("Off")
                                    .w(px(68.))
                                    .on_change(move |checked, _, cx| {
                                        switch_entity.update(cx, |this, cx| {
                                            this.notifications = checked;
                                            cx.notify();
                                        });
                                    }),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div().flex().flex_col().gap_1().child("Badge").child(
                                    div().text_sm().text_color(rgb(0x667085)).child(
                                        "Drag the red badge; release nearby to spring back.",
                                    ),
                                ),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_6()
                                    .child(Badge::new("badge-basic", badge_anchor()).count(5))
                                    .child(
                                        Badge::new("badge-max", badge_anchor()).count(120).max(99),
                                    )
                                    .child(Badge::new("badge-dot", badge_anchor()).dot())
                                    .child(
                                        Badge::new("badge-drag", badge_anchor())
                                            .count(8)
                                            .dismissible(&self.badge)
                                            .on_dismiss(move |_, cx| {
                                                badge_entity.update(cx, |this, cx| {
                                                    this.badge_dismissals += 1;
                                                    cx.notify();
                                                });
                                            }),
                                    )
                                    .child(
                                        div()
                                            .id("reset-badge")
                                            .px_3()
                                            .py_1()
                                            .rounded(px(7.))
                                            .border_1()
                                            .border_color(rgb(0xd0d5dd))
                                            .cursor_pointer()
                                            .child(format!("Reset ({})", self.badge_dismissals))
                                            .on_click(move |_, _, cx| {
                                                reset_badge.update(cx, |state, cx| state.reset(cx));
                                            }),
                                    ),
                            ),
                    )
                    .child(
                        RadioGroup::new("density", self.density)
                            .label("Interface density")
                            .option(Density::Compact, "Compact")
                            .option(Density::Comfortable, "Comfortable")
                            .option(Density::Spacious, "Spacious")
                            .on_change(move |density, _, cx| {
                                radio_entity.update(cx, |this, cx| {
                                    this.density = density;
                                    cx.notify();
                                });
                            }),
                    )
                    .child(
                        Popover::new(&self.popover)
                            .label("Open popover")
                            .placement(PopoverPlacement::BottomStart)
                            .trigger(
                                div()
                                    .px_3()
                                    .py_2()
                                    .rounded(px(8.))
                                    .bg(rgb(0x1677ff))
                                    .text_color(rgb(0xffffff))
                                    .child("Open popover"),
                            )
                            .content(move |_, _| {
                                let close_state = popover_state.clone();
                                div()
                                    .w(px(260.))
                                    .flex()
                                    .flex_col()
                                    .gap_3()
                                    .child(
                                        div()
                                            .font_weight(gpui::FontWeight::SEMIBOLD)
                                            .child("Workspace options"),
                                    )
                                    .child("The surface flips when it would overflow the viewport.")
                                    .child(
                                        div()
                                            .id("close-example-popover")
                                            .px_3()
                                            .py_2()
                                            .rounded(px(7.))
                                            .bg(rgb(0xe8f1ff))
                                            .cursor_pointer()
                                            .on_click(move |_, window, cx| {
                                                close_state.update(cx, |state, cx| {
                                                    state.close(window, cx)
                                                });
                                            })
                                            .child("Close"),
                                    )
                            }),
                    )
                    .child(
                        div()
                            .h(px(180.))
                            .rounded(px(10.))
                            .border_1()
                            .border_color(rgb(0xd9dee8))
                            .overflow_hidden()
                            .flex()
                            .child(
                                div()
                                    .id("example-scroll-content")
                                    .flex_1()
                                    .overflow_y_scroll()
                                    .track_scroll(&self.scroll)
                                    .children((1..=40).map(|index| {
                                        div()
                                            .h(px(32.))
                                            .px_3()
                                            .flex()
                                            .items_center()
                                            .border_b_1()
                                            .border_color(rgb(0xebedf2))
                                            .child(format!("Scrollable row {index}"))
                                    })),
                            )
                            .child(
                                Scrollbar::vertical(
                                    "example-scrollbar",
                                    &self.scrollbar,
                                    &self.scroll,
                                )
                                .auto_hide(false),
                            ),
                    ),
            )
    }
}

fn badge_anchor() -> gpui::Div {
    div().size(px(48.)).rounded(px(10.)).bg(rgb(0x5b6472))
}

fn main() {
    gpui_platform::application().run(|cx| {
        cx.open_window(WindowOptions::default(), |window, cx| {
            cx.new(|cx| ControlsExample::new(window, cx))
        })
        .expect("failed to open controls example window");
    });
}
