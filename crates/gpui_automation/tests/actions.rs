use gpui::{
    AccessibleAction, AutomationAction, Context, FocusHandle, Role, TestAppContext, Window,
    accesskit::ActionData, div, prelude::*, px, size,
};
use gpui_automation::{Selector, Session};
use std::time::Duration;

struct Controls {
    clicks: usize,
    value: String,
    disabled: bool,
    present: bool,
    semantic_click: bool,
    focus: FocusHandle,
}

impl Render for Controls {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.entity().downgrade();
        div()
            .size_full()
            .flex()
            .flex_col()
            .when(self.present, |root| {
                root.child(
                    div()
                        .automation_id("increment")
                        .role(Role::Button)
                        .aria_disabled(self.disabled)
                        .size(px(50.))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.clicks += 1;
                            cx.notify();
                        }))
                        .when(self.semantic_click, |button| {
                            let entity = cx.entity().downgrade();
                            button.on_a11y_action(AccessibleAction::Click, move |_, _, cx| {
                                entity
                                    .update(cx, |this, cx| {
                                        this.clicks += 10;
                                        cx.notify();
                                    })
                                    .unwrap();
                            })
                        }),
                )
            })
            .child(
                div()
                    .automation_id("value")
                    .role(Role::TextInput)
                    .track_focus(&self.focus)
                    .aria_value(self.value.clone())
                    .on_a11y_action(AccessibleAction::SetValue, move |data, _, cx| {
                        if let Some(ActionData::Value(value)) = data {
                            entity
                                .update(cx, |this, cx| {
                                    this.value = value.to_string();
                                    cx.notify();
                                })
                                .unwrap();
                        }
                    }),
            )
            .child(div().automation_id("plain").role(Role::TextInput))
    }
}

fn open(cx: &mut TestAppContext) -> gpui::WindowHandle<Controls> {
    let handle = cx.open_window(size(px(320.), px(200.)), |_, cx| Controls {
        clicks: 0,
        value: String::new(),
        disabled: false,
        present: true,
        semantic_click: false,
        focus: cx.focus_handle(),
    });
    cx.update(|cx| Session::enable(handle.into(), cx).unwrap());
    draw(handle, cx);
    handle
}

fn draw(handle: gpui::WindowHandle<Controls>, cx: &mut TestAppContext) {
    cx.update_window(handle.into(), |_, window, cx| window.draw(cx).clear())
        .unwrap();
}

#[gpui::test]
fn actions_change_state_and_reject_stale_or_unavailable_targets(cx: &mut TestAppContext) {
    let handle = open(cx);
    let snapshot = cx.update(|cx| Session::snapshot(handle.into(), cx).unwrap());
    let node = snapshot.get_by_id("increment").one().unwrap();
    let generation = cx.update(|cx| {
        let generation = Session::click(handle.into(), &Selector::id("increment"), cx).unwrap();
        assert!(Session::click(handle.into(), &Selector::id("increment"), cx).is_err());
        generation
    });
    assert_eq!(handle.read_with(cx, |view, _| view.clicks).unwrap(), 1);
    draw(handle, cx);
    cx.update_window(handle.into(), |_, window, cx| {
        assert!(
            window
                .perform_automation_action(generation, node.id, AutomationAction::Click, cx)
                .is_err()
        );
    })
    .unwrap();

    cx.update(|cx| {
        Session::set_value(handle.into(), &Selector::id("value"), "café 世界", cx).unwrap()
    });
    draw(handle, cx);
    let value = cx.update(|cx| Session::snapshot(handle.into(), cx).unwrap());
    assert_eq!(
        value.get_by_id("value").one().unwrap().value.as_deref(),
        Some("café 世界")
    );
    cx.update(|cx| {
        assert!(Session::set_value(handle.into(), &Selector::id("plain"), "unused", cx).is_err());
        Session::focus(handle.into(), &Selector::id("value"), cx).unwrap();
    });
    draw(handle, cx);
    let focused = cx.update(|cx| Session::snapshot(handle.into(), cx).unwrap());
    assert!(focused.get_by_id("value").one().unwrap().focused);

    handle
        .update(cx, |view, _, cx| {
            view.disabled = true;
            cx.notify();
        })
        .unwrap();
    draw(handle, cx);
    cx.update(|cx| assert!(Session::click(handle.into(), &Selector::id("increment"), cx).is_err()));
    assert_eq!(handle.read_with(cx, |view, _| view.clicks).unwrap(), 1);
    handle
        .update(cx, |view, _, cx| {
            view.present = false;
            cx.notify();
        })
        .unwrap();
    draw(handle, cx);
    cx.update(|cx| assert!(Session::click(handle.into(), &Selector::id("increment"), cx).is_err()));
}

#[gpui::test]
fn explicit_semantic_click_takes_precedence_and_updates_with_the_view(cx: &mut TestAppContext) {
    let handle = open(cx);
    handle
        .update(cx, |view, _, cx| {
            view.semantic_click = true;
            cx.notify();
        })
        .unwrap();
    draw(handle, cx);
    cx.update(|cx| Session::click(handle.into(), &Selector::id("increment"), cx).unwrap());
    assert_eq!(handle.read_with(cx, |view, _| view.clicks).unwrap(), 10);
    handle
        .update(cx, |view, _, cx| {
            view.semantic_click = false;
            cx.notify();
        })
        .unwrap();
    draw(handle, cx);
    cx.update(|cx| Session::click(handle.into(), &Selector::id("increment"), cx).unwrap());
    assert_eq!(handle.read_with(cx, |view, _| view.clicks).unwrap(), 11);
}

#[gpui::test]
async fn waits_observe_new_draws_and_report_timeouts(cx: &mut TestAppContext) {
    let handle = open(cx);
    let generation =
        cx.update(|cx| Session::click(handle.into(), &Selector::id("increment"), cx).unwrap());
    let task = cx.spawn(move |mut cx| async move {
        Session::wait_for_draw(handle.into(), generation, Duration::from_secs(1), &mut cx).await
    });
    cx.run_until_parked();
    draw(handle, cx);
    cx.executor().advance_clock(Duration::from_millis(20));
    assert!(task.await.unwrap().data().generation > generation);

    let timeout = cx.spawn(move |mut cx| async move {
        Session::wait_for(
            handle.into(),
            Duration::from_millis(30),
            |_| Ok(false),
            &mut cx,
        )
        .await
    });
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_millis(31));
    assert!(
        timeout
            .await
            .err()
            .unwrap()
            .to_string()
            .contains("timed out")
    );

    cx.update(|cx| Session::disable(handle.into(), cx).unwrap());
    let result = Session::wait_for(
        handle.into(),
        Duration::from_secs(1),
        |_| Ok(true),
        &mut cx.to_async(),
    )
    .await;
    assert!(result.err().unwrap().to_string().contains("disabled"));
}
