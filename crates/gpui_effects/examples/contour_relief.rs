use std::{cell::Cell, rc::Rc, sync::Arc, time::Instant};

use gpui::{
    App, Bounds, Context, FontWeight, Image, ImageFormat, ImageSource, Pixels, Render, Window,
    WindowBounds, WindowOptions, canvas, div, img, prelude::*, px, rgb, size,
};
use gpui_effects::{ContourReliefOptions, subtree_contour_relief};
use gpui_platform::application;

const MARK: &[u8] =
    br##"<svg xmlns="http://www.w3.org/2000/svg" width="240" height="110" viewBox="0 0 240 110">
<g fill="none" stroke="#8090a8" stroke-width="12" stroke-linecap="round" stroke-linejoin="round">
<circle cx="52" cy="55" r="34"/>
<path d="m44 40 23 15-23 15Z" stroke-width="7"/>
<path d="M176 12 188 41 219 55 188 69 176 98 164 69 133 55 164 41Z"/>
</g></svg>"##;

struct ReliefPreview {
    options: ContourReliefOptions,
    target_light: [f32; 3],
    last_frame: Option<Instant>,
    regions: [Rc<Cell<Bounds<Pixels>>>; 3],
    mark: ImageSource,
    enabled: bool,
}

impl ReliefPreview {
    fn new() -> Self {
        let options = ContourReliefOptions::default();
        Self {
            target_light: options.light.direction,
            last_frame: None,
            options,
            regions: std::array::from_fn(|_| Rc::new(Cell::new(Bounds::default()))),
            mark: Arc::new(Image::from_bytes(ImageFormat::Svg, MARK.to_vec())).into(),
            enabled: true,
        }
    }

    fn content(&self) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .gap_8()
            .w_full()
            .text_color(rgb(0x8090a8))
            .child(
                div()
                    .text_size(px(100.))
                    .font_weight(FontWeight::BOLD)
                    .line_height(px(120.))
                    .child("Aa"),
            )
            .child(
                div()
                    .text_size(px(30.))
                    .font_weight(FontWeight::BOLD)
                    .child("浮光刻影"),
            )
            .child(img(self.mark.clone()).w(px(240.)).h(px(110.)))
            .child(div().text_size(px(22.)).child("Form & light"))
    }

    fn panel(&self, index: usize, cx: &mut Context<Self>) -> gpui::Stateful<gpui::Div> {
        let region = self.regions[index].clone();
        let depth = match index {
            0 => px(0.),
            1 => self.options.depth,
            _ => -self.options.depth,
        };
        div()
            .id(("relief", index))
            .relative()
            .flex_1()
            .min_w_0()
            .rounded(px(24.))
            .bg(rgb(0x202b3d))
            .p_8()
            .flex()
            .flex_col()
            .gap_5()
            .child(
                canvas(move |bounds, _, _| region.set(bounds), |_, _, _, _| {})
                    .absolute()
                    .inset_0()
                    .size_full(),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(0x9faec4))
                    .child(["FLAT", "RAISED", "RECESSED"][index]),
            )
            .child(
                subtree_contour_relief(
                    self.content(),
                    ContourReliefOptions {
                        depth,
                        ..self.options
                    },
                )
                .enabled(self.enabled && index != 0),
            )
            .on_mouse_move(
                cx.listener(move |this, event: &gpui::MouseMoveEvent, _, cx| {
                    let bounds = this.regions[index].get();
                    if bounds.size.width <= px(0.) || bounds.size.height <= px(0.) {
                        return;
                    }
                    let local = event.position - bounds.origin;
                    this.target_light = [
                        (f32::from(local.x) / f32::from(bounds.size.width)).clamp(0., 1.) * 1.4
                            - 0.7,
                        (f32::from(local.y) / f32::from(bounds.size.height)).clamp(0., 1.) * 1.4
                            - 0.7,
                        0.8,
                    ];
                    this.last_frame.get_or_insert_with(Instant::now);
                    cx.notify();
                }),
            )
    }

    fn control(
        &self,
        index: usize,
        label: &'static str,
        value: String,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap_3()
            .child(div().text_xs().text_color(rgb(0x889ab7)).child(label))
            .child(
                div()
                    .flex()
                    .justify_between()
                    .items_center()
                    .gap_3()
                    .child(value)
                    .child(div().flex().gap_1().children(
                        [(-1., "−"), (1., "+")].into_iter().enumerate().map(
                            |(button, (delta, label))| {
                                div()
                                    .id(("adjust", index * 2 + button))
                                    .size(px(30.))
                                    .rounded(px(8.))
                                    .bg(rgb(0x29374e))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .cursor_pointer()
                                    .hover(|s| s.bg(rgb(0x3a4c68)))
                                    .child(label)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        match index {
                                            0 => {
                                                this.options.width =
                                                    px((f32::from(this.options.width) + delta)
                                                        .clamp(1., 12.))
                                            }
                                            1 => {
                                                this.options.depth =
                                                    px((f32::from(this.options.depth)
                                                        + delta * 0.5)
                                                        .clamp(0.5, 8.))
                                            }
                                            _ => {
                                                this.options.roughness = (this.options.roughness
                                                    + delta * 0.1)
                                                    .clamp(0., 1.)
                                            }
                                        }
                                        cx.notify();
                                    }))
                            },
                        ),
                    )),
            )
    }
}

