use std::{rc::Rc, sync::Arc, time::Duration as StdDuration};

use chrono::{
    Datelike as _, Duration, FixedOffset, NaiveDate, TimeZone as _, Timelike as _, Utc, Weekday,
};
use gpui::{
    AnyElement, App, Bounds, ElementId, Entity, FontWeight, Hsla, IntoElement, RenderOnce, Role,
    SharedString, StyleRefinement, Styled, TextAlign, TextRun, Window, canvas, div, point,
    prelude::*, px, quad, relative, rgb, size,
};

use super::{
    CalendarEvent, CalendarEventTime, CalendarLocale, CalendarMergePolicy,
    CalendarSegmentLabelPolicy, CalendarSelection, CalendarState, CalendarTimeSelectionMergePolicy,
    CalendarView, DateRange, YearMonth,
    date::{month_grid_range, week_range},
    layout::{EventGroup, EventSegment, MergeKeyFn, layout_month, resolve_event_groups},
    time::{CalendarClockRange, CalendarTimeSelection, TimeAxis},
    timeline_interaction::{LongPressPhase, TimelineInteraction},
};

type DayRenderer = Rc<dyn Fn(DayRenderContext) -> AnyElement>;
type EventRenderer<T> = Rc<dyn Fn(EventRenderContext<T>) -> AnyElement>;
type EventStyler<T> = Rc<dyn Fn(&EventRenderContext<T>) -> StyleRefinement>;

struct TimelineState<'a> {
    selected_event: Option<&'a SharedString>,
    date_selection: &'a CalendarSelection,
    time_selections: &'a [CalendarTimeSelection],
    expanded_time_ranges: &'a [CalendarClockRange],
}

#[derive(Clone)]
struct TimelineSelectionMapper {
    axis: TimeAxis,
    dates: Arc<[NaiveDate]>,
    display_offset: FixedOffset,
    precision_minutes: u32,
    max_slots_per_range: Option<usize>,
    last_minute: u32,
}

impl TimelineSelectionMapper {
    fn slot_at(
        &self,
        position: gpui::Point<gpui::Pixels>,
        bounds: Bounds<gpui::Pixels>,
    ) -> Option<CalendarTimeSelection> {
        let width = f32::from(bounds.size.width).max(1.);
        let height = f32::from(bounds.size.height).max(1.);
        let x = f32::from(position.x - bounds.left()).clamp(0., width - 0.001);
        let y = f32::from(position.y - bounds.top()).clamp(0., height - 0.001);
        let day_index = ((x / width) * self.dates.len() as f32).floor() as usize;
        let minute = self.axis.minute_for_y(y)?;
        let start_minute = (minute / self.precision_minutes) * self.precision_minutes;
        if start_minute + self.precision_minutes > self.last_minute {
            return None;
        }
        let date = self.dates[day_index.min(self.dates.len() - 1)];
        let local_start = date
            .and_hms_opt(start_minute / 60, start_minute % 60, 0)
            .expect("selected minute is inside a day");
        let start = self
            .display_offset
            .from_local_datetime(&local_start)
            .single()
            .expect("fixed offsets have one local datetime")
            .with_timezone(&Utc);
        Some(CalendarTimeSelection {
            start,
            end: start + Duration::minutes(i64::from(self.precision_minutes)),
        })
    }

    fn range_between(
        &self,
        origin: gpui::Point<gpui::Pixels>,
        position: gpui::Point<gpui::Pixels>,
        bounds: Bounds<gpui::Pixels>,
    ) -> Option<CalendarTimeSelection> {
        let first = self.slot_at(origin, bounds)?;
        let last = self
            .slot_at(position, bounds)
            .unwrap_or_else(|| first.clone());
        let mut start = first.start.min(last.start);
        let mut end = first.end.max(last.end);
        if let Some(maximum) = self.max_slots_per_range {
            let maximum_minutes = maximum.saturating_mul(self.precision_minutes as usize);
            let maximum = Duration::minutes(i64::try_from(maximum_minutes).unwrap_or(i64::MAX));
            if last.start >= first.start {
                end = end.min(first.start + maximum);
            } else {
                start = start.max(first.end - maximum);
            }
        }
        Some(CalendarTimeSelection { start, end })
    }
}

/// Semantic colors for managed calendar states. Outer layout and surface style
/// remain available through [`Styled`].
#[derive(Clone, Debug, uic_macros::Chainable)]
pub struct CalendarAppearance {
    pub accent: Hsla,
    pub selected_day: Hsla,
    pub selected_day_text: Hsla,
    pub today_ring: Hsla,
    pub hover_day: Hsla,
    pub outside_text: Hsla,
    pub secondary_text: Hsla,
    pub grid_line: Hsla,
    pub event_background: Hsla,
    pub event_text: Hsla,
    pub selected_event_ring: Hsla,
    pub now_line: Hsla,
    pub time_selection_background: Hsla,
}

impl Default for CalendarAppearance {
    fn default() -> Self {
        Self {
            accent: rgb(0x4263eb).into(),
            selected_day: rgb(0x4263eb).into(),
            selected_day_text: rgb(0xffffff).into(),
            today_ring: rgb(0x4263eb).into(),
            hover_day: gpui::rgba(0x4263eb0a).into(),
            outside_text: rgb(0xa7b0c0).into(),
            secondary_text: rgb(0x718096).into(),
            grid_line: rgb(0xe9edf3).into(),
            event_background: rgb(0x4c6ef5).into(),
            event_text: rgb(0x3156d8).into(),
            selected_event_ring: rgb(0x1e293b).into(),
            now_line: rgb(0xf04452).into(),
            time_selection_background: gpui::rgba(0x4263eb24).into(),
        }
    }
}

/// Context for content rendered inside a calendar day cell.
#[derive(Clone)]
pub struct DayRenderContext {
    pub date: NaiveDate,
    pub view: CalendarView,
    pub is_today: bool,
    pub is_selected: bool,
    pub is_outside: bool,
    pub event_count: usize,
}

