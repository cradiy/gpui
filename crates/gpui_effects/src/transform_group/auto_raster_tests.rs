use super::*;
use gpui::{
    AppContext, Context, Entity, MouseButton, MouseDownEvent, PlatformInput, Point, Render,
    TestAppContext, canvas, div, point, px, radians, size,
};
use std::{cell::Cell, rc::Rc};

#[test]
fn automatic_density_handles_affine_stretch_and_boundary_hysteresis() {
    for angle in [0., 0.3, 1., 2., 3., 4., 5.] {
        let matrix = TransformationMatrix::unit()
            .translate(point(px(200.), px(-90.)).scale(1.))
            .rotate(radians(angle));
        assert_eq!(auto_raster_scale(matrix, None), 1.);
        assert_eq!(auto_raster_scale(matrix.scale(size(2., 0.5)), None), 2.);
        assert_eq!(auto_raster_scale(matrix.scale(size(-3., 0.5)), None), 4.);
    }
    for (linear, expected) in [
        ([[1., 1.], [0., 1.]], 2.),
        ([[1., 2.], [0., 1.]], 4.),
        ([[0.1, 0.], [0., 0.2]], 1.),
        ([[1e30, 0.], [0., 1e30]], 4.),
    ] {
        assert_eq!(
            auto_raster_scale(
                TransformationMatrix {
                    rotation_scale: linear,
                    ..TransformationMatrix::unit()
                },
                None
            ),
            expected
        );
    }
    let mut previous = None;
    for (zoom, expected) in [
        (1., 1.),
        (1.01, 2.),
        (0.99, 2.),
        (0.81, 2.),
        (0.79, 1.),
        (2.01, 4.),
        (1.99, 4.),
        (1.61, 4.),
        (1.59, 2.),
        (0.5, 1.),
    ] {
        let tier = auto_raster_scale(
            TransformationMatrix::unit().scale(size(zoom, zoom)),
            previous,
        );
        assert_eq!(tier, expected, "zoom {zoom}");
        previous = Some(tier);
    }
}

#[derive(Clone, Default)]
struct Observations {
    renders: Rc<Cell<usize>>,
    prepaint: Rc<Cell<f32>>,
    paint: Rc<Cell<f32>>,
    bounds: Rc<Cell<Bounds<Pixels>>>,
    click: Rc<Cell<Option<Point<Pixels>>>>,
}

struct Content(Observations);

impl Render for Content {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let o = self.0.clone();
        o.renders.set(o.renders.get() + 1);
        let paint = o.paint.clone();
        div()
            .w(px(100.))
            .h(px(80.))
            .on_mouse_down(MouseButton::Left, move |event, _, _| {
                o.click.set(Some(event.position))
            })
            .child(
                canvas(
                    move |bounds, window, _| {
                        o.prepaint
                            .set(window.raster_scale_factor() / window.scale_factor());
                        o.bounds.set(bounds);
                    },
                    move |bounds, _, window, _| {
                        paint.set(window.raster_scale_factor() / window.scale_factor());
                        window.paint_quad(gpui::fill(bounds, gpui::rgb(0xff0000)));
                    },
                )
                .size_full(),
            )
    }
}

struct Root {
    zoom: f32,
    content: Entity<Content>,
}

impl Render for Root {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        transform_group(
            self.content
                .clone()
                .cached(div().w(px(100.)).h(px(80.)).style().clone())
                .cache_across_transforms(),
            TransformationMatrix::unit().scale(size(self.zoom, self.zoom)),
        )
        .auto_raster_scale("density")
    }
}

#[gpui::test]
fn automatic_density_persists_across_frames_and_preserves_cached_input(cx: &mut TestAppContext) {
    for supported in [true, false] {
        let observations = Observations::default();
        let handle = cx.open_window(size(px(400.), px(200.)), {
            let observations = observations.clone();
            move |_, cx| Root {
                zoom: 1.,
                content: cx.new(|_| Content(observations)),
            }
        });
        cx.set_subtree_effects_supported(handle.into(), supported);
        cx.update_window(handle.into(), |_, window, cx| window.draw(cx).clear())
            .unwrap();
        let bounds = observations.bounds.get();
        let mut last_tier = 1.;
        for (zoom, tier) in [
            (1.01, 2.),
            (0.99, 2.),
            (0.81, 2.),
            (0.79, 1.),
            (2.01, 4.),
            (1.99, 4.),
            (1.61, 4.),
            (1.59, 2.),
        ] {
            let tier = if supported { tier } else { 1. };
            let before = observations.renders.get();
            handle
                .update(cx, |root, _, cx| {
                    root.zoom = zoom;
                    cx.notify();
                })
                .unwrap();
            cx.update_window(handle.into(), |_, window, cx| window.draw(cx).clear())
                .unwrap();
            assert_eq!(observations.prepaint.get(), tier, "prepaint at {zoom}");
            assert_eq!(observations.paint.get(), tier, "paint at {zoom}");
            assert_eq!(observations.bounds.get(), bounds);
            if tier == last_tier {
                assert_eq!(
                    observations.renders.get(),
                    before,
                    "cached content at {zoom}"
                );
            } else {
                assert!(observations.renders.get() > before);
            }
            last_tier = tier;
            observations.click.set(None);
            let source = point(px(5.), px(5.));
            cx.update_window(handle.into(), |_, window, cx| {
                window.dispatch_event(
                    PlatformInput::MouseDown(MouseDownEvent {
                        position: bounds.origin + source * if supported { zoom } else { 1. },
                        ..Default::default()
                    }),
                    cx,
                );
            })
            .unwrap();
            let actual = observations
                .click
                .get()
                .expect("transformed click reached content");
            assert!((actual.x - (bounds.origin.x + source.x)).abs() < px(0.001));
            assert!((actual.y - (bounds.origin.y + source.y)).abs() < px(0.001));
        }
    }
}
