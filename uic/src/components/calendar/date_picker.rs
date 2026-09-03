use std::rc::Rc;

use chrono::{Datelike as _, NaiveDate, Weekday};
use gpui::{
    AnyElement, App, ElementId, Entity, FontWeight, Hsla, IntoElement, RenderOnce, Role,
    SharedString, StyleRefinement, Styled, Window, div, prelude::*, px, relative, rgb, svg,
};

use crate::assets::LucideIcons;

use super::{
    CalendarLocale, CalendarSelection, CalendarSelectionMode, CalendarState, CalendarView,
    DatePickerView, YearMonth, date::month_grid_range,
};

type DateEnabledPredicate = Rc<dyn Fn(NaiveDate) -> bool>;
type DatePickerDayRenderer = Rc<dyn Fn(DatePickerDayContext) -> AnyElement>;
type DatePickerDayLabelProvider = Rc<dyn Fn(NaiveDate) -> Option<DatePickerDayLabel>>;

/// A short annotation shown below a date, such as "Holiday", "Weekend", or
/// "Workday".
#[derive(Clone, Debug, PartialEq)]
pub struct DatePickerDayLabel {
    pub text: SharedString,
    pub color: Option<Hsla>,
}

impl DatePickerDayLabel {
    pub fn new(text: impl Into<SharedString>) -> Self {
        Self {
            text: text.into(),
            color: None,
        }
    }

    pub fn color(mut self, color: Hsla) -> Self {
        self.color = Some(color);
        self
    }
}

/// Semantic colors used by [`CalendarDatePicker`]. Its outer surface uses
/// [`Styled`] instead.
#[derive(Clone, Debug, uic_macros::Chainable)]
pub struct DatePickerAppearance {
    pub accent: gpui::Hsla,
    pub selected_background: gpui::Hsla,
    pub selected_foreground: gpui::Hsla,
    pub range_background: gpui::Hsla,
    pub hover_background: gpui::Hsla,
    pub muted_foreground: gpui::Hsla,
    pub disabled_foreground: gpui::Hsla,
    pub today_ring: gpui::Hsla,
    pub label_foreground: gpui::Hsla,
}

impl Default for DatePickerAppearance {
    fn default() -> Self {
        Self {
            accent: rgb(0x4263eb).into(),
            selected_background: rgb(0x4263eb).into(),
            selected_foreground: rgb(0xffffff).into(),
            range_background: gpui::rgba(0x4263eb1f).into(),
            hover_background: gpui::rgba(0x4263eb0d).into(),
            muted_foreground: rgb(0x98a2b3).into(),
            disabled_foreground: rgb(0xd0d5dd).into(),
            today_ring: rgb(0x4263eb).into(),
            label_foreground: rgb(0x718096).into(),
        }
    }
}

/// Context for replacing a date picker's managed day content.
#[derive(Clone, Debug)]
pub struct DatePickerDayContext {
    pub date: NaiveDate,
    pub is_today: bool,
    pub is_outside: bool,
    pub is_disabled: bool,
    pub is_selected: bool,
    pub is_range_start: bool,
    pub is_range_end: bool,
    pub is_in_range: bool,
    pub label: Option<DatePickerDayLabel>,
}

/// A compact month selector intended for forms, popovers, and modal surfaces.
///
/// It owns no overlay behavior. Applications can compose it into any surface
/// and observe selection through [`super::CalendarStateEvent`]. Paging keeps a
/// pending range start, so the second date can be selected in another month.
#[derive(IntoElement)]
pub struct CalendarDatePicker {
    id: ElementId,
    state: Entity<CalendarState>,
    locale: CalendarLocale,
    appearance: DatePickerAppearance,
    selection_mode: CalendarSelectionMode,
    first_weekday: Option<Weekday>,
    today: NaiveDate,
    min_date: Option<NaiveDate>,
    max_date: Option<NaiveDate>,
    show_outside_days: bool,
    date_enabled: Option<DateEnabledPredicate>,
    day_renderer: Option<DatePickerDayRenderer>,
    day_label: Option<DatePickerDayLabelProvider>,
    style: StyleRefinement,
}

impl CalendarDatePicker {
    pub fn new(id: impl Into<ElementId>, state: &Entity<CalendarState>) -> Self {
        Self {
            id: id.into(),
            state: state.clone(),
            locale: CalendarLocale::default(),
            appearance: DatePickerAppearance::default(),
            selection_mode: CalendarSelectionMode::Single,
            first_weekday: None,
            today: chrono::Local::now().date_naive(),
            min_date: None,
            max_date: None,
            show_outside_days: true,
            date_enabled: None,
            day_renderer: None,
            day_label: None,
            style: StyleRefinement::default()
                .w(px(328.))
                .p_3()
                .rounded(px(16.))
                .border_1()
                .border_color(rgb(0xe4e9f1))
                .bg(rgb(0xffffff))
                .shadow_lg(),
        }
    }