/// Context for custom event content and styling.
pub struct EventRenderContext<T> {
    pub id: SharedString,
    pub title: SharedString,
    pub events: Arc<[CalendarEvent<T>]>,
    pub event_range: DateRange,
    pub segment_range: DateRange,
    pub visible_range: DateRange,
    pub view: CalendarView,
    pub lane: usize,
    pub continues_before: bool,
    pub continues_after: bool,
    pub is_all_day: bool,
    pub is_selected: bool,
}

impl<T> Clone for EventRenderContext<T> {
    fn clone(&self) -> Self {
        Self {
            id: self.id.clone(),
            title: self.title.clone(),
            events: self.events.clone(),
            event_range: self.event_range,
            segment_range: self.segment_range,
            visible_range: self.visible_range,
            view: self.view,
            lane: self.lane,
            continues_before: self.continues_before,
            continues_after: self.continues_after,
            is_all_day: self.is_all_day,
            is_selected: self.is_selected,
        }
    }
}

#[derive(IntoElement)]
pub(super) struct CalendarCore<T: 'static = ()> {
    id: ElementId,
    state: Entity<CalendarState>,
    events: Arc<[CalendarEvent<T>]>,
    view: CalendarView,
    locale: CalendarLocale,
    appearance: CalendarAppearance,
    merge_policy: CalendarMergePolicy,
    merge_key: Option<MergeKeyFn<T>>,
    label_policy: CalendarSegmentLabelPolicy,
    first_weekday: Option<Weekday>,
    display_offset: FixedOffset,
    today: NaiveDate,
    show_outside_days: bool,
    max_event_lanes: usize,
    day_start_hour: u32,
    day_end_hour: u32,
    time_selection_precision_minutes: u32,
    time_selection_max_slots_per_range: Option<usize>,
    time_selection_merge_policy: CalendarTimeSelectionMergePolicy,
    time_selection_long_press_delay: StdDuration,
    collapsed_time_ranges: Arc<[CalendarClockRange]>,
    day_renderer: Option<DayRenderer>,
    event_renderer: Option<EventRenderer<T>>,
    event_styler: Option<EventStyler<T>>,
    style: StyleRefinement,
}

