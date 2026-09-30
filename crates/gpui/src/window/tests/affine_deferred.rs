use crate::{
    AnchoredPositionMode, AppContext, Bounds, Context, Entity, MouseButton, MouseDownEvent, Pixels,
    PlatformInput, Point, PointerTransform, Render, TestAppContext, TransformationMatrix, Window,
    anchored, canvas, deferred, div, point, prelude::*, px, rgb, size,
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

struct Popup {
    renders: Rc<Cell<usize>>,
    clicks: Rc<RefCell<Vec<Point<Pixels>>>>,
    map_anchor: bool,
    local: bool,
    position: Point<Pixels>,
    submenu: bool,
    snap: bool,
}

impl Render for Popup {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        self.renders.set(self.renders.get() + 1);
        let clicks = self.clicks.clone();
        let menu = div()
            .id("menu")
            .relative()
            .w(px(80.))
            .h(px(40.))
            .bg(rgb(0xff0000))
            .on_mouse_down(MouseButton::Left, move |event, _, _| {
                clicks.borrow_mut().push(event.position)
            })
            .when(self.submenu, |menu| {
                menu.child(deferred(
                    anchored()
                        .map_anchor(true)
                        .position_mode(AnchoredPositionMode::Local)
                        .position(point(px(85.), px(0.)))
                        .child(div().w(px(20.)).h(px(20.)).bg(rgb(0x0000ff))),
                ))
            });
        let anchor = anchored()
            .position(self.position)
            .map_anchor(self.map_anchor)
            .position_mode(if self.local {
                AnchoredPositionMode::Local
            } else {
                AnchoredPositionMode::Window
            })
            .offset(point(px(4.), px(6.)));
        let anchor = if self.snap {
            anchor.snap_to_window()
        } else {
            anchor
        };
        anchor.child(menu)
    }
}

struct Source {
    popup: Entity<Popup>,
    transform: PointerTransform,
}

impl Render for Source {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let popup = self.popup.clone();
        let transform = self.transform.clone();
        let paint_transform = transform.clone();
        canvas(
            move |bounds, window, cx| {
                // Two deferred rounds before positioning, with a cached view in the overlay.
                let mut element = deferred(deferred(
                    popup
                        .clone()
                        .cached(div().w(px(200.)).h(px(100.)).style().clone()),
                ))
                .into_any_element();
                window.with_pointer_transform(bounds, transform, |window| {
                    element.prepaint_as_root(
                        point(px(10.), px(15.)),
                        size(px(200.), px(100.)).into(),
                        window,
                        cx,
                    );
                });
                element
            },
            move |bounds, mut element, window, cx| {
                window.with_pointer_transform(bounds, paint_transform, |window| {
                    element.paint(window, cx)
                });
            },
        )
        .size_full()
    }
}

fn transform(offset: f32) -> PointerTransform {
    PointerTransform::chain([
        PointerTransform::affine(TransformationMatrix {
            translation: [10., 5.],
            ..TransformationMatrix::unit()
        })
        .unwrap(),
        PointerTransform::affine(TransformationMatrix {
            rotation_scale: [[2., 0.], [0., 2.]],
            translation: [offset, 50.],
        })
        .unwrap(),
    ])
}

fn assert_geometry(window: &mut Window, origin: Point<Pixels>, submenu: bool) {
    let menu = Bounds::new(origin, size(px(80.), px(40.)));
    let quads = &window.rendered_frame.scene.quads;
    assert_eq!(quads.len(), if submenu { 2 } else { 1 });
    assert_eq!(quads[0].bounds, menu.scale(window.scale_factor()));
    if submenu {
        assert_eq!(
            quads[1].bounds,
            Bounds::new(origin + point(px(85.), px(0.)), size(px(20.), px(20.)))
                .scale(window.scale_factor())
        );
    }
    assert!(window.pointer_mapping.is_identity());
    assert!(window.deferred_anchor_mapping.is_identity());
}

#[crate::test]
fn affine_deferred_anchor_keeps_paint_hits_submenus_and_cached_views_aligned(
    cx: &mut TestAppContext,
) {
    let renders = Rc::new(Cell::new(0));
    let clicks = Rc::new(RefCell::new(Vec::new()));
    let handle = cx.open_window(size(px(800.), px(600.)), {
        let renders = renders.clone();
        let clicks = clicks.clone();
        move |_, cx| Source {
            popup: cx.new(|_| Popup {
                renders,
                clicks,
                map_anchor: true,
                local: false,
                position: point(px(20.), px(30.)),
                submenu: true,
                snap: true,
            }),
            transform: transform(0.),
        }
    });
    cx.update_window(handle.into(), |_, window, cx| window.draw(cx).clear())
        .unwrap();
    renders.set(0);
    for (offset, count) in [(100., 1), (100., 1), (150., 2), (150., 2)] {
        handle
            .update(cx, |root, _, cx| {
                root.transform = transform(offset);
                cx.notify();
            })
            .unwrap();
        clicks.borrow_mut().clear();
        cx.update_window(handle.into(), |_, window, cx| {
            window.draw(cx).clear();
            assert_geometry(window, point(px(offset + 64.), px(126.)), true);
        })
        .unwrap();
        assert_eq!(renders.get(), count);
    }
    cx.update_window(handle.into(), |_, window, cx| {
        window.dispatch_event(
            PlatformInput::MouseDown(MouseDownEvent {
                position: point(px(254.), px(146.)),
                ..Default::default()
            }),
            cx,
        );
    })
    .unwrap();
    assert_eq!(&*clicks.borrow(), &[point(px(254.), px(146.))]);
}

#[crate::test]
fn affine_deferred_anchor_respects_local_positions_fitting_and_opt_out(cx: &mut TestAppContext) {
    for (map_anchor, local, source, origin, mapping, snap) in [
        (
            true,
            true,
            point(px(20.), px(30.)),
            point(px(184.), px(156.)),
            transform(100.),
            true,
        ),
        (
            true,
            false,
            point(px(400.), px(300.)),
            point(px(720.), px(560.)),
            transform(100.),
            true,
        ),
        (
            false,
            false,
            point(px(20.), px(30.)),
            point(px(24.), px(36.)),
            transform(100.),
            true,
        ),
        (
            true,
            false,
            point(px(20.), px(30.)),
            point(px(24.), px(36.)),
            PointerTransform::new(|p, _, _| p),
            true,
        ),
        (
            true,
            false,
            point(px(300.), px(200.)),
            point(px(644.), px(466.)),
            transform(100.),
            false,
        ),
    ] {
        let clicks = Rc::new(RefCell::new(Vec::new()));
        let handle = cx.open_window(size(px(800.), px(600.)), {
            let clicks = clicks.clone();
            move |_, cx| Source {
                popup: cx.new(|_| Popup {
                    renders: Rc::new(Cell::new(0)),
                    clicks,
                    map_anchor,
                    local,
                    position: source,
                    submenu: false,
                    snap,
                }),
                transform: mapping,
            }
        });
        cx.update_window(handle.into(), |_, window, cx| {
            window.draw(cx).clear();
            assert_geometry(window, origin, false);
            window.dispatch_event(
                PlatformInput::MouseDown(MouseDownEvent {
                    position: origin + point(px(40.), px(20.)),
                    ..Default::default()
                }),
                cx,
            );
        })
        .unwrap();
        assert_eq!(&*clicks.borrow(), &[origin + point(px(40.), px(20.))]);
    }
}