    pub fn locale(mut self, locale: CalendarLocale) -> Self {
        self.locale = locale;
        self
    }

    pub fn appearance(mut self, appearance: DatePickerAppearance) -> Self {
        self.appearance = appearance;
        self
    }

    pub fn selection_mode(mut self, mode: CalendarSelectionMode) -> Self {
        self.selection_mode = mode;
        self
    }

    pub fn first_weekday(mut self, weekday: Weekday) -> Self {
        self.first_weekday = Some(weekday);
        self
    }

    pub fn today(mut self, today: NaiveDate) -> Self {
        self.today = today;
        self
    }

    pub fn min_date(mut self, date: NaiveDate) -> Self {
        self.min_date = Some(date);
        self
    }

    pub fn max_date(mut self, date: NaiveDate) -> Self {
        self.max_date = Some(date);
        self
    }

    /// Adds application-specific date availability without changing visuals.
    pub fn date_enabled(mut self, predicate: impl Fn(NaiveDate) -> bool + 'static) -> Self {
        self.date_enabled = Some(Rc::new(predicate));
        self
    }

    pub fn show_outside_days(mut self, show: bool) -> Self {
        self.show_outside_days = show;
        self
    }

    /// Supplies an optional short annotation for each date without replacing
    /// the managed date content or interaction.
    pub fn day_label(
        mut self,
        provider: impl Fn(NaiveDate) -> Option<DatePickerDayLabel> + 'static,
    ) -> Self {
        self.day_label = Some(Rc::new(provider));
        self
    }

    /// Replaces the content inside the managed day circle while preserving
    /// selection, range, disabled, hover, and accessibility behavior.
    pub fn day_content<E: IntoElement>(
        mut self,
        renderer: impl Fn(DatePickerDayContext) -> E + 'static,
    ) -> Self {
        self.day_renderer = Some(Rc::new(move |context| renderer(context).into_any_element()));
        self
    }

    fn is_enabled(&self, date: NaiveDate) -> bool {
        self.min_date.is_none_or(|minimum| date >= minimum)
            && self.max_date.is_none_or(|maximum| date <= maximum)
            && self
                .date_enabled
                .as_ref()
                .is_none_or(|predicate| predicate(date))
    }
}

impl RenderOnce for CalendarDatePicker {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let first_weekday = self.first_weekday.unwrap_or(self.locale.first_weekday);
        let state = self.state.read(cx);
        let anchor = state.anchor_date();
        let month = YearMonth::from_date(anchor);
        let selection = state.selection().clone();
        let picker_view = state.date_picker_view();
        let visible = month_grid_range(month, first_weekday);
        let (range_start, range_end) = match selection {
            CalendarSelection::Range { start, end } => (start, end),
            _ => (None, None),
        };
        let selected_reference = match &selection {
            CalendarSelection::Single(Some(date)) => Some(*date),
            CalendarSelection::Range {
                start: Some(date), ..
            } => Some(*date),
            _ => None,
        };

