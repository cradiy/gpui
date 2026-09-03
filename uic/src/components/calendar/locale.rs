use std::sync::Arc;

use chrono::{Datelike as _, NaiveDate, Weekday};
use gpui::SharedString;

use super::{CalendarView, DateRange, YearMonth};

type DateFormatter = Arc<dyn Fn(NaiveDate) -> SharedString + Send + Sync>;
type MonthFormatter = Arc<dyn Fn(YearMonth) -> SharedString + Send + Sync>;
type RangeFormatter = Arc<dyn Fn(DateRange) -> SharedString + Send + Sync>;
type MoreFormatter = Arc<dyn Fn(usize) -> SharedString + Send + Sync>;
type HourFormatter = Arc<dyn Fn(u32) -> SharedString + Send + Sync>;
type ClockFormatter = Arc<dyn Fn(u32) -> SharedString + Send + Sync>;

/// Every short piece of UI text emitted by calendar components.
#[derive(Clone, Debug)]
pub struct CalendarLocaleLabels {
    pub year: SharedString,
    pub month: SharedString,
    pub week: SharedString,
    pub day: SharedString,
    pub agenda: SharedString,
    pub today: SharedString,
    pub all_day: SharedString,
    pub no_events: SharedString,
    pub previous: SharedString,
    pub next: SharedString,
    pub cancel: SharedString,
    pub expand_time_range: SharedString,
    pub collapse_time_range: SharedString,
}

impl Default for CalendarLocaleLabels {
    fn default() -> Self {
        Self {
            year: "Year".into(),
            month: "Month".into(),
            week: "Week".into(),
            day: "Day".into(),
            agenda: "Agenda".into(),
            today: "Today".into(),
            all_day: "All day".into(),
            no_events: "No events".into(),
            previous: "Previous".into(),
            next: "Next".into(),
            cancel: "Cancel".into(),
            expand_time_range: "Expand time range".into(),
            collapse_time_range: "Collapse time range".into(),
        }
    }
}

/// Localized names and formatters. The default is deterministic English.
///
/// Applications can replace individual labels, name arrays, or formatter
/// closures. The calendar never reads process-global locale state.
#[derive(Clone)]
pub struct CalendarLocale {
    pub labels: CalendarLocaleLabels,
    pub month_names: Arc<[SharedString; 12]>,
    pub short_month_names: Arc<[SharedString; 12]>,
    pub weekday_names: Arc<[SharedString; 7]>,
    pub short_weekday_names: Arc<[SharedString; 7]>,
    pub first_weekday: Weekday,
    year_title: DateFormatter,
    month_title: MonthFormatter,
    week_title: RangeFormatter,
    day_title: DateFormatter,
    day_number: DateFormatter,
    date_label: DateFormatter,
    more_label: MoreFormatter,
    hour_label: HourFormatter,
    clock_label: ClockFormatter,
}

impl Default for CalendarLocale {
    fn default() -> Self {
        let months: Arc<[SharedString; 12]> = Arc::new([
            "January".into(),
            "February".into(),
            "March".into(),
            "April".into(),
            "May".into(),
            "June".into(),
            "July".into(),
            "August".into(),
            "September".into(),
            "October".into(),
            "November".into(),
            "December".into(),
        ]);
        let short_months: Arc<[SharedString; 12]> = Arc::new([
            "Jan".into(),
            "Feb".into(),
            "Mar".into(),
            "Apr".into(),
            "May".into(),
            "Jun".into(),
            "Jul".into(),
            "Aug".into(),
            "Sep".into(),
            "Oct".into(),
            "Nov".into(),
            "Dec".into(),
        ]);
        let weekdays: Arc<[SharedString; 7]> = Arc::new([
            "Monday".into(),
            "Tuesday".into(),
            "Wednesday".into(),
            "Thursday".into(),
            "Friday".into(),
            "Saturday".into(),
            "Sunday".into(),
        ]);
        let short_weekdays: Arc<[SharedString; 7]> = Arc::new([
            "Mon".into(),
            "Tue".into(),
            "Wed".into(),
            "Thu".into(),
            "Fri".into(),
            "Sat".into(),
            "Sun".into(),
        ]);

        let month_titles = months.clone();
        let week_months = short_months.clone();
        let day_months = months.clone();
        let day_weekdays = weekdays.clone();
        let label_months = months.clone();
        let label_weekdays = weekdays.clone();

        Self {
            labels: CalendarLocaleLabels::default(),
            month_names: months,
            short_month_names: short_months,
            weekday_names: weekdays,
            short_weekday_names: short_weekdays,
            first_weekday: Weekday::Sun,
            year_title: Arc::new(|date| date.year().to_string().into()),
            month_title: Arc::new(move |month| {
                format!("{} {}", month_titles[month.month as usize - 1], month.year).into()
            }),
            week_title: Arc::new(move |range| {
                let end = range.end_exclusive.pred_opt().unwrap_or(range.start);
                if range.start.year() == end.year() && range.start.month() == end.month() {
                    format!(
                        "{} {}–{}, {}",
                        week_months[range.start.month0() as usize],
                        range.start.day(),
                        end.day(),
                        end.year()
                    )
                    .into()
                } else {
                    format!(
                        "{} {} – {} {}, {}",
                        week_months[range.start.month0() as usize],
                        range.start.day(),
                        week_months[end.month0() as usize],
                        end.day(),
                        end.year()
                    )
                    .into()
                }
            }),
            day_title: Arc::new(move |date| {
                format!(
                    "{}, {} {}, {}",
                    day_weekdays[date.weekday().num_days_from_monday() as usize],
                    day_months[date.month0() as usize],
                    date.day(),
                    date.year()
                )
                .into()
            }),
            day_number: Arc::new(|date| date.day().to_string().into()),
            date_label: Arc::new(move |date| {
                format!(
                    "{}, {} {}, {}",
                    label_weekdays[date.weekday().num_days_from_monday() as usize],
                    label_months[date.month0() as usize],
                    date.day(),
                    date.year()
                )
                .into()
            }),
            more_label: Arc::new(|count| format!("+{count} more").into()),
            hour_label: Arc::new(|hour| format!("{hour:02}:00").into()),
            clock_label: Arc::new(|minute| format!("{:02}:{:02}", minute / 60, minute % 60).into()),
        }
    }
}

