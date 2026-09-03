use chrono::{Datelike as _, Duration, NaiveDate};
use gpui::{Context, SharedString};
use std::sync::Arc;

use super::{
    CalendarClockRange, CalendarTimeSelection, CalendarTimeSelectionMergePolicy, DateRange,
    YearMonth,
    date::{month_grid_range, week_range, week_start},
};

/// The active calendar projection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CalendarView {
    Year,
    Month,
    Week,
    Day,
}

/// Date selection is stateful but independent from event selection.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum CalendarSelection {
    #[default]
    None,
    Single(Option<NaiveDate>),
    Range {
        start: Option<NaiveDate>,
        end: Option<NaiveDate>,
    },
}

/// How activating a date updates [`CalendarSelection`].
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum CalendarSelectionMode {
    #[default]
    Single,
    Range,
}

/// The active panel inside a compact [`super::CalendarDatePicker`].
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum DatePickerView {
    #[default]
    Days,
    Months,
    Years,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CalendarStateEvent {
    ViewChanged(CalendarView),
    AnchorChanged(NaiveDate),
    SelectionChanged(CalendarSelection),
    TimeSelectionsChanged(Arc<[CalendarTimeSelection]>),
    TimeRangeExpansionChanged {
        range: CalendarClockRange,
        expanded: bool,
    },
    EventSelected(Option<SharedString>),
    DatePickerViewChanged(DatePickerView),
}

pub struct CalendarState {
    anchor_date: NaiveDate,
    focused_date: NaiveDate,
    view: CalendarView,
    selection: CalendarSelection,
    time_selections: Vec<CalendarTimeSelection>,
    time_selection_preview: Option<CalendarTimeSelection>,
    expanded_time_ranges: Vec<CalendarClockRange>,
    selected_event: Option<SharedString>,
    date_picker_view: DatePickerView,
}

impl gpui::EventEmitter<CalendarStateEvent> for CalendarState {}

impl CalendarState {
    pub fn new(anchor_date: NaiveDate, view: CalendarView) -> Self {
        Self {
            anchor_date,
            focused_date: anchor_date,
            view,
            selection: CalendarSelection::None,
            time_selections: Vec::new(),
            time_selection_preview: None,
            expanded_time_ranges: Vec::new(),
            selected_event: None,
            date_picker_view: DatePickerView::Days,
        }
    }

    pub fn anchor_date(&self) -> NaiveDate {
        self.anchor_date
    }
    pub fn focused_date(&self) -> NaiveDate {
        self.focused_date
    }
    pub fn view(&self) -> CalendarView {
        self.view
    }
    pub fn selection(&self) -> &CalendarSelection {
        &self.selection
    }
    pub fn selected_event(&self) -> Option<&SharedString> {
        self.selected_event.as_ref()
    }
    pub fn date_picker_view(&self) -> DatePickerView {
        self.date_picker_view
    }
    pub fn time_selections(&self) -> &[CalendarTimeSelection] {
        &self.time_selections
    }
    pub(crate) fn time_selections_with_preview(
        &self,
        merge_policy: CalendarTimeSelectionMergePolicy,
    ) -> Vec<CalendarTimeSelection> {
        normalized_time_selections(
            self.time_selections
                .iter()
                .cloned()
                .chain(self.time_selection_preview.iter().cloned()),
            merge_policy,
        )
    }
    pub fn expanded_time_ranges(&self) -> &[CalendarClockRange] {
        &self.expanded_time_ranges
    }

    /// Returns the date range a data source should populate for the active view.
    pub fn visible_range(&self, first_weekday: chrono::Weekday) -> DateRange {
        self.visible_range_for(self.view, first_weekday)
    }

    /// Returns the data range for an explicitly composed projection.
    pub fn visible_range_for(
        &self,
        view: CalendarView,
        first_weekday: chrono::Weekday,
    ) -> DateRange {
        match view {
            CalendarView::Year => DateRange::new(
                NaiveDate::from_ymd_opt(self.anchor_date.year(), 1, 1).expect("valid year"),
                NaiveDate::from_ymd_opt(self.anchor_date.year() + 1, 1, 1).expect("valid year"),
            ),
            CalendarView::Month => {
                month_grid_range(YearMonth::from_date(self.anchor_date), first_weekday)
            }
            CalendarView::Week => week_range(self.anchor_date, first_weekday),
            CalendarView::Day => DateRange::single(self.anchor_date),
        }
    }

    pub fn set_view(&mut self, view: CalendarView, cx: &mut Context<Self>) {
        if self.view == view {
            return;
        }
        self.view = view;
        cx.emit(CalendarStateEvent::ViewChanged(view));
        cx.notify();
    }

    pub fn go_to(&mut self, date: NaiveDate, cx: &mut Context<Self>) {
        if self.anchor_date == date && self.focused_date == date {
            return;
        }
        self.anchor_date = date;
        self.focused_date = date;
        cx.emit(CalendarStateEvent::AnchorChanged(date));
        cx.notify();
    }

    pub fn previous(&mut self, first_weekday: chrono::Weekday, cx: &mut Context<Self>) {
        self.previous_in(self.view, first_weekday, cx);
    }

    pub fn previous_in(
        &mut self,
        view: CalendarView,
        first_weekday: chrono::Weekday,
        cx: &mut Context<Self>,
    ) {
        let date = match view {
            CalendarView::Year => self
                .anchor_date
                .with_year(self.anchor_date.year() - 1)
                .unwrap_or(self.anchor_date),
            CalendarView::Month => shift_month(self.anchor_date, -1),
            CalendarView::Week => week_start(self.anchor_date, first_weekday) - Duration::days(7),
            CalendarView::Day => self.anchor_date - Duration::days(1),
        };
        self.go_to(date, cx);
    }

    pub fn next(&mut self, first_weekday: chrono::Weekday, cx: &mut Context<Self>) {
        self.next_in(self.view, first_weekday, cx);
    }

    pub fn next_in(
        &mut self,
        view: CalendarView,
        first_weekday: chrono::Weekday,
        cx: &mut Context<Self>,
    ) {
        let date = match view {
            CalendarView::Year => self
                .anchor_date
                .with_year(self.anchor_date.year() + 1)
                .unwrap_or(self.anchor_date),
            CalendarView::Month => shift_month(self.anchor_date, 1),
            CalendarView::Week => week_start(self.anchor_date, first_weekday) + Duration::days(7),
            CalendarView::Day => self.anchor_date + Duration::days(1),
        };
        self.go_to(date, cx);
    }

    pub fn select_date(&mut self, date: NaiveDate, cx: &mut Context<Self>) {
        self.anchor_date = date;
        self.focused_date = date;
        self.selection = match self.selection {
            CalendarSelection::Range {
                start: Some(start),
                end: None,
            } if start != date => CalendarSelection::Range {
                start: Some(start.min(date)),
                end: Some(start.max(date)),
            },
            CalendarSelection::Range { .. } => CalendarSelection::Range {
                start: Some(date),
                end: None,
            },
            _ => CalendarSelection::Single(Some(date)),
        };
        cx.emit(CalendarStateEvent::SelectionChanged(self.selection.clone()));
        cx.notify();
    }

    /// Selects a date using an explicit interaction mode without navigating
    /// the active calendar projection.
    ///
    /// Range mode starts a new range after the previous range is complete. Its
    /// second activation completes the range in either chronological order.
    pub fn select_date_with_mode(
        &mut self,
        date: NaiveDate,
        mode: CalendarSelectionMode,
        cx: &mut Context<Self>,
    ) {
        self.focused_date = date;
        self.selection = selection_after_date(&self.selection, date, mode);
        cx.emit(CalendarStateEvent::SelectionChanged(self.selection.clone()));
        cx.notify();
    }

    pub fn set_selection(&mut self, selection: CalendarSelection, cx: &mut Context<Self>) {
        if self.selection == selection {
            return;
        }
        self.selection = selection;
        cx.emit(CalendarStateEvent::SelectionChanged(self.selection.clone()));
        cx.notify();
    }

    pub fn set_date_picker_view(&mut self, view: DatePickerView, cx: &mut Context<Self>) {
        if self.date_picker_view == view {
            return;
        }
        self.date_picker_view = view;
        cx.emit(CalendarStateEvent::DatePickerViewChanged(view));
        cx.notify();
    }

    pub fn set_time_selections(
        &mut self,
        selections: impl IntoIterator<Item = CalendarTimeSelection>,
        cx: &mut Context<Self>,
    ) {
        self.set_time_selections_with_merge_policy(
            selections,
            CalendarTimeSelectionMergePolicy::SeparateAdjacent,
            cx,
        );
    }

    pub fn set_time_selections_with_merge_policy(
        &mut self,
        selections: impl IntoIterator<Item = CalendarTimeSelection>,
        merge_policy: CalendarTimeSelectionMergePolicy,
        cx: &mut Context<Self>,
    ) {
        let selections = normalized_time_selections(selections, merge_policy);
        if self.time_selections == selections {
            return;
        }
        self.time_selections = selections;
        self.emit_time_selections(cx);
    }

    /// Toggles one precision-sized slot. The merge policy decides whether an
    /// adjacent slot joins an existing range. Activating an already selected
    /// range removes that whole range.
    pub fn toggle_time_selection(
        &mut self,
        selection: CalendarTimeSelection,
        precision_minutes: u32,
        max_slots_per_range: Option<usize>,
        merge_policy: CalendarTimeSelectionMergePolicy,
        cx: &mut Context<Self>,
    ) -> bool {
        if let Some(index) = self.time_selections.iter().position(|candidate| {
            candidate.start <= selection.start && candidate.end >= selection.end
        }) {
            self.time_selections.remove(index);
            self.emit_time_selections(cx);
            return true;
        }

        let mut next = self.time_selections.clone();
        next.push(selection);
        next = normalized_time_selections(next, merge_policy);
        if exceeds_slot_limit(&next, precision_minutes, max_slots_per_range) {
            return false;
        }
        self.time_selections = next;
        self.emit_time_selections(cx);
        true
    }

    pub fn clear_time_selections(&mut self, cx: &mut Context<Self>) {
        if self.time_selections.is_empty() && self.time_selection_preview.is_none() {
            return;
        }
        self.time_selections.clear();
        self.time_selection_preview = None;
        self.emit_time_selections(cx);
    }

    pub(crate) fn begin_time_selection(
        &mut self,
        selection: CalendarTimeSelection,
        cx: &mut Context<Self>,
    ) {
        self.time_selection_preview = Some(selection);
        cx.notify();
    }

    pub(crate) fn update_time_selection(
        &mut self,
        selection: CalendarTimeSelection,
        cx: &mut Context<Self>,
    ) {
        if self.time_selection_preview.as_ref() == Some(&selection) {
            return;
        }
        self.time_selection_preview = Some(selection);
        cx.notify();
    }

    pub(crate) fn commit_time_selection(
        &mut self,
        precision_minutes: u32,
        max_slots_per_range: Option<usize>,
        merge_policy: CalendarTimeSelectionMergePolicy,
        cx: &mut Context<Self>,
    ) {
        let Some(selection) = self.time_selection_preview.take() else {
            return;
        };
        let next = normalized_time_selections(
            self.time_selections
                .iter()
                .cloned()
                .chain(std::iter::once(selection)),
            merge_policy,
        );
        if exceeds_slot_limit(&next, precision_minutes, max_slots_per_range) {
            cx.notify();
            return;
        }
        if next == self.time_selections {
            cx.notify();
            return;
        }
        self.time_selections = next;
        self.emit_time_selections(cx);
    }

    fn emit_time_selections(&self, cx: &mut Context<Self>) {
        cx.emit(CalendarStateEvent::TimeSelectionsChanged(
            self.time_selections.clone().into(),
        ));
        cx.notify();
    }

    pub fn expand_time_range(&mut self, range: CalendarClockRange, cx: &mut Context<Self>) {
        if self.expanded_time_ranges.contains(&range) {
            return;
        }
        self.expanded_time_ranges.push(range);
        cx.emit(CalendarStateEvent::TimeRangeExpansionChanged {
            range,
            expanded: true,
        });
        cx.notify();
    }

    pub fn collapse_time_range(&mut self, range: CalendarClockRange, cx: &mut Context<Self>) {
        let previous_len = self.expanded_time_ranges.len();
        self.expanded_time_ranges
            .retain(|candidate| *candidate != range);
        if self.expanded_time_ranges.len() == previous_len {
            return;
        }
        cx.emit(CalendarStateEvent::TimeRangeExpansionChanged {
            range,
            expanded: false,
        });
        cx.notify();
    }

    pub fn select_event(&mut self, id: Option<SharedString>, cx: &mut Context<Self>) {
        if self.selected_event == id {
            return;
        }
        self.selected_event = id.clone();
        cx.emit(CalendarStateEvent::EventSelected(id));
        cx.notify();
    }
}

fn normalized_time_selections(
    selections: impl IntoIterator<Item = CalendarTimeSelection>,
    merge_policy: CalendarTimeSelectionMergePolicy,
) -> Vec<CalendarTimeSelection> {
    let mut selections: Vec<_> = selections
        .into_iter()
        .filter(|selection| selection.start < selection.end)
        .collect();
    selections.sort_by_key(|selection| selection.start);
    let mut normalized: Vec<CalendarTimeSelection> = Vec::with_capacity(selections.len());
    for selection in selections {
        if let Some(previous) = normalized.last_mut()
            && (selection.start < previous.end
                || (merge_policy == CalendarTimeSelectionMergePolicy::MergeAdjacent
                    && selection.start == previous.end))
        {
            previous.end = previous.end.max(selection.end);
        } else {
            normalized.push(selection);
        }
    }
    normalized
}

fn exceeds_slot_limit(
    selections: &[CalendarTimeSelection],
    precision_minutes: u32,
    maximum: Option<usize>,
) -> bool {
    let Some(maximum) = maximum else {
        return false;
    };
    let maximum_minutes =
        i64::try_from(maximum.saturating_mul(precision_minutes as usize)).unwrap_or(i64::MAX);
    selections
        .iter()
        .any(|selection| (selection.end - selection.start).num_minutes() > maximum_minutes)
}

fn shift_month(date: NaiveDate, delta: i32) -> NaiveDate {
    let month = YearMonth::from_date(date).add_months(delta);
    NaiveDate::from_ymd_opt(month.year, month.month, date.day().min(month.days()))
        .expect("clamped month date")
}

fn selection_after_date(
    selection: &CalendarSelection,
    date: NaiveDate,
    mode: CalendarSelectionMode,
) -> CalendarSelection {
    match mode {
        CalendarSelectionMode::Single => CalendarSelection::Single(Some(date)),
        CalendarSelectionMode::Range => match selection {
            CalendarSelection::Range {
                start: Some(start),
                end: None,
            } => CalendarSelection::Range {
                start: Some((*start).min(date)),
                end: Some((*start).max(date)),
            },
            _ => CalendarSelection::Range {
                start: Some(date),
                end: None,
            },
        },
    }
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone as _, Utc};

    use super::*;

    fn selection(start_minute: u32, end_minute: u32) -> CalendarTimeSelection {
        let midnight = Utc
            .with_ymd_and_hms(2026, 9, 3, 0, 0, 0)
            .single()
            .expect("valid test date");
        CalendarTimeSelection {
            start: midnight + Duration::minutes(i64::from(start_minute)),
            end: midnight + Duration::minutes(i64::from(end_minute)),
        }
    }

    #[test]
    fn slot_limit_applies_per_continuous_range() {
        let separate = [selection(0, 15), selection(30, 45)];
        assert!(!exceeds_slot_limit(&separate, 15, Some(1)));

        let joined = normalized_time_selections(
            [selection(0, 15), selection(15, 30)],
            CalendarTimeSelectionMergePolicy::MergeAdjacent,
        );
        assert!(exceeds_slot_limit(&joined, 15, Some(1)));
    }

    #[test]
    fn adjacent_selection_merging_is_explicit() {
        let selections = [selection(0, 15), selection(15, 30)];
        assert_eq!(
            normalized_time_selections(
                selections.clone(),
                CalendarTimeSelectionMergePolicy::SeparateAdjacent,
            )
            .len(),
            2
        );
        assert_eq!(
            normalized_time_selections(
                selections,
                CalendarTimeSelectionMergePolicy::MergeAdjacent,
            )
            .len(),
            1
        );
    }

    #[test]
    fn explicit_date_selection_mode_controls_single_and_range_progression() {
        let start = NaiveDate::from_ymd_opt(2026, 11, 7).unwrap();
        let end = NaiveDate::from_ymd_opt(2026, 10, 28).unwrap();

        let single = selection_after_date(
            &CalendarSelection::None,
            start,
            CalendarSelectionMode::Single,
        );
        assert_eq!(single, CalendarSelection::Single(Some(start)));

        let pending = selection_after_date(&single, start, CalendarSelectionMode::Range);
        assert_eq!(
            pending,
            CalendarSelection::Range {
                start: Some(start),
                end: None,
            }
        );

        let completed = selection_after_date(&pending, end, CalendarSelectionMode::Range);
        assert_eq!(
            completed,
            CalendarSelection::Range {
                start: Some(end),
                end: Some(start),
            }
        );
    }

    #[test]
    fn completed_range_restarts_and_same_day_can_complete_a_range() {
        let date = NaiveDate::from_ymd_opt(2026, 9, 12).unwrap();
        let pending =
            selection_after_date(&CalendarSelection::None, date, CalendarSelectionMode::Range);
        let completed = selection_after_date(&pending, date, CalendarSelectionMode::Range);
        assert_eq!(
            completed,
            CalendarSelection::Range {
                start: Some(date),
                end: Some(date),
            }
        );

        let restarted = selection_after_date(&completed, date, CalendarSelectionMode::Range);
        assert_eq!(restarted, pending);
    }
}
