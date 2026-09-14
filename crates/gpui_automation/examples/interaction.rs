use anyhow::Result;
use gpui::{
    AccessibleAction, AnyWindowHandle, App, AsyncApp, Bounds, Context, FocusHandle, Role, Window,
    WindowBounds, WindowOptions, accesskit::ActionData, div, prelude::*, px, rgb, size,
};
use gpui_automation::{Selector, Session};
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(3);

struct Controls {
    value: String,
    count: usize,
    running: bool,
    result: String,
    focus: FocusHandle,
}

impl Controls {
    fn run(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.running {
            return;
        }
        self.running = true;
        self.count = 0;
        self.value.clear();
        self.result = "Running".into();
        cx.notify();
        let handle = window.window_handle();
        cx.spawn(async move |this, cx| {
            let result = verify(handle, cx).await;
            this.update(cx, |this, cx| {
                this.running = false;
                this.result = match result {
                    Ok(()) => "Passed: value, focus and activation verified".into(),
                    Err(error) => format!("Failed: {error:#}"),
                };
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

async fn verify(handle: AnyWindowHandle, cx: &mut AsyncApp) -> Result<()> {
    Session::wait_for(
        handle,
        TIMEOUT,
        |snapshot| Ok(snapshot.get_by_id("result").one()?.label.as_deref() == Some("Running")),
        cx,
    )
    .await?;

    let field = Selector::id("value");
    let generation = cx.update(|cx| Session::set_value(handle, &field, "Ready to automate", cx))?;
    Session::wait_for(
        handle,
        TIMEOUT,
        |snapshot| {
            Ok(snapshot.data().generation > generation
                && snapshot.find(&field).one()?.value.as_deref() == Some("Ready to automate"))
        },
        cx,
    )
    .await?;

    let generation = cx.update(|cx| Session::focus(handle, &field, cx))?;
    Session::wait_for(
        handle,
        TIMEOUT,
        |snapshot| {
            Ok(snapshot.data().generation > generation && snapshot.find(&field).one()?.focused)
        },
        cx,
    )
    .await?;

    let generation = cx.update(|cx| Session::click(handle, &Selector::id("increment"), cx))?;
    Session::wait_for(
        handle,
        TIMEOUT,
        |snapshot| {
            Ok(snapshot.data().generation > generation
                && snapshot.get_by_id("count").one()?.numeric_value == Some(1.0))
        },
        cx,
    )
    .await?;
    Ok(())
}

impl Render for Controls {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.entity().downgrade();
        div()
            .size_full()
            .p_8()
            .flex()
            .flex_col()
            .gap_4()
            .bg(rgb(0x121b29))
            .text_color(rgb(0xe6edf5))
            .child(div().text_xl().child("Semantic automation"))
            .child("Inspect · Set value · Focus · Activate · Verify")
            .child(
                div()
                    .automation_id("value")
                    .role(Role::TextInput)
                    .track_focus(&self.focus)
                    .aria_label("Value")
                    .aria_value(self.value.clone())
                    .on_a11y_action(AccessibleAction::SetValue, move |data, _, cx| {
                        if let Some(ActionData::Value(value)) = data {
                            entity
                                .update(cx, |this, cx| {
                                    this.value = value.to_string();
                                    cx.notify();
                                })
                                .ok();
                        }
                    })
                    .p_4()
                    .rounded_lg()
                    .border_1()
                    .border_color(if self.focus.is_focused(window) {
                        rgb(0x6cd4df)
                    } else {
                        rgb(0x38485f)
                    })
                    .bg(rgb(0x1c2a3e))
                    .child(if self.value.is_empty() {
                        "Value is empty".into()
                    } else {
                        self.value.clone()
                    }),
            )
            .child(
                div()
                    .automation_id("count")
                    .role(Role::Label)
                    .aria_numeric_value(self.count as f64)
                    .child(format!("Count: {}", self.count)),
            )
            .child(
                div()
                    .flex()
                    .gap_3()
                    .child(
                        div()
                            .automation_id("increment")
                            .role(Role::Button)
                            .aria_label("Increment")
                            .px_4()
                            .py_2()
                            .rounded_lg()
                            .bg(rgb(0x304b68))
                            .cursor_pointer()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.count += 1;
                                cx.notify();
                            }))
                            .child("Increment"),
                    )
                    .child(
                        div()
                            .automation_id("run")
                            .role(Role::Button)
                            .aria_label("Run automation")
                            .aria_disabled(self.running)
                            .px_4()
                            .py_2()
                            .rounded_lg()
                            .bg(rgb(0x256677))
                            .cursor_pointer()
                            .on_click(cx.listener(|this, _, window, cx| this.run(window, cx)))
                            .child("Run automation"),
                    ),
            )
            .child(
                div()
                    .automation_id("result")
                    .role(Role::Status)
                    .aria_label(self.result.clone())
                    .child(self.result.clone()),
            )
    }
}

fn main() {
    gpui_platform::application().run(|cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(660.), px(400.)), cx);
        let handle = cx
            .open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    ..Default::default()
                },
                |window, cx| {
                    window.set_window_title("Automation interaction");
                    cx.new(|cx| Controls {
                        value: String::new(),
                        count: 0,
                        running: false,
                        result: "Ready".into(),
                        focus: cx.focus_handle(),
                    })
                },
            )
            .unwrap();
        Session::enable(handle.into(), cx).unwrap();
        cx.activate(true);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    async fn verifies_the_example_workflow(cx: &mut gpui::TestAppContext) {
        let handle = cx.open_window(size(px(660.), px(400.)), |_, cx| Controls {
            value: String::new(),
            count: 0,
            running: false,
            result: "Ready".into(),
            focus: cx.focus_handle(),
        });
        cx.update(|cx| Session::enable(handle.into(), cx).unwrap());
        handle
            .update(cx, |this, window, cx| this.run(window, cx))
            .unwrap();
        let snapshot = Session::wait_for(
            handle.into(),
            TIMEOUT,
            |snapshot| {
                let label = snapshot
                    .get_by_id("result")
                    .one()?
                    .label
                    .as_deref()
                    .unwrap_or("");
                Ok(label.starts_with("Passed") || label.starts_with("Failed"))
            },
            &mut cx.to_async(),
        )
        .await
        .unwrap();
        let result = snapshot
            .get_by_id("result")
            .one()
            .unwrap()
            .label
            .as_deref()
            .unwrap();
        assert!(result.starts_with("Passed"), "{result}");
    }
}
