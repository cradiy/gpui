use super::animation_frame_interval;
use crate::{AnimationFramePolicy, RequestFrameOptions, ThermalState};
use crate::{
    AppContext as _, Bounds, Context, IntoElement, ParentElement as _, Pixels, Render, Styled as _,
    TestAppContext, Window, canvas, div, px, size,
};
use std::{cell::Cell, rc::Rc};

#[test]
fn animation_frame_policy_only_relaxes_unfocused_throttling() {
    for active in [false, true] {
        for (options, callbacks) in [
            (RequestFrameOptions::default(), true),
            (
                RequestFrameOptions {
                    require_presentation: true,
                    force_render: false,
                },
                false,
            ),
        ] {
            assert_eq!(
                animation_frame_interval(
                    AnimationFramePolicy::Default,
                    active,
                    None,
                    options,
                    callbacks
                ),
                (!active).then_some(std::time::Duration::from_micros(33333)),
            );
            assert_eq!(
                animation_frame_interval(
                    AnimationFramePolicy::FollowDisplay,
                    active,
                    None,
                    options,
                    callbacks
                ),
                None,
            );
        }
    }
}

#[test]
fn animation_frame_policy_preserves_thermal_limits() {
    for active in [false, true] {
        for thermal in [ThermalState::Serious, ThermalState::Critical] {
            assert_eq!(
                animation_frame_interval(
                    AnimationFramePolicy::FollowDisplay,
                    active,
                    Some(thermal),
                    RequestFrameOptions::default(),
                    true
                ),
                Some(std::time::Duration::from_micros(16667)),
            );
        }
    }
}

#[test]
fn animation_frame_policy_never_delays_recovery_or_event_driven_draws() {
    for policy in [
        AnimationFramePolicy::Default,
        AnimationFramePolicy::FollowDisplay,
    ] {
        for (options, callbacks) in [
            (
                RequestFrameOptions {
                    force_render: true,
                    require_presentation: true,
                },
                true,
            ),
            (RequestFrameOptions::default(), false),
        ] {
            assert_eq!(
                animation_frame_interval(
                    policy,
                    false,
                    Some(ThermalState::Critical),
                    options,
                    callbacks
                ),
                None,
            );
        }
    }
}

struct PointerProbe {
    events: Rc<std::cell::RefCell<Vec<(&'static str, crate::Point<Pixels>, crate::Point<Pixels>)>>>,
    transform: crate::PointerTransform,
}

impl Render for PointerProbe {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let events = self.events.clone();
        let transform = self.transform.clone();
        let prepaint_transform = transform.clone();
        canvas(
            move |bounds, window, _| {
                window.with_pointer_transform(bounds, prepaint_transform, |window| {
                    window.insert_hitbox(
                        Bounds::new(crate::point(px(20.), px(20.)), size(px(40.), px(40.))),
                        super::HitboxBehavior::Normal,
                    )
                })
            },
            move |bounds, hitbox, window, _| {
                let outside = events.clone();
                window.on_mouse_event(move |event: &crate::MouseMoveEvent, phase, window, _| {
                    if phase == crate::DispatchPhase::Bubble {
                        outside
                            .borrow_mut()
                            .push(("raw", event.position, window.mouse_position()));
                    }
                });
                window.with_pointer_transform(bounds, transform, |window| {
                    let down_hitbox = hitbox.clone();
                    let down_events = events.clone();
                    window.on_mouse_event(
                        move |event: &crate::MouseDownEvent, phase, window, _| {
                            if phase == crate::DispatchPhase::Bubble
                                && down_hitbox.is_hovered(window)
                            {
                                window.capture_pointer(down_hitbox.id);
                                down_events.borrow_mut().push((
                                    "down",
                                    event.position,
                                    window.mouse_position(),
                                ));
                            }
                        },
                    );
                    let move_hitbox = hitbox.clone();
                    let move_events = events.clone();
                    window.on_mouse_event(
                        move |event: &crate::MouseMoveEvent, phase, window, _| {
                            if phase == crate::DispatchPhase::Bubble
                                && move_hitbox.is_hovered(window)
                            {
                                move_events.borrow_mut().push((
                                    "move",
                                    event.position,
                                    window.mouse_position(),
                                ));
                            }
                        },
                    );
                    window.on_mouse_event(move |event: &crate::MouseUpEvent, phase, window, _| {
                        if phase == crate::DispatchPhase::Bubble {
                            events.borrow_mut().push((
                                "up",
                                event.position,
                                window.mouse_position(),
                            ));
                        }
                    });
                });
            },
        )
        .w(px(400.))
        .h(px(200.))
    }
}

