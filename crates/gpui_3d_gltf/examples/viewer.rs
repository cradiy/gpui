use std::{
    cell::Cell,
    path::PathBuf,
    rc::Rc,
    time::{Duration, Instant},
};

use anyhow::{Context as _, Result};
use gpui::{
    App, Bounds, Context, MouseButton, Pixels, Render, Subscription, Task, Window, WindowBounds,
    WindowOptions, canvas, div, prelude::*, px, rgb, size,
};
use gpui_3d::{Camera, NodeHandle, OrbitController, Projection, Scene, viewport3d};
use gpui_3d_gltf::{AnimationPlayback, ImageCache, SceneLoadQueue, SceneLoadSlot};
use gpui_platform::application;

#[path = "support/files.rs"]
mod files;

const USAGE: &str = "Usage: viewer <asset.gltf|asset.glb> [SCENE_INDEX] [ANIMATION_INDEX]";

#[cfg(all(feature = "wgpu", not(target_family = "wasm")))]
#[path = "support/gpu.rs"]
mod gpu;
#[path = "support/model.rs"]
mod model;
#[cfg(all(feature = "wgpu", not(target_family = "wasm")))]
#[path = "support/picking.rs"]
mod picking;
use model::{Model, fitted_size, load, publish};

struct Viewer {
    path: PathBuf,
    scene_index: Option<usize>,
    animation_index: Option<usize>,
    last_frame: Instant,
    slot: SceneLoadSlot,
    model: Option<Model>,
    images: ImageCache,
    queue: SceneLoadQueue,
    task: Option<Task<()>>,
    controls: OrbitController,
    bounds: Rc<Cell<Bounds<Pixels>>>,
    frame_pending: bool,
    selected: Option<NodeHandle>,
    source_camera: Option<NodeHandle>,
    error: Option<String>,
    #[cfg(all(feature = "wgpu", not(target_family = "wasm")))]
    picking: picking::Picking,
    _activation: Subscription,
}

