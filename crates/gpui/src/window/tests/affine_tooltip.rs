use crate::{
    AppContext, ContentMask, Context, Modifiers, PointerTransform, Render, TestAppContext,
    TransformationMatrix, VisualTestContext, Window, canvas, div, point, prelude::*, px, size,
};
use std::{cell::Cell, rc::Rc, time::Duration};

struct Tip;
impl Render for Tip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().w(px(30.)).h(px(20.))
    }
}

struct Owner {
    matrix: TransformationMatrix,
    builds: Rc<Cell<usize>>,
    hoverable: bool,
    clipped: bool,
    text: bool,
}

impl Render for Owner {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let mapping = PointerTransform::affine(self.matrix).unwrap();
        let paint_mapping = mapping.clone();
        let builds = self.builds.clone();
        let hoverable = self.hoverable;
        let clipped = self.clipped;
        let text = self.text;
        canvas(
            move |bounds, window, cx| {
                let build = move |_: &mut Window, cx: &mut crate::App| {
                    builds.set(builds.get() + 1);
                    cx.new(|_| Tip).into()
                };
                let mut element = if text {
                    crate::InteractiveText::new("target", crate::StyledText::new("Tooltip text"))
                        .tooltip(move |_, window, cx| Some(build(window, cx)))
                        .into_any_element()
                } else {
                    let target = div().id("target").w(px(50.)).h(px(50.));
                    let target = if hoverable {
                        target.hoverable_tooltip(build)
                    } else {
                        target.tooltip(build)
                    };
                    target
                        .tooltip_show_delay(Duration::from_millis(1))
                        .into_any_element()
                };
                window.with_pointer_transform(bounds, mapping, |window| {
                    window.with_content_mask(
                        clipped.then(|| ContentMask {
                            bounds: crate::Bounds::new(point(px(0.), px(0.)), size(px(5.), px(5.))),
                        }),
                        |window| {
                            element.prepaint_as_root(
                                point(px(0.), px(0.)),
                                bounds.size.into(),
                                window,
                                cx,
                            );
                        },
                    );
                });
                element
            },
            move |bounds, mut element, window, cx| {
                window.with_pointer_transform(bounds, paint_mapping, |window| {
                    element.paint(window, cx)
                });
            },
        )
        .size_full()
    }
}

fn show(visual: &mut VisualTestContext, position: crate::Point<crate::Pixels>) {
    visual.simulate_mouse_move(position, None, Modifiers::default());
    visual.cx.run_until_parked();
    visual.cx.dispatcher.advance_clock(Duration::from_secs(1));
    visual.cx.run_until_parked();
    visual.update(|window, cx| window.draw(cx).clear());
}

#[crate::test]
fn tooltip_uses_current_affine_geometry_and_clip_after_delayed_show(cx: &mut TestAppContext) {
    for (matrix, text) in [
        TransformationMatrix::unit(),
        TransformationMatrix {
            translation: [150., 0.],
            ..TransformationMatrix::unit()
        },
        TransformationMatrix {
            rotation_scale: [[0., -2.], [2., 0.]],
            translation: [250., 50.],
        },
    ]
    .into_iter()
    .flat_map(|matrix| [(matrix, false), (matrix, true)])
    {
        let builds = Rc::new(Cell::new(0));
        let handle = cx.open_window(size(px(400.), px(300.)), {
            let builds = builds.clone();
            move |_, _| Owner {
                matrix,
                builds,
                hoverable: false,
                clipped: false,
                text,
            }
        });
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        visual.update(|window, cx| window.draw(cx).clear());
        let pointer = matrix.apply(point(px(10.), px(10.)));
        show(&mut visual, pointer);
        visual.update(|window, _| {
            assert_eq!(
                window.tooltip_bounds.as_ref().unwrap().bounds.origin,
                pointer + point(px(1.), px(1.))
            );
        });
        assert_eq!(builds.get(), 1);
        handle
            .update(&mut visual.cx, |view, _, cx| {
                view.clipped = true;
                cx.notify();
            })
            .unwrap();
        visual.update(|window, cx| {
            window.draw(cx).clear();
            assert!(
                window.tooltip_bounds.is_none(),
                "current source clipping hides the tooltip"
            );
        });
        handle
            .update(&mut visual.cx, |view, _, cx| {
                view.clipped = false;
                cx.notify();
            })
            .unwrap();
        visual.update(|window, cx| window.draw(cx).clear());
        show(&mut visual, pointer);
        assert_eq!(builds.get(), 2);
        handle
            .update(&mut visual.cx, |view, _, cx| {
                view.matrix.translation[0] += 100.;
                cx.notify();
            })
            .unwrap();
        visual.update(|window, cx| {
            window.draw(cx).clear();
            assert!(
                window.tooltip_bounds.is_none(),
                "moving the owner away must use the new mapping"
            );
        });
    }
}

#[crate::test]
fn transformed_hoverable_tooltip_accepts_pointer_in_window_coordinates(cx: &mut TestAppContext) {
    let handle = cx.open_window(size(px(400.), px(300.)), |_, _| Owner {
        matrix: TransformationMatrix {
            translation: [150., 0.],
            ..TransformationMatrix::unit()
        },
        builds: Default::default(),
        hoverable: true,
        clipped: false,
        text: false,
    });
    let mut visual = VisualTestContext::from_window(handle.into(), cx);
    visual.update(|window, cx| window.draw(cx).clear());
    show(&mut visual, point(px(195.), px(45.)));
    let tip = visual.update(|window, _| window.tooltip_bounds.as_ref().unwrap().bounds);
    show(&mut visual, tip.bottom_right() - point(px(1.), px(1.)));
    visual.cx.dispatcher.advance_clock(Duration::from_secs(1));
    visual.cx.run_until_parked();
    visual.update(|window, cx| {
        window.draw(cx).clear();
        assert!(window.tooltip_bounds.is_some());
    });
    show(&mut visual, point(px(350.), px(250.)));
    visual.cx.dispatcher.advance_clock(Duration::from_secs(1));
    visual.cx.run_until_parked();
    visual.update(|window, cx| {
        window.draw(cx).clear();
        assert!(window.tooltip_bounds.is_none());
    });
}
