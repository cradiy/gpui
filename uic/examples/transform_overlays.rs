use gpui::{
    AppContext, Bounds, Context, Entity, IntoElement, Render, TransformationMatrix, Window,
    WindowBounds, WindowOptions, div, prelude::*, px, rgb, size,
};
use gpui_effects::transform_group;
use uic::components::{
    context_menu::{self, ContextMenu, ContextMenuExt, ContextMenuTrigger},
    dropdown::{DropdownState, dropdown},
    popover::{Popover, PopoverState},
};

#[path = "support/context_menu_style.rs"]
#[allow(dead_code)] // The material-switching example uses the other shared variants.
mod context_menu_style;
use context_menu_style::MenuMaterial;

struct Demo {
    zoom: f32,
    dropdown: Entity<DropdownState>,
    popover: Entity<PopoverState>,
}

fn button(label: &'static str) -> impl IntoElement {
    div()
        .w(px(104.))
        .h(px(32.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(7.))
        .border_1()
        .border_color(rgb(0x3b4858))
        .bg(rgb(0x283442))
        .text_size(px(13.))
        .text_color(rgb(0xf1f5f9))
        .child(label)
}

fn menu() -> ContextMenu {
    context_menu_style::menu(MenuMaterial::DarkFrosted)
        .action("Open", |_, _| {})
        .submenu("More", |menu| {
            menu.action("Details", |_, _| {}).action("Copy", |_, _| {})
        })
}

impl Render for Demo {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let content = div()
            .relative()
            .w(px(780.))
            .h(px(320.))
            .bg(rgb(0x19232e))
            .context_menu(|_, _| menu())
            .child(
                div().absolute().left(px(24.)).top(px(36.)).child(
                    dropdown(&self.dropdown)
                        .w(px(192.))
                        .p(px(5.))
                        .rounded(px(10.))
                        .border_color(rgb(0x3b4858))
                        .shadow_md()
                        .bg(rgb(0x222e3b))
                        .text_size(px(13.))
                        .text_color(rgb(0xf1f5f9))
                        .trigger(button("Dropdown"))
                        .menu(
                            div()
                                .flex()
                                .flex_col()
                                .child(div().px_3().py_2().child("Recent files"))
                                .child(
                                    div()
                                        .px_3()
                                        .py_2()
                                        .text_color(rgb(0x93a5b8))
                                        .child("Shared with me"),
                                ),
                        ),
                ),
            )
            .child(
                div().absolute().left(px(140.)).top(px(36.)).child(
                    Popover::new(&self.popover)
                        .p_3()
                        .rounded(px(10.))
                        .border_color(rgb(0x3b4858))
                        .shadow_md()
                        .bg(rgb(0x222e3b))
                        .text_size(px(13.))
                        .text_color(rgb(0xf1f5f9))
                        .trigger(button("Popover"))
                        .content(|_, _| {
                            div()
                                .w(px(176.))
                                .flex()
                                .flex_col()
                                .gap_2()
                                .child("Popover")
                                .child(
                                    div()
                                        .text_color(rgb(0x93a5b8))
                                        .child("Content stays readable at every zoom level."),
                                )
                        }),
                ),
            )
            .child(
                div()
                    .absolute()
                    .left(px(256.))
                    .top(px(36.))
                    .child(ContextMenuTrigger::new(button("Menu"), |_, _| menu()).id("menu")),
            );
        div()
            .size_full()
            .p_6()
            .flex()
            .flex_col()
            .gap_4()
            .bg(rgb(0x111923))
            .text_color(rgb(0xe5edf5))
            .child(
                div()
                    .w(px(780.))
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(div().text_size(px(22.)).child("Anchored overlays"))
                    .child(
                        div()
                            .px_3()
                            .py_1()
                            .rounded_md()
                            .bg(rgb(0x202c39))
                            .text_size(px(12.))
                            .text_color(rgb(0xa3b4c6))
                            .child(format!("{:.0}%", self.zoom * 100.)),
                    ),
            )
            .child(
                div()
                    .text_size(px(13.))
                    .text_color(rgb(0x93a5b8))
                    .child("Popups follow their triggers while keeping their size."),
            )
            .child(
                div()
                    .rounded(px(12.))
                    .overflow_hidden()
                    .w(px(780.))
                    .on_scroll_wheel(cx.listener(|this, event: &gpui::ScrollWheelEvent, _, cx| {
                        let delta = event.delta.pixel_delta(px(20.));
                        this.zoom = (this.zoom * (1. + f32::from(delta.y) * 0.003)).clamp(0.6, 2.);
                        cx.notify();
                    }))
                    .child(
                        transform_group(
                            content,
                            TransformationMatrix {
                                rotation_scale: [[self.zoom, 0.], [0., self.zoom]],
                                translation: [0., 0.],
                            },
                        )
                        .auto_raster_scale("demo-density"),
                    ),
            )
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(rgb(0x788c9f))
                    .child("Scroll on the canvas to zoom · Right-click for a menu · Esc to close"),
            )
            .child(context_menu::layer(cx))
    }
}

fn main() {
    gpui_platform::application().run(|cx| {
        context_menu::init(cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(840.), px(480.)),
                    cx,
                ))),
                ..Default::default()
            },
            |window, cx| {
                cx.new(|cx| Demo {
                    zoom: 1.5,
                    dropdown: cx.new(|cx| DropdownState::new(window, cx)),
                    popover: cx.new(|cx| PopoverState::new(window, cx)),
                })
            },
        )
        .expect("open overlay example");
    });
}
