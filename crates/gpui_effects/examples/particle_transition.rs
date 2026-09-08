use std::{cell::Cell, rc::Rc, sync::Arc, time::Instant};

use gpui::{
    App, Bounds, Context, FontWeight, Image, ImageFormat, ImageSource, MouseButton, Pixels, Render,
    Window, WindowBounds, WindowOptions, canvas, div, img, prelude::*, px, rgb, size,
};
use gpui_effects::{ParticleTransitionOptions, subtree_particle_transition};
use gpui_platform::application;

const ART: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" width="300" height="240" viewBox="0 0 300 240">
<defs><linearGradient id="light"><stop stop-color="#65deec"/><stop offset=".55" stop-color="#a1b7ff"/><stop offset="1" stop-color="#e6a6ed"/></linearGradient></defs>
<g fill="none" stroke="url(#light)" stroke-width="12" stroke-linejoin="round">
<circle cx="150" cy="120" r="78"/><path d="m150 43 22 55 55 22-55 22-22 55-22-55-55-22 55-22Z"/>
</g><circle cx="150" cy="120" r="13" fill="#d0eaff"/></svg>"##;

struct TransitionPreview {
    options: ParticleTransitionOptions,
    progress: f32,
    target: f32,
    paused: bool,
    duration: f32,
    last_frame: Instant,
    track: Rc<Cell<Bounds<Pixels>>>,
    artwork: bool,
    image: ImageSource,
}

impl TransitionPreview {
    fn new() -> Self {
        Self {
            options: ParticleTransitionOptions::default(),
            progress: 0.,
            target: 0.,
            paused: false,
            duration: 2.4,
            last_frame: Instant::now(),
            track: Rc::new(Cell::new(Bounds::default())),
            artwork: false,
            image: Arc::new(Image::from_bytes(ImageFormat::Svg, ART.to_vec())).into(),
        }
    }

    fn action(
        &self,
        index: usize,
        label: &'static str,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        div()
            .id(("action", index))
            .px_4()
            .py_2()
            .rounded_full()
            .bg(rgb(if index < 2 { 0x354b70 } else { 0x1f2b42 }))
            .hover(|s| s.bg(rgb(0x435b81)))
            .cursor_pointer()
            .text_sm()
            .child(label)
            .on_click(cx.listener(move |this, _, _, cx| {
                match index {
                    0 => {
                        this.target = 1.;
                        this.paused = false;
                    }
                    1 => {
                        this.target = 0.;
                        this.paused = false;
                    }
                    2 => this.paused = !this.paused,
                    _ => {
                        this.artwork = !this.artwork;
                        this.progress = 0.;
                        this.target = 0.;
                        this.paused = false;
                    }
                }
                this.last_frame = Instant::now();
                cx.notify();
            }))
    }

    fn control(
        &self,
        index: usize,
        label: &'static str,
        value: String,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        div()
            .flex_1()
            .flex()
            .flex_col()
            .gap_3()
            .child(div().text_xs().text_color(rgb(0x8a9ab9)).child(label))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(value)
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .children([-1., 1.].into_iter().enumerate().map(|(button, delta)| {
                                div()
                                    .id(("adjust", index * 2 + button))
                                    .size(px(30.))
                                    .rounded(px(9.))
                                    .bg(rgb(0x26364f))
                                    .hover(|s| s.bg(rgb(0x374c6a)))
                                    .cursor_pointer()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .child(if delta < 0. { "−" } else { "+" })
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        match index {
                                            0 => {
                                                this.duration =
                                                    (this.duration + delta * 0.4).clamp(0.8, 6.)
                                            }
                                            1 => {
                                                this.options.spread =
                                                    px((f32::from(this.options.spread)
                                                        + delta * 10.)
                                                        .clamp(0., 100.))
                                            }
                                            _ => {
                                                this.options.cell_size =
                                                    px((f32::from(this.options.cell_size)
                                                        + delta * 0.5)
                                                        .clamp(1., 6.))
                                            }
                                        }
                                        if index != 0 {
                                            this.progress = 0.;
                                            this.target = 0.;
                                        }
                                        this.last_frame = Instant::now();
                                        cx.notify();
                                    }))
                            })),
                    ),
            )
    }
}

