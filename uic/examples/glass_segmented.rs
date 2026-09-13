use gpui::{
    Bounds, Context, IntoElement, Render, Window, WindowBounds, WindowOptions, div,
    linear_color_stop, multi_linear_gradient, prelude::*, px, rgb, rgba, size,
};
use uic::components::glass::{GlassSegmentedAppearance, GlassSegmentedControl};

struct Demo {
    selected: usize,
    dark: bool,
    textured: bool,
    animated: bool,
    opaque: bool,
    disabled: bool,
}

impl Demo {
    fn button(
        &self,
        id: &'static str,
        label: &'static str,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div()
            .id(id)
            .px_3()
            .py_2()
            .rounded_lg()
            .bg(rgba(0xffffff16))
            .hover(|style| style.bg(rgba(0xffffff28)))
            .cursor_pointer()
            .child(label)
            .on_click(cx.listener(move |this, _, _, cx| {
                match id {
                    "theme" => this.dark = !this.dark,
                    "background" => this.textured = !this.textured,
                    "motion" => this.animated = !this.animated,
                    "transparency" => this.opaque = !this.opaque,
                    "disabled" => this.disabled = !this.disabled,
                    _ => {}
                }
                cx.notify();
            }))
    }
}

impl Render for Demo {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.entity();
        let appearance = if self.dark {
            GlassSegmentedAppearance::dark()
        } else {
            GlassSegmentedAppearance::light()
        };
        let foreground = rgb(if self.dark { 0xf3f5fc } else { 0x344a60 });
        let colors = if self.dark {
            [0x28385b, 0x534872, 0x284f62]
        } else {
            [0xc9dfed, 0xe4d5ec, 0xdbebdc]
        };
        div()
            .size_full()
            .p_8()
            .flex()
            .flex_col()
            .gap_5()
            .bg(rgb(0x101824))
            .text_color(rgb(0xe3edf5))
            .child(div().text_size(px(28.)).child("Glass selection"))
            .child(
                div()
                    .text_sm()
                    .text_color(rgb(0x94a9bb))
                    .child("Choose a view · Arrow keys to navigate"),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_3()
                    .child(self.button("theme", if self.dark { "Dark" } else { "Light" }, cx))
                    .child(self.button(
                        "background",
                        if self.textured {
                            "Detailed background"
                        } else {
                            "Plain background"
                        },
                        cx,
                    ))
                    .child(self.button(
                        "motion",
                        if self.animated {
                            "Motion: On"
                        } else {
                            "Motion: Off"
                        },
                        cx,
                    ))
                    .child(self.button(
                        "transparency",
                        if self.opaque { "Opaque" } else { "Translucent" },
                        cx,
                    ))
                    .child(self.button(
                        "disabled",
                        if self.disabled { "Disabled" } else { "Enabled" },
                        cx,
                    )),
            )
            .child(
                div()
                    .relative()
                    .w_full()
                    .flex_1()
                    .min_h(px(300.))
                    .rounded(px(28.))
                    .overflow_hidden()
                    .bg(rgb(colors[0]))
                    .when(self.textured, |stage| {
                        stage
                            .child(div().absolute().inset_0().bg(multi_linear_gradient(
                                125.,
                                [
                                    linear_color_stop(rgb(colors[0]), 0.),
                                    linear_color_stop(rgb(colors[1]), 0.5),
                                    linear_color_stop(rgb(colors[2]), 1.),
                                ],
                            )))
                            .child(
                                div()
                                    .absolute()
                                    .left(px(80.))
                                    .top(px(70.))
                                    .size(px(360.))
                                    .rounded_full()
                                    .bg(multi_linear_gradient(
                                        145.,
                                        [
                                            linear_color_stop(rgba(0x82cbbd88), 0.),
                                            linear_color_stop(rgba(0x82cbbd00), 1.),
                                        ],
                                    )),
                            )
                            .children((0..18).map(|index| {
                                div()
                                    .absolute()
                                    .left(px(index as f32 * 72.))
                                    .top_0()
                                    .bottom_0()
                                    .w(px(1.))
                                    .bg(rgba(0xffffff28))
                            }))
                            .children((0..14).map(|index| {
                                div()
                                    .absolute()
                                    .top(px(index as f32 * 48.))
                                    .left_0()
                                    .right_0()
                                    .h(px(1.))
                                    .bg(rgba(0xffffff28))
                            }))
                    })
                    .child(
                        div()
                            .relative()
                            .size_full()
                            .flex()
                            .flex_col()
                            .items_center()
                            .justify_center()
                            .gap_8()
                            .child(
                                div()
                                    .text_color(foreground)
                                    .text_size(px(13.))
                                    .child("WORKSPACE"),
                            )
                            .child(
                                GlassSegmentedControl::new("workspace-view", self.selected)
                                    .label("Workspace view")
                                    .appearance(appearance)
                                    .text_color(foreground)
                                    .text_size(px(16.))
                                    .line_height(px(24.))
                                    .rounded(px(28.))
                                    .animated(self.animated)
                                    .reduced_transparency(self.opaque)
                                    .disabled(self.disabled)
                                    .option(0, "Overview")
                                    .option(1, "Recent activity")
                                    .option(2, "Files")
                                    .disabled_option(3, "Archived")
                                    .on_change(move |value, _, cx| {
                                        entity.update(cx, |this, cx| {
                                            this.selected = value;
                                            cx.notify();
                                        });
                                    }),
                            )
                            .child(
                                div()
                                    .text_color(foreground.opacity(0.7))
                                    .text_size(px(14.))
                                    .child(
                                        [
                                            "Overview selected",
                                            "Recent activity selected",
                                            "Files selected",
                                        ][self.selected],
                                    ),
                            ),
                    ),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(rgb(0x94a9bb))
                    .child("Variable-width options · Stable content · Interruptible motion"),
            )
    }
}

fn main() {
    gpui_platform::application().run(|cx| {
        uic::init(cx);
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
                cx.new(|_| Demo {
                    selected: 0,
                    dark: false,
                    textured: true,
                    animated: true,
                    opaque: false,
                    disabled: false,
                })
            },
        )
        .expect("glass selection window");
    });
}
