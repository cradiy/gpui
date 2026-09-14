use gpui::{
    Context, Entity, IntoElement, Render, Role, StyleRefinement, TestAppContext, Text, Window, div,
    prelude::*, px, size,
};
use gpui_automation::{LookupError, Selector, Session};

struct Form {
    second: bool,
    value: String,
}

impl Render for Form {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .automation_id("form")
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div().automation_id("primary").child(
                    div()
                        .automation_id("apply")
                        .role(Role::Button)
                        .aria_label("Apply")
                        .w(px(90.))
                        .h(px(32.)),
                ),
            )
            .when(self.second, |root| {
                root.child(
                    div().automation_id("secondary").aria_disabled(true).child(
                        div()
                            .automation_id("apply")
                            .role(Role::Button)
                            .aria_label("Apply")
                            .w(px(90.))
                            .h(px(32.)),
                    ),
                )
            })
            .child(
                div()
                    .automation_id("query")
                    .role(Role::TextInput)
                    .aria_label("Query")
                    .aria_value(self.value.clone()),
            )
            .child(
                div()
                    .automation_id("secret")
                    .role(Role::PasswordInput)
                    .aria_label("Password")
                    .aria_value("private-value")
                    .child(Text::new("secret-content".into(), "private-value".into())),
            )
    }
}

struct Root {
    form: Entity<Form>,
}
impl Render for Root {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().p(px(12.)).child(
            self.form
                .clone()
                .cached(StyleRefinement::default().w(px(280.)).h(px(180.))),
        )
    }
}

#[gpui::test]
fn snapshots_track_cached_subtrees_and_reject_ambiguous_queries(cx: &mut TestAppContext) {
    let handle = cx.open_window(size(px(400.), px(300.)), |window, cx| {
        window.set_window_title("Inspection");
        Root {
            form: cx.new(|_| Form {
                second: true,
                value: "one".into(),
            }),
        }
    });
    handle
        .update(cx, |_, window, _| {
            assert!(window.automation_snapshot().is_none())
        })
        .unwrap();
    cx.update(|cx| {
        assert_eq!(Session::windows(cx).unwrap().len(), 1);
        assert_eq!(
            Session::window_by_title("Inspection", cx).unwrap(),
            handle.into()
        );
        Session::enable(handle.into(), cx).unwrap();
    });
    cx.update_window(handle.into(), |_, window, cx| {
        window.draw(cx).clear();
    })
    .unwrap();
    let first = cx.update(|cx| Session::snapshot(handle.into(), cx).unwrap());
    assert_eq!(
        first
            .get_by_role(Role::Button)
            .named("Apply")
            .one()
            .unwrap_err(),
        LookupError::Ambiguous { count: 2 }
    );
    let group = first.get_by_id("primary").one().unwrap();
    let button = first.get_by_id("apply").within(group.id).one().unwrap();
    let button_id = button.id;
    let selector = Selector::id("apply").within(Selector::id("primary"));
    assert_eq!(first.find(&selector).one().unwrap().id, button_id);
    assert_eq!(button.bounds.unwrap().size, size(px(90.), px(32.)));
    assert!(!button.disabled);
    let other = first.get_by_id("secondary").one().unwrap();
    assert!(
        first
            .get_by_id("apply")
            .within(other.id)
            .one()
            .unwrap()
            .disabled
    );
    assert_eq!(
        first.get_by_id("query").one().unwrap().value.as_deref(),
        Some("one")
    );
    let secret = first.get_by_id("secret").one().unwrap();
    assert!(secret.redacted && secret.value.is_none());
    assert_eq!(secret.label.as_deref(), Some("Password"));
    assert!(!secret.children.is_empty());
    for child in &secret.children {
        let node = first.node(*child).unwrap();
        assert!(node.redacted && node.label.is_none() && node.value.is_none());
    }
    assert_eq!(
        first.get_by_id("missing").one().unwrap_err(),
        LookupError::NotFound
    );
    assert_eq!(
        first.get_by_id("apply").within(u64::MAX).one().unwrap_err(),
        LookupError::UnknownScope(u64::MAX)
    );

    // An unchanged cached view must still be present in subsequent snapshots.
    cx.update_window(handle.into(), |_, window, cx| {
        window.draw(cx).clear();
    })
    .unwrap();
    let unchanged = cx.update(|cx| Session::snapshot(handle.into(), cx).unwrap());
    assert!(unchanged.data().generation > first.data().generation);
    assert_eq!(
        unchanged
            .get_by_id("apply")
            .within(group.id)
            .one()
            .unwrap()
            .id,
        button_id
    );

    handle
        .update(cx, |root, _, cx| {
            root.form.update(cx, |form, cx| {
                form.second = false;
                form.value = "two".into();
                cx.notify();
            });
        })
        .unwrap();
    cx.update_window(handle.into(), |_, window, cx| {
        window.draw(cx).clear();
    })
    .unwrap();
    let latest = cx.update(|cx| Session::snapshot(handle.into(), cx).unwrap());
    assert_eq!(latest.get_by_id("apply").one().unwrap().id, button_id);
    assert_eq!(latest.find(&selector).one().unwrap().id, button_id);
    assert_eq!(
        latest.get_by_id("query").one().unwrap().value.as_deref(),
        Some("two")
    );
    assert_eq!(
        first.get_by_id("query").one().unwrap().value.as_deref(),
        Some("one")
    );
    assert!(latest.node(other.id).is_none());
    cx.update(|cx| Session::disable(handle.into(), cx).unwrap());
    handle
        .update(cx, |_, window, _| {
            assert!(window.automation_snapshot().is_none())
        })
        .unwrap();
    cx.update(|cx| Session::enable(handle.into(), cx).unwrap());
    cx.update_window(handle.into(), |_, window, cx| {
        window.draw(cx).clear();
    })
    .unwrap();
    let resumed = cx.update(|cx| Session::snapshot(handle.into(), cx).unwrap());
    assert!(resumed.data().generation > latest.data().generation);
}

#[gpui::test]
fn window_discovery_reports_duplicate_titles_and_closed_handles(cx: &mut TestAppContext) {
    let create = |window: &mut Window, _: &mut Context<Form>| {
        window.set_window_title("Same title");
        Form {
            second: false,
            value: String::new(),
        }
    };
    let first = cx.open_window(size(px(300.), px(200.)), create);
    let second = cx.open_window(size(px(300.), px(200.)), create);
    cx.update(|cx| {
        assert_eq!(Session::windows(cx).unwrap().len(), 2);
        assert!(Session::window_by_title("Same title", cx).is_err());
    });
    second
        .update(cx, |_, window, _| window.remove_window())
        .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(
            Session::window_by_title("Same title", cx).unwrap(),
            first.into()
        );
        assert!(Session::snapshot(second.into(), cx).is_err());
    });
}