impl<T: 'static> CalendarCore<T> {
    pub(super) fn new(
        id: impl Into<ElementId>,
        state: &Entity<CalendarState>,
        events: impl Into<Arc<[CalendarEvent<T>]>>,
        view: CalendarView,
    ) -> Self {
        Self {
            id: id.into(),
            state: state.clone(),
            events: events.into(),
            view,
            locale: CalendarLocale::default(),
            appearance: CalendarAppearance::default(),
            merge_policy: CalendarMergePolicy::ExplicitOnly,
            merge_key: None,
            label_policy: CalendarSegmentLabelPolicy::Each,
            first_weekday: None,
            display_offset: FixedOffset::east_opt(0).expect("UTC offset"),
            today: chrono::Local::now().date_naive(),
            show_outside_days: true,
            max_event_lanes: 3,
            day_start_hour: 6,
            day_end_hour: 22,
            time_selection_precision_minutes: 15,
            time_selection_max_slots_per_range: None,
            time_selection_merge_policy: CalendarTimeSelectionMergePolicy::SeparateAdjacent,
            time_selection_long_press_delay: StdDuration::from_millis(350),
            collapsed_time_ranges: Arc::from([]),
            day_renderer: None,
            event_renderer: None,
            event_styler: None,
            style: StyleRefinement::default()
                .min_w(px(680.))
                .min_h(px(620.))
                .rounded(px(16.))
                .border_1()
                .border_color(rgb(0xe4e9f1))
                .bg(rgb(0xffffff))
                .shadow_sm()
                .overflow_hidden(),
        }
    }

    pub(super) fn locale(mut self, locale: CalendarLocale) -> Self {
        self.locale = locale;
        self
    }

    pub(super) fn appearance(mut self, appearance: CalendarAppearance) -> Self {
        self.appearance = appearance;
        self
    }

    pub(super) fn merge_policy(mut self, policy: CalendarMergePolicy) -> Self {
        self.merge_policy = policy;
        self
    }

    pub(super) fn merge_key(
        mut self,
        key: impl Fn(&CalendarEvent<T>) -> Option<SharedString> + 'static,
    ) -> Self {
        self.merge_key = Some(Arc::new(key));
        self
    }

    pub(super) fn segment_label_policy(mut self, policy: CalendarSegmentLabelPolicy) -> Self {
        self.label_policy = policy;
        self
    }

    pub(super) fn first_weekday(mut self, weekday: Weekday) -> Self {
        self.first_weekday = Some(weekday);
        self
    }

    pub(super) fn display_offset(mut self, offset: FixedOffset) -> Self {
        self.display_offset = offset;
        self
    }

    pub(super) fn today(mut self, today: NaiveDate) -> Self {
        self.today = today;
        self
    }

    pub(super) fn show_outside_days(mut self, show: bool) -> Self {
        self.show_outside_days = show;
        self
    }

    pub(super) fn max_event_lanes(mut self, lanes: usize) -> Self {
        self.max_event_lanes = lanes.max(1);
        self
    }

    /// Sets the visible hour range used by week and day views.
    pub(super) fn time_range(mut self, start_hour: u32, end_hour: u32) -> Self {
        assert!(start_hour < end_hour && end_hour <= 24);
        self.day_start_hour = start_hour;
        self.day_end_hour = end_hour;
        self
    }

    pub(super) fn time_selection_precision_minutes(mut self, minutes: u32) -> Self {
        assert!(minutes > 0 && minutes <= 60 && 60 % minutes == 0);
        self.time_selection_precision_minutes = minutes;
        self
    }

    pub(super) fn time_selection_max_slots_per_range(mut self, maximum: Option<usize>) -> Self {
        assert!(maximum.is_none_or(|maximum| maximum > 0));
        self.time_selection_max_slots_per_range = maximum;
        self
    }

    pub(super) fn time_selection_merge_policy(
        mut self,
        policy: CalendarTimeSelectionMergePolicy,
    ) -> Self {
        self.time_selection_merge_policy = policy;
        self
    }

    pub(super) fn time_selection_long_press_delay(mut self, delay: StdDuration) -> Self {
        self.time_selection_long_press_delay = delay;
        self
    }

    pub(super) fn collapsed_time_ranges(
        mut self,
        ranges: impl Into<Arc<[CalendarClockRange]>>,
    ) -> Self {
        self.collapsed_time_ranges = ranges.into();
        self
    }

    /// Replaces only the managed day cell's inner content in month, week, and
    /// day views. Year view deliberately uses a compact paint layer.
    pub(super) fn day_content<E: IntoElement>(
        mut self,
        renderer: impl Fn(DayRenderContext) -> E + 'static,
    ) -> Self {
        self.day_renderer = Some(Rc::new(move |context| renderer(context).into_any_element()));
        self
    }

    /// Replaces only the managed event bar's inner content.
    pub(super) fn event_content<E: IntoElement>(
        mut self,
        renderer: impl Fn(EventRenderContext<T>) -> E + 'static,
    ) -> Self {
        self.event_renderer = Some(Rc::new(move |context| renderer(context).into_any_element()));
        self
    }

    /// Refines managed event wrapper styles without replacing interaction.
    pub(super) fn event_style(
        mut self,
        styler: impl Fn(&EventRenderContext<T>) -> StyleRefinement + 'static,
    ) -> Self {
        self.event_styler = Some(Rc::new(styler));
        self
    }

    pub(super) fn visible_range(&self, cx: &App) -> DateRange {
        let state = self.state.read(cx);
        visible_range(self.view, state.anchor_date(), self.weekday())
    }

    fn weekday(&self) -> Weekday {
        self.first_weekday.unwrap_or(self.locale.first_weekday)
    }

    fn render_month(
        &self,
        anchor: NaiveDate,
        visible: DateRange,
        groups: &[EventGroup<T>],
        selected_event: Option<&SharedString>,
        selection: &CalendarSelection,
    ) -> AnyElement {
        let month = YearMonth::from_date(anchor);
        let weekdays = ordered_weekdays(self.weekday());
        let layout = layout_month(groups, visible, self.max_event_lanes, self.label_policy);

        let weekday_header = div()
            .h(px(42.))
            .grid()
            .grid_cols(7)
            .border_b_1()
            .border_color(self.appearance.grid_line)
            .bg(rgb(0xfafbfc))
            .children(weekdays.into_iter().map(|weekday| {
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(self.appearance.secondary_text)
                    .child(self.locale.weekday_name(weekday, true))
            }));

        let mut weeks = div().flex_1().min_h_0().flex().flex_col();
        for week_index in 0..6 {
            let week_start = visible.start + Duration::days((week_index * 7) as i64);
            let mut week = div()
                .relative()
                .flex_1()
                .min_h(px(96.))
                .grid()
                .grid_cols(7)
                .when(week_index < 5, |element| {
                    element.border_b_1().border_color(self.appearance.grid_line)
                });
            for column in 0..7 {
                let date = week_start + Duration::days(column as i64);
                let event_count = groups
                    .iter()
                    .filter(|group| group.range.contains(date))
                    .count();
                week = week.child(self.render_day_cell(
                    date,
                    CalendarView::Month,
                    date.month() != month.month || date.year() != month.year,
                    event_count,
                    selection,
                    column < 6,
                ));
            }
            for segment in layout.segments.iter().filter(|segment| {
                segment.week_index == week_index && segment.lane < self.max_event_lanes
            }) {
                let left = segment.start_column as f32 / 7.0;
                let width = segment.span_columns as f32 / 7.0;
                week = week.child(
                    div()
                        .absolute()
                        .left(relative(left))
                        .top(px(38. + segment.lane as f32 * 19.))
                        .w(relative(width))
                        .h(px(18.))
                        .px(px(3.))
                        .child(self.render_segment(
                            segment,
                            CalendarView::Month,
                            visible,
                            selected_event,
                        )),
                );
            }
            for (date, count) in layout
                .overflow
                .iter()
                .filter(|(date, _)| week_start <= **date && **date < week_start + Duration::days(7))
            {
                let column = (*date - week_start).num_days() as f32;
                week = week.child(
                    div()
                        .absolute()
                        .left(relative(column / 7.0))
                        .top(px(38. + self.max_event_lanes as f32 * 19.))
                        .w(relative(1.0 / 7.0))
                        .px_3()
                        .text_xs()
                        .text_color(self.appearance.secondary_text)
                        .child(self.locale.more_text(*count)),
                );
            }
            weeks = weeks.child(week);
        }

        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(weekday_header)
            .child(weeks)
            .into_any_element()
    }

    fn render_day_cell(
        &self,
        date: NaiveDate,
        view: CalendarView,
        outside: bool,
        event_count: usize,
        selection: &CalendarSelection,
        border_right: bool,
    ) -> AnyElement {
        let selected = selection_contains(selection, date);
        let is_today = date == self.today;
        let context = DayRenderContext {
            date,
            view,
            is_today,
            is_selected: selected,
            is_outside: outside,
            event_count,
        };
        let content = self.day_renderer.as_ref().map(|renderer| renderer(context));
        let default_content = content.is_none();
        let state = self.state.clone();
        let visible = self.show_outside_days || !outside;

        div()
            .id((self.id.clone(), format!("day-{date}")))
            .role(Role::Button)
            .aria_label(self.locale.date_accessible_label(date))
            .aria_selected(selected)
            .relative()
            .size_full()
            .min_w_0()
            .p_3()
            .when(
                matches!(view, CalendarView::Week | CalendarView::Day),
                |element| element.p(px(6.)),
            )
            .when(is_today, |element| {
                element.bg(self.appearance.accent.opacity(0.035))
            })
            .when(border_right, |element| {
                element.border_r_1().border_color(self.appearance.grid_line)
            })
            .when(visible, |element| {
                element
                    .cursor_pointer()
                    .hover(|style| style.bg(self.appearance.hover_day))
                    .on_click(move |_, _, cx| {
                        state.update(cx, |state, cx| state.select_date(date, cx));
                    })
            })
            .children(content)
            .when(default_content && visible, |element| {
                let day_number = div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(px(28.))
                    .rounded_full()
                    .text_sm()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(if selected || is_today {
                        self.appearance.selected_day_text
                    } else if outside {
                        self.appearance.outside_text
                    } else {
                        gpui::black()
                    })
                    .when(selected, |day| day.bg(self.appearance.selected_day))
                    .when(is_today && !selected, |day| {
                        day.bg(self.appearance.today_ring)
                    })
                    .child(self.locale.day_number_text(date));

                if matches!(view, CalendarView::Week | CalendarView::Day) {
                    element.child(
                        div()
                            .flex()
                            .flex_col()
                            .items_center()
                            .justify_center()
                            .gap_1()
                            .child(
                                div()
                                    .text_xs()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(self.appearance.secondary_text)
                                    .child(self.locale.weekday_name(date.weekday(), true)),
                            )
                            .child(day_number),
                    )
                } else {
                    element.child(day_number)
                }
            })
            .into_any_element()
    }

    fn render_segment(
        &self,
        segment: &EventSegment<T>,
        view: CalendarView,
        visible: DateRange,
        selected_event: Option<&SharedString>,
    ) -> AnyElement {
        let context = EventRenderContext {
            id: segment.group.id.clone(),
            title: segment.group.title.clone(),
            events: segment.group.events.clone(),
            event_range: segment.group.range,
            segment_range: segment.range,
            visible_range: visible,
            view,
            lane: segment.lane,
            continues_before: segment.continues_before,
            continues_after: segment.continues_after,
            is_all_day: segment
                .group
                .events
                .first()
                .is_some_and(|event| event.time.is_all_day()),
            is_selected: selected_event == Some(&segment.group.id),
        };
        self.render_event(context, segment.group.color, segment.show_label)
    }

    fn render_event(
        &self,
        context: EventRenderContext<T>,
        color: Option<Hsla>,
        show_label: bool,
    ) -> AnyElement {
        let content = self
            .event_renderer
            .as_ref()
            .map(|renderer| renderer(context.clone()));
        let default_content = content.is_none();
        let event_color = color.unwrap_or(self.appearance.event_background);
        let event_text = color
            .map(readable_event_color)
            .unwrap_or(self.appearance.event_text);
        let compact = context.is_all_day || context.view == CalendarView::Month;
        let clock_range = (!compact)
            .then(|| self.event_clock_range(&context))
            .flatten();
        let state = self.state.clone();
        let id = context.id.clone();
        let mut event = div()
            .id((
                self.id.clone(),
                format!(
                    "event-{}-{}-{}",
                    context.id, context.segment_range.start, context.lane
                ),
            ))
            .role(Role::Button)
            .aria_label(context.title.clone())
            .size_full()
            .min_w_0()
            .px(px(9.))
            .flex()
            .items_center()
            .relative()
            .overflow_hidden()
            .whitespace_nowrap()
            .rounded(px(6.))
            .when(!context.continues_before, |element| {
                element.rounded_tl(px(7.)).rounded_bl(px(7.))
            })
            .when(!context.continues_after, |element| {
                element.rounded_tr(px(7.)).rounded_br(px(7.))
            })
            .border_1()
            .border_color(event_color.opacity(0.22))
            .bg(event_color.opacity(0.11))
            .text_color(event_text)
            .text_xs()
            .font_weight(FontWeight::MEDIUM)
            .cursor_pointer()
            .hover(|style| {
                style
                    .bg(event_color.opacity(0.17))
                    .border_color(event_color.opacity(0.34))
            })
            .when(context.is_selected, |element| {
                element
                    .border_1()
                    .border_color(self.appearance.selected_event_ring)
            })
            .on_click(move |_, _, cx| {
                state.update(cx, |state, cx| state.select_event(Some(id.clone()), cx));
                cx.stop_propagation();
            })
            .children(content)
            .when(default_content && show_label, |element| {
                if let Some(clock_range) = clock_range {
                    element.child(
                        div()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .justify_center()
                            .child(
                                div()
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(context.title.clone()),
                            )
                            .child(
                                div()
                                    .mt(px(1.))
                                    .text_size(px(10.))
                                    .font_weight(FontWeight::NORMAL)
                                    .text_color(event_text.opacity(0.72))
                                    .child(clock_range),
                            ),
                    )
                } else {
                    element.child(
                        div()
                            .min_w_0()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(context.title.clone()),
                    )
                }
            });
        if let Some(styler) = &self.event_styler {
            event.style().refine(&styler(&context));
        }
        event.into_any_element()
    }

    fn event_clock_range(&self, context: &EventRenderContext<T>) -> Option<SharedString> {
        let CalendarEventTime::Timed { start, end } = context.events.first()?.time else {
            return None;
        };
        if end - start < Duration::minutes(45) {
            return None;
        }
        let start = start.with_timezone(&self.display_offset);
        let end = end.with_timezone(&self.display_offset);
        Some(
            format!(
                "{} – {}",
                self.locale.clock_text(start.hour() * 60 + start.minute()),
                self.locale.clock_text(end.hour() * 60 + end.minute())
            )
            .into(),
        )
    }

    fn render_year(
        &self,
        anchor: NaiveDate,
        groups: &[EventGroup<T>],
        selection: &CalendarSelection,
    ) -> AnyElement {
        let mut rows = div()
            .id((self.id.clone(), "year-scroll"))
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .p_4()
            .bg(rgb(0xf8fafc))
            .flex()
            .flex_col()
            .gap_3();
        for row in 0..4 {
            let mut months = div().flex().gap_3();
            for column in 0..3 {
                let month_number = row * 3 + column + 1;
                let month = YearMonth::new(anchor.year(), month_number as u32);
                let state = self.state.clone();
                let month_date = month.first_day();
                let event_colors: Vec<_> = groups
                    .iter()
                    .filter(|group| group.range.intersects(month.range()))
                    .take(3)
                    .map(|group| group.color.unwrap_or(self.appearance.event_background))
                    .collect();
                let card = div()
                    .id((
                        self.id.clone(),
                        format!("year-month-{}-{}", month.year, month.month),
                    ))
                    .role(Role::Button)
                    .aria_label(
                        self.locale
                            .title(CalendarView::Month, month_date, month.range()),
                    )
                    .flex_1()
                    .min_w_0()
                    .h(px(210.))
                    .p(px(14.))
                    .flex()
                    .flex_col()
                    .rounded(px(12.))
                    .border_1()
                    .border_color(self.appearance.grid_line)
                    .bg(rgb(0xffffff))
                    .cursor_pointer()
                    .hover(|style| {
                        style
                            .border_color(self.appearance.accent)
                            .bg(rgb(0xf8fafc))
                            .shadow_sm()
                    })
                    .on_click(move |_, _, cx| {
                        state.update(cx, |state, cx| {
                            state.go_to(month_date, cx);
                            state.set_view(CalendarView::Month, cx);
                        });
                    })
                    .child(
                        div()
                            .mb_3()
                            .flex()
                            .items_center()
                            .justify_between()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(0x172033))
                            .child(self.locale.month_names[month_number - 1].clone())
                            .child(
                                div().flex().items_center().gap_1().children(
                                    event_colors
                                        .into_iter()
                                        .map(|color| div().size(px(5.)).rounded_full().bg(color)),
                                ),
                            ),
                    )
                    .child(self.render_mini_month_grid(month, groups, selection));
                months = months.child(card);
            }
            rows = rows.child(months);
        }
        rows.into_any_element()
    }

    fn render_mini_month_grid(
        &self,
        month: YearMonth,
        groups: &[EventGroup<T>],
        selection: &CalendarSelection,
    ) -> AnyElement {
        let visible = month_grid_range(month, self.weekday());
        let weekday_labels: Vec<SharedString> = ordered_weekdays(self.weekday())
            .into_iter()
            .map(|weekday| {
                self.locale
                    .weekday_name(weekday, true)
                    .to_string()
                    .chars()
                    .next()
                    .unwrap_or(' ')
                    .to_string()
                    .into()
            })
            .collect();
        let event_ranges: Vec<_> = groups.iter().map(|group| group.range).collect();
        let selection = selection.clone();
        let appearance = self.appearance.clone();
        let today = self.today;

        canvas(
            |_, _, _| (),
            move |bounds, _, window, cx| {
                let cell_width = bounds.size.width / 7.;
                let row_height = (bounds.size.height - px(20.)) / 6.;
                for (column, label) in weekday_labels.iter().enumerate() {
                    paint_mini_text(
                        label.clone(),
                        point(bounds.left() + cell_width * column as f32, bounds.top()),
                        cell_width,
                        appearance.secondary_text,
                        window,
                        cx,
                    );
                }

                for index in 0..42 {
                    let date = visible.start + Duration::days(index);
                    if date.month() != month.month {
                        continue;
                    }
                    let column = index as usize % 7;
                    let row = index as usize / 7;
                    let cell_origin = point(
                        bounds.left() + cell_width * column as f32,
                        bounds.top() + px(20.) + row_height * row as f32,
                    );
                    let selected = selection_contains(&selection, date);
                    if selected || date == today {
                        let indicator_size = px(20.);
                        let indicator = Bounds::new(
                            point(
                                cell_origin.x + (cell_width - indicator_size) / 2.,
                                cell_origin.y + (row_height - indicator_size) / 2.,
                            ),
                            size(indicator_size, indicator_size),
                        );
                        window.paint_quad(quad(
                            indicator,
                            px(7.),
                            if selected {
                                appearance.selected_day
                            } else {
                                gpui::transparent_black()
                            },
                            if selected { px(0.) } else { px(1.) },
                            appearance.today_ring,
                            Default::default(),
                        ));
                    }
                    paint_mini_text(
                        date.day().to_string().into(),
                        cell_origin,
                        cell_width,
                        if selected {
                            appearance.selected_day_text
                        } else {
                            gpui::black()
                        },
                        window,
                        cx,
                    );
                    if event_ranges.iter().any(|range| range.contains(date)) {
                        let dot = Bounds::new(
                            point(
                                cell_origin.x + (cell_width - px(3.)) / 2.,
                                cell_origin.y + row_height - px(4.),
                            ),
                            size(px(3.), px(3.)),
                        );
                        window.paint_quad(quad(
                            dot,
                            px(1.5),
                            appearance.accent,
                            px(0.),
                            gpui::transparent_black(),
                            Default::default(),
                        ));
                    }
                }
            },
        )
        .flex_1()
        .min_h_0()
        .into_any_element()
    }

    fn render_timeline(
        &self,
        view: CalendarView,
        visible: DateRange,
        groups: &[EventGroup<T>],
        timeline_state: TimelineState<'_>,
    ) -> AnyElement {
        let day_count = visible.days() as usize;
        let dates: Vec<_> = (0..day_count)
            .map(|day| visible.start + Duration::days(day as i64))
            .collect();

        let mut date_header = div().flex().flex_none();
        date_header = date_header.child(div().w(px(64.)).flex_none());
        for (index, date) in dates.iter().copied().enumerate() {
            let events = groups
                .iter()
                .filter(|group| group.range.contains(date))
                .count();
            date_header = date_header.child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h(px(60.))
                    .when(date == self.today, |element| {
                        element.bg(self.appearance.accent.opacity(0.035))
                    })
                    .when(index + 1 < day_count, |element| {
                        element.border_r_1().border_color(self.appearance.grid_line)
                    })
                    .child(self.render_day_cell(
                        date,
                        view,
                        false,
                        events,
                        timeline_state.date_selection,
                        false,
                    )),
            );
        }

        let all_day_groups: Vec<_> = groups
            .iter()
            .filter(|group| {
                group
                    .events
                    .first()
                    .is_some_and(|event| event.time.is_all_day())
            })
            .cloned()
            .collect();
        let all_day_layout = layout_month(&all_day_groups, visible, 2, self.label_policy);
        let mut all_day_events = div().relative().flex_1().min_w_0().h(px(52.));
        for index in 1..day_count {
            all_day_events = all_day_events.child(
                div()
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .left(relative(index as f32 / day_count as f32))
                    .border_l_1()
                    .border_color(self.appearance.grid_line),
            );
        }
        for segment in all_day_layout
            .segments
            .iter()
            .filter(|segment| segment.week_index == 0 && segment.lane < 2)
        {
            all_day_events = all_day_events.child(
                div()
                    .absolute()
                    .left(relative(segment.start_column as f32 / day_count as f32))
                    .top(px(6. + segment.lane as f32 * 23.))
                    .w(relative(segment.span_columns as f32 / day_count as f32))
                    .h(px(21.))
                    .px(px(3.))
                    .child(self.render_segment(
                        segment,
                        view,
                        visible,
                        timeline_state.selected_event,
                    )),
            );
        }
        let header = div()
            .flex()
            .flex_col()
            .flex_none()
            .border_b_1()
            .border_color(self.appearance.grid_line)
            .bg(rgb(0xfcfdff))
            .child(date_header)
            .child(
                div()
                    .flex()
                    .flex_none()
                    .border_t_1()
                    .border_color(self.appearance.grid_line)
                    .child(
                        div()
                            .w(px(64.))
                            .flex_none()
                            .px_3()
                            .py_2()
                            .text_xs()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(self.appearance.secondary_text)
                            .child(self.locale.labels.all_day.clone()),
                    )
                    .child(all_day_events),
            );

        let first_minute = self.day_start_hour * 60;
        let last_minute = self.day_end_hour * 60;
        let hour_height = 56.;
        let collapsed_time_ranges: Vec<_> = self
            .collapsed_time_ranges
            .iter()
            .copied()
            .filter(|range| !timeline_state.expanded_time_ranges.contains(range))
            .collect();
        let axis = TimeAxis::new(
            first_minute,
            last_minute,
            &collapsed_time_ranges,
            hour_height,
            20.,
        );
        let selection_mapper = TimelineSelectionMapper {
            axis: axis.clone(),
            dates: dates.clone().into(),
            display_offset: self.display_offset,
            precision_minutes: self.time_selection_precision_minutes,
            max_slots_per_range: self.time_selection_max_slots_per_range,
            last_minute,
        };
        let max_slots_per_range = self.time_selection_max_slots_per_range;
        let selection_precision_minutes = self.time_selection_precision_minutes;
        let time_selection_merge_policy = self.time_selection_merge_policy;
        let grid_axis = axis.clone();
        let grid_line = self.appearance.grid_line;
        let grid = canvas(
            |_, _, _| (),
            move |bounds, _, window, _| {
                let mut minute = first_minute;
                while minute <= last_minute {
                    let is_collapsed = grid_axis.segments.iter().any(|segment| {
                        segment.collapsed
                            && minute >= segment.start_minute
                            && minute <= segment.end_minute
                    });
                    if !is_collapsed {
                        let y = grid_axis.y_for_minute(minute);
                        window.paint_quad(gpui::fill(
                            Bounds::new(
                                point(bounds.left(), bounds.top() + px(y)),
                                size(bounds.size.width, px(1.)),
                            ),
                            grid_line,
                        ));
                    }
                    minute += 60;
                }
            },
        )
        .absolute()
        .inset_0();
        let mut canvas = div()
            .relative()
            .h(px(axis.height))
            .ml(px(64.))
            .flex()
            .child(grid)
            .child(
                TimelineInteraction::new(
                    (self.id.clone(), "timeline-interaction"),
                    {
                        let mapper = selection_mapper.clone();
                        let state = self.state.clone();
                        move |position, bounds, _, cx| {
                            let Some(selection) = mapper.slot_at(position, bounds) else {
                                return;
                            };
                            state.update(cx, |state, cx| {
                                state.toggle_time_selection(
                                    selection,
                                    selection_precision_minutes,
                                    max_slots_per_range,
                                    time_selection_merge_policy,
                                    cx,
                                );
                            });
                        }
                    },
                    {
                        let mapper = selection_mapper;
                        let state = self.state.clone();
                        move |origin, position, bounds, phase, _, cx| {
                            let Some(selection) = mapper.range_between(origin, position, bounds)
                            else {
                                return;
                            };
                            state.update(cx, |state, cx| match phase {
                                LongPressPhase::Start => state.begin_time_selection(selection, cx),
                                LongPressPhase::Update => {
                                    state.update_time_selection(selection, cx)
                                }
                                LongPressPhase::End => {
                                    state.update_time_selection(selection, cx);
                                    state.commit_time_selection(
                                        selection_precision_minutes,
                                        max_slots_per_range,
                                        time_selection_merge_policy,
                                        cx,
                                    );
                                }
                            });
                        }
                    },
                )
                .long_press_delay(self.time_selection_long_press_delay)
                .absolute()
                .inset_0()
                .cursor_pointer(),
            );

        for segment in axis.segments.iter().filter(|segment| segment.collapsed) {
            let ranges_to_expand: Arc<[CalendarClockRange]> = collapsed_time_ranges
                .iter()
                .copied()
                .filter(|range| {
                    u32::from(range.start_minute) < segment.end_minute
                        && u32::from(range.end_minute) > segment.start_minute
                })
                .collect::<Vec<_>>()
                .into();
            let state = self.state.clone();
            canvas = canvas.child(
                div()
                    .id((
                        self.id.clone(),
                        format!(
                            "expand-time-{}-{}",
                            segment.start_minute, segment.end_minute
                        ),
                    ))
                    .role(Role::Button)
                    .aria_label(format!(
                        "{} {}–{}",
                        self.locale.labels.expand_time_range,
                        self.locale.clock_text(segment.start_minute),
                        self.locale.clock_text(segment.end_minute)
                    ))
                    .absolute()
                    .left(px(-20.))
                    .top(px(segment.top + segment.height / 2. - 10.))
                    .size(px(20.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_full()
                    .border_1()
                    .border_color(self.appearance.grid_line)
                    .bg(rgb(0xffffff))
                    .shadow_sm()
                    .text_xs()
                    .text_color(self.appearance.secondary_text)
                    .cursor_pointer()
                    .hover(|style| {
                        style
                            .border_color(self.appearance.accent)
                            .text_color(self.appearance.accent)
                    })
                    .on_click(move |_, _, cx| {
                        state.update(cx, |state, cx| {
                            for range in ranges_to_expand.iter().copied() {
                                state.expand_time_range(range, cx);
                            }
                        });
                        cx.stop_propagation();
                    })
                    .child("⌄"),
            );
        }
        for range in self
            .collapsed_time_ranges
            .iter()
            .copied()
            .filter(|range| timeline_state.expanded_time_ranges.contains(range))
        {
            let start = u32::from(range.start_minute).max(first_minute);
            let end = u32::from(range.end_minute).min(last_minute);
            if start >= end {
                continue;
            }
            let state = self.state.clone();
            canvas = canvas.child(
                div()
                    .id((self.id.clone(), format!("collapse-time-{start}-{end}")))
                    .role(Role::Button)
                    .aria_label(format!(
                        "{} {}–{}",
                        self.locale.labels.collapse_time_range,
                        self.locale.clock_text(start),
                        self.locale.clock_text(end)
                    ))
                    .absolute()
                    .left(px(-20.))
                    .top(px(axis.y_for_minute(start) + 8.))
                    .size(px(20.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_full()
                    .border_1()
                    .border_color(self.appearance.grid_line)
                    .bg(rgb(0xffffff))
                    .shadow_sm()
                    .text_xs()
                    .text_color(self.appearance.secondary_text)
                    .cursor_pointer()
                    .hover(|style| {
                        style
                            .border_color(self.appearance.accent)
                            .text_color(self.appearance.accent)
                    })
                    .on_click(move |_, _, cx| {
                        state.update(cx, |state, cx| state.collapse_time_range(range, cx));
                        cx.stop_propagation();
                    })
                    .child("⌃"),
            );
        }
        for hour in self.day_start_hour..=self.day_end_hour {
            let minute = hour * 60;
            if axis.segments.iter().any(|segment| {
                segment.collapsed && minute >= segment.start_minute && minute <= segment.end_minute
            }) {
                continue;
            }
            canvas = canvas.child(
                div()
                    .absolute()
                    .left(px(-64.))
                    .right_0()
                    .top(px(axis.y_for_minute(minute)))
                    .h(px(1.))
                    .child(
                        div()
                            .absolute()
                            .left_0()
                            .top(if minute == first_minute {
                                px(3.)
                            } else {
                                px(-7.)
                            })
                            .w(px(42.))
                            .text_right()
                            .text_xs()
                            .text_color(self.appearance.secondary_text)
                            .child(self.locale.hour_text(hour)),
                    ),
            );
        }
        for (day_index, date) in dates.iter().copied().enumerate() {
            canvas = canvas.child(
                div()
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .left(relative(day_index as f32 / day_count as f32))
                    .w(relative(1.0 / day_count as f32))
                    .when(date == self.today, |element| {
                        element.bg(self.appearance.accent.opacity(0.025))
                    })
                    .when(day_index + 1 < day_count, |element| {
                        element.border_r_1().border_color(self.appearance.grid_line)
                    }),
            );
            for selection in timeline_state.time_selections {
                let start = selection.start.with_timezone(&self.display_offset);
                let end = selection.end.with_timezone(&self.display_offset);
                if start.date_naive() <= date && end.date_naive() >= date {
                    let start_minute = if start.date_naive() < date {
                        first_minute
                    } else {
                        start.hour() * 60 + start.minute()
                    }
                    .max(first_minute);
                    let end_minute = if end.date_naive() > date {
                        last_minute
                    } else {
                        end.hour() * 60 + end.minute()
                    }
                    .min(last_minute);
                    if end_minute > start_minute {
                        let top = axis.y_for_minute(start_minute);
                        let bottom = axis.y_for_minute(end_minute);
                        canvas = canvas.child(
                            div()
                                .absolute()
                                .left(relative(day_index as f32 / day_count as f32))
                                .top(px(top))
                                .w(relative(1.0 / day_count as f32))
                                .h(px((bottom - top).max(2.)))
                                .px(px(3.))
                                .py(px(1.))
                                .child(
                                    div()
                                        .size_full()
                                        .relative()
                                        .overflow_hidden()
                                        .rounded(px(6.))
                                        .border_1()
                                        .border_color(self.appearance.accent.opacity(0.72))
                                        .bg(self.appearance.time_selection_background),
                                ),
                        );
                    }
                }
            }
            let timed: Vec<_> = self
                .events
                .iter()
                .filter_map(|event| {
                    let CalendarEventTime::Timed { start, end } = event.time else {
                        return None;
                    };
                    let start = start.with_timezone(&self.display_offset);
                    let end = end.with_timezone(&self.display_offset);
                    if start.date_naive() > date || end.date_naive() < date {
                        return None;
                    }
                    let start_minute = if start.date_naive() < date {
                        first_minute
                    } else {
                        start.hour() * 60 + start.minute()
                    }
                    .max(first_minute);
                    let end_minute = if end.date_naive() > date {
                        last_minute
                    } else {
                        end.hour() * 60 + end.minute()
                    }
                    .min(last_minute);
                    (end_minute > start_minute).then_some((event, start_minute, end_minute))
                })
                .collect();
            let columns = overlap_columns(&timed);
            for ((event, start, end), (column, column_count)) in timed.into_iter().zip(columns) {
                let event_range = event.date_range(self.display_offset);
                let context = EventRenderContext {
                    id: event.id.clone(),
                    title: event.title.clone(),
                    events: Arc::from([event.clone()]),
                    event_range,
                    segment_range: DateRange::single(date),
                    visible_range: visible,
                    view,
                    lane: column,
                    continues_before: event_range.start < date,
                    continues_after: event_range.end_exclusive > date + Duration::days(1),
                    is_all_day: false,
                    is_selected: timeline_state.selected_event == Some(&event.id),
                };
                let day_width = 1.0 / day_count as f32;
                let event_width = day_width / column_count as f32;
                canvas = canvas.child(
                    div()
                        .absolute()
                        .left(relative(
                            day_index as f32 * day_width + column as f32 * event_width,
                        ))
                        .top(px(axis.y_for_minute(start)))
                        .w(relative(event_width))
                        .h(px(
                            (axis.y_for_minute(end) - axis.y_for_minute(start)).max(18.)
                        ))
                        .px(px(3.))
                        .py(px(2.))
                        .child(self.render_event(context, event.color, true)),
                );
            }
        }

        let now = Utc::now().with_timezone(&self.display_offset);
        let now_minute = now.hour() * 60 + now.minute();
        if now.date_naive() == self.today
            && visible.contains(self.today)
            && (first_minute..=last_minute).contains(&now_minute)
        {
            let day_index = (self.today - visible.start).num_days() as f32;
            let day_width = 1.0 / day_count as f32;
            canvas = canvas.child(
                div()
                    .absolute()
                    .left(relative(day_index * day_width))
                    .top(px(axis.y_for_minute(now_minute)))
                    .w(relative(day_width))
                    .h(px(1.))
                    .bg(self.appearance.now_line)
                    .child(
                        div()
                            .absolute()
                            .left(px(-3.))
                            .top(px(-3.))
                            .size(px(7.))
                            .rounded_full()
                            .bg(self.appearance.now_line),
                    ),
            );
        }

        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(header)
            .child(
                div()
                    .id((self.id.clone(), "time-scroll"))
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(canvas),
            )
            .into_any_element()
    }
}

impl<T: 'static> RenderOnce for CalendarCore<T> {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let (view, anchor, selection, selected_event, time_selections, expanded_time_ranges) = {
            let state = self.state.read(cx);
            (
                self.view,
                state.anchor_date(),
                state.selection().clone(),
                state.selected_event().cloned(),
                state.time_selections_with_preview(self.time_selection_merge_policy),
                state.expanded_time_ranges().to_vec(),
            )
        };
        let visible = visible_range(view, anchor, self.weekday());
        let groups = resolve_event_groups(
            &self.events,
            self.merge_policy,
            self.merge_key.as_ref(),
            self.display_offset,
        );
        let body = match view {
            CalendarView::Year => self.render_year(anchor, &groups, &selection),
            CalendarView::Month => self.render_month(
                anchor,
                visible,
                &groups,
                selected_event.as_ref(),
                &selection,
            ),
            CalendarView::Week | CalendarView::Day => self.render_timeline(
                view,
                visible,
                &groups,
                TimelineState {
                    selected_event: selected_event.as_ref(),
                    date_selection: &selection,
                    time_selections: &time_selections,
                    expanded_time_ranges: &expanded_time_ranges,
                },
            ),
        };

        let id = self.id.clone();
        let mut root = div()
            .id(id)
            .debug_selector(|| "uic-calendar".to_string())
            .flex()
            .flex_col()
            .text_color(rgb(0x172033))
            .child(body);
        root.style().refine(&self.style);
        root
    }
}

