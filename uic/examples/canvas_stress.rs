use std::{collections::VecDeque, sync::Arc};

use gpui::{
    Bounds, Context, Entity, FrameDiagnostics, Image, ImageFormat, IntoElement, MouseButton,
    Pixels, Point, Render, TransformationMatrix, Window, WindowBounds, WindowOptions, div, img,
    point, prelude::*, px, rgb, size,
};
use gpui_effects::transform_group;
use uic::components::{
    context_menu::{self, ContextMenuTrigger},
    input::{Input, TextInput},
    popover::{Popover, PopoverState},
};

#[path = "support/context_menu_style.rs"]
#[allow(dead_code)]
mod context_menu_style;

const PHASE_FRAMES: usize = 90;
const PHASES: [&str; 6] = ["warmup", "pan", "zoom", "nested", "empty", "restored"];

struct Node {
    id: usize,
    clicks: usize,
    input: Entity<TextInput>,
    popover: Entity<PopoverState>,
    thumbnail: Arc<Image>,
}

impl Render for Node {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.entity();
        let id = self.id;
        div()
            .size_full()
            .p(px(4.))
            .rounded(px(6.))
            .border_1()
            .border_color(rgb(0x364455))
            .bg(rgb(0x202c3a))
            .flex()
            .flex_col()
            .gap(px(3.))
            .line_height(px(14.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        ContextMenuTrigger::new(
                            div()
                                .flex()
                                .items_center()
                                .gap_1()
                                .cursor_pointer()
                                .child(
                                    img(self.thumbnail.clone())
                                        .size(px(12.))
                                        .object_fit(gpui::ObjectFit::Contain),
                                )
                                .child(format!("{:03} · {}", id, self.clicks)),
                            move |_, _| {
                                let entity = entity.clone();
                                context_menu_style::menu(
                                    context_menu_style::MenuMaterial::DarkFrosted,
                                )
                                .action("Count click", move |_, cx| {
                                    entity.update(cx, |node, cx| {
                                        node.clicks += 1;
                                        cx.notify();
                                    })
                                })
                                .submenu("More", |menu| menu.action("Inspect anchor", |_, _| {}))
                            },
                        )
                        .id(("node-menu", id)),
                    )
                    .child(
                        Popover::new(&self.popover)
                            .bg(rgb(0x263444))
                            .text_color(rgb(0xe5edf5))
                            .p_3()
                            .rounded_lg()
                            .trigger(div().cursor_pointer().child("＋"))
                            .content(move |_, _| {
                                div()
                                    .w(px(160.))
                                    .child(format!("Node {id} · anchored popover"))
                            }),
                    ),
            )
            .child(
                Input::new(&self.input)
                    .h(px(20.))
                    .rounded(px(4.))
                    .border_color(rgb(0x364455))
                    .min_w_0()
                    .px_1()
                    .py_0()
                    .text_size(px(10.))
                    .bg(rgb(0x17212c))
                    .text_color(rgb(0xe5edf5)),
            )
    }
}

struct Board {
    nodes: Vec<Entity<Node>>,
    next_id: usize,
    cached: bool,
    nested: bool,
    thumbnail: Arc<Image>,
}

impl Board {
    fn resize(&mut self, count: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.nodes.truncate(count);
        while self.nodes.len() < count {
            let id = self.next_id;
            self.next_id += 1;
            self.nodes.push(cx.new(|cx| Node {
                id,
                clicks: 0,
                input: cx.new(|cx| TextInput::new(cx).initial_value(format!("Layer {id}"))),
                popover: cx.new(|cx| PopoverState::new(window, cx)),
                thumbnail: self.thumbnail.clone(),
            }));
        }
        cx.notify();
    }
}

impl Render for Board {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .relative()
            .w(px(1000.))
            .h(px(600.))
            .bg(rgb(0x17212c))
            .children(self.nodes.iter().enumerate().map(|(index, node)| {
                let content = if self.cached {
                    node.clone()
                        .cached(div().w(px(90.)).h(px(50.)).style().clone())
                        .cache_across_transforms()
                        .into_any_element()
                } else {
                    div()
                        .w(px(90.))
                        .h(px(50.))
                        .child(node.clone())
                        .into_any_element()
                };
                let content = if self.nested && index % 10 == 0 {
                    transform_group(
                        content,
                        TransformationMatrix {
                            rotation_scale: [[1.05, 0.], [0., 1.05]],
                            translation: [0., 0.],
                        },
                    )
                    .auto_raster_scale(("nested", index))
                    .into_any_element()
                } else {
                    content
                };
                div()
                    .absolute()
                    .left(px(10. + (index % 10) as f32 * 98.))
                    .top(px(10. + (index / 10) as f32 * 57.))
                    .child(content)
            }))
    }
}