        let previous_state = self.state.clone();
        let next_state = self.state.clone();
        let appearance = self.appearance.clone();
        let navigation_id = self.id.clone();
        let navigation_button = move |id: &'static str,
                                      icon: LucideIcons,
                                      label: gpui::SharedString,
                                      state: Entity<CalendarState>,
                                      delta: i8| {
            div()
                .id((navigation_id.clone(), id))
                .role(Role::Button)
                .aria_label(label)
                .size(px(32.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(9.))
                .text_color(appearance.muted_foreground)
                .cursor_pointer()
                .hover(|style| {
                    style
                        .bg(appearance.hover_background)
                        .text_color(appearance.accent)
                })
                .on_click(move |_, _, cx| {
                    state.update(cx, |state, cx| match picker_view {
                        DatePickerView::Days => {
                            if delta < 0 {
                                state.previous_in(CalendarView::Month, first_weekday, cx);
                            } else {
                                state.next_in(CalendarView::Month, first_weekday, cx);
                            }
                        }
                        DatePickerView::Months => {
                            if delta < 0 {
                                state.previous_in(CalendarView::Year, first_weekday, cx);
                            } else {
                                state.next_in(CalendarView::Year, first_weekday, cx);
                            }
                        }
                        DatePickerView::Years => {
                            let target =
                                shift_years(state.anchor_date(), if delta < 0 { -16 } else { 16 });
                            state.go_to(target, cx);
                        }
                    });
                })
                .child(
                    svg()
                        .path(icon)
                        .size_4()
                        .text_color(appearance.muted_foreground),
                )
        };

        let header_center: AnyElement = match picker_view {
            DatePickerView::Days => {
                let month_state = self.state.clone();
                let year_state = self.state.clone();
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(
                        div()
                            .id((self.id.clone(), "choose-month"))
                            .role(Role::Button)
                            .px_2()
                            .h(px(32.))
                            .flex()
                            .items_center()
                            .gap_1()
                            .rounded(px(9.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .cursor_pointer()
                            .hover(|style| style.bg(self.appearance.hover_background))
                            .on_click(move |_, _, cx| {
                                month_state.update(cx, |state, cx| {
                                    state.set_date_picker_view(DatePickerView::Months, cx);
                                });
                            })
                            .child(self.locale.month_names[month.month as usize - 1].clone())
                            .child(
                                svg()
                                    .path(LucideIcons::ChevronDown)
                                    .size(px(13.))
                                    .text_color(self.appearance.muted_foreground),
                            ),
                    )
                    .child(
                        div()
                            .id((self.id.clone(), "choose-year"))
                            .role(Role::Button)
                            .px_2()
                            .h(px(32.))
                            .flex()
                            .items_center()
                            .gap_1()
                            .rounded(px(9.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .cursor_pointer()
                            .hover(|style| style.bg(self.appearance.hover_background))
                            .on_click(move |_, _, cx| {
                                year_state.update(cx, |state, cx| {
                                    state.set_date_picker_view(DatePickerView::Years, cx);
                                });
                            })
                            .child(month.year.to_string())
                            .child(
                                svg()
                                    .path(LucideIcons::ChevronDown)
                                    .size(px(13.))
                                    .text_color(self.appearance.muted_foreground),
                            ),
                    )
                    .into_any_element()
            }
            DatePickerView::Months => {
                let state = self.state.clone();
                div()
                    .id((self.id.clone(), "months-year"))
                    .role(Role::Button)
                    .px_2()
                    .h(px(32.))
                    .flex()
                    .items_center()
                    .gap_1()
                    .rounded(px(9.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .cursor_pointer()
                    .hover(|style| style.bg(self.appearance.hover_background))
                    .on_click(move |_, _, cx| {
                        state.update(cx, |state, cx| {
                            state.set_date_picker_view(DatePickerView::Years, cx);
                        });
                    })
                    .child(month.year.to_string())
                    .child(
                        svg()
                            .path(LucideIcons::ChevronDown)
                            .size(px(13.))
                            .text_color(self.appearance.muted_foreground),
                    )
                    .into_any_element()
            }
            DatePickerView::Years => {
                let start = year_page_start(month.year);
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(format!("{start} – {}", start + 15))
                    .into_any_element()
            }
        };

        let close_state = self.state.clone();
        let close_button = div()
            .id((self.id.clone(), "cancel-navigation"))
            .role(Role::Button)
            .aria_label(self.locale.labels.cancel.clone())
            .size(px(32.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(9.))
            .text_color(self.appearance.muted_foreground)
            .cursor_pointer()
            .hover(|style| {
                style
                    .bg(self.appearance.hover_background)
                    .text_color(self.appearance.accent)
            })
            .on_click(move |_, _, cx| {
                close_state.update(cx, |state, cx| {
                    state.set_date_picker_view(DatePickerView::Days, cx);
                });
            })
            .child(
                svg()
                    .path(LucideIcons::X)
                    .size_4()
                    .text_color(self.appearance.muted_foreground),
            );

        let right_controls = div()
            .w(px(64.))
            .flex()
            .justify_end()
            .child(navigation_button(
                "next",
                LucideIcons::ChevronRight,
                self.locale.labels.next.clone(),
                next_state,
                1,
            ))
            .when(picker_view != DatePickerView::Days, |controls| {
                controls.child(close_button)
            });

        let header = div()
            .h(px(44.))
            .flex()
            .items_center()
            .child(div().w(px(64.)).child(navigation_button(
                "previous",
                LucideIcons::ChevronLeft,
                self.locale.labels.previous.clone(),
                previous_state,
                -1,
            )))
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(header_center),
            )
            .child(right_controls);

        let weekday_header = div().h(px(32.)).grid().grid_cols(7).children(
            ordered_weekdays(first_weekday).into_iter().map(|weekday| {
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(self.appearance.muted_foreground)
                    .child(self.locale.weekday_name(weekday, true))
            }),
        );

        let mut days = div().grid().grid_cols(7);
        let has_day_labels = self.day_label.is_some();
        for index in 0..42 {
            let date = visible.start + chrono::Duration::days(index);
            let column = index as usize % 7;
            let outside = date.month() != month.month || date.year() != month.year;
            let visible_day = self.show_outside_days || !outside;
            let enabled = visible_day && self.is_enabled(date);
            let is_range_start = range_start == Some(date);
            let is_range_end = range_end == Some(date);
            let is_single =
                matches!(selection, CalendarSelection::Single(Some(value)) if value == date);
            let is_selected = is_single || is_range_start || is_range_end;
            let is_in_range = range_start
                .zip(range_end)
                .is_some_and(|(start, end)| start <= date && date <= end);
            let label = self.day_label.as_ref().and_then(|provider| provider(date));
            let context = DatePickerDayContext {
                date,
                is_today: date == self.today,
                is_outside: outside,
                is_disabled: !enabled,
                is_selected,
                is_range_start,
                is_range_end,
                is_in_range,
                label: label.clone(),
            };
            let content = self
                .day_renderer
                .as_ref()
                .map(|renderer| renderer(context.clone()));
            let state = self.state.clone();
            let selection_mode = self.selection_mode;
            let fills_before =
                is_in_range && !is_range_start && range_start.is_some_and(|start| start < date);
            let fills_after =
                is_in_range && !is_range_end && range_end.is_some_and(|end| date < end);
            let rounds_before = is_range_start || column == 0;
            let rounds_after = is_range_end || column == 6;

            let mut cell = div()
                .relative()
                .h(px(if has_day_labels { 54. } else { 40. }))
                .flex()
                .flex_col()
                .items_center()
                .when(has_day_labels, |cell| cell.gap(px(2.)))
                .when(!has_day_labels, |cell| cell.justify_center());
            if is_in_range && range_start != range_end {
                cell = cell.child(
                    div()
                        .absolute()
                        .top(px(if has_day_labels { 0. } else { 4. }))
                        .h(px(32.))
                        .left(relative(if fills_before { 0. } else { 0.5 }))
                        .right(relative(if fills_after { 0. } else { 0.5 }))
                        .when(rounds_before, |ribbon| {
                            ribbon.rounded_tl(px(10.)).rounded_bl(px(10.))
                        })
                        .when(rounds_after, |ribbon| {
                            ribbon.rounded_tr(px(10.)).rounded_br(px(10.))
                        })
                        .bg(self.appearance.range_background),
                );
            }

            cell = cell.child(
                div()
                    .id((self.id.clone(), format!("day-{date}")))
                    .role(Role::Button)
                    .aria_label(self.locale.date_accessible_label(date))
                    .aria_selected(is_selected || is_in_range)
                    .size(px(32.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_full()
                    .text_sm()
                    .font_weight(if is_selected {
                        FontWeight::SEMIBOLD
                    } else {
                        FontWeight::NORMAL
                    })
                    .text_color(if is_selected {
                        self.appearance.selected_foreground
                    } else if !enabled {
                        self.appearance.disabled_foreground
                    } else if outside {
                        self.appearance.muted_foreground
                    } else {
                        rgb(0x172033).into()
                    })
                    .when(is_selected, |day| {
                        day.bg(self.appearance.selected_background).shadow_sm()
                    })
                    .when(date == self.today && !is_selected, |day| {
                        day.border_1().border_color(self.appearance.today_ring)
                    })
                    .when(enabled, |day| {
                        day.cursor_pointer()
                            .when(!is_selected, |day| {
                                day.hover(|style| style.bg(self.appearance.hover_background))
                            })
                            .on_click(move |_, _, cx| {
                                state.update(cx, |state, cx| {
                                    state.select_date_with_mode(date, selection_mode, cx);
                                });
                            })
                    })
                    .children(content)
                    .when(self.day_renderer.is_none() && visible_day, |day| {
                        day.child(self.locale.day_number_text(date))
                    }),
            );
            if let Some(label) = label
                && visible_day
                && !outside
            {
                cell = cell.child(
                    div()
                        .max_w_full()
                        .h(px(14.))
                        .px(px(1.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .truncate()
                        .text_size(px(9.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(if enabled {
                            label.color.unwrap_or(self.appearance.label_foreground)
                        } else {
                            self.appearance.disabled_foreground
                        })
                        .child(label.text),
                );
            }
            days = days.child(cell);
        }

        let body_height = if has_day_labels { 356. } else { 272. };
        let day_body = div()
            .h(px(body_height))
            .child(weekday_header)
            .child(days)
            .into_any_element();

        let mut months = div().h(px(body_height)).p_2().grid().grid_cols(3).gap_2();
        for month_number in 1..=12 {
            let candidate = YearMonth::new(month.year, month_number);
            let range = candidate.range();
            let enabled = self
                .min_date
                .is_none_or(|minimum| minimum < range.end_exclusive)
                && self.max_date.is_none_or(|maximum| maximum >= range.start);
            let active = selected_reference
                .is_some_and(|date| date.year() == month.year && date.month() == month_number);
            let is_current = self.today.year() == month.year && self.today.month() == month_number;
            let target = date_in_year_month(month.year, month_number, anchor.day());
            let state = self.state.clone();
            months = months.child(
                div()
                    .id((self.id.clone(), format!("month-{month_number}")))
                    .role(Role::Button)
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(11.))
                    .text_sm()
                    .font_weight(if active {
                        FontWeight::SEMIBOLD
                    } else {
                        FontWeight::NORMAL
                    })
                    .text_color(if active {
                        self.appearance.selected_foreground
                    } else if enabled {
                        rgb(0x172033).into()
                    } else {
                        self.appearance.disabled_foreground
                    })
                    .when(active, |item| {
                        item.bg(self.appearance.selected_background).shadow_sm()
                    })
                    .when(is_current && !active, |item| {
                        item.border_1().border_color(self.appearance.today_ring)
                    })
                    .when(enabled, |item| {
                        item.cursor_pointer()
                            .when(!active, |item| {
                                item.hover(|style| style.bg(self.appearance.hover_background))
                            })
                            .on_click(move |_, _, cx| {
                                state.update(cx, |state, cx| {
                                    state.go_to(target, cx);
                                    state.set_date_picker_view(DatePickerView::Days, cx);
                                });
                            })
                    })
                    .child(self.locale.short_month_names[month_number as usize - 1].clone()),
            );
        }

        let first_year = year_page_start(month.year);
        let mut years = div().h(px(body_height)).p_2().grid().grid_cols(4).gap_2();
        for year in first_year..first_year + 16 {
            let enabled = self.min_date.is_none_or(|minimum| minimum.year() <= year)
                && self.max_date.is_none_or(|maximum| maximum.year() >= year);
            let active = selected_reference.is_some_and(|date| date.year() == year);
            let is_current = self.today.year() == year;
            let target = date_in_year_month(year, anchor.month(), anchor.day());
            let state = self.state.clone();
            years = years.child(
                div()
                    .id((self.id.clone(), format!("year-{year}")))
                    .role(Role::Button)
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(11.))
                    .text_sm()
                    .font_weight(if active {
                        FontWeight::SEMIBOLD
                    } else {
                        FontWeight::NORMAL
                    })
                    .text_color(if active {
                        self.appearance.selected_foreground
                    } else if enabled {
                        rgb(0x172033).into()
                    } else {
                        self.appearance.disabled_foreground
                    })
                    .when(active, |item| {
                        item.bg(self.appearance.selected_background).shadow_sm()
                    })
                    .when(is_current && !active, |item| {
                        item.border_1().border_color(self.appearance.today_ring)
                    })
                    .when(enabled, |item| {
                        item.cursor_pointer()
                            .when(!active, |item| {
                                item.hover(|style| style.bg(self.appearance.hover_background))
                            })
                            .on_click(move |_, _, cx| {
                                state.update(cx, |state, cx| {
                                    state.go_to(target, cx);
                                    state.set_date_picker_view(DatePickerView::Days, cx);
                                });
                            })
                    })
                    .child(year.to_string()),
            );
        }

        let body = match picker_view {
            DatePickerView::Days => day_body,
            DatePickerView::Months => months.into_any_element(),
            DatePickerView::Years => years.into_any_element(),
        };

        let mut root = div()
            .id(self.id)
            .debug_selector(|| "uic-calendar-date-picker".to_string())
            .text_color(rgb(0x172033))
            .child(header)
            .child(body);
        root.style().refine(&self.style);
        root
    }
}

impl Styled for CalendarDatePicker {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
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

fn year_page_start(year: i32) -> i32 {
    year.div_euclid(16) * 16
}

fn shift_years(date: NaiveDate, years: i32) -> NaiveDate {
    date_in_year_month(date.year() + years, date.month(), date.day())
}

fn date_in_year_month(year: i32, month: u32, preferred_day: u32) -> NaiveDate {
    let month = YearMonth::new(year, month);
    NaiveDate::from_ymd_opt(year, month.month, preferred_day.min(month.days()))
        .expect("clamped picker date")
}
