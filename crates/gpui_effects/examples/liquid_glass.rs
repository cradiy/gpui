use gpui::{
    App, Bounds, Context, CursorStyle, FontWeight, MouseButton, MouseDownEvent, MouseMoveEvent,
    Pixels, Point, Render, Window, WindowBounds, WindowOptions, div, linear_color_stop,
    multi_linear_gradient, point, prelude::*, px, rgb, rgba, size,
};
use gpui_effects::{LiquidGlass, LiquidGlassAppearance};
use gpui_platform::application;

const GLASS_WIDTH: f32 = 460.0;
const GLASS_HEIGHT: f32 = 280.0;

struct GlassStudy {
    appearance: LiquidGlassAppearance,
    radius: f32,
    position: Point<Pixels>,
    drag_offset: Option<Point<Pixels>>,
    grid: bool,
    dark_background: bool,
}

impl GlassStudy {
    fn new() -> Self {
        Self {
            appearance: LiquidGlassAppearance::regular(),
            radius: 40.0,
            position: point(px(480.0), px(245.0)),
            drag_offset: None,
            grid: true,
            dark_background: false,
        }
    }

    fn controls(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let button = |id: &'static str, label: String| {
            div()
                .id(id)
                .px_3()
                .py_2()
                .rounded_lg()
                .bg(rgba(0xffffffcc))
                .cursor_pointer()
                .hover(|style| style.bg(rgb(0xffffff)))
                .child(label)
        };
        div()
            .absolute()
            .left(px(32.0))
            .top(px(32.0))
            .w(px(300.0))
            .p_5()
            .rounded(px(20.0))
            .bg(rgba(0xf0f4f9f5))
            .text_color(rgb(0x21334a))
            .text_sm()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .font_weight(FontWeight::BOLD)
                    .child("LIQUID GLASS / MATERIAL STUDY"),
            )
            .child("Drag the glass across the background.")
            .child(
                div()
                    .flex()
                    .gap_2()
                    .children(["Regular", "Clear", "Dark"].into_iter().map(|preset| {
                        button(preset, preset.into()).on_click(cx.listener(
                            move |this, _, _, cx| {
                                this.appearance = match preset {
                                    "Clear" => LiquidGlassAppearance::clear(),
                                    "Dark" => LiquidGlassAppearance::dark(),
                                    _ => LiquidGlassAppearance::regular(),
                                };
                                cx.notify();
                            },
                        ))
                    })),
            )
            .children(Knob::ALL.into_iter().map(|knob| {
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(div().flex_1().child(format!(
                        "{} {:.2}",
                        knob.label(),
                        knob.value(self)
                    )))
                    .child(
                        div()
                            .id(("less", knob as usize))
                            .px_3()
                            .py_2()
                            .rounded_lg()
                            .bg(rgba(0xffffffcc))
                            .cursor_pointer()
                            .child("−")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                knob.adjust(this, -1.0);
                                cx.notify();
                            })),
                    )
                    .child(
                        div()
                            .id(("more", knob as usize))
                            .px_3()
                            .py_2()
                            .rounded_lg()
                            .bg(rgba(0xffffffcc))
                            .cursor_pointer()
                            .child("+")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                knob.adjust(this, 1.0);
                                cx.notify();
                            })),
                    )
            }))
            .child(
                button(
                    "grid",
                    format!("Grid: {}", if self.grid { "on" } else { "off" }),
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.grid = !this.grid;
                    cx.notify();
                })),
            )
            .child(
                button("background", "Switch background".into()).on_click(cx.listener(
                    |this, _, _, cx| {
                        this.dark_background = !this.dark_background;
                        cx.notify();
                    },
                )),
            )
            .child("Refraction = 0 disables bending.\nDispersion = 0 disables RGB separation.")
    }
}

#[derive(Clone, Copy)]
enum Knob {
    Blur,
    Clarity,
    Refraction,
    Thickness,
    Highlight,
    Dispersion,
    Radius,
}

impl Knob {
    const ALL: [Self; 7] = [
        Self::Blur,
        Self::Clarity,
        Self::Refraction,
        Self::Thickness,
        Self::Highlight,
        Self::Dispersion,
        Self::Radius,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::Blur => "Blur (px)",
            Self::Clarity => "Clarity",
            Self::Refraction => "Refraction (px)",
            Self::Thickness => "Thickness (px)",
            Self::Highlight => "Highlight",
            Self::Dispersion => "Dispersion",
            Self::Radius => "Corner radius (px)",
        }
    }

    fn value(self, study: &GlassStudy) -> f32 {
        match self {
            Self::Blur => study.appearance.blur_radius.as_f32(),
            Self::Clarity => study.appearance.clarity,
            Self::Refraction => study.appearance.refraction.as_f32(),
            Self::Thickness => study.appearance.thickness.as_f32(),
            Self::Highlight => study.appearance.highlight,
            Self::Dispersion => study.appearance.dispersion,
            Self::Radius => study.radius,
        }
    }

    fn adjust(self, study: &mut GlassStudy, direction: f32) {
        let (step, max) = match self {
            Self::Blur => (1.0, 24.0),
            Self::Clarity | Self::Highlight => (0.1, 1.0),
            Self::Refraction => (1.0, 24.0),
            Self::Thickness => (2.0, 64.0),
            Self::Dispersion => (0.01, 0.1),
            Self::Radius => (8.0, GLASS_HEIGHT * 0.5),
        };
        let value = (self.value(study) + step * direction).clamp(0.0, max);
        match self {
            Self::Blur => study.appearance.blur_radius = px(value),
            Self::Clarity => study.appearance.clarity = value,
            Self::Refraction => study.appearance.refraction = px(value),
            Self::Thickness => study.appearance.thickness = px(value),
            Self::Highlight => study.appearance.highlight = value,
            Self::Dispersion => study.appearance.dispersion = value,
            Self::Radius => study.radius = value,
        }
    }
}