#[derive(Default)]
struct Samples {
    last_sequence: u64,
    frames: VecDeque<FrameDiagnostics>,
}

impl Samples {
    fn record(&mut self, frame: &FrameDiagnostics) {
        if frame.sequence == self.last_sequence || frame.platform_draw_time.is_none() {
            return;
        }
        self.last_sequence = frame.sequence;
        if self.frames.len() == 600 {
            self.frames.pop_front();
        }
        self.frames.push_back(frame.clone());
    }

    fn timing(&self, platform: bool) -> String {
        let mut values: Vec<f64> = self
            .frames
            .iter()
            .map(|frame| {
                if platform {
                    frame.platform_draw_time.unwrap_or_default()
                } else {
                    frame.build_time
                }
                .as_secs_f64()
                    * 1000.
            })
            .collect();
        if values.is_empty() {
            return "—".into();
        }
        values.sort_by(f64::total_cmp);
        format!(
            "{:.2} / {:.2} ms",
            values[values.len() / 2],
            values[(values.len() * 95 / 100).min(values.len() - 1)]
        )
    }

    fn report(&self, phase: &str) {
        let view_hits: u64 = self.frames.iter().map(|f| f.view_cache.hits).sum();
        let view_misses: u64 = self.frames.iter().map(|f| f.view_cache.misses).sum();
        let mut reasons = gpui::ViewCacheMisses::default();
        for frame in &self.frames {
            reasons.cold += frame.view_cache_misses.cold;
            reasons.accessibility += frame.view_cache_misses.accessibility;
            reasons.refresh += frame.view_cache_misses.refresh;
            reasons.dirty += frame.view_cache_misses.dirty;
            reasons.context += frame.view_cache_misses.context;
        }
        let capture_hits: u64 = self
            .frames
            .iter()
            .filter_map(|f| f.renderer.as_ref())
            .map(|r| r.capture_cache.hits)
            .sum();
        let capture_misses: u64 = self
            .frames
            .iter()
            .filter_map(|f| f.renderer.as_ref())
            .map(|r| r.capture_cache.misses)
            .sum();
        let bytes = |f: &FrameDiagnostics| {
            f.renderer.as_ref().map_or(0, |r| {
                r.capture_textures
                    .iter()
                    .map(|t| t.estimated_bytes)
                    .sum::<u64>()
            })
        };
        let peak = self.frames.iter().map(bytes).max().unwrap_or(0);
        let end = self.frames.back().map_or(0, bytes);
        let renderer_samples = self
            .frames
            .iter()
            .filter(|frame| frame.renderer.is_some())
            .count();
        println!(
            "phase={phase} samples={} renderer_samples={renderer_samples} build_p50_p95={} platform_p50_p95={} view_hits={view_hits} view_misses={view_misses} capture_hits={capture_hits} capture_misses={capture_misses} capture_peak_bytes={peak} capture_end_bytes={end}",
            self.frames.len(),
            self.timing(false),
            self.timing(true)
        );
        println!("phase={phase} view_rebuilds={reasons:?}");
        if let Some(frame) = self.frames.back() {
            println!(
                "phase={phase} viewport={:?} scale={}",
                frame.viewport_size, frame.scale_factor
            );
        }
    }
}

struct Demo {
    board: Entity<Board>,
    zoom: f32,
    pan: Point<Pixels>,
    dragging: Option<Point<Pixels>>,
    running: bool,
    tick_pending: bool,
    automated: bool,
    step: usize,
    samples: Samples,
}

fn button(id: &'static str, label: impl Into<gpui::SharedString>) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .px_3()
        .py_2()
        .rounded_md()
        .bg(rgb(0x283646))
        .hover(|s| s.bg(rgb(0x35475b)))
        .cursor_pointer()
        .child(label.into())
}

