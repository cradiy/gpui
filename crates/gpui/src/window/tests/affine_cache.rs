use crate::{
    AnyElement, AppContext, Context, Entity, MouseButton, MouseDownEvent, Pixels, PlatformInput,
    Point, PointerTransform, Render, TestAppContext, TransformationMatrix, Window, canvas, div,
    point, prelude::*, px, size,
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

struct Node {
    renders: Rc<Cell<usize>>,
    events: Rc<RefCell<Vec<Point<Pixels>>>>,
}

impl Render for Node {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        self.renders.set(self.renders.get() + 1);
        let events = self.events.clone();
        div()
            .size_full()
            .on_mouse_down(MouseButton::Left, move |event, _, _| {
                events.borrow_mut().push(event.position);
            })
    }
}

struct MappedGrid {
    nodes: Vec<Entity<Node>>,
    matrix: TransformationMatrix,
    clip_width: f32,
    callback: bool,
}

impl Render for MappedGrid {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let nodes = self.nodes.clone();
        let matrix = self.matrix;
        let transform = if self.callback {
            let inverse = matrix.inverse().unwrap();
            PointerTransform::new(move |position, _, _| inverse.apply(position))
        } else {
            // Constructed afresh on each render, including the chain allocation.
            PointerTransform::chain([
                PointerTransform::affine(matrix).unwrap(),
                PointerTransform::identity(),
            ])
        };
        let paint_transform = transform.clone();
        div()
            .w(px(self.clip_width))
            .h(px(400.))
            .overflow_hidden()
            .child(
                canvas(
                    move |bounds, window, cx| {
                        window.with_pointer_transform(bounds, transform, |window| {
                            nodes
                                .iter()
                                .enumerate()
                                .map(|(index, node)| {
                                    let mut element = node
                                        .clone()
                                        .cached(div().size(px(40.)).style().clone())
                                        .into_any_element();
                                    element.prepaint_as_root(
                                        point(
                                            px((index % 10) as f32 * 40.),
                                            px((index / 10) as f32 * 40.),
                                        ),
                                        size(px(40.), px(40.)).into(),
                                        window,
                                        cx,
                                    );
                                    element
                                })
                                .collect::<Vec<AnyElement>>()
                        })
                    },
                    move |bounds, mut elements, window, cx| {
                        window.with_pointer_transform(bounds, paint_transform, |window| {
                            for element in &mut elements {
                                element.paint(window, cx);
                            }
                        });
                    },
                )
                .size(px(400.)),
            )
    }
}

#[crate::test]
fn affine_scope_reuses_100_cached_views_and_updates_input_when_changed(cx: &mut TestAppContext) {
    let renders = Rc::new(Cell::new(0));
    let events = Rc::new(RefCell::new(Vec::new()));
    let handle = cx.add_window({
        let renders = renders.clone();
        let events = events.clone();
        move |_, cx| MappedGrid {
            nodes: (0..100)
                .map(|_| {
                    cx.new(|_| Node {
                        renders: renders.clone(),
                        events: events.clone(),
                    })
                })
                .collect(),
            matrix: TransformationMatrix::unit(),
            clip_width: 400.,
            callback: false,
        }
    });
    for (scale, translation, clip, callback, expected) in [
        (1., 0., 400., false, 100),
        (1., 0., 400., false, 100),
        (2., 80., 400., false, 200),
        (2., 80., 400., false, 200),
        (2., 80., 300., false, 300),
        (2., 80., 300., false, 300),
        (2., 80., 300., true, 400),
        (2., 80., 300., true, 500),
    ] {
        handle
            .update(cx, |root, _, cx| {
                root.matrix = TransformationMatrix {
                    rotation_scale: [[scale, 0.], [0., scale]],
                    translation: [translation, 0.],
                };
                root.clip_width = clip;
                root.callback = callback;
                cx.notify();
            })
            .unwrap();
        cx.update_window(handle.into(), |_, window, cx| {
            let before_draw = renders.get();
            window.draw(cx).clear();
            if callback {
                assert_eq!(
                    renders.get(),
                    before_draw + 100,
                    "callback scopes must invalidate on every draw"
                );
            }
            events.borrow_mut().clear();
            window.dispatch_event(
                PlatformInput::MouseDown(MouseDownEvent {
                    position: point(px(translation + 10. * scale), px(10. * scale)),
                    ..Default::default()
                }),
                cx,
            );
        })
        .unwrap();
        if !callback {
            assert_eq!(
                renders.get(),
                expected,
                "scale={scale}, translation={translation}, clip={clip}"
            );
        }
        assert_eq!(&*events.borrow(), &[point(px(10.), px(10.))]);
    }
}
