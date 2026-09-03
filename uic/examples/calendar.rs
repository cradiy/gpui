use std::sync::Arc;

use chrono::{NaiveDate, TimeZone as _, Utc};
use gpui::{
    AnyElement, App, Bounds, Context, Entity, FontWeight, Render, Subscription, Window,
    WindowBounds, WindowOptions, div, prelude::*, px, rgb, rgba, size,
};
use gpui_platform::application;
use uic::components::calendar::{
    CalendarClockRange, CalendarEvent, CalendarEventMerge, CalendarMergePolicy, CalendarPager,
    CalendarState, CalendarTimeSelectionMergePolicy, CalendarTitle, CalendarTodayButton,
    CalendarView, CalendarViewSwitcher, DayCalendar, MonthCalendar, WeekCalendar, YearCalendar,
};

struct CalendarExample {
    calendar: Entity<CalendarState>,
    events: Arc<[CalendarEvent<&'static str>]>,
    merge_adjacent: bool,
    _subscription: Subscription,
}

impl CalendarExample {
    fn new(cx: &mut Context<Self>) -> Self {
        let anchor = date(9);
        let calendar = cx.new(|_| CalendarState::new(anchor, CalendarView::Month));
        let subscription = cx.subscribe(&calendar, |_, _, _, cx| cx.notify());
        Self {
            calendar,
            events: sample_events(),
            merge_adjacent: true,
            _subscription: subscription,
        }
    }
}

impl Render for CalendarExample {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.entity();
        let merge_label = if self.merge_adjacent {
            "Adjacent merge · On"
        } else {
            "Adjacent merge · Off"
        };
        let policy = if self.merge_adjacent {
            CalendarMergePolicy::AdjacentByKey
        } else {
            CalendarMergePolicy::ExplicitOnly
        };
        let calendar_view: AnyElement = match self.calendar.read(cx).view() {
            CalendarView::Year => {
                YearCalendar::new("showcase-year", &self.calendar, self.events.clone())
                    .today(date(9))
                    .flex_1()
                    .min_h_0()
                    .border_0()
                    .rounded_none()
                    .into_any_element()
            }
            CalendarView::Month => {
                MonthCalendar::new("showcase-month", &self.calendar, self.events.clone())
                    .today(date(9))
                    .merge_policy(policy)
                    .flex_1()
                    .min_h_0()
                    .border_0()
                    .rounded_none()
                    .into_any_element()
            }
            CalendarView::Week => {
                WeekCalendar::new("showcase-week", &self.calendar, self.events.clone())
                    .today(date(9))
                    .merge_policy(policy)
                    .time_range(0, 24)
                    .time_selection_precision_minutes(15)
                    .time_selection_max_slots_per_range(Some(4))
                    .time_selection_merge_policy(CalendarTimeSelectionMergePolicy::SeparateAdjacent)
                    .collapsed_time_ranges(Arc::from([CalendarClockRange::hours(0, 6)]))
                    .flex_1()
                    .min_h_0()
                    .border_0()
                    .rounded_none()
                    .into_any_element()
            }
            CalendarView::Day => {
                DayCalendar::new("showcase-day", &self.calendar, self.events.clone())
                    .today(date(9))
                    .merge_policy(policy)
                    .time_range(0, 24)
                    .time_selection_precision_minutes(15)
                    .time_selection_max_slots_per_range(Some(4))
                    .time_selection_merge_policy(CalendarTimeSelectionMergePolicy::SeparateAdjacent)
                    .collapsed_time_ranges(Arc::from([CalendarClockRange::hours(0, 6)]))
                    .flex_1()
                    .min_h_0()
                    .border_0()
                    .rounded_none()
                    .into_any_element()
            }
        };