impl Demo {
    fn tick(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.tick_pending = false;
        if let Some(frame) = window.submitted_frame_diagnostics() {
            self.samples.record(frame);
        }
        if !self.running {
            return;
        }
        if self.automated && self.step > 0 && self.step.is_multiple_of(PHASE_FRAMES) {
            self.samples.report(PHASES[self.step / PHASE_FRAMES - 1]);
            self.samples.frames.clear();
            if self.step == PHASE_FRAMES * PHASES.len() {
                cx.quit();
                return;
            }
        }
        let phase = if self.automated {
            self.step / PHASE_FRAMES
        } else {
            2
        };
        let t = (self.step % PHASE_FRAMES) as f32 / PHASE_FRAMES as f32;
        if self.automated && self.step.is_multiple_of(PHASE_FRAMES) {
            self.board.update(cx, |board, cx| {
                board.nested = phase == 3;
                board.resize(if phase == 4 { 0 } else { 100 }, window, cx);
            });
        }
        self.zoom = if phase == 2 { 1. + t * 2.2 } else { 1. };
        self.pan = if phase == 1 || !self.automated {
            point(px((t * std::f32::consts::TAU).sin() * 50.), px(0.))
        } else {
            point(px(0.), px(0.))
        };
        self.step += 1;
        cx.notify();
    }
}

impl Render for Demo {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(frame) = window.submitted_frame_diagnostics() {
            self.samples.record(frame);
        }
        if self.running && !self.tick_pending {
            self.tick_pending = true;
            cx.on_next_frame(window, Self::tick);
        }
        let board = self.board.read(cx);
        let count = board.nodes.len();
        let cached = board.cached;
        let nested = board.nested;
        let last = window.submitted_frame_diagnostics();
        let cache = last.map_or("—".into(), |f| {
            let reason = if f.view_cache_misses.accessibility > 0 {
                " · accessibility rebuild"
            } else {
                ""
            };
            format!(
                "{} hit / {} miss{reason}",
                f.view_cache.hits, f.view_cache.misses
            )
        });
        let gpu = last.and_then(|f| f.renderer.as_ref());
        let captures = gpu.map_or("Unavailable".into(), |r| {
            format!(
                "{} targets · {:.1} MiB · {} hit / {} miss",
                r.capture_textures.len(),
                r.capture_textures
                    .iter()
                    .map(|t| t.estimated_bytes)
                    .sum::<u64>() as f64
                    / 1048576.,
                r.capture_cache.hits,
                r.capture_cache.misses
            )
        });
        let density = gpu.map_or("—".into(), |r| {
            r.capture_textures
                .iter()
                .take(4)
                .map(|t| {
                    format!(
                        "{}×{} @ {}",
                        t.width,
                        t.height,
                        t.raster_scale
                            .map_or("scratch".into(), |s| format!("{s:.1}×"))
                    )
                })
                .collect::<Vec<_>>()
                .join(" · ")
        });
        let viewport = if count == 0 {
            div()
                .w(px(1000.))
                .h(px(600.))
                .bg(rgb(0x17212c))
                .into_any_element()
        } else {
            transform_group(
                self.board.clone(),
                TransformationMatrix {
                    rotation_scale: [[self.zoom, 0.], [0., self.zoom]],
                    translation: [self.pan.x.into(), self.pan.y.into()],
                },
            )
            .auto_raster_scale("stress-density")
            .into_any_element()
        };
        div().size_full().p_5().flex().flex_col().gap_3().bg(rgb(0x111923))
            .text_color(rgb(0xe5edf5)).text_size(px(12.))
            .child(div().flex().items_center().justify_between()
                .child(div().text_size(px(24.)).child("Canvas workload"))
                .child(format!("{count} nodes · {:.0}%", self.zoom * 100.)))
            .child(div().flex().flex_wrap().gap_2()
                .child(button("run", if self.running { "Pause" } else { "Animate" })
                    .on_click(cx.listener(|this, _, _, cx| { this.running = !this.running; cx.notify(); })))
                .child(button("add", "+25").on_click(cx.listener(|this, _, w, cx| {
                    this.board.update(cx, |b, cx| b.resize((b.nodes.len()+25).min(200), w, cx));
                })))
                .child(button("remove", "−25").on_click(cx.listener(|this, _, w, cx| {
                    this.board.update(cx, |b, cx| b.resize(b.nodes.len().saturating_sub(25), w, cx));
                })))
                .child(button("clear", "Clear").on_click(cx.listener(|this, _, w, cx| {
                    context_menu::dismiss(w, cx);
                    this.board.update(cx, |b, cx| b.resize(0, w, cx));
                })))
                .child(button("reset", "Restore 100").on_click(cx.listener(|this, _, w, cx| {
                    this.zoom = 1.; this.pan = point(px(0.), px(0.));
                    this.board.update(cx, |b, cx| b.resize(100, w, cx));
                    cx.notify();
                })))
                .child(button("cache", if cached { "Cache: on" } else { "Cache: off" })
                    .on_click(cx.listener(|this, _, _, cx| this.board.update(cx, |b, cx| { b.cached = !b.cached; cx.notify(); }))))
                .child(button("nested", if nested { "Nested: on" } else { "Nested: off" })
                    .on_click(cx.listener(|this, _, _, cx| this.board.update(cx, |b, cx| { b.nested = !b.nested; cx.notify(); }))))
                .child(button("report", "Print / reset stats").on_click(cx.listener(|this, _, _, cx| {
                    this.samples.report("manual"); this.samples.frames.clear(); cx.notify();
                }))))
            .child(div().flex().flex_wrap().gap_4().text_color(rgb(0xa6b7c9))
                .child(format!("Build p50 / p95: {}", self.samples.timing(false)))
                .child(format!("Platform p50 / p95: {}", self.samples.timing(true)))
                .child(format!("Views: {cache}")))
            .child(div().text_color(rgb(0xa6b7c9)).child(format!("Captures: {captures}")))
            .child(div().text_color(rgb(0x7f95aa)).child(density))
            .child(div().id("viewport").relative().flex_1().min_h_0().overflow_hidden().rounded_lg()
                .on_scroll_wheel(cx.listener(|this, event: &gpui::ScrollWheelEvent, _, cx| {
                    this.zoom = (this.zoom * (1. + f32::from(event.delta.pixel_delta(px(20.)).y) * 0.003)).clamp(0.5, 4.);
                    cx.notify();
                }))
                .on_mouse_down(MouseButton::Middle, cx.listener(|this, event: &gpui::MouseDownEvent, _, _| { this.dragging = Some(event.position); }))
                .on_mouse_move(cx.listener(|this, event: &gpui::MouseMoveEvent, _, cx| {
                    if event.pressed_button == Some(MouseButton::Middle) {
                        if let Some(previous) = this.dragging.replace(event.position) {
                            this.pan += event.position - previous; cx.notify();
                        }
                    } else { this.dragging = None; }
                }))
                .on_mouse_up(MouseButton::Middle, cx.listener(|this, _, _, _| this.dragging = None))
                .child(viewport))
            .child(div().text_color(rgb(0x7f95aa)).child("Wheel: zoom · Middle-drag: pan · Node title: menu · ＋: popover · Edit text to test IME"))
            .child(div().text_color(rgb(0x7f95aa)).child("CPU wall times; capture storage estimate excludes atlas, MSAA and driver memory. Offscreen nodes remain clipped."))
            .child(context_menu::layer(cx))
    }
}