impl CalendarLocale {
    pub fn labels(mut self, labels: CalendarLocaleLabels) -> Self {
        self.labels = labels;
        self
    }

    pub fn first_weekday(mut self, weekday: Weekday) -> Self {
        self.first_weekday = weekday;
        self
    }

    pub fn year_title(
        mut self,
        formatter: impl Fn(NaiveDate) -> SharedString + Send + Sync + 'static,
    ) -> Self {
        self.year_title = Arc::new(formatter);
        self
    }

    pub fn month_title(
        mut self,
        formatter: impl Fn(YearMonth) -> SharedString + Send + Sync + 'static,
    ) -> Self {
        self.month_title = Arc::new(formatter);
        self
    }

    pub fn week_title(
        mut self,
        formatter: impl Fn(DateRange) -> SharedString + Send + Sync + 'static,
    ) -> Self {
        self.week_title = Arc::new(formatter);
        self
    }

    pub fn day_title(
        mut self,
        formatter: impl Fn(NaiveDate) -> SharedString + Send + Sync + 'static,
    ) -> Self {
        self.day_title = Arc::new(formatter);
        self
    }

    pub fn day_number(
        mut self,
        formatter: impl Fn(NaiveDate) -> SharedString + Send + Sync + 'static,
    ) -> Self {
        self.day_number = Arc::new(formatter);
        self
    }

    pub fn date_label(
        mut self,
        formatter: impl Fn(NaiveDate) -> SharedString + Send + Sync + 'static,
    ) -> Self {
        self.date_label = Arc::new(formatter);
        self
    }

    pub fn more_label(
        mut self,
        formatter: impl Fn(usize) -> SharedString + Send + Sync + 'static,
    ) -> Self {
        self.more_label = Arc::new(formatter);
        self
    }

    pub fn hour_label(
        mut self,
        formatter: impl Fn(u32) -> SharedString + Send + Sync + 'static,
    ) -> Self {
        self.hour_label = Arc::new(formatter);
        self
    }

    pub fn clock_label(
        mut self,
        formatter: impl Fn(u32) -> SharedString + Send + Sync + 'static,
    ) -> Self {
        self.clock_label = Arc::new(formatter);
        self
    }

    pub fn title(&self, view: CalendarView, anchor: NaiveDate, visible: DateRange) -> SharedString {
        match view {
            CalendarView::Year => (self.year_title)(anchor),
            CalendarView::Month => (self.month_title)(YearMonth::from_date(anchor)),
            CalendarView::Week => (self.week_title)(visible),
            CalendarView::Day => (self.day_title)(anchor),
        }
    }

    pub fn view_label(&self, view: CalendarView) -> SharedString {
        match view {
            CalendarView::Year => self.labels.year.clone(),
            CalendarView::Month => self.labels.month.clone(),
            CalendarView::Week => self.labels.week.clone(),
            CalendarView::Day => self.labels.day.clone(),
        }
    }

    pub fn day_number_text(&self, date: NaiveDate) -> SharedString {
        (self.day_number)(date)
    }

    pub fn date_accessible_label(&self, date: NaiveDate) -> SharedString {
        (self.date_label)(date)
    }

    pub fn more_text(&self, count: usize) -> SharedString {
        (self.more_label)(count)
    }

    pub fn hour_text(&self, hour: u32) -> SharedString {
        (self.hour_label)(hour)
    }

    pub fn clock_text(&self, minute: u32) -> SharedString {
        (self.clock_label)(minute)
    }

    pub fn weekday_name(&self, weekday: Weekday, short: bool) -> SharedString {
        let index = weekday.num_days_from_monday() as usize;
        if short {
            self.short_weekday_names[index].clone()
        } else {
            self.weekday_names[index].clone()
        }
    }
}