impl<T: 'static> Styled for CalendarCore<T> {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

fn readable_event_color(color: Hsla) -> Hsla {
    Hsla {
        l: color.l.min(0.4),
        a: 1.,
        ..color
    }
}

fn visible_range(view: CalendarView, anchor: NaiveDate, first_weekday: Weekday) -> DateRange {
    match view {
        CalendarView::Year => DateRange::new(
            NaiveDate::from_ymd_opt(anchor.year(), 1, 1).expect("valid year"),
            NaiveDate::from_ymd_opt(anchor.year() + 1, 1, 1).expect("valid year"),
        ),
        CalendarView::Month => month_grid_range(YearMonth::from_date(anchor), first_weekday),
        CalendarView::Week => week_range(anchor, first_weekday),
        CalendarView::Day => DateRange::single(anchor),
    }
}

fn ordered_weekdays(first: Weekday) -> [Weekday; 7] {
    let weekdays = [
        Weekday::Mon,
        Weekday::Tue,
        Weekday::Wed,
        Weekday::Thu,
        Weekday::Fri,
        Weekday::Sat,
        Weekday::Sun,
    ];
    let start = first.num_days_from_monday() as usize;
    std::array::from_fn(|index| weekdays[(start + index) % 7])
}

fn selection_contains(selection: &CalendarSelection, date: NaiveDate) -> bool {
    match selection {
        CalendarSelection::None => false,
        CalendarSelection::Single(selected) => *selected == Some(date),
        CalendarSelection::Range {
            start: Some(start),
            end: Some(end),
        } => *start <= date && date <= *end,
        CalendarSelection::Range {
            start: Some(start),
            end: None,
        } => *start == date,
        CalendarSelection::Range { .. } => false,
    }
}

