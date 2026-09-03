use gpui::{
    App, Bounds, Context, Entity, FontWeight, Render, Subscription, Window, WindowBounds,
    WindowOptions, div, prelude::*, px, rgb, size,
};
use gpui_platform::application;
use uic::assets::LucideAssets;
use uic::components::collapsible::{
    Collapsible, CollapsibleIndicatorPosition, CollapsibleMode, CollapsibleState,
};

struct CollapsibleExample {
    multiple: Entity<CollapsibleState>,
    single: Entity<CollapsibleState>,
    _subscriptions: Vec<Subscription>,
}

impl CollapsibleExample {
    fn new(cx: &mut Context<Self>) -> Self {
        let multiple = cx
            .new(|_| CollapsibleState::new(CollapsibleMode::Multiple).with_expanded(["overview"]));
        let single =
            cx.new(|_| CollapsibleState::new(CollapsibleMode::Single).with_expanded(["account"]));
        let multiple_subscription = cx.subscribe(&multiple, |_, _, _, cx| cx.notify());
        let single_subscription = cx.subscribe(&single, |_, _, _, cx| cx.notify());
        Self {
            multiple,
            single,
            _subscriptions: vec![multiple_subscription, single_subscription],
        }
    }
}

impl Render for CollapsibleExample {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .bg(rgb(0xf4f7fb))
            .p_8()
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .w(px(780.))
                    .flex()
                    .flex_col()
                    .gap_8()
                    .child(
                        section("Independent panels", "Open any combination of sections.")
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap_3()
                                    .child(
                                        Collapsible::new(
                                            "overview-panel",
                                            &self.multiple,
                                            "overview",
                                        )
                                        .header_text("Project overview")
                                        .content(body(
                                            "A compact summary can live here without imposing a surrounding page layout.",
                                        )),
                                    )
                                    .child(
                                        Collapsible::new(
                                            "permissions-panel",
                                            &self.multiple,
                                            "permissions",
                                        )
                                        .header(
                                            div()
                                                .flex()
                                                .items_center()
                                                .justify_between()
                                                .child("Permissions")
                                                .child(
                                                    div()
                                                        .px_2()
                                                        .py_1()
                                                        .rounded_full()
                                                        .bg(rgb(0xeef2ff))
                                                        .text_xs()
                                                        .text_color(rgb(0x4263eb))
                                                        .child("4 members"),
                                                ),
                                        )
                                        .aria_label("Permissions")
                                        .content(body(
                                            "Header content is arbitrary, while the component keeps interaction and accessibility behavior.",
                                        )),
                                    )
                                    .child(
                                        Collapsible::new(
                                            "disabled-panel",
                                            &self.multiple,
                                            "disabled",
                                        )
                                        .header_text("Unavailable section")
                                        .content(body("This content cannot be opened."))
                                        .disabled(true),
                                    ),
                            ),
                    )
                    .child(
                        section("Accordion", "Only one section remains open.").child(
                            div()
                                .flex()
                                .flex_col()
                                .gap_3()
                                .child(
                                    Collapsible::new("account-panel", &self.single, "account")
                                        .header_text("Account")
                                        .indicator_position(CollapsibleIndicatorPosition::End)
                                        .content(body(
                                            "Profile, identity, and sign-in preferences.",
                                        )),
                                )
                                .child(
                                    Collapsible::new(
                                        "notifications-panel",
                                        &self.single,
                                        "notifications",
                                    )
                                    .header_text("Notifications")
                                    .indicator_position(CollapsibleIndicatorPosition::End)
                                    .content(body(
                                        "Email, desktop, and push notification preferences.",
                                    )),
                                ),
                        ),
                    ),
            )
    }
}

fn section(title: &'static str, description: &'static str) -> gpui::Div {
    div().flex().flex_col().gap_3().child(
        div()
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .text_lg()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(title),
            )
            .child(div().text_sm().text_color(rgb(0x667085)).child(description)),
    )
}

fn body(text: &'static str) -> gpui::Div {
    div().text_sm().line_height(px(22.)).child(text)
}

fn main() {
    application()
        .with_assets(LucideAssets::new())
        .run(|cx: &mut App| {
            uic::init(cx);
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                        None,
                        size(px(960.), px(860.)),
                        cx,
                    ))),
                    ..Default::default()
                },
                |_, cx| cx.new(CollapsibleExample::new),
            )
            .expect("failed to open collapsible example");
        });
}