impl Viewer {
    fn new(
        path: PathBuf,
        scene_index: Option<usize>,
        animation_index: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let activation = cx.observe_window_activation(window, |this, window, _| {
            if !window.is_window_active() {
                this.controls.cancel_drag();
                if let Some(playback) = this.model.as_mut().and_then(|m| m.playback.as_mut()) {
                    playback.pause();
                }
            }
            this.last_frame = Instant::now();
        });
        let mut viewer = Self {
            path,
            scene_index,
            animation_index,
            last_frame: Instant::now(),
            slot: SceneLoadSlot::new(),
            model: None,
            images: ImageCache::default(),
            queue: SceneLoadQueue::new(2.try_into().unwrap(), 2),
            task: None,
            controls: OrbitController::new(Camera::orbit(0.5, 0.3, 5.)).unwrap(),
            bounds: Rc::new(Cell::new(Bounds::default())),
            frame_pending: false,
            selected: None,
            source_camera: None,
            error: None,
            #[cfg(all(feature = "wgpu", not(target_family = "wasm")))]
            picking: picking::Picking::new(),
            _activation: activation,
        };
        viewer.reload(cx);
        viewer
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        let request = self.slot.begin();
        let path = self.path.clone();
        let scene = self.scene_index;
        let animation = self.animation_index;
        let images = self.images.clone();
        let queue = self.queue.clone();
        let worker = cx.background_executor().spawn(async move {
            request
                .run(queue.run(move || async move { load(&path, scene, animation, &images).await }))
                .await
        });
        self.error = None;
        self.task = Some(cx.spawn(async move |this, cx| {
            let completion = worker.await;
            let _ = this.update(cx, |this, cx| {
                if publish(&mut this.slot, &mut this.model, completion) {
                    #[cfg(all(feature = "wgpu", not(target_family = "wasm")))]
                    this.picking.clear();
                    this.frame_pending = true;
                    this.selected = None;
                    this.source_camera = None;
                    this.last_frame = Instant::now();
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn frame(&mut self, selected: bool) -> Result<()> {
        let Some(model) = &self.model else {
            return Ok(());
        };
        if selected && self.selected.is_none() {
            return Ok(());
        }
        let bounds = model.display_bounds(if selected { self.selected } else { None })?;
        let Some(bounds) = bounds else {
            return Ok(());
        };
        let rect = self.bounds.get();
        let camera =
            self.controls
                .camera()
                .frame_bounds(bounds, rect.size.width / rect.size.height, 1.2)?;
        self.controls.set_camera(camera)?;
        self.source_camera = None;
        Ok(())
    }

    fn change_playback(
        &mut self,
        update: impl FnOnce(&mut AnimationPlayback) -> Result<()>,
        cx: &mut Context<Self>,
    ) {
        if let Some(model) = &mut self.model {
            match model.control(update) {
                Ok(()) => self.error = None,
                Err(error) => {
                    self.error = Some(format!("{error:#}"));
                    if let Some(playback) = &mut model.playback {
                        playback.pause();
                    }
                }
            }
        }
        self.last_frame = Instant::now();
        cx.notify();
    }

    fn playback_controls(&self, cx: &mut Context<Self>) -> gpui::Div {
        let mut row = div().flex().flex_wrap().items_center().gap_2().text_sm();
        let Some(model) = &self.model else {
            return row;
        };
        let Some(playback) = &model.playback else {
            return row;
        };
        row = row
            .child(model.animation_label().unwrap())
            .child(
                self.button(
                    "play",
                    if playback.is_playing() {
                        "Pause"
                    } else {
                        "Play"
                    },
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.change_playback(
                        |p| {
                            if p.is_playing() {
                                p.pause();
                            } else {
                                p.play();
                            }
                            Ok(())
                        },
                        cx,
                    );
                })),
            )
            .child(
                self.button("start", "Start")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.change_playback(
                            |p| {
                                p.seek(Duration::ZERO);
                                Ok(())
                            },
                            cx,
                        )
                    })),
            )
            .child(
                self.button("back", "−0.25 s")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.change_playback(
                            |p| {
                                p.seek(p.position().saturating_sub(Duration::from_millis(250)));
                                Ok(())
                            },
                            cx,
                        )
                    })),
            )
            .child(
                self.button("forward", "+0.25 s")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.change_playback(
                            |p| {
                                p.seek(p.position().saturating_add(Duration::from_millis(250)));
                                Ok(())
                            },
                            cx,
                        )
                    })),
            )
            .child(
                self.button(
                    "loop",
                    if playback.is_looping() {
                        "Loop"
                    } else {
                        "Once"
                    },
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.change_playback(
                        |p| {
                            p.set_looping(!p.is_looping());
                            Ok(())
                        },
                        cx,
                    )
                })),
            )
            .child(
                self.button("rate", format!("{:.1}×", playback.rate()))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.change_playback(
                            |p| {
                                let rates = [0.5, 1., 2., -1.];
                                let next =
                                    (rates.iter().position(|rate| *rate == p.rate()).unwrap_or(0)
                                        + 1)
                                        % rates.len();
                                p.set_rate(rates[next])
                            },
                            cx,
                        )
                    })),
            )
            .child(format!(
                "{:.2} / {:.2} s · {} outside-scene tracks",
                playback.position().as_secs_f64(),
                playback.duration().as_secs_f64(),
                model.skipped_tracks
            ));
        row
    }

    fn button(
        &self,
        id: &'static str,
        label: impl Into<gpui::SharedString>,
    ) -> gpui::Stateful<gpui::Div> {
        div()
            .id(id)
            .px_3()
            .py_2()
            .rounded(px(8.))
            .bg(rgb(0x243951))
            .hover(|s| s.bg(rgb(0x345574)))
            .cursor_pointer()
            .child(label.into())
    }

    fn input(&mut self, result: Result<bool, gpui_3d::OrbitError>, cx: &mut Context<Self>) {
        match result {
            Ok(true) => cx.notify(),
            Ok(false) => {}
            Err(error) => {
                self.error = Some(error.to_string());
                cx.notify();
            }
        }
    }
}