        div()
            .size_full()
            .bg(rgb(0xf1f5f9))
            .text_color(rgb(0x0f172a))
            .p_8()
            .flex()
            .flex_col()
            .gap_5()
            .child(
                div()
                    .flex()
                    .items_end()
                    .justify_between()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .text_2xl()
                                    .font_weight(FontWeight::BOLD)
                                    .child("Event calendar"),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(rgb(0x64748b))
                                    .child("Year, month, week, and day share one event model."),
                            ),
                    )
                    .child(
                        div()
                            .id("toggle-calendar-merge")
                            .px_3()
                            .h(px(34.))
                            .flex()
                            .items_center()
                            .rounded(px(10.))
                            .border_1()
                            .border_color(rgb(0xcbd5e1))
                            .bg(rgb(0xffffff))
                            .text_sm()
                            .cursor_pointer()
                            .hover(|style| style.border_color(rgb(0x2563eb)))
                            .on_click(move |_, _, cx| {
                                entity.update(cx, |this, cx| {
                                    this.merge_adjacent = !this.merge_adjacent;
                                    cx.notify();
                                });
                            })
                            .child(merge_label),
                    ),
            )
            .child(
                div()
                    .w_full()
                    .h(px(700.))
                    .flex()
                    .flex_col()
                    .rounded(px(18.))
                    .border_1()
                    .border_color(rgb(0xe2e8f0))
                    .bg(rgb(0xffffff))
                    .overflow_hidden()
                    .shadow_lg()
                    .child(
                        div()
                            .h(px(72.))
                            .flex_none()
                            .px_5()
                            .flex()
                            .items_center()
                            .justify_between()
                            .border_b_1()
                            .border_color(rgb(0xe2e8f0))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_3()
                                    .child(CalendarPager::new(
                                        "showcase-calendar-pager",
                                        &self.calendar,
                                    ))
                                    .child(CalendarTitle::new(&self.calendar)),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_3()
                                    .child(
                                        CalendarTodayButton::new(
                                            "showcase-calendar-today",
                                            &self.calendar,
                                        )
                                        .today(date(9)),
                                    )
                                    .child(CalendarViewSwitcher::new(
                                        "showcase-calendar-views",
                                        &self.calendar,
                                    )),
                            ),
                    )
                    .child(calendar_view),
            )
    }
}

fn sample_events() -> Arc<[CalendarEvent<&'static str>]> {
    let events = vec![
        CalendarEvent::all_day_range(
            "design-week",
            "Design systems week",
            date(7),
            date(12),
            "focus",
        )
        .color(rgba(0x7c3aedff).into()),
        CalendarEvent::all_day("holiday-1", "Autumn holiday", date(14), "holiday")
            .color(rgba(0xf97316ff).into())
            .merge(CalendarEventMerge::Key("autumn-holiday".into())),
        CalendarEvent::all_day("holiday-2", "Autumn holiday", date(15), "holiday")
            .color(rgba(0xf97316ff).into())
            .merge(CalendarEventMerge::Key("autumn-holiday".into())),
        CalendarEvent::all_day("holiday-3", "Autumn holiday", date(16), "holiday")
            .color(rgba(0xf97316ff).into())
            .merge(CalendarEventMerge::Key("autumn-holiday".into())),
        CalendarEvent::all_day_range("release", "Release window", date(24), date(29), "release")
            .color(rgba(0x2563ebff).into()),
        CalendarEvent::timed(
            "planning",
            "Weekly planning",
            utc(9, 9, 30),
            utc(9, 10, 45),
            "meeting",
        )
        .color(rgba(0x0284c7ff).into()),
        CalendarEvent::timed(
            "review",
            "Product review",
            utc(9, 10, 0),
            utc(9, 11, 30),
            "meeting",
        )
        .color(rgba(0x0284c7ff).into()),
        CalendarEvent::timed(
            "deep-work",
            "Focus time",
            utc(10, 13, 0),
            utc(10, 16, 0),
            "focus",
        )
        .color(rgba(0x7c3aedff).into()),
    ];
    events.into()
}

fn date(day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 9, day).unwrap()
}

fn utc(day: u32, hour: u32, minute: u32) -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 9, day, hour, minute, 0)
        .single()
        .unwrap()
}

fn main() {
    application().run(|cx: &mut App| {
        uic::init(cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(1280.), px(860.)),
                    cx,
                ))),
                ..Default::default()
            },
            |_, cx| cx.new(CalendarExample::new),
        )
        .expect("failed to open calendar example");
    });
}