#[test]
fn pointer_mapping_routes_hits_and_captured_events_in_source_coordinates() {
    run_pointer_mapping_probe(crate::PointerTransform::new(|position, _, _| {
        position - crate::point(px(100.), px(0.))
    }));
}

#[test]
fn pointer_mapping_affine_routes_hits_and_captured_events_in_source_coordinates() {
    run_pointer_mapping_probe(
        crate::PointerTransform::affine(crate::TransformationMatrix {
            translation: [100., 0.],
            ..crate::TransformationMatrix::unit()
        })
        .unwrap(),
    );
}

fn run_pointer_mapping_probe(transform: crate::PointerTransform) {
    use crate::{MouseDownEvent, MouseMoveEvent, MouseUpEvent, PlatformInput, point};
    let mut cx = TestAppContext::single();
    let events = Rc::new(std::cell::RefCell::new(Vec::new()));
    let window = cx.add_window({
        let events = events.clone();
        move |_, _| PointerProbe { events, transform }
    });
    cx.update_window(window.into(), |_, window, cx| {
        window.draw(cx).clear();
        window.dispatch_event(
            PlatformInput::MouseDown(MouseDownEvent {
                position: point(px(30.), px(30.)),
                ..Default::default()
            }),
            cx,
        );
        assert!(events.borrow().is_empty());
        window.dispatch_event(
            PlatformInput::MouseMove(MouseMoveEvent {
                position: point(px(130.), px(30.)),
                ..Default::default()
            }),
            cx,
        );
        window.dispatch_event(
            PlatformInput::MouseDown(MouseDownEvent {
                position: point(px(130.), px(30.)),
                ..Default::default()
            }),
            cx,
        );
        window.dispatch_event(
            PlatformInput::MouseMove(MouseMoveEvent {
                position: point(px(450.), px(30.)),
                pressed_button: Some(crate::MouseButton::Left),
                ..Default::default()
            }),
            cx,
        );
        window.dispatch_event(
            PlatformInput::MouseUp(MouseUpEvent {
                position: point(px(450.), px(30.)),
                ..Default::default()
            }),
            cx,
        );
        assert_eq!(window.mouse_position(), point(px(450.), px(30.)));
    })
    .unwrap();
    let events = events.borrow();
    assert!(events.contains(&("down", point(px(30.), px(30.)), point(px(30.), px(30.)))));
    assert!(events.contains(&("move", point(px(350.), px(30.)), point(px(350.), px(30.)))));
    assert!(events.contains(&("up", point(px(350.), px(30.)), point(px(350.), px(30.)))));
    assert!(events.contains(&("raw", point(px(450.), px(30.)), point(px(450.), px(30.)))));
}

mod affine_a11y;
mod affine_cache;
mod affine_deferred;
mod affine_ime;
mod affine_tooltip;
mod cached_a11y;
mod deferred_overlay;
mod element_bounds;
mod prepaint_retry;
mod raster_capture;

struct RootView {
    explicit_size: bool,
    child_bounds: Rc<Cell<Bounds<Pixels>>>,
}

impl Render for RootView {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let child_bounds = self.child_bounds.clone();
        let root = div().flex().flex_col().child(
            canvas(
                move |bounds, _, _| child_bounds.set(bounds),
                |_, _, _, _| {},
            )
            .size_full(),
        );
        if self.explicit_size {
            root.w(px(300.)).h(px(200.))
        } else {
            root
        }
    }
}

