//! Parent-anchored menus and process-local drag-and-drop between native windows.
//! F2 opens a passive popup; Command-Q exits.
use gpui::{popup::*, prelude::*, *};
use gpui_platform::application;

actions!(native_windows, [Quit, OpenPopup]);

struct Preview;
impl Render for Preview {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .p_4()
            .rounded_md()
            .bg(rgb(0x3878da))
            .text_color(white())
            .child("GPUI drag")
    }
}
struct Popup;
impl Render for Popup {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .p_4()
            .bg(rgb(0xe4edff))
            .text_color(black())
            .child("Anchored popup")
    }
}
struct NativeWindow {
    status: String,
}
impl Render for NativeWindow {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let on_drag_end = cx.listener(|this, event: &DragEnd, _, cx| {
            this.status = format!("Drag ended: {event:?}");
            cx.notify();
        });
        div()
            .size_full()
            .p_6()
            .flex()
            .flex_col()
            .gap_6()
            .bg(rgb(0x18202c))
            .text_color(white())
            .child(
                div()
                    .id("popup")
                    .p_4()
                    .bg(rgb(0x345078))
                    .child("Open anchored menu")
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, event: &MouseDownEvent, window, cx| {
                            let result = cx.open_window(
                                WindowOptions {
                                    kind: WindowKind::AnchoredPopup(PopupOptions {
                                        parent: window.window_handle(),
                                        anchor_rect: Bounds::new(
                                            event.position,
                                            size(px(1.), px(1.)),
                                        ),
                                        anchor: PopupAnchor::BottomLeft,
                                        gravity: PopupGravity::BottomRight,
                                        constraint_adjustment: PopupConstraintAdjustment::FLIP_X
                                            | PopupConstraintAdjustment::FLIP_Y
                                            | PopupConstraintAdjustment::SLIDE_X
                                            | PopupConstraintAdjustment::SLIDE_Y,
                                        offset: point(px(0.), px(0.)),
                                        grab: true,
                                    }),
                                    titlebar: None,
                                    window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                                        point(px(0.), px(0.)),
                                        size(px(300.), px(100.)),
                                    ))),
                                    ..Default::default()
                                },
                                |_, cx| cx.new(|_| Popup),
                            );
                            this.status = match result {
                                Ok(_) => "Menu opened".into(),
                                Err(error) => format!("Menu: {error:#}"),
                            };
                            cx.notify();
                        }),
                    ),
            )
            .child(
                div()
                    .id("drag")
                    .p_4()
                    .bg(rgb(0x3878da))
                    .child("Drag to the other window")
                    .on_drag(7u32, |_, _, _, cx| cx.new(|_| Preview))
                    .on_drag_move::<u32>(|_, window, cx| {
                        if let Err(error) = window.promote_active_drag_to_system(cx) {
                            eprintln!("drag: {error:#}");
                        }
                    })
                    .on_drag_end::<u32>(move |event, _, window, cx| on_drag_end(event, window, cx)),
            )
            .child(
                div()
                    .id("drop")
                    .h_32()
                    .p_4()
                    .border_2()
                    .border_color(rgb(0x7ed9b2))
                    .child("Drop here")
                    .on_drop(cx.listener(|this, _: &u32, _, cx| {
                        this.status = "Drop accepted".into();
                        cx.notify();
                    })),
            )
            .child(self.status.clone())
    }
}
fn main() {
    application().run(|cx| {
        cx.bind_keys([
            KeyBinding::new("cmd-q", Quit, None),
            KeyBinding::new("f2", OpenPopup, None),
        ]);
        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.on_action(|_: &OpenPopup, cx| {
            let Some(parent) = cx.active_window() else {
                return;
            };
            let result = cx.open_window(
                WindowOptions {
                    kind: WindowKind::AnchoredPopup(PopupOptions {
                        parent,
                        anchor_rect: Bounds::new(point(px(40.), px(40.)), size(px(160.), px(40.))),
                        anchor: PopupAnchor::BottomLeft,
                        gravity: PopupGravity::BottomRight,
                        constraint_adjustment: PopupConstraintAdjustment::SLIDE_X
                            | PopupConstraintAdjustment::SLIDE_Y,
                        offset: point(px(0.), px(0.)),
                        grab: false,
                    }),
                    titlebar: None,
                    window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                        point(px(0.), px(0.)),
                        size(px(300.), px(100.)),
                    ))),
                    ..Default::default()
                },
                |_, cx| cx.new(|_| Popup),
            );
            if let Err(error) = result {
                eprintln!("popup: {error:#}");
            }
        });
        for (x, title) in [(80., "GPUI native A"), (560., "GPUI native B")] {
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                        point(px(x), px(120.)),
                        size(px(440.), px(450.)),
                    ))),
                    titlebar: Some(TitlebarOptions {
                        title: Some(title.into()),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                |_, cx| {
                    cx.new(|_| NativeWindow {
                        status: "Ready".into(),
                    })
                },
            )
            .unwrap();
        }
        cx.activate(true);
    });
}