fn main() {
    let automated = std::env::args().any(|arg| arg == "--automated");
    let cached = !std::env::args().any(|arg| arg == "--no-cache");
    let accessible = !std::env::args().any(|arg| arg == "--no-accessibility");
    let application = if accessible {
        gpui_platform::application()
    } else {
        gpui::Application::new_inaccessible(gpui_platform::current_platform(false))
    };
    application.run(move |cx| {
        uic::init(cx);
        cx.open_window(WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(None, size(px(1100.), px(880.)), cx))),
            ..Default::default()
        }, |window, cx| {
            window.set_frame_diagnostics_enabled(true);
            if automated {
                println!("canvas_stress profile={} cached={cached} accessibility_allowed={accessible} scale={} viewport={:?} phase_frames={PHASE_FRAMES}",
                    if cfg!(debug_assertions) { "debug" } else { "release" }, window.scale_factor(), window.viewport_size());
            }
            cx.new(|cx| {
                let board = cx.new(|cx| {
                    let mut board = Board { nodes: Vec::new(), next_id: 0, cached, nested: false,
                        thumbnail: Arc::new(Image::from_bytes(ImageFormat::Png, include_bytes!("../../crates/gpui/examples/image/app-icon.png").to_vec())) };
                    board.resize(100, window, cx);
                    board
                });
                Demo { board, zoom: 1., pan: point(px(0.), px(0.)), dragging: None,
                    running: automated, tick_pending: false, automated, step: 0, samples: Samples::default() }
            })
        }).expect("open canvas workload");
        cx.activate(true);
    });
}
