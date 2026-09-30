use super::*;
use gpui::{
    AnchoredPositionMode, AppContext, Context, Entity, MouseButton, MouseDownEvent, PlatformInput,
    Point, Render, TestAppContext, anchored, canvas, deferred, div, point, prelude::*, px, size,
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

type Clicks = Rc<RefCell<Vec<(&'static str, Point<Pixels>)>>>;

struct Content {
    renders: Rc<Cell<usize>>,
    clicks: Clicks,
    popup: Rc<Cell<Bounds<Pixels>>>,
}

impl Render for Content {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        self.renders.set(self.renders.get() + 1);
        let clicks = self.clicks.clone();
        let popup_clicks = self.clicks.clone();
        let popup = self.popup.clone();
        transform_group(div(), TransformationMatrix::unit())
            .id("content")
            .role(accesskit::Role::Group)
            .automation_id("content")
            .relative()
            .w(px(200.))
            .h(px(150.))
            .child(
                div()
                    .id("target")
                    .absolute()
                    .left(px(20.))
                    .top(px(20.))
                    .w(px(40.))
                    .h(px(30.))
                    .on_mouse_down(MouseButton::Left, move |e, _, _| {
                        clicks.borrow_mut().push(("target", e.position))
                    })
                    .child(deferred(
                        anchored()
                            .map_anchor(true)
                            .position_mode(AnchoredPositionMode::Local)
                            .position(point(px(0.), px(30.)))
                            .offset(point(px(2.), px(3.)))
                            .child(
                                div()
                                    .id("popup")
                                    .w(px(30.))
                                    .h(px(10.))
                                    .on_mouse_down(MouseButton::Left, move |e, _, cx| {
                                        popup_clicks.borrow_mut().push(("popup", e.position));
                                        cx.stop_propagation();
                                    })
                                    .child(
                                        canvas(
                                            move |bounds, _, _| popup.set(bounds),
                                            |_, _, _, _| {},
                                        )
                                        .size_full(),
                                    ),
                            ),
                    )),
            )
    }
}

struct Probe {
    content: Entity<Content>,
    offset: f32,
}

impl Render for Probe {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let content = self.content.clone();
        let offset = self.offset;
        canvas(
            move |_, window, cx| {
                let mut element = transform_group(
                    transform_group(
                        content.cached(div().w(px(200.)).h(px(150.)).style().clone()),
                        TransformationMatrix {
                            translation: [10., 5.],
                            ..TransformationMatrix::unit()
                        },
                    ),
                    TransformationMatrix {
                        rotation_scale: [[2., 0.], [0., 2.]],
                        translation: [offset, 0.],
                    },
                )
                .into_any_element();
                element.prepaint_as_root(
                    point(px(10.), px(15.)),
                    size(px(200.), px(150.)).into(),
                    window,
                    cx,
                );
                element
            },
            |_, mut element, window, cx| element.paint(window, cx),
        )
        .size_full()
    }
}

#[gpui::test]
fn transform_group_nested_cache_input_popup_and_accessibility(cx: &mut TestAppContext) {
    for supported in [true, false] {
        let renders = Rc::new(Cell::new(0));
        let clicks: Clicks = Default::default();
        let popup = Rc::new(Cell::new(Bounds::default()));
        let handle = cx.open_window(size(px(600.), px(400.)), {
            let renders = renders.clone();
            let clicks = clicks.clone();
            let popup = popup.clone();
            move |_, cx| Probe {
                content: cx.new(|_| Content {
                    renders,
                    clicks,
                    popup,
                }),
                offset: 20.,
            }
        });
        cx.set_subtree_effects_supported(handle.into(), supported);
        cx.update_window(handle.into(), |_, w, cx| w.draw(cx).clear())
            .unwrap();
        renders.set(0);
        for (offset, expected) in [(30., 1), (30., 1), (40., 2), (40., 2)] {
            handle
                .update(cx, |root, _, cx| {
                    root.offset = offset;
                    cx.notify();
                })
                .unwrap();
            cx.update_window(handle.into(), |_, w, cx| w.draw(cx).clear())
                .unwrap();
            assert_eq!(renders.get(), if supported { expected } else { 0 });
            assert_eq!(
                popup.get(),
                Bounds::new(
                    if supported {
                        point(px(72. + offset), px(128.))
                    } else {
                        point(px(32.), px(68.))
                    },
                    size(px(30.), px(10.)),
                )
            );
        }
        let click = if supported {
            point(px(130.), px(85.))
        } else {
            point(px(40.), px(45.))
        };
        cx.update_window(handle.into(), |_, w, cx| {
            w.dispatch_event(
                PlatformInput::MouseDown(MouseDownEvent {
                    position: click,
                    ..Default::default()
                }),
                cx,
            );
            let popup_click = popup.get().origin + point(px(5.), px(5.));
            w.dispatch_event(
                PlatformInput::MouseDown(MouseDownEvent {
                    position: popup_click,
                    ..Default::default()
                }),
                cx,
            );
        })
        .unwrap();
        assert_eq!(
            &*clicks.borrow(),
            &[
                ("target", point(px(40.), px(45.))),
                ("popup", popup.get().origin + point(px(5.), px(5.)))
            ]
        );

        // The wrapped root itself must receive the scope, not only its descendants.
        cx.update_window(handle.into(), |_, w, cx| {
            w.set_automation_enabled(true).unwrap();
            w.draw(cx).clear();
            let snapshot = w.automation_snapshot().unwrap();
            let node = snapshot
                .nodes
                .iter()
                .find(|n| n.automation_id.as_deref() == Some("content"))
                .unwrap();
            assert_eq!(
                node.bounds,
                Some(if supported {
                    Bounds::new(point(px(70.), px(25.)), size(px(400.), px(300.)))
                } else {
                    Bounds::new(point(px(10.), px(15.)), size(px(200.), px(150.)))
                })
            );
        })
        .unwrap();
    }
}

#[test]
fn transform_group_sampling_matches_affine_mapping_at_fractional_origins_and_density() {
    let bounds = Bounds::new(point(px(10.3), px(20.7)), size(px(100.), px(80.)));
    for matrix in [
        TransformationMatrix {
            rotation_scale: [[2., 0.], [0., 0.75]],
            translation: [12., -7.],
        },
        TransformationMatrix {
            rotation_scale: [[0., -1.], [1., 0.]],
            translation: [60., 0.],
        },
        TransformationMatrix {
            rotation_scale: [[-1., 0.25], [0.5, 1.]],
            translation: [60., 4.],
        },
    ] {
        let matrix = window_matrix(matrix, bounds);
        let mapping = PointerTransform::affine(matrix).unwrap();
        for scale in [1., 1.5, 2.] {
            let capture = Bounds::new(
                bounds
                    .origin
                    .map(|v| px((f32::from(v) * scale).round() / scale)),
                bounds.size,
            );
            let params = uniforms(matrix, capture, scale);
            for local in [
                point(px(5.), px(10.)),
                point(px(30.), px(20.)),
                point(px(70.), px(50.)),
            ] {
                let source = bounds.origin + local;
                let display = mapping.source_to_display(source).unwrap();
                let p = (display - capture.origin).scale(scale);
                let row = |i: usize| {
                    let r = params.slots()[i];
                    (r[0] * p.x.0 + r[1] * p.y.0 + r[2]) / scale
                };
                let sampled = capture.origin + point(px(row(0)), px(row(1)));
                assert!((sampled.x - source.x).abs() < px(0.0001));
                assert!((sampled.y - source.y).abs() < px(0.0001));
            }
        }
    }
}
