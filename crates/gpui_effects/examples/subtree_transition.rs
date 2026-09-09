use gpui::{
    App, Bounds, Context, Image, ImageFormat, ImageSource, MouseButton, Pixels, Render, Window,
    WindowBounds, WindowOptions, canvas, div, img, prelude::*, px, relative, rgb, size,
};
use gpui_effects::{TransitionKind, subtree_transition};
use gpui_platform::application;
use std::{cell::Cell, rc::Rc, sync::Arc, time::Instant};

struct Preview {
    progress: f32,
    target: f32,
    playing: bool,
    last_frame: Instant,
    kind: TransitionKind,
    track: Rc<Cell<Bounds<Pixels>>>,
    dragging: bool,
    image: ImageSource,
}

impl Preview {
    fn button(
        &self,
        index: usize,
        label: &'static str,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div()
            .id(("control", index))
            .px_4()
            .py_2()
            .rounded_full()
            .cursor_pointer()
            .bg(rgb(0x24364d))
            .hover(|s| s.bg(rgb(0x395776)))
            .child(label)
            .on_click(cx.listener(move |this, _, _, cx| {
                match index {
                    0 => {
                        this.target = 0.;
                        this.playing = true;
                    }
                    1 => {
                        this.target = 1.;
                        this.playing = true;
                    }
                    2 => {
                        if !this.playing && this.progress == this.target {
                            this.target = 1. - this.target;
                        }
                        this.playing = !this.playing;
                    }
                    3 => this.kind = TransitionKind::BlurFade,
                    4 => this.kind = TransitionKind::CrossFade,
                    _ => this.kind = TransitionKind::WipeRight,
                }
                this.last_frame = Instant::now();
                cx.notify();
            }))
    }

    fn card(&self, second: bool) -> impl IntoElement {
        let (title, subtitle, color) = if second {
            ("Daybreak", "A little warmth for the morning", 0xf4b982)
        } else {
            ("After hours", "Quiet sounds for a slower evening", 0x9ae2ed)
        };
        div()
            .size_full()
            .p_8()
            .rounded(px(28.))
            .bg(rgb(if second { 0x342b35 } else { 0x182b40 }))
            .flex()
            .flex_col()
            .justify_between()
            .gap_4()
            .child(
                div()
                    .flex()
                    .gap_6()
                    .items_center()
                    .child(img(self.image.clone()).size(px(136.)).rounded(px(20.)))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap_3()
                            .child(div().text_xs().text_color(rgb(color)).child(if second {
                                "MORNING MIX / 02"
                            } else {
                                "NIGHT MIX / 01"
                            }))
                            .child(div().text_size(px(38.)).child(title))
                            .child(div().text_sm().text_color(rgb(0xa3b6c8)).child(subtitle)),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_end()
                    .gap_2()
                    .h(px(96.))
                    .children((0..28).map(|index| {
                        let phase = index as f32 * if second { 0.53 } else { 0.31 };
                        div()
                            .flex_1()
                            .rounded_full()
                            .h(px(12. + (phase.sin() * phase.cos()).abs() * 130.))
                            .bg(rgb(color))
                    })),
            )
            .child(
                div()
                    .flex()
                    .justify_between()
                    .text_sm()
                    .text_color(rgb(0xb7c8d8))
                    .child(if second {
                        "08 tracks · Acoustic / Ambient"
                    } else {
                        "12 tracks · Electronic / Jazz"
                    })
                    .child(if second { "32:16" } else { "48:20" }),
            )
    }
}

