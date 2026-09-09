use crate::{
    Hit, ObjectId, Texture,
    spatial::picking::{DragProjection, PickSnapshot},
};
use gpui::{
    Context, DispatchPhase, Hitbox, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    Pixels, Point, PointerTransform, ScrollWheelEvent, Size, Subscription, Window, point, px,
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

struct Drag {
    projection: DragProjection,
    size: Size<Pixels>,
    last: Cell<Point<Pixels>>,
}

#[derive(Default)]
struct InputState {
    snapshot: Rc<RefCell<Option<PickSnapshot>>>,
    target: Option<ObjectId>,
    size: Size<Pixels>,
    drag: Option<Drag>,
    generation: u64,
}

impl InputState {
    fn hit(&self, position: Point<Pixels>) -> Option<Hit> {
        let snapshot = self.snapshot.borrow();
        let snapshot = snapshot.as_ref()?;
        let hit = snapshot.pick(position)?;
        (hit.object_id.as_ref() == self.target.as_ref()
            && self.target.is_some()
            && matches!(
                snapshot.scene.objects[hit.object_index].material.texture,
                Texture::Ui
            ))
        .then_some(hit)
    }

    fn map(&self, position: Point<Pixels>) -> Point<Pixels> {
        if let Some(drag) = &self.drag {
            if let Some(uv) = drag.projection.project(position) {
                drag.last.set(source_position(uv, drag.size));
            }
            return drag.last.get();
        }
        self.hit(position).map_or(point(px(-1e6), px(-1e6)), |hit| {
            source_position(hit.uv.map(|v| v.clamp(0., 1.)), self.size)
        })
    }

    fn cancel(&mut self, window: &mut Window) {
        self.generation += 1;
        if self.drag.take().is_some() {
            window.release_pointer();
        }
    }
}

fn source_position(uv: [f32; 2], size: Size<Pixels>) -> Point<Pixels> {
    point(size.width * uv[0], size.height * uv[1])
}

pub(crate) struct UiInput {
    state: Rc<RefCell<InputState>>,
    _activation: Subscription,
}

impl UiInput {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            state: Rc::default(),
            _activation: cx.observe_window_activation(window, |this, window, _| {
                if !window.is_window_active() {
                    this.state.borrow_mut().cancel(window);
                }
            }),
        }
    }

    pub fn prepare(&self, target: Option<ObjectId>, size: Size<Pixels>, window: &mut Window) {
        let mut state = self.state.borrow_mut();
        if state.target != target || state.size != size {
            state.cancel(window);
        }
        state.target = target;
        state.size = size;
    }

    pub fn set_snapshot(&self, snapshot: Rc<RefCell<Option<PickSnapshot>>>) {
        self.state.borrow_mut().snapshot = snapshot;
    }

    pub fn transform(&self) -> PointerTransform {
        let state = self.state.clone();
        let visible = state.clone();
        PointerTransform::projected(
            move |position, _, _| state.borrow().map(position),
            move |position, _, _| {
                let state = visible.borrow();
                state
                    .hit(position)
                    .map(|hit| source_position(hit.uv.map(|v| v.clamp(0., 1.)), state.size))
            },
        )
    }

    pub fn paint(&self, hitbox: &Hitbox, window: &mut Window) {
        let state = self.state.clone();
        let down_hitbox = hitbox.clone();
        window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
            if event.button != MouseButton::Left || !down_hitbox.is_hovered(window) {
                return;
            }
            if phase == DispatchPhase::Capture {
                let mut state = state.borrow_mut();
                if let Some(hit) = state.hit(event.position) {
                    let projection =
                        DragProjection::new(state.snapshot.borrow().as_ref().unwrap(), &hit);
                    state.generation += 1;
                    state.drag = Some(Drag {
                        projection,
                        size: state.size,
                        last: Cell::new(source_position(hit.uv, state.size)),
                    });
                }
            } else if state.borrow().drag.is_some() {
                cx.stop_propagation();
            }
        });
        let state = self.state.clone();
        window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
            if phase == DispatchPhase::Capture && event.pressed_button != Some(MouseButton::Left) {
                state.borrow_mut().cancel(window);
            } else if phase == DispatchPhase::Bubble && state.borrow().drag.is_some() {
                cx.stop_propagation();
            }
        });
        let state = self.state.clone();
        window.on_mouse_event(move |event: &MouseUpEvent, phase, _, cx| {
            if event.button != MouseButton::Left || state.borrow().drag.is_none() {
                return;
            }
            if phase == DispatchPhase::Capture {
                let generation = state.borrow().generation;
                let state = state.clone();
                cx.defer(move |_| {
                    let mut state = state.borrow_mut();
                    if state.generation == generation {
                        state.drag = None;
                    }
                });
            } else {
                cx.stop_propagation();
            }
        });
        let hitbox = hitbox.clone();
        window.on_mouse_event(move |_: &ScrollWheelEvent, phase, window, cx| {
            if phase == DispatchPhase::Bubble && hitbox.should_handle_scroll(window) {
                cx.stop_propagation();
            }
        });
    }
}

