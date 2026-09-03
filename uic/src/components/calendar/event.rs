use std::sync::Arc;

use chrono::{DateTime, Duration, FixedOffset, NaiveDate, NaiveTime, Utc};
use gpui::{Hsla, SharedString};

use super::DateRange;

/// Time semantics for an event. All-day end dates are exclusive.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CalendarEventTime {
    AllDay {
        start: NaiveDate,
        end_exclusive: NaiveDate,
    },
    Timed {
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    },
}

impl CalendarEventTime {
    pub fn all_day(start: NaiveDate, end_exclusive: NaiveDate) -> Self {
        assert!(
            start < end_exclusive,
            "all-day event must span at least one day"
        );
        Self::AllDay {
            start,
            end_exclusive,
        }
    }

    pub fn timed(start: DateTime<Utc>, end: DateTime<Utc>) -> Self {
        assert!(start < end, "timed event end must follow its start");
        Self::Timed { start, end }
    }

    pub fn date_range(self, offset: FixedOffset) -> DateRange {
        match self {
            Self::AllDay {
                start,
                end_exclusive,
            } => DateRange::new(start, end_exclusive),
            Self::Timed { start, end } => {
                let local_start = start.with_timezone(&offset);
                let local_end = end.with_timezone(&offset);
                let end_exclusive = if local_end.time() == NaiveTime::MIN {
                    local_end.date_naive()
                } else {
                    local_end.date_naive() + Duration::days(1)
                };
                DateRange::new(local_start.date_naive(), end_exclusive)
            }
        }
    }

    pub fn is_all_day(self) -> bool {
        matches!(self, Self::AllDay { .. })
    }
}

/// Per-event participation in visual merging.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum CalendarEventMerge {
    #[default]
    Inherit,
    Never,
    Key(SharedString),
}

/// Calendar-wide multi-day merge behavior.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum CalendarMergePolicy {
    /// Split every event into independent day chips.
    None,
    /// Keep explicit multi-day ranges connected, but do not join separate events.
    #[default]
    ExplicitOnly,
    /// Also join adjacent or overlapping all-day events that share a merge key.
    AdjacentByKey,
}

/// Which visible segment of a multi-week event receives its label.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum CalendarSegmentLabelPolicy {
    #[default]
    Each,
    First,
    Longest,
}

/// Application event data. `data` is kept behind an [`Arc`] so view changes are cheap.
pub struct CalendarEvent<T = ()> {
    pub id: SharedString,
    pub title: SharedString,
    pub time: CalendarEventTime,
    pub color: Option<Hsla>,
    pub data: Arc<T>,
    pub merge: CalendarEventMerge,
}

impl<T> Clone for CalendarEvent<T> {
    fn clone(&self) -> Self {
        Self {
            id: self.id.clone(),
            title: self.title.clone(),
            time: self.time,
            color: self.color,
            data: self.data.clone(),
            merge: self.merge.clone(),
        }
    }
}

impl<T> CalendarEvent<T> {
    pub fn all_day(
        id: impl Into<SharedString>,
        title: impl Into<SharedString>,
        date: NaiveDate,
        data: T,
    ) -> Self {
        Self::all_day_range(id, title, date, date + Duration::days(1), data)
    }

    pub fn all_day_range(
        id: impl Into<SharedString>,
        title: impl Into<SharedString>,
        start: NaiveDate,
        end_exclusive: NaiveDate,
        data: T,
    ) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            time: CalendarEventTime::all_day(start, end_exclusive),
            color: None,
            data: Arc::new(data),
            merge: CalendarEventMerge::Inherit,
        }
    }

    pub fn timed(
        id: impl Into<SharedString>,
        title: impl Into<SharedString>,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
        data: T,
    ) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            time: CalendarEventTime::timed(start, end),
            color: None,
            data: Arc::new(data),
            merge: CalendarEventMerge::Inherit,
        }
    }

    pub fn color(mut self, color: Hsla) -> Self {
        self.color = Some(color);
        self
    }

    pub fn merge(mut self, merge: CalendarEventMerge) -> Self {
        self.merge = merge;
        self
    }

    pub fn date_range(&self, offset: FixedOffset) -> DateRange {
        self.time.date_range(offset)
    }
}
