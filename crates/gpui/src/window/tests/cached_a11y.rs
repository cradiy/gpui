use crate::{
    AppContext, Context, Entity, FocusHandle, MouseButton, PointerTransform, Render,
    TestAppContext, TransformationMatrix, Window, canvas, div, point, prelude::*, px, size,
};
use std::{
    cell::Cell,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

struct Control {
    callback_owner: Entity<()>,
    renders: Rc<Cell<usize>>,
    actions: Rc<Cell<usize>>,
    clicks: Rc<Cell<usize>>,
    focus: FocusHandle,
    label: &'static str,
}

impl Render for Control {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        self.renders.set(self.renders.get() + 1);
        let actions = self.actions.clone();
        let callback_owner = self.callback_owner.clone();
        let clicks = self.clicks.clone();
        div()
            .id("control")
            .role(accesskit::Role::Button)
            .aria_label(self.label)
            .track_focus(&self.focus)
            .size_full()
            .on_a11y_action(accesskit::Action::Increment, move |_, _, _| {
                let _ = &callback_owner;
                actions.set(actions.get() + 1)
            })
            .on_mouse_down(MouseButton::Left, move |_, _, _| {
                clicks.set(clicks.get() + 1)
            })
    }
}

struct Board {
    control: Option<Entity<Control>>,
    offset: f32,
}

impl Render for Board {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let control = self.control.clone();
        let transform = PointerTransform::affine(TransformationMatrix {
            rotation_scale: [[2., 0.], [0., 2.]],
            translation: [self.offset, 50.],
        })
        .unwrap();
        let paint_transform = transform.clone();
        canvas(
            move |bounds, window, cx| {
                control.map(|control| {
                    let mut element = control
                        .cached(div().size_full().style().clone())
                        .cache_across_transforms()
                        .into_any_element();
                    window.with_pointer_transform(bounds, transform, |window| {
                        element.prepaint_as_root(
                            point(px(10.), px(20.)),
                            size(px(40.), px(20.)).into(),
                            window,
                            cx,
                        );
                    });
                    element
                })
            },
            move |bounds, element, window, cx| {
                if let Some(mut element) = element {
                    window.with_pointer_transform(bounds, paint_transform, |window| {
                        element.paint(window, cx)
                    });
                }
            },
        )
        .size_full()
    }
}

#[crate::test]
fn cached_a11y_preserves_actions_focus_and_geometry_through_transforms(cx: &mut TestAppContext) {
    let renders = Rc::new(Cell::new(0));
    let actions = Rc::new(Cell::new(0));
    let clicks = Rc::new(Cell::new(0));
    let active = Arc::new(AtomicBool::new(true));
    let handle = cx.add_window({
        let (renders, actions, clicks, active) = (
            renders.clone(),
            actions.clone(),
            clicks.clone(),
            active.clone(),
        );
        move |window, cx| {
            window.a11y = crate::window::a11y::A11y::new(active, false, None);
            window.mouse_position = point(px(900.), px(900.));
            Board {
                control: Some(cx.new(|cx| Control {
                    callback_owner: cx.new(|_| ()),
                    renders,
                    actions,
                    clicks,
                    focus: cx.focus_handle(),
                    label: "First",
                })),
                offset: 100.,
            }
        }
    });
    let callback_owner = handle
        .update(cx, |board, _, cx| {
            board
                .control
                .as_ref()
                .unwrap()
                .read(cx)
                .callback_owner
                .downgrade()
        })
        .unwrap();
    let mut node_id = None;
    for (index, offset) in [100., 130., 160.].into_iter().enumerate() {
        handle
            .update(cx, |board, _, cx| {
                board.offset = offset;
                cx.notify();
            })
            .unwrap();
        cx.update_window(handle.into(), |_, window, cx| {
            window.draw(cx).clear();
            let (&id, &bounds) = window.a11y.node_bounds.iter().next().unwrap();
            assert_eq!(*node_id.get_or_insert(id), id);
            assert_eq!(bounds.origin, point(px(offset + 20.), px(90.)));
            assert_eq!(bounds.size, size(px(80.), px(40.)));
            assert!(window.a11y.focus_ids.contains_key(&id));
            window.handle_a11y_action(
                accesskit::ActionRequest {
                    action: accesskit::Action::Increment,
                    target_tree: accesskit::TreeId::ROOT,
                    target_node: id,
                    data: None,
                },
                cx,
            );
            window.mouse_position = point(px(900.), px(900.));
        })
        .unwrap();
        assert_eq!(
            renders.get(),
            1,
            "affine movement should reuse the accessible control"
        );
        assert_eq!(actions.get(), index + 1);
    }
    cx.update_window(handle.into(), |_, window, cx| {
        window.handle_a11y_action(
            accesskit::ActionRequest {
                action: accesskit::Action::Click,
                target_tree: accesskit::TreeId::ROOT,
                target_node: node_id.unwrap(),
                data: None,
            },
            cx,
        );
        assert_eq!(clicks.get(), 1);
        window.mouse_position = point(px(900.), px(900.));
        window.handle_a11y_action(
            accesskit::ActionRequest {
                action: accesskit::Action::Focus,
                target_tree: accesskit::TreeId::ROOT,
                target_node: node_id.unwrap(),
                data: None,
            },
            cx,
        );
        assert!(window.focus.is_some());
        window.draw(cx).clear();
    })
    .unwrap();
    let before = renders.get();
    // Activation transitions rebuild once, then reuse the newly collected tree.
    for enabled in [false, true] {
        active.store(enabled, Ordering::SeqCst);
        cx.update_window(handle.into(), |_, window, cx| {
            window.draw(cx).clear();
            window.draw(cx).clear();
        })
        .unwrap();
    }
    assert_eq!(renders.get(), before + 2);
    handle
        .update(cx, |board, _, cx| {
            board.control.as_ref().unwrap().update(cx, |control, cx| {
                control.label = "Changed";
                cx.notify();
            });
        })
        .unwrap();
    assert!(
        renders.get() > before + 2,
        "entity notifications must rebuild accessibility content"
    );
    handle
        .update(cx, |board, _, cx| {
            board.control = None;
            cx.notify();
        })
        .unwrap();
    cx.update_window(handle.into(), |_, window, cx| {
        window.draw(cx).clear();
        assert!(window.a11y.node_bounds.is_empty());
        assert!(window.a11y.focus_ids.is_empty());
        assert!(window.a11y.action_listeners.is_empty());
    })
    .unwrap();
    assert!(
        callback_owner.upgrade().is_none(),
        "removed controls must release cached action closures"
    );
}