#[cfg(all(test, feature = "test-support"))]
mod tests {
    use super::*;
    use crate::{
        Camera, Material, Mesh, Object, PickBehavior, Scene, math::transform,
        spatial::picking::PickSurface,
    };
    use gpui::{
        AppContext as _, Bounds, Entity, IntoElement, PlatformInput, Render, ScrollDelta,
        ScrollHandle, TestAppContext, UiTexture3d, canvas, div, prelude::*, size,
    };
    use uic::components::slider::{Slider, SliderState};

    struct Controls {
        input: Entity<UiInput>,
        slider: Entity<SliderState>,
        scroll: ScrollHandle,
        viewport: Rc<Cell<Bounds<Pixels>>>,
        clicks: usize,
        hovered: bool,
        camera_down: usize,
        camera_scroll: usize,
        covered: bool,
        overlay: bool,
        overlay_clicks: usize,
        enabled: bool,
        orthographic: bool,
    }

    impl Controls {
        fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
            Self {
                input: cx.new(|cx| UiInput::new(window, cx)),
                slider: cx.new(|cx| SliderState::new(50., 0. ..=100., cx)),
                scroll: ScrollHandle::new(),
                viewport: Rc::default(),
                clicks: 0,
                hovered: false,
                camera_down: 0,
                camera_scroll: 0,
                covered: false,
                overlay: false,
                overlay_clicks: 0,
                enabled: true,
                orthographic: false,
            }
        }

        fn scene(&self) -> Scene {
            let mut camera = Camera::orbit(0.2, 0.1, 6.);
            if self.orthographic {
                camera.projection = crate::Projection::Orthographic { vertical_size: 5. };
            }
            let scene = Scene::new().camera(camera).object(
                Object::new(Mesh::plane(), Material::ui())
                    .id("panel")
                    .scale([4., 3., 1.])
                    .rotation([0.1, -0.3, 0.]),
            );
            if self.covered {
                scene.object(
                    Object::new(Mesh::plane(), Material::color(gpui::white()))
                        .position([0., 0., 2.])
                        .scale([20., 20., 1.])
                        .pick_behavior(PickBehavior::Occlude),
                )
            } else {
                scene
            }
        }