impl Render for GlassStudy {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let viewport = window.viewport_size();
        let background = if self.dark_background {
            [rgb(0x13243a), rgb(0x354f69), rgb(0x182c3c)]
        } else {
            [rgb(0xe9d4c7), rgb(0xb7d7e0), rgb(0xa6bbdc)]
        };
        let ink = if self.dark_background {
            rgba(0xffffff20)
        } else {
            rgba(0x29425c25)
        };
        div()
            .id("glass-study")
            .relative()
            .size_full()
            .overflow_hidden()
            .bg(multi_linear_gradient(
                125.0,
                [
                    linear_color_stop(background[0], 0.0),
                    linear_color_stop(background[1], 0.5),
                    linear_color_stop(background[2], 1.0),
                ],
            ))
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, window, cx| {
                if event.pressed_button != Some(MouseButton::Left) {
                    this.drag_offset = None;
                    return;
                }
                if let Some(offset) = this.drag_offset {
                    let viewport = window.viewport_size();
                    let position = event.position - offset;
                    this.position = point(
                        position
                            .x
                            .clamp(px(0.0), (viewport.width - px(GLASS_WIDTH)).max(px(0.0))),
                        position
                            .y
                            .clamp(px(0.0), (viewport.height - px(GLASS_HEIGHT)).max(px(0.0))),
                    );
                    cx.notify();
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.drag_offset = None;
                    cx.notify();
                }),
            )
            .child(
                div()
                    .absolute()
                    .left(px(450.0))
                    .top(px(110.0))
                    .size(px(350.0))
                    .rounded_full()
                    .bg(multi_linear_gradient(
                        30.0,
                        [
                            linear_color_stop(rgba(0xffb19eaa), 0.0),
                            linear_color_stop(rgba(0xd598c122), 1.0),
                        ],
                    )),
            )
            .child(
                div()
                    .absolute()
                    .right(px(-100.0))
                    .bottom(px(-180.0))
                    .size(px(650.0))
                    .rounded_full()
                    .bg(multi_linear_gradient(
                        60.0,
                        [
                            linear_color_stop(rgba(0x51a0bb88), 0.0),
                            linear_color_stop(rgba(0x6896e500), 1.0),
                        ],
                    )),
            )
            .when(self.grid, |root| {
                root.children((0..(viewport.width.as_f32() / 48.0) as usize).map(|index| {
                    div()
                        .absolute()
                        .left(px(index as f32 * 48.0))
                        .top_0()
                        .w(px(1.0))
                        .h_full()
                        .bg(ink)
                }))
                .children(
                    (0..(viewport.height.as_f32() / 48.0) as usize).map(|index| {
                        div()
                            .absolute()
                            .top(px(index as f32 * 48.0))
                            .left_0()
                            .h(px(1.0))
                            .w_full()
                            .bg(ink)
                    }),
                )
            })
            .child(
                div()
                    .absolute()
                    .left(px(410.0))
                    .top(px(310.0))
                    .text_size(px(104.0))
                    .font_weight(FontWeight::BOLD)
                    .text_color(ink)
                    .child("REFRACTION"),
            )
            .child(
                LiquidGlass::with_appearance(self.appearance)
                    .id("test-glass")
                    .absolute()
                    .left(self.position.x)
                    .top(self.position.y)
                    .w(px(GLASS_WIDTH))
                    .h(px(GLASS_HEIGHT))
                    .rounded(px(self.radius))
                    .p_8()
                    .flex()
                    .flex_col()
                    .justify_center()
                    .gap_3()
                    .text_color(if self.appearance.tint.a > 0.2 {
                        rgb(0xffffff)
                    } else {
                        rgb(0x1c3049)
                    })
                    .shadow(vec![
                        gpui::BoxShadow::new(px(0.0), px(18.0), rgba(0x182a4528).into())
                            .blur_radius(px(42.0))
                            .spread_radius(px(-12.0)),
                    ])
                    .cursor(if self.drag_offset.is_some() {
                        CursorStyle::ClosedHand
                    } else {
                        CursorStyle::OpenHand
                    })
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, event: &MouseDownEvent, _, cx| {
                            this.drag_offset = Some(event.position - this.position);
                            cx.notify();
                        }),
                    )
                    .child(
                        div()
                            .text_size(px(30.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Liquid glass"),
                    )
                    .child("Drag to inspect the curved edge.")
                    .child(
                        div()
                            .text_sm()
                            .opacity(0.7)
                            .child("The background refracts. This foreground stays sharp."),
                    ),
            )
            .child(self.controls(cx))
    }
}

fn main() {
    application().run(|cx: &mut App| {
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(1200.0), px(800.0)),
                    cx,
                ))),
                ..Default::default()
            },
            |_, cx| cx.new(|_| GlassStudy::new()),
        )
        .expect("failed to open liquid-glass material study");
        cx.activate(true);
    });
}