impl Render for Viewer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        #[cfg(all(feature = "wgpu", not(target_family = "wasm")))]
        if self.model.as_ref().is_some_and(Model::gpu_enabled) {
            if let Some(selection) = self.picking.poll(window)
                && selection.is_none_or(|node| {
                    self.model
                        .as_ref()
                        .is_some_and(|model| model.instance.source_primitive(node).is_some())
                })
            {
                self.selected = selection;
            }
        } else {
            self.picking.clear();
        }
        let now = Instant::now();
        if let Some(model) = &mut self.model {
            if let Err(error) = model.advance(now.duration_since(self.last_frame)) {
                self.error = Some(format!("{error:#}"));
                if let Some(playback) = &mut model.playback {
                    playback.pause();
                }
            }
            if model
                .playback
                .as_ref()
                .is_some_and(AnimationPlayback::is_playing)
            {
                window.request_animation_frame();
            }
        }
        self.last_frame = now;
        #[cfg(all(feature = "wgpu", not(target_family = "wasm")))]
        if let Some(model) = &mut self.model
            && let Err(error) = model.poll_gpu(window)
        {
            self.error = Some(format!("{error:#}"));
            if let Some(playback) = &mut model.playback {
                playback.pause();
            }
        }
        if self.frame_pending
            && self.bounds.get().size.height > px(0.)
            && self.bounds.get().size.width > px(0.)
        {
            self.frame_pending = false;
            if let Err(error) = self.frame(false) {
                self.error = Some(error.to_string());
            }
        }
        let aspect = self.model.as_ref().and_then(|model| {
            self.source_camera
                .and_then(|node| model.evaluated.node(node))
                .and_then(|node| node.camera)
                .and_then(|camera| camera.aspect_ratio)
        });
        let viewport_size = fitted_size(self.bounds.get().size, aspect);
        let scene = self.model.as_ref().map_or_else(Scene::new, |model| {
            if let Some(camera) = self.source_camera {
                match model.evaluated.scene_from_camera(camera) {
                    Ok(scene) => return scene,
                    Err(error) => self.error = Some(error.to_string()),
                }
            }
            model.evaluated.scene(self.controls.camera())
        });
        let viewport = if let Some(model) = &self.model {
            model.viewport(scene)
        } else {
            Ok(Some(viewport3d("model", scene)))
        };
        let viewport = match viewport {
            Ok(Some(viewport)) => {
                #[cfg(all(feature = "wgpu", not(target_family = "wasm")))]
                let viewport = if self.model.as_ref().is_some_and(Model::gpu_enabled) {
                    viewport.pick_capture(self.picking.capture.clone())
                } else {
                    viewport
                };
                viewport
                    .w(viewport_size.width)
                    .h(viewport_size.height)
                    .on_object_click(cx.listener(|this, hit: &gpui_3d::Hit, _, cx| {
                        if !this.model.as_ref().is_some_and(Model::gpu_enabled) {
                            this.selected = hit.node;
                            cx.notify();
                        }
                    }))
                    .into_any_element()
            }
            Ok(None) => {
                #[cfg(all(feature = "wgpu", not(target_family = "wasm")))]
                self.picking.clear();
                div()
                    .p_4()
                    .child("Preparing GPU geometry…")
                    .into_any_element()
            }
            Err(error) => {
                #[cfg(all(feature = "wgpu", not(target_family = "wasm")))]
                self.picking.clear();
                self.error = Some(format!("{error:#}"));
                if let Some(playback) = self
                    .model
                    .as_mut()
                    .and_then(|model| model.playback.as_mut())
                {
                    playback.pause();
                }
                div()
                    .p_4()
                    .child("GPU geometry is unavailable")
                    .into_any_element()
            }
        };
        let mut stage = div()
            .id("stage")
            .relative()
            .flex()
            .items_center()
            .justify_center()
            .flex_1()
            .h_full()
            .min_w_0()
            .overflow_hidden()
            .rounded(px(12.))
            .bg(rgb(0x162337));
        #[cfg(all(feature = "wgpu", not(target_family = "wasm")))]
        {
            stage = stage.on_click(cx.listener(|this, event, _, cx| {
                if !this.model.as_ref().is_some_and(Model::gpu_enabled) {
                    return;
                }
                let gpui::ClickEvent::Mouse(event) = event else {
                    return;
                };
                let delta = event.up.position - event.down.position;
                if event.down.button == MouseButton::Left
                    && f32::from(delta.x).hypot(f32::from(delta.y)) <= 4.
                {
                    this.picking.click(event.up.position);
                    cx.notify();
                }
            }));
        }
        for button in [MouseButton::Right, MouseButton::Middle] {
            stage = stage
                .on_mouse_down(
                    button,
                    cx.listener(move |this, event: &gpui::MouseDownEvent, _, cx| {
                        if this.source_camera.is_some() {
                            return;
                        }
                        let result =
                            this.controls
                                .begin_drag(button, event.position, this.bounds.get());
                        this.input(result, cx);
                    }),
                )
                .on_mouse_up(
                    button,
                    cx.listener(move |this, _, _, _| {
                        this.controls.end_drag(button);
                    }),
                )
                .on_mouse_up_out(
                    button,
                    cx.listener(move |this, _, _, _| {
                        this.controls.end_drag(button);
                    }),
                );
        }
        let bounds = self.bounds.clone();
        let view = cx.entity_id();
        stage = stage
            .on_mouse_move(cx.listener(|this, event: &gpui::MouseMoveEvent, _, cx| {
                if this.source_camera.is_some() {
                    return;
                }
                let result = this.controls.update_drag(
                    event.position,
                    event.pressed_button,
                    this.bounds.get(),
                );
                this.input(result, cx);
            }))
            .on_hover(cx.listener(|this, hovered: &bool, _, _| {
                if !hovered {
                    this.controls.cancel_drag();
                }
            }))
            .on_scroll_wheel(cx.listener(|this, event: &gpui::ScrollWheelEvent, _, cx| {
                if this.source_camera.is_some() {
                    return;
                }
                let result = this
                    .controls
                    .scroll(f32::from(event.delta.pixel_delta(px(20.)).y));
                this.input(result, cx);
                cx.stop_propagation();
            }))
            .child(viewport)
            .child(
                canvas(
                    move |rect, _, cx| {
                        if bounds.replace(rect) != rect {
                            cx.notify(view);
                        }
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .inset_0()
                .size_full(),
            );
        let details = self
            .model
            .as_ref()
            .map(|m| m.details(self.selected))
            .unwrap_or_else(|| vec!["No model loaded.".into()]);
        let error = self
            .error
            .clone()
            .or_else(|| self.slot.error().map(|e| format!("{e:#}")));
        #[cfg(all(feature = "wgpu", not(target_family = "wasm")))]
        let error = error.or_else(|| self.picking.error.clone());
        let deformation_controls = div();
        #[cfg(all(feature = "wgpu", not(target_family = "wasm")))]
        let deformation_controls = deformation_controls.child(
            self.button(
                "deformation",
                if self.model.as_ref().is_some_and(Model::gpu_enabled) {
                    "GPU deformation"
                } else {
                    "CPU deformation"
                },
            )
            .on_click(cx.listener(|this, _, window, cx| {
                this.picking.clear();
                if let Some(model) = &mut this.model {
                    match model.toggle_gpu(window) {
                        Ok(()) => this.error = None,
                        Err(error) => this.error = Some(format!("{error:#}")),
                    }
                }
                this.last_frame = Instant::now();
                cx.notify();
            })),
        );
        div()
            .size_full()
            .p_5()
            .flex()
            .flex_col()
            .gap_3()
            .bg(rgb(0x0b1422))
            .text_color(rgb(0xeaf2fc))
            .child(div().text_size(px(26.)).child("Model viewer"))
            .child(deformation_controls)
            .child(
                div()
                    .text_sm()
                    .truncate()
                    .text_color(rgb(0x9eb1cb))
                    .child(self.path.to_string_lossy().into_owned()),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .child(
                        self.button("reload", "Reload")
                            .on_click(cx.listener(|this, _, _, cx| this.reload(cx))),
                    )
                    .child(self.button("cancel", "Cancel").on_click(cx.listener(
                        |this, _, _, cx| {
                            this.slot.cancel();
                            this.task = None;
                            cx.notify();
                        },
                    )))
                    .child(self.button("frame", "Frame all").on_click(cx.listener(
                        |this, _, _, cx| {
                            if let Err(error) = this.frame(false) {
                                this.error = Some(error.to_string());
                            }
                            cx.notify();
                        },
                    )))
                    .child(
                        self.button("frame-selection", "Frame selected")
                            .on_click(cx.listener(|this, _, _, cx| {
                                if let Err(error) = this.frame(true) {
                                    this.error = Some(error.to_string());
                                }
                                cx.notify();
                            })),
                    )
                    .child(
                        self.button("projection", "Perspective / Ortho")
                            .on_click(cx.listener(|this, _, _, cx| {
                                let mut camera = this.controls.camera();
                                let distance = camera
                                    .eye
                                    .iter()
                                    .zip(camera.target)
                                    .map(|(a, b)| (a - b).powi(2))
                                    .sum::<f32>()
                                    .sqrt();
                                camera.projection = match camera.projection {
                                    Projection::Perspective { vertical_fov } => {
                                        Projection::Orthographic {
                                            vertical_size: 2.
                                                * distance
                                                * (vertical_fov * 0.5).tan(),
                                        }
                                    }
                                    Projection::Orthographic { vertical_size } => {
                                        Projection::Perspective {
                                            vertical_fov: 2.
                                                * (vertical_size / (2. * distance)).atan(),
                                        }
                                    }
                                };
                                if let Err(error) = this.controls.set_camera(camera) {
                                    this.error = Some(error.to_string());
                                }
                                this.source_camera = None;
                                cx.notify();
                            })),
                    )
                    .child(
                        self.button(
                            "camera",
                            if self.source_camera.is_some() {
                                "Next camera"
                            } else {
                                "Asset camera"
                            },
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            if let Some(model) = &this.model {
                                let cameras: Vec<_> = model
                                    .asset()
                                    .nodes()
                                    .iter()
                                    .filter(|n| n.camera_index.is_some())
                                    .filter_map(|n| model.instance.node(n.index))
                                    .collect();
                                this.source_camera = match this.source_camera {
                                    None => cameras.first().copied(),
                                    Some(current) => cameras
                                        .iter()
                                        .position(|h| *h == current)
                                        .and_then(|i| cameras.get(i + 1))
                                        .copied(),
                                };
                                this.controls.cancel_drag();
                                cx.notify();
                            }
                        })),
                    ),
            )
            .child(self.playback_controls(cx))
            .child(
                div().flex().flex_1().min_h_0().gap_4().child(stage).child(
                    div()
                        .id("inspector")
                        .w(px(260.))
                        .flex_shrink_0()
                        .h_full()
                        .overflow_y_scroll()
                        .flex()
                        .flex_col()
                        .gap_4()
                        .child(
                            div()
                                .text_sm()
                                .text_color(rgb(0x8ed6df))
                                .child("SOURCE DETAILS"),
                        )
                        .children(details.into_iter().map(|s| div().text_sm().child(s))),
                ),
            )
            .child(div().text_sm().text_color(rgb(0x9eb1cb)).child(format!(
                "{:?} · {} · Right-drag: orbit · Middle-drag: pan · Scroll: zoom",
                self.slot.status(),
                if self.source_camera.is_some() {
                    "Asset camera"
                } else {
                    "Orbit camera"
                }
            )))
            .when_some(error, |root, error| {
                root.child(
                    div()
                        .id("error")
                        .max_h(px(96.))
                        .overflow_y_scroll()
                        .text_color(rgb(0xf0a59a))
                        .child(error),
                )
            })
            .when(!window.supports_scene3d(), |root| {
                root.child("3D viewports are unavailable on this renderer.")
            })
    }
}

fn main() -> Result<()> {
    let mut args = std::env::args_os().skip(1);
    let path = args.next().context(USAGE)?;
    if path == "--help" || path == "-h" {
        println!("{USAGE}");
        return Ok(());
    }
    let scene = args
        .next()
        .map(|s| s.to_string_lossy().parse::<usize>())
        .transpose()
        .context("invalid scene index")?;
    let animation = args
        .next()
        .map(|s| s.to_string_lossy().parse::<usize>())
        .transpose()
        .context("invalid animation index")?;
    anyhow::ensure!(args.next().is_none(), USAGE);
    application().run(move |cx: &mut App| {
        cx.open_window(
            WindowOptions {
                window_min_size: Some(size(px(760.), px(520.))),
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(1200.), px(800.)),
                    cx,
                ))),
                ..Default::default()
            },
            move |window, cx| cx.new(|cx| Viewer::new(path.into(), scene, animation, window, cx)),
        )
        .expect("failed to open model viewer");
    });
    Ok(())
}

#[cfg(test)]
#[path = "support/viewer_tests.rs"]
mod tests;