        fn screen(&self, x: f32, y: f32) -> Point<Pixels> {
            let scene = self.scene();
            let world = transform(
                scene.objects[0].transform.matrices().0,
                [x / 400. - 0.5, 0.5 - y / 300., 0., 1.],
            );
            let bounds = self.viewport.get();
            let p = transform(
                scene.camera.matrix(bounds.size.width / bounds.size.height),
                world,
            );
            bounds.origin
                + point(
                    bounds.size.width * (p[0] / p[3] + 1.) * 0.5,
                    bounds.size.height * (1. - p[1] / p[3]) * 0.5,
                )
        }
    }

    impl Render for Controls {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let mut ui = div()
                .relative()
                .w(px(400.))
                .h(px(300.))
                .child(
                    div()
                        .id("button")
                        .absolute()
                        .left(px(20.))
                        .top(px(20.))
                        .w(px(120.))
                        .h(px(40.))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.clicks += 1;
                            cx.notify();
                        }))
                        .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                            this.hovered = *hovered;
                            cx.notify();
                        })),
                )
                .child(
                    Slider::new(&self.slider)
                        .absolute()
                        .left(px(20.))
                        .top(px(100.))
                        .w(px(360.))
                        .h(px(28.)),
                )
                .child(
                    div()
                        .id("scroll")
                        .absolute()
                        .left(px(20.))
                        .top(px(170.))
                        .w(px(360.))
                        .h(px(100.))
                        .overflow_y_scroll()
                        .track_scroll(&self.scroll)
                        .child(div().h(px(600.))),
                )
                .into_any_element();
            let snapshot_scene = self.scene();
            let input = self.input.clone();
            let paint_input = input.clone();
            let viewport = self.viewport.clone();
            let enabled = self.enabled;
            let config = UiTexture3d::new(size(px(400.), px(300.)), 2.);
            div()
                .relative()
                .w(px(880.))
                .h(px(680.))
                .p(px(40.))
                .id("orbit")
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, _, _| this.camera_down += 1),
                )
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(|this, _, _, _| this.camera_down += 1),
                )
                .on_scroll_wheel(cx.listener(|this, _, _, _| this.camera_scroll += 1))
                .child(
                    canvas(
                        move |bounds, window, cx| {
                            viewport.set(bounds);
                            let surfaces = snapshot_scene
                                .objects
                                .iter()
                                .map(|_| PickSurface::Solid)
                                .collect();
                            let input = input.read(cx);
                            input.prepare(
                                enabled.then(|| "panel".into()),
                                config.logical_size(),
                                window,
                            );
                            input.set_snapshot(Rc::new(RefCell::new(Some(PickSnapshot {
                                scene: snapshot_scene,
                                bounds,
                                surfaces,
                            }))));
                            let mapping = input.transform();
                            let hitbox =
                                window.with_pointer_transform(bounds, mapping.clone(), |window| {
                                    window.with_scene3d_texture(config, |window| {
                                        let hitbox = window.insert_hitbox(
                                            Bounds::new(Default::default(), config.logical_size()),
                                            gpui::HitboxBehavior::Normal,
                                        );
                                        ui.prepaint_as_root(
                                            Default::default(),
                                            config.logical_size().into(),
                                            window,
                                            cx,
                                        );
                                        hitbox
                                    })
                                });
                            (ui, mapping, hitbox)
                        },
                        move |bounds, (mut ui, mapping, hitbox), window, cx| {
                            paint_input.read(cx).paint(&hitbox, window);
                            window.with_pointer_transform(bounds, mapping, |window| {
                                window.with_scene3d_texture(config, |window| ui.paint(window, cx));
                            });
                        },
                    )
                    .size_full(),
                )
                .when(self.overlay, |root| {
                    root.child(
                        div()
                            .id("overlay")
                            .absolute()
                            .inset_0()
                            .occlude()
                            .on_click(cx.listener(|this, _, _, _| this.overlay_clicks += 1)),
                    )
                })
        }
    }

    fn event(cx: &mut TestAppContext, handle: gpui::WindowHandle<Controls>, event: PlatformInput) {
        cx.update_window(handle.into(), |_, window, cx| {
            window.draw(cx).clear();
            window.dispatch_event(event, cx);
        })
        .unwrap();
    }

    fn button(
        cx: &mut TestAppContext,
        handle: gpui::WindowHandle<Controls>,
        position: Point<Pixels>,
        down: bool,
        button: MouseButton,
    ) {
        event(
            cx,
            handle,
            if down {
                PlatformInput::MouseDown(MouseDownEvent {
                    position,
                    button,
                    ..Default::default()
                })
            } else {
                PlatformInput::MouseUp(MouseUpEvent {
                    position,
                    button,
                    ..Default::default()
                })
            },
        );
    }

    #[gpui::test]
    fn mapped_controls_preserve_click_drag_scroll_and_occlusion(cx: &mut TestAppContext) {
        let handle = cx.add_window(Controls::new);
        cx.update_window(handle.into(), |_, window, cx| window.draw(cx).clear())
            .unwrap();
        let point = |cx: &mut TestAppContext, x, y| {
            handle.update(cx, |view, _, _| view.screen(x, y)).unwrap()
        };
        let p = point(cx, 30., 30.);
        event(
            cx,
            handle,
            PlatformInput::MouseMove(MouseMoveEvent {
                position: p,
                ..Default::default()
            }),
        );
        assert!(handle.update(cx, |view, _, _| view.hovered).unwrap());
        button(cx, handle, p, true, MouseButton::Left);
        button(cx, handle, p, false, MouseButton::Left);
        handle
            .update(cx, |view, _, _| {
                assert_eq!(view.clicks, 1);
                assert_eq!(view.camera_down, 0);
            })
            .unwrap();

        let start = point(cx, 200., 114.);
        let outside = point(cx, 800., 114.);
        button(cx, handle, start, true, MouseButton::Left);
        event(
            cx,
            handle,
            PlatformInput::MouseMove(MouseMoveEvent {
                position: outside,
                pressed_button: Some(MouseButton::Left),
                ..Default::default()
            }),
        );
        button(cx, handle, outside, false, MouseButton::Left);
        handle
            .update(cx, |view, window, cx| {
                assert!(view.slider.read(cx).value() > 99.);
                assert_eq!(view.camera_down, 0);
                assert!(window.captured_hitbox().is_none());
                assert!(view.input.read(cx).state.borrow().drag.is_none());
            })
            .unwrap();

        let p = point(cx, 180., 220.);
        event(
            cx,
            handle,
            PlatformInput::ScrollWheel(ScrollWheelEvent {
                position: p,
                delta: ScrollDelta::Pixels(gpui::point(px(0.), px(-50.))),
                ..Default::default()
            }),
        );
        handle
            .update(cx, |view, _, _| {
                assert!(view.scroll.offset().y < px(0.));
                assert_eq!(view.camera_scroll, 0);
            })
            .unwrap();
        let p = point(cx, 30., 30.);
        handle
            .update(cx, |view, _, cx| {
                view.covered = true;
                cx.notify();
            })
            .unwrap();
        button(cx, handle, p, true, MouseButton::Left);
        button(cx, handle, p, false, MouseButton::Left);
        assert_eq!(handle.update(cx, |view, _, _| view.clicks).unwrap(), 1);
        handle
            .update(cx, |view, _, cx| {
                view.covered = false;
                view.overlay = true;
                cx.notify();
            })
            .unwrap();
        button(cx, handle, p, true, MouseButton::Left);
        button(cx, handle, p, false, MouseButton::Left);
        handle
            .update(cx, |view, _, _| {
                assert_eq!(view.clicks, 1);
                assert_eq!(view.overlay_clicks, 1);
            })
            .unwrap();
        handle
            .update(cx, |view, _, cx| {
                view.overlay = false;
                cx.notify();
            })
            .unwrap();
        let camera_down = handle.update(cx, |view, _, _| view.camera_down).unwrap();
        button(cx, handle, p, true, MouseButton::Right);
        button(cx, handle, p, false, MouseButton::Right);
        assert_eq!(
            handle.update(cx, |view, _, _| view.camera_down).unwrap(),
            camera_down + 1
        );
        event(
            cx,
            handle,
            PlatformInput::ScrollWheel(ScrollWheelEvent {
                position: gpui::point(px(10.), px(10.)),
                delta: ScrollDelta::Pixels(gpui::point(px(0.), px(-50.))),
                ..Default::default()
            }),
        );
        assert_eq!(
            handle.update(cx, |view, _, _| view.camera_scroll).unwrap(),
            1
        );
        button(cx, handle, p, true, MouseButton::Left);
        button(cx, handle, outside, false, MouseButton::Left);
        assert_eq!(handle.update(cx, |view, _, _| view.clicks).unwrap(), 1);
    }

    #[gpui::test]
    fn ui_gesture_cancels_on_surface_removal_and_deactivation(cx: &mut TestAppContext) {
        let handle = cx.add_window(Controls::new);
        cx.update_window(handle.into(), |_, window, cx| {
            window.activate_window();
            window.draw(cx).clear();
        })
        .unwrap();
        cx.run_until_parked();
        let p = handle
            .update(cx, |view, _, _| view.screen(200., 114.))
            .unwrap();
        button(cx, handle, p, true, MouseButton::Left);
        handle
            .update(cx, |view, window, cx| {
                assert!(window.captured_hitbox().is_some());
                assert!(view.input.read(cx).state.borrow().drag.is_some());
                view.enabled = false;
                cx.notify();
            })
            .unwrap();
        cx.update_window(handle.into(), |_, window, cx| window.draw(cx).clear())
            .unwrap();
        handle
            .update(cx, |view, window, cx| {
                assert!(window.captured_hitbox().is_none());
                assert!(view.input.read(cx).state.borrow().drag.is_none());
                view.enabled = true;
                cx.notify();
            })
            .unwrap();
        button(cx, handle, p, true, MouseButton::Left);
        handle
            .update(cx, |view, window, cx| {
                assert!(window.captured_hitbox().is_some());
                assert!(view.input.read(cx).state.borrow().drag.is_some());
            })
            .unwrap();
        let other = cx.add_window(Controls::new);
        cx.update_window(other.into(), |_, window, _| window.activate_window())
            .unwrap();
        cx.run_until_parked();
        handle
            .update(cx, |view, window, cx| {
                assert!(window.captured_hitbox().is_none());
                assert!(view.input.read(cx).state.borrow().drag.is_none());
            })
            .unwrap();
    }

    #[gpui::test]
    fn orthographic_surface_routes_clicks_and_captured_drag(cx: &mut TestAppContext) {
        let handle = cx.add_window(Controls::new);
        handle
            .update(cx, |view, _, cx| {
                view.orthographic = true;
                cx.notify();
            })
            .unwrap();
        cx.update_window(handle.into(), |_, window, cx| window.draw(cx).clear())
            .unwrap();
        let p = handle
            .update(cx, |view, _, _| view.screen(30., 30.))
            .unwrap();
        button(cx, handle, p, true, MouseButton::Left);
        button(cx, handle, p, false, MouseButton::Left);
        assert_eq!(handle.update(cx, |view, _, _| view.clicks).unwrap(), 1);
        let (start, outside) = handle
            .update(cx, |view, _, _| {
                (view.screen(200., 114.), view.screen(800., 114.))
            })
            .unwrap();
        button(cx, handle, start, true, MouseButton::Left);
        event(
            cx,
            handle,
            PlatformInput::MouseMove(MouseMoveEvent {
                position: outside,
                pressed_button: Some(MouseButton::Left),
                ..Default::default()
            }),
        );
        button(cx, handle, outside, false, MouseButton::Left);
        handle
            .update(cx, |view, window, cx| {
                assert!(view.slider.read(cx).value() > 99.);
                assert_eq!(view.camera_down, 0);
                assert!(window.captured_hitbox().is_none());
            })
            .unwrap();
    }
}
