use crate::{
    AnyElement, AppContext, Context, Entity, MouseButton, MouseDownEvent, Pixels, PlatformInput,
    Point, PointerTransform, Render, TestAppContext, TransformationMatrix, Window, canvas, div,
    point, prelude::*, px, rgb, size,
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

struct Node {
    renders: Rc<Cell<usize>>,
    events: Rc<RefCell<Vec<Point<Pixels>>>>,
    hover: bool,
    color: u32,
}

impl Render for Node {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        self.renders.set(self.renders.get() + 1);
        let events = self.events.clone();
        let node = div().size_full().bg(rgb(self.color)).on_mouse_down(
            MouseButton::Left,
            move |event, window, _| {
                assert_eq!(event.position, window.mouse_position());
                events.borrow_mut().push(event.position);
            },
        );
        if self.hover {
            node.id("node")
                .hover(|style| style.bg(rgb(0xff0000)))
                .into_any_element()
        } else {
            node.into_any_element()
        }
    }
}

#[crate::test]
fn affine_cache_redraws_hover_styles_when_content_moves_under_stationary_pointer(
    cx: &mut TestAppContext,
) {
    let renders = Rc::new(Cell::new(0));
    let handle = cx.add_window({
        let renders = renders.clone();
        move |_, cx| MappedGrid {
            nodes: vec![cx.new(|_| Node {
                renders,
                events: Default::default(),
                hover: true,
                color: 0x000000,
            })],
            matrix: TransformationMatrix::unit(),
            clip_width: 400.,
            callback: false,
            cache_across_transforms: true,
        }
    });
    cx.update_window(handle.into(), |_, window, cx| {
        window.mouse_position = point(px(10.), px(10.));
        window.refresh();
        window.draw(cx).clear();
        assert_eq!(
            window.rendered_frame.scene.quads[0].background,
            rgb(0xff0000).into()
        );
    })
    .unwrap();
    renders.set(0);
    for (offset, color, expected_renders) in
        [(80., 0x000000, 1), (80., 0x000000, 1), (0., 0xff0000, 2)]
    {
        handle
            .update(cx, |root, _, cx| {
                root.matrix.translation[0] = offset;
                cx.notify();
            })
            .unwrap();
        cx.update_window(handle.into(), |_, window, cx| {
            window.draw(cx).clear();
            assert_eq!(
                window.rendered_frame.scene.quads[0].background,
                rgb(color).into()
            );
        })
        .unwrap();
        assert_eq!(renders.get(), expected_renders);
    }
}

struct MappedGrid {
    nodes: Vec<Entity<Node>>,
    matrix: TransformationMatrix,
    clip_width: f32,
    callback: bool,
    cache_across_transforms: bool,
}

impl Render for MappedGrid {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let nodes = self.nodes.clone();
        let matrix = self.matrix;
        let cache_across_transforms = self.cache_across_transforms;
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
                                        .when(cache_across_transforms, |view| {
                                            view.cache_across_transforms()
                                        })
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
    check_grid_cache(cx, false);
}

#[crate::test]
fn affine_scope_rebases_100_cached_views_without_rendering_on_matrix_changes(
    cx: &mut TestAppContext,
) {
    check_grid_cache(cx, true);
}

fn check_grid_cache(cx: &mut TestAppContext, cache_across_transforms: bool) {
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
                        hover: false,
                        color: 0x000000,
                    })
                })
                .collect(),
            matrix: TransformationMatrix::unit(),
            clip_width: 400.,
            callback: false,
            cache_across_transforms,
        }
    });
    cx.update_window(handle.into(), |_, window, _| {
        window.mouse_position = point(px(500.), px(500.));
        assert!(window.frame_diagnostics().is_none());
        window.set_frame_diagnostics_enabled(true);
    })
    .unwrap();
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
            window.mouse_position = point(px(500.), px(500.));
            let before_draw = renders.get();
            window.draw(cx).clear();
            let diagnostics = window.frame_diagnostics().unwrap();
            let reasons = diagnostics.view_cache_misses;
            assert_eq!(
                reasons.cold
                    + reasons.accessibility
                    + reasons.refresh
                    + reasons.dirty
                    + reasons.context,
                diagnostics.view_cache.misses
            );
            assert_eq!(
                diagnostics.view_cache.misses,
                (renders.get() - before_draw) as u64
            );
            assert_eq!(
                diagnostics.view_cache.hits + diagnostics.view_cache.misses,
                100
            );
            assert!(diagnostics.platform_draw_time.is_none());
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
            window.mouse_position = point(px(500.), px(500.));
        })
        .unwrap();
        if !callback {
            assert_eq!(
                renders.get(),
                if cache_across_transforms && expected >= 200 {
                    expected - 100
                } else {
                    expected
                },
                "scale={scale}, translation={translation}, clip={clip}"
            );
        }
        assert_eq!(&*events.borrow(), &[point(px(10.), px(10.))]);
    }

    if cache_across_transforms {
        for frame in 0..101 {
            handle
                .update(cx, |root, _, cx| {
                    root.callback = false;
                    root.clip_width = 400.;
                    root.matrix = TransformationMatrix {
                        rotation_scale: [[1. + frame as f32 * 0.005, 0.], [0., 1.5]],
                        translation: [frame as f32 * 0.25, 10.],
                    };
                    cx.notify();
                })
                .unwrap();
            cx.update_window(handle.into(), |_, window, cx| {
                window.draw(cx).clear();
                assert_eq!(window.rendered_frame.scene.quads.len(), 100);
            })
            .unwrap();
            if frame == 0 {
                renders.set(0);
            } else {
                assert_eq!(
                    renders.get(),
                    0,
                    "matrix animation must not render unchanged nodes"
                );
            }
        }
        handle
            .update(cx, |root, _, cx| {
                root.nodes[0].update(cx, |node, cx| {
                    node.color = 0x0000ff;
                    cx.notify();
                });
            })
            .unwrap();
        cx.update_window(handle.into(), |_, window, cx| {
            window.draw(cx).clear();
            assert_eq!(
                window.rendered_frame.scene.quads[0].background,
                rgb(0x0000ff).into()
            );
        })
        .unwrap();
        assert_eq!(renders.get(), 1, "notified node must render");
        cx.update_window(handle.into(), |_, window, cx| {
            window.refresh();
            window.draw(cx).clear();
        })
        .unwrap();
        assert_eq!(renders.get(), 101, "refresh must render all nodes");
    }
    cx.update_window(handle.into(), |_, window, _| {
        window.set_frame_diagnostics_enabled(false);
        assert!(window.frame_diagnostics().is_none());
    })
    .unwrap();
}
