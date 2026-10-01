use crate::{
    AppContext, Context, Entity, Modifiers, MouseButton, Render, TestAppContext, VisualTestContext,
    Window, anchored, canvas, deferred_overlay, div, point, prelude::*, px, rgb, size,
};
use std::{cell::Cell, rc::Rc};

struct Content {
    visible: Rc<Cell<bool>>,
    renders: Rc<Cell<usize>>,
    overlays: Rc<Cell<usize>>,
    clicks: Rc<Cell<usize>>,
}

impl Render for Content {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        self.renders.set(self.renders.get() + 1);
        let visible = self.visible.clone();
        let overlays = self.overlays.clone();
        let clicks = self.clicks.clone();
        div()
            .size_full()
            .child(
                canvas(
                    |_, window, _| {
                        let _: Result<(), ()> = window.transact(|window| {
                            window.defer_overlay(
                                Rc::new(|_, _| panic!("discarded overlay rendered")),
                                0,
                            );
                            Err(())
                        });
                    },
                    |_, _, _, _| {},
                )
                .absolute(),
            )
            .child(
                deferred_overlay(move |_, _| {
                    overlays.set(overlays.get() + 1);
                    if !visible.get() {
                        return None;
                    }
                    let clicks = clicks.clone();
                    Some(
                        anchored()
                            .position(point(px(40.), px(50.)))
                            .child(
                                div()
                                    .id("surface")
                                    .w(px(100.))
                                    .h(px(60.))
                                    .bg(rgb(0xff0000))
                                    .on_mouse_down(MouseButton::Left, move |_, _, _| {
                                        clicks.set(clicks.get() + 1)
                                    })
                                    .child(
                                        deferred_overlay(|_, _| {
                                            Some(
                                                anchored()
                                                    .position(point(px(160.), px(50.)))
                                                    .child(
                                                        div()
                                                            .w(px(20.))
                                                            .h(px(20.))
                                                            .bg(rgb(0x0000ff)),
                                                    )
                                                    .into_any_element(),
                                            )
                                        })
                                        .priority(2),
                                    ),
                            )
                            .into_any_element(),
                    )
                })
                .priority(1),
            )
    }
}

struct Host(Entity<Content>);

impl Render for Host {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        self.0
            .clone()
            .cached(div().w(px(300.)).h(px(200.)).style().clone())
    }
}

#[crate::test]
fn cached_overlay_resolves_each_frame_and_clears_nested_paint_and_hits(cx: &mut TestAppContext) {
    let visible = Rc::new(Cell::new(true));
    let renders = Rc::new(Cell::new(0));
    let overlays = Rc::new(Cell::new(0));
    let clicks = Rc::new(Cell::new(0));
    let handle = cx.open_window(size(px(400.), px(300.)), {
        let (visible, renders, overlays, clicks) = (
            visible.clone(),
            renders.clone(),
            overlays.clone(),
            clicks.clone(),
        );
        move |_, cx| {
            Host(cx.new(|_| Content {
                visible,
                renders,
                overlays,
                clicks,
            }))
        }
    });
    let mut visual = VisualTestContext::from_window(handle.into(), cx);
    visual.simulate_mouse_move(point(px(350.), px(250.)), None, Modifiers::default());
    visual.update(|window, cx| window.draw(cx).clear());
    let initial_renders = renders.get();
    for shown in [true, false, false, true, true] {
        visible.set(shown);
        let before = overlays.get();
        visual.update(|window, cx| {
            window.draw(cx).clear();
            assert_eq!(
                window.rendered_frame.scene.quads.len(),
                if shown { 2 } else { 0 }
            );
        });
        assert_eq!(overlays.get(), before + 1);
        assert_eq!(renders.get(), initial_renders, "parent stays cached");
    }
    visual.simulate_click(point(px(70.), px(70.)), Modifiers::default());
    assert_eq!(clicks.get(), 1);
    visible.set(false);
    visual.update(|window, cx| window.draw(cx).clear());
    visual.simulate_click(point(px(70.), px(70.)), Modifiers::default());
    assert_eq!(
        clicks.get(),
        1,
        "hidden surface must not retain its input listener"
    );
}
