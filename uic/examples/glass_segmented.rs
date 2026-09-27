use gpui::{
    Bounds, Context, FontWeight, Hsla, IntoElement, Render, Window, WindowBounds, WindowOptions,
    div, prelude::*, px, rgb, rgba, size, svg,
};
use uic::{
    assets::{LucideAssets, LucideIcons},
    components::glass::{GlassSegmentedAppearance, GlassSegmentedControl},
};

struct Demo {
    selected: usize,
    vertical_selected: usize,
    dark: bool,
    samples: bool,
    animated: bool,
    opaque: bool,
    disabled: bool,
}

fn tab(icon: LucideIcons, label: &'static str, color: Hsla) -> impl IntoElement {
    div()
        .w(px(64.))
        .flex()
        .flex_col()
        .items_center()
        .gap(px(6.))
        .child(svg().path(icon.path()).size(px(24.)).text_color(color))
        .child(div().child(label))
}

impl Demo {
    fn control(&self, vertical: bool, color: Hsla, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.entity();
        let selected = if vertical {
            self.vertical_selected
        } else {
            self.selected
        };
        let appearance = if self.dark {
            GlassSegmentedAppearance::dark()
        } else {
            GlassSegmentedAppearance::light()
        };
        GlassSegmentedControl::new(
            if vertical {
                "vertical-view"
            } else {
                "horizontal-view"
            },
            selected,
        )
        .label(if vertical {
            "Vertical workspace view"
        } else {
            "Horizontal workspace view"
        })
        .appearance(appearance)
        .text_color(color)
        .text_size(px(12.))
        .line_height(px(16.))
        .rounded(px(32.))
        .when(vertical, |control| control.flex_col().items_stretch())
        .animated(self.animated)
        .reduced_transparency(self.opaque)
        .disabled(self.disabled)
        .option(0, tab(LucideIcons::House, "Overview", color))
        .option(1, tab(LucideIcons::Activity, "Activity", color))
        .option(2, tab(LucideIcons::Folder, "Files", color))
        .disabled_option(3, tab(LucideIcons::Archive, "Archived", color))
        .on_change(move |value, _, cx| {
            entity.update(cx, |this, cx| {
                if vertical {
                    this.vertical_selected = value;
                } else {
                    this.selected = value;
                }
                cx.notify();
            });
        })
    }

    fn button(
        &self,
        id: &'static str,
        label: &'static str,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div()
            .id(id)
            .px(px(12.))
            .py(px(8.))
            .rounded_full()
            .bg(if self.dark {
                rgba(0xffffff12)
            } else {
                rgba(0x00000006)
            })
            .hover(|s| {
                s.bg(if self.dark {
                    rgba(0xffffff22)
                } else {
                    rgba(0x00000010)
                })
            })
            .cursor_pointer()
            .child(label)
            .on_click(cx.listener(move |this, _, _, cx| {
                match id {
                    "theme" => this.dark = !this.dark,
                    "background" => this.samples = !this.samples,
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
        let foreground = rgb(if self.dark { 0xf0f0f2 } else { 0x27292e });
        let secondary = rgb(if self.dark { 0x93949c } else { 0x787981 });
        div()
            .size_full()
            .p(px(32.))
            .flex()
            .flex_col()
            .gap(px(24.))
            .bg(rgb(if self.dark { 0x16171b } else { 0xfaf9f6 }))
            .text_color(foreground)
            .child(
                div()
                    .flex()
                    .items_baseline()
                    .gap(px(16.))
                    .child(
                        div()
                            .text_size(px(24.))
                            .font_weight(FontWeight::MEDIUM)
                            .child("Glass navigation"),
                    )
                    .child(
                        div()
                            .text_size(px(13.))
                            .text_color(secondary)
                            .child("Drag to explore"),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap(px(8.))
                    .text_size(px(12.))
                    .child(self.button("theme", if self.dark { "Dark" } else { "Light" }, cx))
                    .child(self.button(
                        "background",
                        if self.samples { "Shapes" } else { "Plain" },
                        cx,
                    ))
                    .child(self.button(
                        "motion",
                        if self.animated {
                            "Motion on"
                        } else {
                            "Motion off"
                        },
                        cx,
                    ))
                    .child(self.button(
                        "transparency",
                        if self.opaque { "Opaque" } else { "Glass" },
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
                    .flex_1()
                    .min_h(px(440.))
                    .w_full()
                    .rounded(px(28.))
                    .bg(rgb(if self.dark { 0x303030 } else { 0xa8a8a8 }))
                    .flex()
                    .items_center()
                    .justify_center()
                    .px(px(36.))
                    .child(
                        div()
                            .relative()
                            .w_full()
                            .max_w(px(720.))
                            .h(px(396.))
                            .when(self.samples, |stage| {
                                stage
                                    .child(
                                        div()
                                            .absolute()
                                            .left(px(272.))
                                            .top(px(248.))
                                            .w(px(112.))
                                            .h(px(92.))
                                            .rounded(px(12.))
                                            .bg(rgb(if self.dark { 0x648bb5 } else { 0x729ac2 })),
                                    )
                                    .child(
                                        div()
                                            .absolute()
                                            .right_0()
                                            .top(px(48.))
                                            .size(px(124.))
                                            .rounded_full()
                                            .bg(rgb(if self.dark { 0x8f939b } else { 0xb7bbc3 })),
                                    )
                            })
                            .child(
                                div()
                                    .absolute()
                                    .bottom(px(42.))
                                    .left_0()
                                    .right(px(164.))
                                    .flex()
                                    .justify_center()
                                    .child(self.control(false, foreground.into(), cx)),
                            )
                            .child(
                                div()
                                    .absolute()
                                    .top(px(64.))
                                    .right(px(8.))
                                    .flex()
                                    .flex_col()
                                    .items_center()
                                    .gap(px(14.))
                                    .child(self.control(true, foreground.into(), cx))
                                    .child(div().text_size(px(12.)).text_color(secondary).child(
                                        format!(
                                            "Vertical · {}",
                                            ["Overview", "Activity", "Files"]
                                                [self.vertical_selected]
                                        ),
                                    )),
                            )
                            .child(
                                div()
                                    .absolute()
                                    .bottom_0()
                                    .left_0()
                                    .right(px(164.))
                                    .text_center()
                                    .text_size(px(12.))
                                    .text_color(secondary)
                                    .child(format!(
                                        "Horizontal · {}",
                                        ["Overview", "Activity", "Files"][self.selected]
                                    )),
                            ),
                    ),
            )
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(secondary)
                    .child("Press to select · Hold and drag the glass"),
            )
    }
}

fn main() {
    gpui_platform::application()
        .with_assets(LucideAssets::new())
        .run(|cx| {
            uic::init(cx);
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                        None,
                        size(px(960.), px(720.)),
                        cx,
                    ))),
                    ..Default::default()
                },
                |_, cx| {
                    cx.new(|_| Demo {
                        selected: 0,
                        vertical_selected: 0,
                        dark: false,
                        samples: true,
                        animated: true,
                        opaque: false,
                        disabled: false,
                    })
                },
            )
            .expect("glass navigation window");
        });
}
