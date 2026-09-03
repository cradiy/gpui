use chrono::{Datelike as _, Duration, NaiveDate, Weekday};

/// A half-open range of civil dates: `[start, end_exclusive)`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DateRange {
    pub start: NaiveDate,
    pub end_exclusive: NaiveDate,
}

impl DateRange {
    pub fn new(start: NaiveDate, end_exclusive: NaiveDate) -> Self {
        assert!(start <= end_exclusive, "date range must not be inverted");
        Self {
            start,
            end_exclusive,
        }
    }

    pub fn single(date: NaiveDate) -> Self {
        Self::new(date, date + Duration::days(1))
    }

    pub fn contains(&self, date: NaiveDate) -> bool {
        self.start <= date && date < self.end_exclusive
    }

    pub fn intersects(&self, other: Self) -> bool {
        self.start < other.end_exclusive && other.start < self.end_exclusive
    }

    pub fn intersection(&self, other: Self) -> Option<Self> {
        let start = self.start.max(other.start);
        let end_exclusive = self.end_exclusive.min(other.end_exclusive);
        (start < end_exclusive).then(|| Self::new(start, end_exclusive))
    }

    pub fn days(&self) -> i64 {
        (self.end_exclusive - self.start).num_days()
    }
}

/// A calendar year and month independent of a day.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct YearMonth {
    pub year: i32,
    pub month: u32,
}

impl YearMonth {
    pub fn new(year: i32, month: u32) -> Self {
        assert!((1..=12).contains(&month), "month must be between 1 and 12");
        Self { year, month }
    }

    pub fn from_date(date: NaiveDate) -> Self {
        Self::new(date.year(), date.month())
    }

    pub fn first_day(self) -> NaiveDate {
        NaiveDate::from_ymd_opt(self.year, self.month, 1).expect("valid year-month")
    }

    pub fn days(self) -> u32 {
        let next = self.add_months(1).first_day();
        (next - self.first_day()).num_days() as u32
    }

    pub fn add_months(self, delta: i32) -> Self {
        let absolute = self.year * 12 + self.month as i32 - 1 + delta;
        Self::new(absolute.div_euclid(12), absolute.rem_euclid(12) as u32 + 1)
    }

    pub fn range(self) -> DateRange {
        DateRange::new(self.first_day(), self.add_months(1).first_day())
    }
}

pub(crate) fn week_start(date: NaiveDate, first_weekday: Weekday) -> NaiveDate {
    let date_index = date.weekday().num_days_from_monday() as i64;
    let first_index = first_weekday.num_days_from_monday() as i64;
    date - Duration::days((date_index - first_index).rem_euclid(7))
}

pub(crate) fn month_grid_range(month: YearMonth, first_weekday: Weekday) -> DateRange {
    let start = week_start(month.first_day(), first_weekday);
    DateRange::new(start, start + Duration::days(42))
}

pub(crate) fn week_range(date: NaiveDate, first_weekday: Weekday) -> DateRange {
    let start = week_start(date, first_weekday);
    DateRange::new(start, start + Duration::days(7))
}
