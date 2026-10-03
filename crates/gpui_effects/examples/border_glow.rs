use std::time::Instant;

use gpui::{
    App, Bounds, Context, Render, Window, WindowBounds, WindowOptions, border_color_stop,
    border_gradient, div, prelude::*, px, rgb, size,
};
use gpui_effects::{BorderGlowOptions, border_glow};
use gpui_platform::application;

struct Preview {
    options: BorderGlowOptions,
    phase: f32,
    last_frame: Instant,
    paused: bool,
    enabled: bool,
}

impl Preview {
    fn new() -> Self {
        Self {
            options: BorderGlowOptions::default(),
            phase: 0.,
            last_frame: Instant::now(),
            paused: false,
            enabled: true,
        }
    }

    fn panel(&self, dark: bool) -> impl IntoElement {
        let surface = if dark { 0x11131b } else { 0xffffff };
        let muted = if dark { 0x9196ad } else { 0x74798b };
        let gradient = border_gradient([
            border_color_stop(rgb(0x4285f4), 0.),
            border_color_stop(rgb(0xa16bfa), 0.20),
            border_color_stop(rgb(0xea4335), 0.42),
            border_color_stop(rgb(0xfbbc05), 0.60),
            border_color_stop(rgb(0x34a853), 0.80),
        ])
        .phase(self.phase);
        let options = BorderGlowOptions {
            intensity: if self.enabled {
                self.options.intensity
            } else {
                0.
            },
            ..self.options
        };

        div()
            .w_full()
            .flex_1()
            .min_h(px(180.))
            .px(px(42.))
            .py(px(28.))
            .rounded(px(22.))
            .bg(rgb(if dark { 0x0c0e15 } else { 0xf4f5f9 }))
            .flex()
            .flex_col()
            .justify_center()
            .gap(px(30.))
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(rgb(muted))
                    .child(if dark { "DARK" } else { "LIGHT" }),
            )
            .child(
                div()
                    .relative()
                    .w_full()
                    .p(px(1.5))
                    .child(
                        border_glow(gradient, options)
                            .absolute()
                            .inset_0()
                            .border(px(1.5))
                            .rounded_full(),
                    )
                    .child(
                        div()
                            .relative()
                            .w_full()
                            .h(px(60.))
                            .px(px(22.))
                            .rounded_full()
                            .bg(rgb(surface))
                            .text_color(rgb(muted))
                            .flex()
                            .items_center()
                            .gap(px(16.))
                            .child(
                                div()
                                    .relative()
                                    .w(px(18.))
                                    .h(px(18.))
                                    .flex_shrink_0()
                                    .child(
                                        div()
                                            .size(px(13.))
                                            .border(px(1.5))
                                            .border_color(rgb(muted))
                                            .rounded_full(),
                                    )
                                    .child(
                                        gpui::canvas(
                                            |_, _, _| {},
                                            move |bounds, _, window, _| {
                                                let mut path = gpui::PathBuilder::stroke(px(1.5));
                                                path.move_to(
                                                    bounds.origin + gpui::point(px(11.), px(11.)),
                                                );
                                                path.line_to(
                                                    bounds.origin + gpui::point(px(17.), px(17.)),
                                                );
                                                window
                                                    .paint_path(path.build().unwrap(), rgb(muted));
                                            },
                                        )
                                        .absolute()
                                        .inset_0(),
                                    ),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_size(px(16.))
                                    .child("Ask anything"),
                            )
                            .child(
                                div()
                                    .text_size(px(20.))
                                    .text_color(rgb(0x9b83ef))
                                    .child("✦"),
                            ),
                    ),
            )
    }

    fn control(&self, id: usize, label: String, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id(("control", id))
            .px(px(14.))
            .py(px(10.))
            .rounded(px(10.))
            .bg(rgb(0x202431))
            .text_size(px(12.))
            .text_color(rgb(0xd6daea))
            .cursor_pointer()
            .hover(|style| style.bg(rgb(0x2d3344)))
            .child(label)
            .on_click(cx.listener(move |this, _, _, cx| {
                match id {
                    0 => this.paused = !this.paused,
                    1 => this.enabled = !this.enabled,
                    2 => {
                        this.options.radius = px(if this.options.radius >= px(40.) {
                            12.
                        } else {
                            f32::from(this.options.radius) + 4.
                        });
                    }
                    _ => {
                        this.options.intensity = if this.options.intensity >= 8. {
                            1.
                        } else {
                            this.options.intensity + 1.
                        };
                    }
                }
                this.last_frame = Instant::now();
                cx.notify();
            }))
    }
}

impl Render for Preview {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let now = Instant::now();
        if !self.paused {
            self.phase = (self.phase + now.duration_since(self.last_frame).as_secs_f32() / 8.) % 1.;
            window.request_animation_frame();
        }
        self.last_frame = now;
        div()
            .size_full()
            .bg(rgb(0x161923))
            .text_color(rgb(0xf0f2fa))
            .p(px(28.))
            .flex()
            .flex_col()
            .gap(px(20.))
            .child(div().text_size(px(26.)).child("Gradient glow"))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap(px(8.))
                    .child(self.control(0, if self.paused { "Play" } else { "Pause" }.into(), cx))
                    .child(self.control(
                        1,
                        if self.enabled { "Glow on" } else { "Glow off" }.into(),
                        cx,
                    ))
                    .child(self.control(
                        2,
                        format!("Radius  {:.0}px", f32::from(self.options.radius)),
                        cx,
                    ))
                    .child(self.control(
                        3,
                        format!("Intensity  {:.0}", self.options.intensity),
                        cx,
                    )),
            )
            .child(self.panel(true))
            .child(self.panel(false))
    }
}

fn main() {
    application().run(|cx: &mut App| {
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(760.), px(620.)),
                    cx,
                ))),
                ..Default::default()
            },
            |window, cx| {
                window.set_window_title("Gradient glow");
                cx.new(|_| Preview::new())
            },
        )
        .unwrap();
        cx.activate(true);
    });
}
