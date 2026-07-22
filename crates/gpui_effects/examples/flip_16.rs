use std::{rc::Rc, sync::Arc};

use gpui::{
    App, Context, FocusHandle, Image, ImageFormat, KeyDownEvent, Render, Window, WindowOptions,
    div, prelude::*, px, rgb,
};
use gpui_effects::{
    Flip, FlipDirection, FlipEntry, FlipEvent, FlipLayout, FlipObjectFit, FlipReadingDirection,
    FlipStyle,
};
use gpui_platform::application;

const PAGE_SVGS: [&[u8]; 16] = [
    include_bytes!("flip-16/page-01.svg"),
    include_bytes!("flip-16/page-02.svg"),
    include_bytes!("flip-16/page-03.svg"),
    include_bytes!("flip-16/page-04.svg"),
    include_bytes!("flip-16/page-05.svg"),
    include_bytes!("flip-16/page-06.svg"),
    include_bytes!("flip-16/page-07.svg"),
    include_bytes!("flip-16/page-08.svg"),
    include_bytes!("flip-16/page-09.svg"),
    include_bytes!("flip-16/page-10.svg"),
    include_bytes!("flip-16/page-11.svg"),
    include_bytes!("flip-16/page-12.svg"),
    include_bytes!("flip-16/page-13.svg"),
    include_bytes!("flip-16/page-14.svg"),
    include_bytes!("flip-16/page-15.svg"),
    include_bytes!("flip-16/page-16.svg"),
];

struct Flip16Example {
    book: gpui::Entity<Flip>,
    style: FlipStyle,
    layout: FlipLayout,
    reading_direction: FlipReadingDirection,
    object_fit: FlipObjectFit,
    position: usize,
    last_direction: Option<FlipDirection>,
    preload_status: String,
    focus_handle: FocusHandle,
}

impl Flip16Example {
    fn new(cx: &mut Context<Self>) -> Self {
        let pages = PAGE_SVGS
            .into_iter()
            .map(|bytes| Arc::new(Image::from_bytes(ImageFormat::Svg, bytes.to_vec())))
            .collect::<Vec<_>>();
        let pages = Rc::new(pages);
        let wide_page = Arc::new(Image::from_bytes(
            ImageFormat::Svg,
            include_bytes!("flip-16/wide-spread.svg").to_vec(),
        ));
        let blank_page = Arc::new(Image::from_bytes(
            ImageFormat::Svg,
            include_bytes!("flip-16/blank.svg").to_vec(),
        ));
        let failed_page = Arc::new(Image::from_bytes(
            ImageFormat::Svg,
            include_bytes!("flip-16/failed.svg").to_vec(),
        ));

        // One logical sequence in reading order. The wide source occupies two
        // cropped slots but is decoded and uploaded only once.
        let mut entries = Vec::with_capacity(16);
        entries.push(FlipEntry::blank());
        entries.extend((0..7).map(|index| FlipEntry::page(pages[index].clone())));
        entries.push(FlipEntry::double_page(wide_page));
        entries.extend((7..14).map(|index| FlipEntry::page(pages[index].clone())));
        let book = cx.new(|_| {
            Flip::from_entries(entries, blank_page, failed_page, FlipLayout::Spread)
                .style(FlipStyle::Natural)
                .start_at(8)
                .object_fit(FlipObjectFit::Contain)
                .page_background(rgb(0x15191f))
                .trigger_width(px(64.))
                .completion_threshold(0.34)
        });

        cx.subscribe(&book, |this, _, event, cx| {
            match *event {
                FlipEvent::PreloadRequested {
                    reason, start, end, ..
                } => {
                    this.preload_status =
                        format!("preload slots {}–{} ({reason:?})", start + 1, end);
                }
                FlipEvent::Flipped {
                    direction,
                    position,
                } => {
                    this.position = position;
                    this.last_direction = Some(direction);
                }
                FlipEvent::PositionChanged { position, .. } => this.position = position,
                FlipEvent::SlotFailed { index } => {
                    this.preload_status = format!("slot {} failed", index + 1);
                }
                FlipEvent::SlotReady { .. } => {}
            }
            cx.notify();
        })
        .detach();

        Self {
            book,
            style: FlipStyle::Natural,
            layout: FlipLayout::Spread,
            reading_direction: FlipReadingDirection::LeftToRight,
            object_fit: FlipObjectFit::Contain,
            position: 8,
            last_direction: None,
            preload_status: "waiting for preload request".to_owned(),
            focus_handle: cx.focus_handle(),
        }
    }

    fn key_down(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        if event.is_held {
            return;
        }
        let direction = match event.keystroke.key.as_str() {
            "left" => FlipDirection::Backward,
            "right" | "space" => FlipDirection::Forward,
            _ => return,
        };
        self.book.update(cx, |book, cx| {
            book.flip(direction, cx);
        });
        cx.stop_propagation();
    }

