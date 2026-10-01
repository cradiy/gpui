use gpui::{
    App, Bounds, Context, CursorStyle, Render, Subscription, Window, WindowBounds, WindowOptions,
    div, linear_color_stop, multi_linear_gradient, prelude::*, px, rgb, rgba, size,
};
use gpui_platform::application;
use uic::components::{
    progress::{Progress, ProgressAppearance},
    slider::{
        Slider, SliderAppearance, SliderEvent, SliderState, SliderThumbVisibility,
        SliderTooltipPlacement,
    },
};

struct RangeControlsExample {
    volume: gpui::Entity<SliderState>,
    timeline: gpui::Entity<SliderState>,
    last_event: String,
    _subscriptions: Vec<Subscription>,
}

impl RangeControlsExample {
    fn new(cx: &mut Context<Self>) -> Self {
        let volume = cx.new(|cx| SliderState::new(0.62, 0.0..=1.0, cx).step(0.01));
        let timeline = cx.new(|cx| SliderState::new(84.0, 0.0..=240.0, cx).step(1.0));
        let volume_subscription = cx.subscribe(&volume, |this, _, event, cx| {
            this.last_event = event_label("Volume", event);
            cx.notify();
        });
        let timeline_subscription = cx.subscribe(&timeline, |this, _, event, cx| {
            this.last_event = event_label("Timeline", event);
            cx.notify();
        });
        Self {
            volume,
            timeline,
            last_event: "Drag a slider or use the arrow keys".into(),
            _subscriptions: vec![volume_subscription, timeline_subscription],
        }
    }
}

impl Render for RangeControlsExample {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let volume = self.volume.read(cx).value();
        let timeline = self.timeline.read(cx).value();
        let accent = multi_linear_gradient(
            90.0,
            [
                linear_color_stop(rgb(0x38bdf8), 0.0),
                linear_color_stop(rgb(0x818cf8), 0.52),
                linear_color_stop(rgb(0xf472b6), 1.0),
            ],
        );
        let progress_appearance = ProgressAppearance::default().fill(accent.clone());
        let volume_slider = SliderAppearance::default().h(px(4.)).rounded_full();
        let custom_slider = SliderAppearance::default()
            .active_track(accent)
            .secondary_track(rgba(0x38bdf840).into())
            .thumb(rgba(0x101827ff).into())
            .thumb_border(rgba(0x7dd3fcff).into())
            .thumb_size(px(22.))
            .h(px(8.))
            .rounded_full()
            .bg(rgba(0xffffff14));

        div()
            .size_full()
            .bg(rgb(0x080d18))
            .text_color(rgb(0xe8eef9))
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .w(px(760.))
                    .p(px(34.))
                    .rounded(px(24.))
                    .border_1()
                    .border_color(rgba(0xffffff1c))
                    .bg(rgba(0x111827f2))
                    .shadow_xl()
                    .flex()
                    .flex_col()
                    .gap(px(26.))
                    .child(
                        div()
                            .text_2xl()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child("Range controls"),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgba(0xe8eef980))
                            .child("Progress is read-only. Slider owns interaction and commits."),
                    )
                    .child(section(
                        "Determinate progress",
                        div()
                            .flex()
                            .flex_col()
                            .gap_3()
                            .child(
                                Progress::new("download-progress", 0.58)
                                    .label("Download progress")
                                    .secondary_value(0.78)
                                    .appearance(progress_appearance.clone())
                                    .h(px(10.))
                                    .bg(rgba(0xffffff12)),
                            )
                            .child(
                                Progress::indeterminate("loading-progress")
                                    .label("Background task")
                                    .appearance(progress_appearance.clone())
                                    .h(px(5.))
                                    .bg(rgba(0xffffff12)),
                            ),
                    ))
                    .child(section(
                        "Volume",
                        div()
                            .flex()
                            .items_center()
                            .gap_4()
                            .child(
                                Slider::new(&self.volume)
                                    .label("Volume")
                                    .appearance(volume_slider)
                                    .thumb_visibility(SliderThumbVisibility::Hover)
                                    .hover_track_height(px(8.))
                                    .value_tooltip(SliderTooltipPlacement::Top, |value, _, _| {
                                        div()
                                            .px_2()
                                            .py_1()
                                            .rounded_md()
                                            .bg(rgba(0x020617f2))
                                            .shadow_lg()
                                            .text_xs()
                                            .child(format!("{:.0}%", value * 100.))
                                    })
                                    .w_full()
                                    .rounded_lg(),
                            )
                            .child(
                                div()
                                    .w(px(52.))
                                    .text_right()
                                    .child(format!("{:.0}%", volume * 100.0)),
                            ),
                    ))
                    .child(section(
                        "Custom media timeline",
                        div()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(
                                Slider::new(&self.timeline)
                                    .label("Playback position")
                                    .cursor(CursorStyle::PointingHand)
                                    .secondary_value(156.0)
                                    .appearance(custom_slider)
                                    .active_content(div().size_full().bg(rgba(0xffffff18)))
                                    .thumb_content(
                                        div().size(px(7.)).rounded_full().bg(rgb(0xe0f2fe)),
                                    ),
                            )
                            .child(
                                div()
                                    .flex()
                                    .justify_between()
                                    .text_xs()
                                    .text_color(rgba(0xe8eef970))
                                    .child(format_time(timeline))
                                    .child("4:00"),
                            ),
                    ))
                    .child(
                        div()
                            .pt_2()
                            .text_sm()
                            .text_color(rgb(0x7dd3fc))
                            .child(self.last_event.clone()),
                    ),
            )
    }
}

fn section(title: &'static str, content: impl IntoElement) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_3()
        .child(
            div()
                .text_sm()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .child(title),
        )
        .child(content)
}

fn event_label(name: &str, event: &SliderEvent) -> String {
    match event {
        SliderEvent::Changing(value) => format!("{name} changing: {value:.2}"),
        SliderEvent::Changed(value) => format!("{name} changed: {value:.2}"),
    }
}

fn format_time(seconds: f64) -> String {
    format!("{}:{:02}", seconds as u64 / 60, seconds as u64 % 60)
}

fn main() {
    application().run(|cx: &mut App| {
        uic::init(cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(1080.), px(760.)),
                    cx,
                ))),
                ..Default::default()
            },
            |_, cx| cx.new(RangeControlsExample::new),
        )
        .expect("failed to open range controls example");
    });
}
