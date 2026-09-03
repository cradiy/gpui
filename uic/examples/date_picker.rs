use chrono::{Datelike as _, NaiveDate, Weekday};
use gpui::{
    App, Bounds, Context, Entity, FontWeight, Render, Subscription, Window, WindowBounds,
    WindowOptions, div, prelude::*, px, rgb, size,
};
use gpui_platform::application;
use uic::assets::LucideAssets;
use uic::components::calendar::{
    CalendarDatePicker, CalendarSelection, CalendarSelectionMode, CalendarState, CalendarView,
    DatePickerDayLabel,
};

struct DatePickerExample {
    calendar: Entity<CalendarState>,
    mode: CalendarSelectionMode,
    _subscription: Subscription,
}

impl DatePickerExample {
    fn new(cx: &mut Context<Self>) -> Self {
        let calendar = cx.new(|_| CalendarState::new(date(12), CalendarView::Month));
        calendar.update(cx, |state, cx| {
            state.set_selection(
                CalendarSelection::Range {
                    start: Some(date(8)),
                    end: Some(date(14)),
                },
                cx,
            );
        });
        let subscription = cx.subscribe(&calendar, |_, _, _, cx| cx.notify());
        Self {
            calendar,
            mode: CalendarSelectionMode::Range,
            _subscription: subscription,
        }
    }
}

impl Render for DatePickerExample {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.entity();
        let selection = selection_text(self.calendar.read(cx).selection());

        div()
            .size_full()
            .bg(rgb(0xf1f5f9))
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .w(px(376.))
                    .p_6()
                    .flex()
                    .flex_col()
                    .gap_5()
                    .rounded(px(22.))
                    .border_1()
                    .border_color(rgb(0xe2e8f0))
                    .bg(rgb(0xffffff))
                    .shadow_lg()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap_1()
                                    .child(
                                        div()
                                            .text_lg()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child("Choose dates"),
                                    )
                                    .child(
                                        div().text_xs().text_color(rgb(0x718096)).child(selection),
                                    ),
                            )
                            .child(mode_switcher(self.mode, entity)),
                    )
                    .child(
                        CalendarDatePicker::new("date-picker", &self.calendar)
                            .selection_mode(self.mode)
                            .today(date(12))
                            .day_label(|date| {
                                if date.month() == 9 && (15..=17).contains(&date.day()) {
                                    Some(
                                        DatePickerDayLabel::new("Holiday")
                                            .color(rgb(0xe5484d).into()),
                                    )
                                } else if matches!(date.weekday(), Weekday::Sat | Weekday::Sun) {
                                    Some(DatePickerDayLabel::new("Weekend"))
                                } else {
                                    None
                                }
                            })
                            .w_full()
                            .border_0()
                            .rounded_none()
                            .shadow_none()
                            .p_0(),
                    ),
            )
    }
}

fn mode_switcher(
    mode: CalendarSelectionMode,
    entity: Entity<DatePickerExample>,
) -> impl IntoElement {
    let mut switcher = div().p(px(3.)).flex().rounded(px(10.)).bg(rgb(0xf1f4f8));
    for (index, (candidate, label)) in [
        (CalendarSelectionMode::Single, "Single"),
        (CalendarSelectionMode::Range, "Range"),
    ]
    .into_iter()
    .enumerate()
    {
        let active = candidate == mode;
        let entity = entity.clone();
        switcher = switcher.child(
            div()
                .id(("date-picker-mode", index))
                .px_3()
                .h(px(28.))
                .flex()
                .items_center()
                .rounded(px(7.))
                .text_xs()
                .font_weight(if active {
                    FontWeight::SEMIBOLD
                } else {
                    FontWeight::NORMAL
                })
                .text_color(if active { rgb(0x4263eb) } else { rgb(0x718096) })
                .when(active, |button| button.bg(rgb(0xffffff)).shadow_sm())
                .cursor_pointer()
                .on_click(move |_, _, cx| {
                    entity.update(cx, |this, cx| {
                        this.mode = candidate;
                        this.calendar.update(cx, |state, cx| {
                            state.set_selection(CalendarSelection::None, cx);
                        });
                        cx.notify();
                    });
                })
                .child(label),
        );
    }
    switcher
}

fn selection_text(selection: &CalendarSelection) -> String {
    match selection {
        CalendarSelection::None
        | CalendarSelection::Single(None)
        | CalendarSelection::Range { start: None, .. } => "No date selected".to_string(),
        CalendarSelection::Single(Some(date)) => date.to_string(),
        CalendarSelection::Range {
            start: Some(start),
            end: None,
        } => format!("Start: {start}"),
        CalendarSelection::Range {
            start: Some(start),
            end: Some(end),
        } => format!("{start} – {end}"),
    }
}

fn date(day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 9, day).unwrap()
}

fn main() {
    application()
        .with_assets(LucideAssets::new())
        .run(|cx: &mut App| {
            uic::init(cx);
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                        None,
                        size(px(720.), px(680.)),
                        cx,
                    ))),
                    ..Default::default()
                },
                |_, cx| cx.new(DatePickerExample::new),
            )
            .expect("date picker example window");
        });
}