impl Render for ReliefPreview {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(previous) = self.last_frame {
            let now = Instant::now();
            let elapsed = now.duration_since(previous).as_secs_f32().min(0.05);
            let blend = 1. - (-elapsed / 0.16).exp();
            let mut settled = true;
            for (current, target) in self
                .options
                .light
                .direction
                .iter_mut()
                .zip(self.target_light)
            {
                *current += (target - *current) * blend;
                settled &= (target - *current).abs() < 0.001;
            }
            if settled {
                self.options.light.direction = self.target_light;
                self.last_frame = None;
            } else {
                self.last_frame = Some(now);
                window.request_animation_frame();
            }
        }
        div()
            .size_full()
            .p_8()
            .bg(rgb(0x0c1320))
            .text_color(rgb(0xe8eefb))
            .flex()
            .flex_col()
            .gap_6()
            .child(
                div()
                    .flex()
                    .justify_between()
                    .items_center()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(div().text_size(px(28.)).child("Contour relief"))
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(rgb(0x8c9bb4))
                                    .child("Move across a panel to guide the light."),
                            ),
                    )
                    .child(
                        div()
                            .id("toggle")
                            .px_4()
                            .py_2()
                            .rounded_full()
                            .bg(rgb(0x2a3b56))
                            .cursor_pointer()
                            .child(if self.enabled {
                                "Lighting · On"
                            } else {
                                "Lighting · Off"
                            })
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.enabled = !this.enabled;
                                cx.notify();
                            })),
                    ),
            )
            .child(
                div()
                    .id("panels")
                    .w_full()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(
                        div()
                            .flex()
                            .w_full()
                            .gap_5()
                            .children((0..3).map(|index| self.panel(index, cx))),
                    ),
            )
            .child(
                div()
                    .p_5()
                    .rounded(px(20.))
                    .bg(rgb(0x162135))
                    .flex()
                    .gap_8()
                    .child(self.control(
                        0,
                        "BEVEL WIDTH",
                        format!("{:.0} px", f32::from(self.options.width)),
                        cx,
                    ))
                    .child(self.control(
                        1,
                        "DEPTH",
                        format!("{:.1} px", f32::from(self.options.depth)),
                        cx,
                    ))
                    .child(self.control(
                        2,
                        "ROUGHNESS",
                        format!("{:.1}", self.options.roughness),
                        cx,
                    )),
            )
            .when(!window.supports_subtree_effects(), |root| {
                root.child("Subtree effects are unavailable on this renderer.")
            })
    }
}

fn main() {
    application().run(|cx: &mut App| {
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(1180.), px(820.)),
                    cx,
                ))),
                ..Default::default()
            },
            |_, cx| cx.new(|_| ReliefPreview::new()),
        )
        .expect("failed to open contour relief example");
    });
}
