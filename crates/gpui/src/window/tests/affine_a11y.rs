use crate::{
    AppContext, Bounds, Context, MouseButton, PointerTransform, Render, TestAppContext,
    TransformationMatrix, Window, canvas, div, point, prelude::*, px, size,
};
use std::{
    cell::RefCell,
    rc::Rc,
    sync::{Arc, atomic::AtomicBool},
};

struct Target {
    offset: f32,
    clicks: Rc<RefCell<Vec<crate::Point<crate::Pixels>>>>,
}

impl Render for Target {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let clicks = self.clicks.clone();
        let transform = PointerTransform::affine(TransformationMatrix {
            rotation_scale: [[2., 0.], [0., 2.]],
            translation: [self.offset, 50.],
        })
        .unwrap();
        let paint_transform = transform.clone();
        canvas(
            move |bounds, window, cx| {
                let mut child = div()
                    .id("target")
                    .role(accesskit::Role::Button)
                    .w(px(40.))
                    .h(px(20.))
                    .on_mouse_down(MouseButton::Left, move |event, _, _| {
                        clicks.borrow_mut().push(event.position);
                    })
                    .into_any_element();
                window.with_pointer_transform(bounds, transform, |window| {
                    child.prepaint_as_root(
                        point(px(10.), px(20.)),
                        size(px(40.), px(20.)).into(),
                        window,
                        cx,
                    );
                });
                child
            },
            move |bounds, mut child, window, cx| {
                window.with_pointer_transform(bounds, paint_transform, |window| {
                    child.paint(window, cx)
                });
            },
        )
        .size_full()
    }
}

#[crate::test]
fn affine_a11y_click_uses_display_bounds_and_tracks_scope_changes(cx: &mut TestAppContext) {
    let clicks = Rc::new(RefCell::new(Vec::new()));
    let handle = cx.add_window({
        let clicks = clicks.clone();
        move |window, _| {
            window.a11y =
                crate::window::a11y::A11y::new(Arc::new(AtomicBool::new(true)), false, None);
            #[cfg(feature = "automation")]
            window.set_automation_enabled(true).unwrap();
            Target {
                offset: 100.,
                clicks,
            }
        }
    });
    for offset in [100., 150.] {
        handle
            .update(cx, |target, _, cx| {
                target.offset = offset;
                cx.notify();
            })
            .unwrap();
        cx.update_window(handle.into(), |_, window, cx| {
            window.draw(cx).clear();
            let (&id, &bounds) = window.a11y.node_bounds.iter().next().unwrap();
            #[cfg(feature = "automation")]
            assert_eq!(
                window
                    .automation_snapshot()
                    .unwrap()
                    .nodes
                    .iter()
                    .find(|node| node.id == id.0)
                    .unwrap()
                    .bounds,
                Some(bounds)
            );
            assert_eq!(
                bounds,
                Bounds::new(point(px(offset + 20.), px(90.)), size(px(80.), px(40.)))
            );
            window.handle_a11y_action(
                accesskit::ActionRequest {
                    action: accesskit::Action::Click,
                    target_tree: accesskit::TreeId::ROOT,
                    target_node: id,
                    data: None,
                },
                cx,
            );
        })
        .unwrap();
    }
    assert_eq!(
        &*clicks.borrow(),
        &[point(px(30.), px(30.)), point(px(30.), px(30.))]
    );
}