    fn option_button(label: &'static str, selected: bool) -> gpui::Stateful<gpui::Div> {
        div()
            .id(label)
            .px_3()
            .py_1()
            .rounded_md()
            .cursor_pointer()
            .bg(if selected {
                rgb(0x46617f)
            } else {
                rgb(0x222832)
            })
            .text_color(if selected {
                rgb(0xffffff)
            } else {
                rgb(0xaeb7c3)
            })
            .child(label)
    }

    fn style_button(
        &self,
        label: &'static str,
        style: FlipStyle,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        Self::option_button(label, self.style == style).on_click(cx.listener(
            move |this, _, _, cx| {
                this.style = style;
                this.book.update(cx, |book, cx| book.set_style(style, cx));
                cx.notify();
            },
        ))
    }

    fn layout_button(
        &self,
        label: &'static str,
        layout: FlipLayout,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        Self::option_button(label, self.layout == layout).on_click(cx.listener(
            move |this, _, _, cx| {
                let (layout, position) = this.book.update(cx, |book, cx| {
                    let _ = book.set_layout(layout, cx);
                    (book.current_layout(), book.position())
                });
                this.layout = layout;
                this.position = position;
                cx.notify();
            },
        ))
    }

    fn direction_button(
        &self,
        label: &'static str,
        direction: FlipReadingDirection,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        Self::option_button(label, self.reading_direction == direction).on_click(cx.listener(
            move |this, _, _, cx| {
                this.reading_direction = this.book.update(cx, |book, cx| {
                    let _ = book.set_reading_direction(direction, cx);
                    book.current_reading_direction()
                });
                cx.notify();
            },
        ))
    }

    fn fit_button(
        &self,
        label: &'static str,
        fit: FlipObjectFit,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        Self::option_button(label, self.object_fit == fit).on_click(cx.listener(
            move |this, _, _, cx| {
                this.object_fit = fit;
                this.book
                    .update(cx, |book, cx| book.set_object_fit(fit, cx));
                cx.notify();
            },
        ))
    }
}

impl Render for Flip16Example {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let status = match self.last_direction {
            Some(FlipDirection::Backward) => "last turn: backward",
            Some(FlipDirection::Forward) => "last turn: forward",
            None => "drag an outer edge to begin",
        };
        let visible_count = if self.layout == FlipLayout::Single {
            1
        } else {
            2
        };
        let slot_count = self.book.read(cx).slot_count();

        div()
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(Self::key_down))
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_3()
            .p_5()
            .bg(rgb(0x101318))
            .text_color(rgb(0xe8e4dc))
            .child(
                div()
                    .w(px(1120.))
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(div().text_xl().child("FlipEntry::DoublePage shared spread"))
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(0xaeb7c3))
                            .child(format!(
                                "Logical slots {}–{} / {slot_count} · {status} · {}",
                                self.position + 1,
                                self.position + visible_count,
                                self.preload_status,
                            )),
                    ),
            )
            .child(
                div()
                    .flex()
                    .gap_5()
                    .child(
                        div()
                            .flex()
                            .gap_1()
                            .child(self.style_button("Rigid", FlipStyle::Rigid, cx))
                            .child(self.style_button("Natural", FlipStyle::Natural, cx))
                            .child(self.style_button("Soft", FlipStyle::Soft, cx)),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_1()
                            .child(self.layout_button("Single", FlipLayout::Single, cx))
                            .child(self.layout_button("Spread", FlipLayout::Spread, cx)),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_1()
                            .child(self.direction_button(
                                "LTR",
                                FlipReadingDirection::LeftToRight,
                                cx,
                            ))
                            .child(self.direction_button(
                                "RTL",
                                FlipReadingDirection::RightToLeft,
                                cx,
                            )),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_1()
                            .child(self.fit_button("Contain", FlipObjectFit::Contain, cx))
                            .child(self.fit_button("Cover", FlipObjectFit::Cover, cx))
                            .child(self.fit_button("Fill", FlipObjectFit::Fill, cx)),
                    ),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(rgb(0x91a5bd))
                    .child("The horizontal SVG is one full item in Single and one aligned two-page spread in Spread. Press ← / → / Space or drag an edge."),
            )
            .child(
                div()
                    .w(px(1120.))
                    .h(px(700.))
                    .shadow_xl()
                    .child(self.book.clone()),
            )
    }
}

fn main() {
    application().run(|cx: &mut App| {
        cx.open_window(WindowOptions::default(), |window, cx| {
            let view = cx.new(Flip16Example::new);
            let focus_handle = view.read(cx).focus_handle.clone();
            focus_handle.focus(window, cx);
            view
        })
        .expect("failed to open flip sequence example");
    });
}
