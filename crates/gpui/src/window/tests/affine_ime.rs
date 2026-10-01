use crate::{
    App, AppContext, Bounds, Context, Entity, FocusHandle, InputHandler, Pixels, PlatformWindow,
    Point, PointerTransform, Render, TestAppContext, TransformationMatrix, UTF16Selection, Window,
    canvas, div, point, prelude::*, px, size,
};
use std::{cell::Cell, ops::Range, rc::Rc};

#[test]
fn ime_candidate_line_follows_the_head_of_a_reversed_preedit_selection() {
    let selection = UTF16Selection {
        range: 2..6,
        reversed: true,
    };
    let bounds = |range: Range<usize>| {
        Some(Bounds::new(
            point(
                px((range.start % 4) as f32 * 10.),
                px((range.start / 4) as f32 * 20.),
            ),
            size(px(1.), px(20.)),
        ))
    };
    let candidate =
        crate::PlatformInputHandler::compute_ime_candidate_bounds(Some(0..8), &selection, bounds)
            .unwrap();
    assert_eq!(candidate.origin, point(px(0.), px(0.)));
}

#[derive(Clone)]
struct TextHandler {
    composing: Rc<Cell<bool>>,
    _owner: Option<Entity<()>>,
}

impl InputHandler for TextHandler {
    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut App,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: 5..5,
            reversed: false,
        })
    }

    fn marked_text_range(&mut self, _: &mut Window, _: &mut App) -> Option<Range<usize>> {
        self.composing.get().then_some(1..5)
    }

    fn text_for_range(
        &mut self,
        _: Range<usize>,
        _: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut App,
    ) -> Option<String> {
        None
    }

    fn replace_text_in_range(
        &mut self,
        _: Option<Range<usize>>,
        _: &str,
        _: &mut Window,
        _: &mut App,
    ) {
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        _: Option<Range<usize>>,
        _: &str,
        _: Option<Range<usize>>,
        _: &mut Window,
        _: &mut App,
    ) {
    }

    fn unmark_text(&mut self, _: &mut Window, _: &mut App) {
        self.composing.set(false);
    }

    fn bounds_for_range(
        &mut self,
        range: Range<usize>,
        _: &mut Window,
        _: &mut App,
    ) -> Option<Bounds<Pixels>> {
        Some(Bounds::new(
            point(
                px(10. + (range.start % 3) as f32 * 10.),
                px(20. + (range.start / 3) as f32 * 20.),
            ),
            size(px(2.), px(10.)),
        ))
    }

    fn character_index_for_point(
        &mut self,
        position: Point<Pixels>,
        _: &mut Window,
        _: &mut App,
    ) -> Option<usize> {
        (position == point(px(30.), px(40.))).then_some(5)
    }

    fn element_bounds(&mut self, _: &mut Window, _: &mut App) -> Option<Bounds<Pixels>> {
        Some(Bounds::new(point(px(0.), px(0.)), size(px(60.), px(60.))))
    }
}

struct TextView {
    focus: FocusHandle,
    handler: TextHandler,
    renders: Rc<Cell<usize>>,
}

impl Render for TextView {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        self.renders.set(self.renders.get() + 1);
        let focus = self.focus.clone();
        let handler = self.handler.clone();
        canvas(
            |_, _, _| (),
            move |_, (), window, cx| {
                window.handle_input(&focus, handler.clone(), cx);
            },
        )
        .size_full()
    }
}

struct MappedText {
    text: Entity<TextView>,
    transform: PointerTransform,
    nested: bool,
    cache_across_transforms: bool,
}

impl Render for MappedText {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let text = self.text.clone();
        let prepaint_transform = self.transform.clone();
        let paint_transform = self.transform.clone();
        let nested = self.nested;
        let cache_across_transforms = self.cache_across_transforms;
        canvas(
            move |bounds, window, cx| {
                let mut element = text
                    .clone()
                    .cached(div().size_full().style().clone())
                    .when(cache_across_transforms, |view| {
                        view.cache_across_transforms()
                    })
                    .into_any_element();
                with_scopes(window, bounds, prepaint_transform, nested, |window| {
                    element.prepaint_as_root(bounds.origin, bounds.size.into(), window, cx);
                });
                element
            },
            move |bounds, mut element, window, cx| {
                with_scopes(window, bounds, paint_transform, nested, |window| {
                    element.paint(window, cx)
                });
            },
        )
        .size_full()
    }
}

fn with_scopes<R>(
    window: &mut Window,
    bounds: Bounds<Pixels>,
    transform: PointerTransform,
    nested: bool,
    f: impl FnOnce(&mut Window) -> R,
) -> R {
    window.with_pointer_transform(bounds, transform, |window| {
        if nested {
            window.with_pointer_transform(
                bounds,
                PointerTransform::affine(TransformationMatrix {
                    translation: [10., 5.],
                    ..TransformationMatrix::unit()
                })
                .unwrap(),
                f,
            )
        } else {
            f(window)
        }
    })
}

#[crate::test]
fn affine_ime_maps_platform_queries_cached_handlers_and_candidate_updates(cx: &mut TestAppContext) {
    check_ime_cache(cx, false);
}

#[crate::test]
fn affine_ime_rebases_cached_handlers_and_candidate_positions_without_rendering(
    cx: &mut TestAppContext,
) {
    check_ime_cache(cx, true);
}