impl Render for TransitionPreview {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let now = Instant::now();
        let dt = now.duration_since(self.last_frame).as_secs_f32().min(0.05);
        self.last_frame = now;
        if !self.paused && self.progress != self.target && window.supports_subtree_effects() {
            let step = dt / self.duration;
            self.progress += (self.target - self.progress).clamp(-step, step);
            if self.progress != self.target {
                window.request_animation_frame();
            }
        }
        let source = div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_5()
            .when(self.artwork, |s| {
                s.child(img(self.image.clone()).w(px(300.)).h(px(240.)))
            })
            .when(!self.artwork, |s| {
                s.child(
                    div()
                        .text_size(px(96.))
                        .line_height(px(120.))
                        .font_weight(FontWeight::BOLD)
                        .text_color(rgb(0x9ae4ee))
                        .child("Stardust"),
                )
                .child(
                    div()
                        .text_size(px(38.))
                        .text_color(rgb(0xc6b0ed))
                        .child("聚散之间"),
                )
            });
        let surface = subtree_particle_transition(source, self.progress, self.options)
            .capture_padding(px(200.));
        let track = self.track.clone();
        div()
            .size_full()
            .p_8()
            .bg(rgb(0x0a111e))
            .text_color(rgb(0xe7efff))
            .flex()
            .flex_col()
            .gap_6()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(div().text_size(px(28.)).child("Gather & scatter"))
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(rgb(0x8797b5))
                                    .child("Every particle has a place to return."),
                            ),
                    )
                    .child(self.action(3, if self.artwork { "Artwork" } else { "Text" }, cx)),
            )
            .child(
                div()
                    .w_full()
                    .flex_1()
                    .min_h_0()
                    .rounded(px(28.))
                    .overflow_hidden()
                    .bg(rgb(0x111e31))
                    .child(surface),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(self.action(0, "Scatter", cx))
                    .child(self.action(1, "Gather", cx))
                    .child(self.action(2, if self.paused { "Resume" } else { "Pause" }, cx))
                    .child(div().flex_1())
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(0x92a6c6))
                            .child(format!("{:.0}%", self.progress * 100.)),
                    ),
            )
            .child(
                div()
                    .id("progress")
                    .relative()
                    .w_full()
                    .h(px(14.))
                    .rounded_full()
                    .bg(rgb(0x24334b))
                    .cursor_pointer()
                    .child(
                        canvas(move |bounds, _, _| track.set(bounds), |_, _, _, _| {})
                            .absolute()
                            .inset_0()
                            .size_full(),
                    )
                    .child(
                        div()
                            .h_full()
                            .w(gpui::relative(self.progress))
                            .rounded_full()
                            .bg(rgb(0x88c9e5)),
                    )
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, event: &gpui::MouseDownEvent, _, cx| {
                            let bounds = this.track.get();
                            if bounds.size.width > px(0.) {
                                this.progress = (f32::from(event.position.x - bounds.origin.x)
                                    / f32::from(bounds.size.width))
                                .clamp(0., 1.);
                                this.paused = true;
                                cx.notify();
                            }
                        }),
                    ),
            )
            .child(
                div()
                    .p_5()
                    .rounded(px(20.))
                    .bg(rgb(0x152238))
                    .flex()
                    .gap_8()
                    .child(self.control(0, "DURATION", format!("{:.1} s", self.duration), cx))
                    .child(self.control(
                        1,
                        "SPREAD",
                        format!("{:.0} px", f32::from(self.options.spread)),
                        cx,
                    ))
                    .child(self.control(
                        2,
                        "FRAGMENT SIZE",
                        format!("{:.1} px", f32::from(self.options.cell_size)),
                        cx,
                    )),
            )
            .when(!window.supports_subtree_effects(), |s| {
                s.child("Subtree effects are unavailable on this renderer.")
            })
    }
}

fn main() {
    application().run(|cx: &mut App| {
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(1100.), px(840.)),
                    cx,
                ))),
                ..Default::default()
            },
            |_, cx| cx.new(|_| TransitionPreview::new()),
        )
        .expect("failed to open particle transition example");
    });
}
