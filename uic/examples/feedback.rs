use std::time::Duration;

use gpui::{
    Context, IntoElement, Render, SharedString, Window, WindowOptions, div, prelude::*, px, rgb,
};
use uic::{
    assets::LucideAssets,
    components::{
        notification::{self, Notification},
        toast::{self, ToastVariant},
    },
};

struct FeedbackExample;

impl FeedbackExample {
    fn new(cx: &mut Context<Self>) -> Self {
        toast::show(
            "Your changes have been saved",
            ToastVariant::Success,
            Duration::from_secs(30),
            cx,
        );
        notification::show(
            Notification::new(
                "Update available",
                "A new version is ready. Restart the application when it is convenient.",
            )
            .info()
            .persistent(),
            cx,
        );
        Self
    }

    fn button(
        &self,
        id: &'static str,
        label: impl Into<SharedString>,
        on_click: impl Fn(&gpui::ClickEvent, &mut Window, &mut gpui::App) + 'static,
    ) -> impl IntoElement {
        div()
            .id(id)
            .h(px(36.))
            .px_4()
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(8.))
            .border_1()
            .border_color(rgb(0xd9d9d9))
            .bg(rgb(0xffffff))
            .text_sm()
            .text_color(rgb(0x000000).opacity(0.88))
            .cursor_pointer()
            .hover(|style| style.border_color(rgb(0x1677ff)).text_color(rgb(0x1677ff)))
            .on_click(on_click)
            .child(label.into())
    }
}

impl Render for FeedbackExample {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let toast_layer = toast::layer(cx);
        let notification_layer = notification::layer(cx);

        div()
            .relative()
            .size_full()
            .bg(rgb(0xf5f5f5))
            .text_color(rgb(0x000000).opacity(0.88))
            .child(
                div()
                    .w_full()
                    .max_w(px(760.))
                    .mx_auto()
                    .pt(px(160.))
                    .px_8()
                    .flex()
                    .flex_col()
                    .gap_8()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(div().text_3xl().child("Feedback surfaces"))
                            .child(
                                div()
                                    .text_color(rgb(0x000000).opacity(0.55))
                                    .child("Messages stay lightweight; notifications carry detail."),
                            ),
                    )
                    .child(
                        div()
                            .p_6()
                            .rounded(px(12.))
                            .border_1()
                            .border_color(rgb(0x000000).opacity(0.06))
                            .bg(rgb(0xffffff))
                            .shadow_sm()
                            .flex()
                            .flex_col()
                            .gap_4()
                            .child(div().text_lg().child("Toast messages"))
                            .child(
                                div()
                                    .flex()
                                    .flex_wrap()
                                    .gap_3()
                                    .child(self.button("toast-info", "Info", |_, _, cx| {
                                        toast::info("A newer version is available", cx);
                                    }))
                                    .child(self.button("toast-success", "Success", |_, _, cx| {
                                        toast::success("Saved successfully", cx);
                                    }))
                                    .child(self.button("toast-warning", "Warning", |_, _, cx| {
                                        toast::warn("Check the highlighted fields", cx);
                                    }))
                                    .child(self.button("toast-error", "Error", |_, _, cx| {
                                        toast::error("Unable to complete the request", cx);
                                    }))
                                    .child(self.button("toast-loading", "Loading", |_, _, cx| {
                                        toast::loading("Synchronizing data...", cx);
                                    }))
                                    .child(self.button("toast-long-message", "Long message", |_, _, cx| {
                                        toast::show(
                                            "The selected files could not be saved. Please check that the destination folder exists and that you have permission to write to it, then try again.",
                                            ToastVariant::Warn,
                                            Duration::from_secs(15),
                                            cx,
                                        );
                                    }))
                                    .child(self.button("toast-long-path", "Long path", |_, _, cx| {
                                        toast::show(
                                            format!("/home/user/Documents/{}.svg", "VeryLongFileNameWithoutSpaces".repeat(8)),
                                            ToastVariant::Error,
                                            Duration::from_secs(15),
                                            cx,
                                        );
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .p_6()
                            .rounded(px(12.))
                            .border_1()
                            .border_color(rgb(0x000000).opacity(0.06))
                            .bg(rgb(0xffffff))
                            .shadow_sm()
                            .flex()
                            .flex_col()
                            .gap_4()
                            .child(div().text_lg().child("Notifications"))
                            .child(
                                div()
                                    .flex()
                                    .flex_wrap()
                                    .gap_3()
                                    .child(self.button("notification-info", "Info", |_, _, cx| {
                                        notification::info(
                                            "Background task started",
                                            "You can continue working while the task runs.",
                                            cx,
                                        );
                                    }))
                                    .child(self.button(
                                        "notification-success",
                                        "Success",
                                        |_, _, cx| {
                                            notification::success(
                                                "Export complete",
                                                "The archive was written to the selected folder.",
                                                cx,
                                            );
                                        },
                                    ))
                                    .child(self.button(
                                        "notification-warning",
                                        "Warning",
                                        |_, _, cx| {
                                            notification::warn(
                                                "Storage is almost full",
                                                "Free some space to keep automatic backups running.",
                                                cx,
                                            );
                                        },
                                    ))
                                    .child(self.button(
                                        "notification-error",
                                        "Error",
                                        |_, _, cx| {
                                            notification::error(
                                                "Connection lost",
                                                "Reconnect to continue synchronizing your changes.",
                                                cx,
                                            );
                                        },
                                    )),
                            ),
                    ),
            )
            .child(notification_layer)
            .child(toast_layer)
    }
}

fn main() {
    gpui_platform::application()
        .with_assets(LucideAssets::new())
        .run(|cx| {
            uic::init(cx);
            cx.open_window(WindowOptions::default(), |_, cx| {
                cx.new(FeedbackExample::new)
            })
            .expect("failed to open feedback example window");
        });
}