fn overlap_columns<T>(events: &[(&CalendarEvent<T>, u32, u32)]) -> Vec<(usize, usize)> {
    let mut indexed: Vec<_> = events.iter().enumerate().collect();
    indexed.sort_by_key(|(_, (_, start, end))| (*start, *end));
    let mut lane_ends: Vec<u32> = Vec::new();
    let mut lanes = vec![0; events.len()];
    for (index, (_, start, end)) in indexed {
        let lane = lane_ends
            .iter()
            .position(|lane_end| *lane_end <= *start)
            .unwrap_or(lane_ends.len());
        if lane == lane_ends.len() {
            lane_ends.push(*end);
        } else {
            lane_ends[lane] = *end;
        }
        lanes[index] = lane;
    }
    let count = lane_ends.len().max(1);
    lanes.into_iter().map(|lane| (lane, count)).collect()
}

fn paint_mini_text(
    text: SharedString,
    origin: gpui::Point<gpui::Pixels>,
    width: gpui::Pixels,
    color: Hsla,
    window: &mut Window,
    cx: &mut App,
) {
    let style = window.text_style();
    let run = TextRun {
        len: text.len(),
        font: style.font(),
        color,
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let line = window.text_system().shape_line(text, px(11.), &[run], None);
    let _ = line.paint(origin, px(18.), TextAlign::Center, Some(width), window, cx);
}