#[test]
fn auto_sized_window_root_fills_the_window() {
    let mut cx = TestAppContext::single();
    let child_bounds = Rc::new(Cell::new(Bounds::default()));
    let window = cx.add_window({
        let child_bounds = child_bounds.clone();
        move |_, _| RootView {
            explicit_size: false,
            child_bounds,
        }
    });

    let viewport_size = cx
        .update_window(window.into(), |_, window, cx| {
            window.draw(cx).clear();
            window.viewport_size()
        })
        .unwrap();

    assert_eq!(child_bounds.get().size, viewport_size);
}

#[test]
fn explicitly_sized_window_root_keeps_its_size() {
    let mut cx = TestAppContext::single();
    let child_bounds = Rc::new(Cell::new(Bounds::default()));
    let window = cx.add_window({
        let child_bounds = child_bounds.clone();
        move |_, _| RootView {
            explicit_size: true,
            child_bounds,
        }
    });

    cx.update_window(window.into(), |_, window, cx| {
        window.draw(cx).clear();
    })
    .unwrap();

    assert_eq!(child_bounds.get().size, size(px(300.), px(200.)));
}

struct TextureContent {
    renders: Rc<Cell<usize>>,
    bounds: Rc<Cell<Bounds<Pixels>>>,
}

impl Render for TextureContent {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        self.renders.set(self.renders.get() + 1);
        let bounds = self.bounds.clone();
        canvas(
            move |actual, _, _| bounds.set(actual),
            |bounds, _, window, _| window.paint_quad(crate::fill(bounds, crate::rgb(0xff0000))),
        )
        .size_full()
    }
}

struct TextureRoot {
    content: crate::Entity<TextureContent>,
    density: f32,
}

impl Render for TextureRoot {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let config = crate::UiTexture3d::new(size(px(640.), px(400.)), self.density);
        let content = self.content.clone();
        div().size(px(60.)).overflow_hidden().child(
            canvas(
                move |_, window, cx| {
                    let original_mask = window.content_mask();
                    let original_scale = window.scale_factor();
                    let content = window.with_scene3d_texture(config, |window| {
                        let style = div().w(px(640.)).h(px(400.)).style().clone();
                        let mut content = content.cached(style).into_any_element();
                        content.prepaint_as_root(
                            Default::default(),
                            config.logical_size().into(),
                            window,
                            cx,
                        );
                        content
                    });
                    assert_eq!(window.content_mask(), original_mask);
                    assert_eq!(window.scale_factor(), original_scale);
                    content
                },
                move |_, mut content, window, cx| {
                    let original_mask = window.content_mask();
                    let original_scale = window.scale_factor();
                    window.with_scene3d_texture(config, |window| content.paint(window, cx));
                    assert_eq!(window.content_mask(), original_mask);
                    assert_eq!(window.scale_factor(), original_scale);
                },
            )
            .size_full(),
        )
    }
}

#[crate::test]
fn ui_texture_density_invalidates_cached_paint_without_changing_layout(cx: &mut TestAppContext) {
    let renders = Rc::new(Cell::new(0));
    let bounds = Rc::new(Cell::new(Bounds::default()));
    let window = cx.add_window({
        let renders = renders.clone();
        let bounds = bounds.clone();
        move |_, cx| TextureRoot {
            content: cx.new(|_| TextureContent { renders, bounds }),
            density: 1.,
        }
    });
    for (density, render_count) in [(1., 1), (1., 1), (2., 2), (2., 2), (1., 3)] {
        window
            .update(cx, |root, _, cx| {
                root.density = density;
                cx.notify();
            })
            .unwrap();
        cx.update_window(window.into(), |_, window, cx| {
            window.draw(cx).clear();
            let quad = window.rendered_frame.scene.quads.last().unwrap();
            assert_eq!(quad.bounds.size.width.0, 640. * density);
            assert_eq!(quad.bounds.size.height.0, 400. * density);
            assert_eq!(quad.content_mask.bounds.size.width.0, 640. * density);
        })
        .unwrap();
        assert_eq!(
            bounds.get(),
            Bounds::new(Default::default(), size(px(640.), px(400.)))
        );
        assert_eq!(renders.get(), render_count);
    }
}
