use gpui::{
    App, AppContext, Bounds, Context, Entity, IntoElement, Render, SharedString, Subscription,
    Window, WindowBounds, WindowOptions, div, prelude::*, px, rgb, rgba, size,
};
use uic::components::{
    color_picker::{AlphaSlider, ColorPicker, ColorPickerState},
    screen_color_picker::{ScreenColorPicker, ScreenColorPickerEvent, ScreenColorPickerState},
};

struct Demo {
    screen: Entity<ScreenColorPickerState>,
    color: Entity<ColorPickerState>,
    status: SharedString,
    _subscriptions: Vec<Subscription>,
}
impl Demo {
    fn new(cx: &mut Context<Self>) -> Self {
        let screen = cx.new(ScreenColorPickerState::new);
        let color = cx.new(|cx| ColorPickerState::new(rgba(0x3984dbcc), cx));
        let observer = cx.observe(&screen, |_, _, cx| cx.notify());
        let color_observer = cx.observe(&color, |_, _, cx| cx.notify());
        let events = cx.subscribe(&screen, |this, _, event: &ScreenColorPickerEvent, cx| {
            match event {
                ScreenColorPickerEvent::Picked(sample) => {
                    let mut sample = *sample;
                    sample.a = this.color.read(cx).value().a;
                    this.color
                        .update(cx, |color, cx| color.set_value(sample, cx));
                    this.status = "Color selected".into();
                }
                ScreenColorPickerEvent::Cancelled => this.status = "Cancelled".into(),
                ScreenColorPickerEvent::Failed(error) => this.status = error.clone(),
            }
            cx.notify();
        });
        Self {
            screen,
            color,
            status: "Pick a color anywhere on the screen".into(),
            _subscriptions: vec![observer, color_observer, events],
        }
    }
}
impl Render for Demo {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let color = self.color.read(cx).value();
        div()
            .size_full()
            .p_6()
            .flex()
            .flex_col()
            .gap_4()
            .bg(rgb(0x202124))
            .text_color(rgb(0xf0f0f0))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(div().text_size(px(22.)).child("Screen color"))
                    .child(ScreenColorPicker::new(&self.screen)),
            )
            .child(div().text_size(px(13.)).child(self.status.clone()))
            .child(ColorPicker::new(&self.color))
            .child(AlphaSlider::new(&self.color))
            .child(div().h(px(60.)).rounded_md().bg(color))
            .child(format!(
                "#{:02X}{:02X}{:02X} · {:.0}%",
                (color.r * 255.).round() as u8,
                (color.g * 255.).round() as u8,
                (color.b * 255.).round() as u8,
                color.a * 100.
            ))
    }
}
fn main() {
    gpui_platform::application()
        .with_assets(uic::assets::LucideAssets::new())
        .run(|cx: &mut App| {
            uic::init(cx);
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                        None,
                        size(px(440.), px(580.)),
                        cx,
                    ))),
                    ..Default::default()
                },
                |_, cx| cx.new(Demo::new),
            )
            .expect("screen color picker window");
        });
}
