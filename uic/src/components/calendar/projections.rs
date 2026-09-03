use std::sync::Arc;
use std::time::Duration;

use chrono::{FixedOffset, NaiveDate, Weekday};
use gpui::{
    App, ElementId, Entity, IntoElement, RenderOnce, SharedString, StyleRefinement, Styled, Window,
};

use super::{
    CalendarAppearance, CalendarClockRange, CalendarEvent, CalendarLocale, CalendarMergePolicy,
    CalendarSegmentLabelPolicy, CalendarState, CalendarTimeSelectionMergePolicy, CalendarView,
    DateRange, DayRenderContext, EventRenderContext, calendar::CalendarCore,
};

macro_rules! calendar_projection {
    ($name:ident, $view:expr, $summary:literal) => {
        #[doc = $summary]
        #[derive(IntoElement)]
        pub struct $name<T: 'static = ()> {
            inner: CalendarCore<T>,
        }

        impl<T: 'static> $name<T> {
            pub fn new(
                id: impl Into<ElementId>,
                state: &Entity<CalendarState>,
                events: impl Into<Arc<[CalendarEvent<T>]>>,
            ) -> Self {
                Self {
                    inner: CalendarCore::new(id, state, events, $view),
                }
            }

            pub fn locale(mut self, locale: CalendarLocale) -> Self {
                self.inner = self.inner.locale(locale);
                self
            }

            pub fn appearance(mut self, appearance: CalendarAppearance) -> Self {
                self.inner = self.inner.appearance(appearance);
                self
            }

            pub fn first_weekday(mut self, weekday: Weekday) -> Self {
                self.inner = self.inner.first_weekday(weekday);
                self
            }

            pub fn display_offset(mut self, offset: FixedOffset) -> Self {
                self.inner = self.inner.display_offset(offset);
                self
            }

            pub fn today(mut self, today: NaiveDate) -> Self {
                self.inner = self.inner.today(today);
                self
            }

            pub fn visible_range(&self, cx: &App) -> DateRange {
                self.inner.visible_range(cx)
            }
        }

        impl<T: 'static> RenderOnce for $name<T> {
            fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
                self.inner
            }
        }

        impl<T: 'static> Styled for $name<T> {
            fn style(&mut self) -> &mut StyleRefinement {
                self.inner.style()
            }
        }
    };
}

macro_rules! event_projection_options {
    ($name:ident) => {
        impl<T: 'static> $name<T> {
            pub fn merge_policy(mut self, policy: CalendarMergePolicy) -> Self {
                self.inner = self.inner.merge_policy(policy);
                self
            }

            pub fn merge_key(
                mut self,
                key: impl Fn(&CalendarEvent<T>) -> Option<SharedString> + 'static,
            ) -> Self {
                self.inner = self.inner.merge_key(key);
                self
            }

            pub fn segment_label_policy(mut self, policy: CalendarSegmentLabelPolicy) -> Self {
                self.inner = self.inner.segment_label_policy(policy);
                self
            }

            pub fn day_content<E: IntoElement>(
                mut self,
                renderer: impl Fn(DayRenderContext) -> E + 'static,
            ) -> Self {
                self.inner = self.inner.day_content(renderer);
                self
            }

            pub fn event_content<E: IntoElement>(
                mut self,
                renderer: impl Fn(EventRenderContext<T>) -> E + 'static,
            ) -> Self {
                self.inner = self.inner.event_content(renderer);
                self
            }

            pub fn event_style(
                mut self,
                styler: impl Fn(&EventRenderContext<T>) -> StyleRefinement + 'static,
            ) -> Self {
                self.inner = self.inner.event_style(styler);
                self
            }
        }
    };
}

calendar_projection!(
    YearCalendar,
    CalendarView::Year,
    "A lightweight year projection using one paint layer per month."
);

macro_rules! timeline_options {
    ($name:ident) => {
        impl<T: 'static> $name<T> {
            pub fn time_range(mut self, start_hour: u32, end_hour: u32) -> Self {
                self.inner = self.inner.time_range(start_hour, end_hour);
                self
            }

            /// Sets time-selection snapping without changing the one-hour grid.
            /// The precision must divide one hour.
            pub fn time_selection_precision_minutes(mut self, minutes: u32) -> Self {
                self.inner = self.inner.time_selection_precision_minutes(minutes);
                self
            }

            /// Limits how many precision-sized slots one continuous range may
            /// contain. `None` leaves each range unbounded.
            pub fn time_selection_max_slots_per_range(mut self, maximum: Option<usize>) -> Self {
                self.inner = self.inner.time_selection_max_slots_per_range(maximum);
                self
            }

            /// Controls whether separately activated adjacent slots stay
            /// independent or merge into one range.
            pub fn time_selection_merge_policy(
                mut self,
                policy: CalendarTimeSelectionMergePolicy,
            ) -> Self {
                self.inner = self.inner.time_selection_merge_policy(policy);
                self
            }

            /// Sets how long a pointer must remain pressed to trigger selection.
            pub fn time_selection_long_press_delay(mut self, delay: Duration) -> Self {
                self.inner = self.inner.time_selection_long_press_delay(delay);
                self
            }

            /// Compresses recurring wall-clock ranges into compact timeline rows.
            pub fn collapsed_time_ranges(
                mut self,
                ranges: impl Into<Arc<[CalendarClockRange]>>,
            ) -> Self {
                self.inner = self.inner.collapsed_time_ranges(ranges);
                self
            }
        }
    };
}

calendar_projection!(
    MonthCalendar,
    CalendarView::Month,
    "A standalone month grid with all-day event lanes."
);
calendar_projection!(
    WeekCalendar,
    CalendarView::Week,
    "A standalone seven-day timeline."
);
calendar_projection!(
    DayCalendar,
    CalendarView::Day,
    "A standalone single-day timeline."
);

event_projection_options!(MonthCalendar);
event_projection_options!(WeekCalendar);
event_projection_options!(DayCalendar);

impl<T: 'static> MonthCalendar<T> {
    pub fn show_outside_days(mut self, show: bool) -> Self {
        self.inner = self.inner.show_outside_days(show);
        self
    }

    pub fn max_event_lanes(mut self, lanes: usize) -> Self {
        self.inner = self.inner.max_event_lanes(lanes);
        self
    }
}

timeline_options!(WeekCalendar);
timeline_options!(DayCalendar);