fn check_ime_cache(cx: &mut TestAppContext, cache_across_transforms: bool) {
    let composing = Rc::new(Cell::new(true));
    let renders = Rc::new(Cell::new(0));
    let scale = PointerTransform::affine(TransformationMatrix {
        rotation_scale: [[2., 0.], [0., 2.]],
        translation: [100., 50.],
    })
    .unwrap();
    let rotation = PointerTransform::affine(TransformationMatrix {
        rotation_scale: [[0., -2.], [2., 0.]],
        translation: [200., 100.],
    })
    .unwrap();
    let handle = cx.add_window({
        let composing = composing.clone();
        let renders = renders.clone();
        move |window, cx| {
            let focus = cx.focus_handle();
            focus.focus(window, cx);
            MappedText {
                text: cx.new(|_| TextView {
                    focus,
                    handler: TextHandler {
                        composing,
                        _owner: None,
                    },
                    renders,
                }),
                transform: PointerTransform::identity(),
                nested: false,
                cache_across_transforms,
            }
        }
    });
    cx.update_window(handle.into(), |_, window, cx| window.draw(cx).clear())
        .unwrap();
    renders.set(0);
    // Source geometry spans two lines; nested scopes also exercise the 2x TestWindow density.
    for (transform, nested, candidate, caret, element, hit, expected_renders) in [
        (
            scale.clone(),
            true,
            (140., 140., 4., 20.),
            (180., 140., 4., 20.),
            (120., 60., 120., 120.),
            (180., 140.),
            1,
        ),
        (
            scale,
            true,
            (140., 140., 4., 20.),
            (180., 140., 4., 20.),
            (120., 60., 120., 120.),
            (180., 140.),
            1,
        ),
        (
            rotation,
            true,
            (90., 140., 20., 4.),
            (90., 180., 20., 4.),
            (70., 120., 120., 120.),
            (110., 180.),
            2,
        ),
        // Unknown forward maps retain source bounds, but character queries use their inverse.
        (
            PointerTransform::new(|p, _, _| p - point(px(7.), px(0.))),
            false,
            (10., 40., 2., 10.),
            (30., 40., 2., 10.),
            (0., 0., 60., 60.),
            (37., 40.),
            0,
        ),
        (
            PointerTransform::identity(),
            false,
            (10., 40., 2., 10.),
            (30., 40., 2., 10.),
            (0., 0., 60., 60.),
            (30., 40.),
            0,
        ),
    ] {
        composing.set(true);
        handle
            .update(cx, |root, _, cx| {
                root.transform = transform;
                root.nested = nested;
                cx.notify();
            })
            .unwrap();
        cx.update_window(handle.into(), |_, window, cx| window.draw(cx).clear())
            .unwrap();
        if expected_renders != 0 {
            assert_eq!(
                renders.get(),
                if cache_across_transforms {
                    1
                } else {
                    expected_renders
                }
            );
        }
        let mut platform = cx.test_window(handle.into());
        assert_eq!(platform.0.lock().ime_position, Some(rect(candidate)));
        let mut input = platform.take_input_handler().unwrap();
        assert_eq!(input.ime_candidate_bounds(), Some(rect(candidate)));
        assert_eq!(input.bounds_for_range(5..5), Some(rect(caret)));
        assert_eq!(input.element_bounds(), Some(rect(element)));
        assert_eq!(
            input.character_index_for_point(point(px(hit.0), px(hit.1))),
            Some(5)
        );
        cx.update_window(handle.into(), |_, window, cx| {
            assert_eq!(input.selected_bounds(window, cx), Some(rect(candidate)));
        })
        .unwrap();
        composing.set(false);
        assert_eq!(input.ime_candidate_bounds(), Some(rect(caret)));
        platform.set_input_handler(input);
    }
}

fn rect((x, y, width, height): (f32, f32, f32, f32)) -> Bounds<Pixels> {
    Bounds::new(point(px(x), px(y)), size(px(width), px(height)))
}

#[crate::test]
fn closing_window_releases_platform_input_owner(cx: &mut TestAppContext) {
    check_input_owner_release(cx, false);
}

#[crate::test]
fn shutdown_releases_platform_input_owner(cx: &mut TestAppContext) {
    check_input_owner_release(cx, true);
}

fn check_input_owner_release(cx: &mut TestAppContext, shutdown: bool) {
    let owner = cx.new(|_| ());
    let weak_owner = owner.downgrade();
    let handle = cx.add_window(move |window, cx| {
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        TextView {
            focus,
            handler: TextHandler {
                composing: Rc::new(Cell::new(true)),
                _owner: Some(owner),
            },
            renders: Default::default(),
        }
    });
    cx.update_window(handle.into(), |_, window, cx| window.draw(cx).clear())
        .unwrap();
    // Native platform state can outlive the GPUI window during asynchronous teardown.
    let mut platform = cx.test_window(handle.into());
    let input = platform
        .take_input_handler()
        .expect("focused input handler");
    platform.set_input_handler(input);
    if shutdown {
        cx.update(|cx| cx.shutdown());
    } else {
        cx.update_window(handle.into(), |_, window, _| window.remove_window())
            .unwrap();
    }
    assert!(
        platform.take_input_handler().is_none(),
        "closed window retained its input handler"
    );
    assert!(
        weak_owner.upgrade().is_none(),
        "input owner survived window teardown"
    );
}