impl Render for Preview {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let now = Instant::now();
        if self.playing {
            let step = now.duration_since(self.last_frame).as_secs_f32().min(0.05) / 1.2;
            self.progress += (self.target - self.progress).clamp(-step, step);
            if self.progress == self.target {
                self.playing = false;
            } else {
                window.request_animation_frame();
            }
        }
        self.last_frame = now;
        let progress = self.progress;
        let track = self.track.clone();
        div()
            .size_full()
            .p_8()
            .bg(rgb(0x0c1422))
            .text_color(rgb(0xe9f2fc))
            .flex()
            .flex_col()
            .gap_6()
            .child(
                div()
                    .flex_shrink_0()
                    .flex()
                    .justify_between()
                    .items_center()
                    .child(div().text_size(px(28.)).child("Changing scenes"))
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(self.button(3, "Blur fade", cx))
                            .child(self.button(4, "Crossfade", cx))
                            .child(self.button(5, "Soft wipe", cx)),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        subtree_transition("scene", self.card(false), self.card(true))
                            .progress(progress)
                            .kind(self.kind)
                            .w_full()
                            .max_w(px(760.))
                            .h(px(390.)),
                    ),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .flex()
                    .flex_col()
                    .gap_4()
                    .p_5()
                    .rounded(px(20.))
                    .bg(rgb(0x132034))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(div().child(format!("Progress · {:.0}%", progress * 100.)))
                            .child(
                                div()
                                    .flex()
                                    .gap_2()
                                    .child(self.button(0, "Previous", cx))
                                    .child(self.button(1, "Next", cx))
                                    .child(self.button(
                                        2,
                                        if self.playing { "Pause" } else { "Play" },
                                        cx,
                                    )),
                            ),
                    )
                    .child(
                        div()
                            .id("scrubber")
                            .relative()
                            .h(px(30.))
                            .w_full()
                            .cursor_pointer()
                            .child(
                                div()
                                    .absolute()
                                    .top(px(12.))
                                    .h(px(6.))
                                    .w_full()
                                    .rounded_full()
                                    .bg(rgb(0x2a3e57)),
                            )
                            .child(
                                div()
                                    .absolute()
                                    .top(px(12.))
                                    .h(px(6.))
                                    .w(relative(self.progress))
                                    .rounded_full()
                                    .bg(rgb(0x9ae2ed)),
                            )
                            .child(
                                div()
                                    .absolute()
                                    .left(relative(self.progress))
                                    .ml(px(-8.))
                                    .top(px(7.))
                                    .size(px(16.))
                                    .rounded_full()
                                    .bg(rgb(0xd4f5ff)),
                            )
                            .child(
                                canvas(move |bounds, _, _| track.set(bounds), |_, _, _, _| {})
                                    .absolute()
                                    .inset_0()
                                    .size_full(),
                            )
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, event: &gpui::MouseDownEvent, _, cx| {
                                    this.dragging = true;
                                    this.playing = false;
                                    let bounds = this.track.get();
                                    this.progress = (f32::from(event.position.x - bounds.left())
                                        / f32::from(bounds.size.width).max(1.))
                                    .clamp(0., 1.);
                                    cx.notify();
                                }),
                            )
                            .on_mouse_move(cx.listener(
                                |this, event: &gpui::MouseMoveEvent, _, cx| {
                                    if this.dragging && event.dragging() {
                                        let bounds = this.track.get();
                                        this.progress =
                                            (f32::from(event.position.x - bounds.left())
                                                / f32::from(bounds.size.width).max(1.))
                                            .clamp(0., 1.);
                                        cx.notify();
                                    }
                                },
                            ))
                            .on_mouse_up(
                                MouseButton::Left,
                                cx.listener(|this, _, _, _| this.dragging = false),
                            )
                            .on_mouse_up_out(
                                MouseButton::Left,
                                cx.listener(|this, _, _, _| this.dragging = false),
                            ),
                    ),
            )
    }
}

fn main() {
    application().run(|cx: &mut App| {
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(1100.), px(760.)),
                    cx,
                ))),
                ..Default::default()
            },
            |_, cx| {
                cx.new(|_| Preview {
                    progress: 0.,
                    target: 1.,
                    playing: false,
                    last_frame: Instant::now(),
                    kind: TransitionKind::BlurFade,
                    track: Rc::new(Cell::new(Bounds::default())),
                    dragging: false,
                    image: Arc::new(Image::from_bytes(
                        ImageFormat::Svg,
                        include_bytes!("album-cover.svg").to_vec(),
                    ))
                    .into(),
                })
            },
        )
        .expect("failed to open subtree transition example");
    });
}
