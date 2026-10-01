use crate::{
    AppContext, ContentMask, Context, Modifiers, PointerTransform, Render, TestAppContext,
    TransformationMatrix, VisualTestContext, Window, canvas, div, point, prelude::*, px, size,
};
use std::{cell::Cell, rc::Rc, time::Duration};

struct OcclusionProbe {
    covered: bool,
    text: bool,
    hoverable: bool,
    deferred: bool,
    behavior: crate::HitboxBehavior,
    cover_hitbox: Rc<Cell<Option<crate::HitboxId>>>,
}

impl Render for OcclusionProbe {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let target = if self.text {
            crate::InteractiveText::new("target", crate::StyledText::new("Tooltip text"))
                .tooltip(|_, _, cx| Some(cx.new(|_| Tip).into()))
                .into_any_element()
        } else if self.hoverable {
            div()
                .id("target")
                .w(px(100.))
                .h(px(50.))
                .hoverable_tooltip(|_, cx| cx.new(|_| Tip).into())
                .into_any_element()
        } else {
            div()
                .id("target")
                .w(px(100.))
                .h(px(50.))
                .tooltip(|_, cx| cx.new(|_| Tip).into())
                .into_any_element()
        };
        let mut root = div().relative().size_full().child(target);
        if self.covered {
            let hitbox = self.cover_hitbox.clone();
            let behavior = self.behavior;
            let cover = canvas(
                move |bounds, window, _| {
                    hitbox.set(Some(window.insert_hitbox(bounds, behavior).id));
                },
                |bounds, _, window, _| window.paint_quad(crate::fill(bounds, crate::rgb(0xff0000))),
            )
            .absolute()
            .top_0()
            .left_0()
            .w(px(100.))
            .h(px(50.));
            root = if self.deferred {
                root.child(crate::deferred(cover).with_priority(1))
            } else {
                root.child(cover)
            };
        }
        root
    }
}

#[crate::test]
fn tooltips_hide_when_the_owner_is_occluded_without_mouse_motion(cx: &mut TestAppContext) {
    for (text, hoverable) in [(false, false), (false, true), (true, false)] {
        for (deferred, behavior) in [false, true].into_iter().flat_map(|deferred| {
            [
                crate::HitboxBehavior::BlockMouse,
                crate::HitboxBehavior::BlockMouseExceptScroll,
            ]
            .map(|behavior| (deferred, behavior))
        }) {
            let cover_hitbox = Rc::new(Cell::new(None));
            let handle = cx.open_window(size(px(400.), px(300.)), {
                let cover_hitbox = cover_hitbox.clone();
                move |_, _| OcclusionProbe {
                    covered: true,
                    text,
                    hoverable,
                    deferred,
                    behavior,
                    cover_hitbox,
                }
            });
            let mut visual = VisualTestContext::from_window(handle.into(), cx);
            visual.update(|window, cx| window.draw(cx).clear());
            let pointer = point(px(10.), px(10.));
            show(&mut visual, pointer);
            visual.update(|window, _| {
                assert!(
                    window.tooltip_bounds.is_none(),
                    "covered owner must not start a tooltip"
                )
            });
            handle
                .update(&mut visual.cx, |view, _, cx| {
                    view.covered = false;
                    cx.notify();
                })
                .unwrap();
            visual.update(|window, cx| window.draw(cx).clear());
            show(&mut visual, pointer);
            visual.update(|window, _| assert!(window.tooltip_bounds.is_some()));
            handle
                .update(&mut visual.cx, |view, _, cx| {
                    view.covered = true;
                    view.behavior = crate::HitboxBehavior::Normal;
                    cx.notify();
                })
                .unwrap();
            visual.update(|window, cx| {
                window.draw(cx).clear();
                assert!(
                    window.tooltip_bounds.is_some(),
                    "nonblocking layers must preserve hover"
                );
            });
            handle
                .update(&mut visual.cx, |view, _, cx| {
                    view.behavior = behavior;
                    cx.notify();
                })
                .unwrap();
            for _ in 0..3 {
                visual.update(|window, cx| {
                    window.draw(cx).clear();
                    let hits = window.rendered_frame.hit_test(pointer);
                    assert_eq!(
                        &hits.ids[..hits.hover_hitbox_count],
                        &[cover_hitbox.get().unwrap()]
                    );
                    if !hoverable {
                        assert!(
                            window.tooltip_bounds.is_none(),
                            "ordinary tooltips must hide in the first covered frame"
                        );
                    }
                });
                visual.cx.run_until_parked();
                visual.cx.dispatcher.advance_clock(Duration::from_secs(1));
            }
            let stationary = visual.update(|window, _| window.tooltip_bounds.is_some());
            assert!(
                !stationary,
                "occluded tooltip must hide without mouse motion: text={text}, hoverable={hoverable}, deferred={deferred}"
            );
            show(&mut visual, point(px(9.), px(10.)));
            let moved_inside = visual.update(|window, _| window.tooltip_bounds.is_some());
            assert!(!moved_inside, "covered owner must not restart its tooltip");
            show(&mut visual, point(px(350.), px(250.)));
            visual.cx.dispatcher.advance_clock(Duration::from_secs(1));
            visual.cx.run_until_parked();
            visual.update(|window, cx| window.draw(cx).clear());
            let moved_outside = visual.update(|window, _| window.tooltip_bounds.is_some());
            assert!(!moved_outside);
            handle
                .update(&mut visual.cx, |view, _, cx| {
                    view.covered = false;
                    cx.notify();
                })
                .unwrap();
            visual.update(|window, cx| window.draw(cx).clear());
            visual.simulate_mouse_move(pointer, None, Modifiers::default());
            visual.cx.run_until_parked();
            handle
                .update(&mut visual.cx, |view, _, cx| {
                    view.covered = true;
                    cx.notify();
                })
                .unwrap();
            visual.update(|window, cx| window.draw(cx).clear());
            for _ in 0..2 {
                visual.cx.dispatcher.advance_clock(Duration::from_secs(1));
                visual.cx.run_until_parked();
                visual.update(|window, cx| window.draw(cx).clear());
            }
            visual.update(|window, _| {
                assert!(
                    window.tooltip_bounds.is_none(),
                    "covering during the show delay must suppress the tooltip"
                )
            });
        }
    }
}

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
