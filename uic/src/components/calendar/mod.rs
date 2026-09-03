//! A data-driven event calendar with year, month, week, and day views.
//!
//! Applications own event loading, editing, persistence, and composition.
//! [`YearCalendar`], [`MonthCalendar`], [`WeekCalendar`], and [`DayCalendar`]
//! are independent projections. [`CalendarPager`], [`CalendarTitle`],
//! [`CalendarTodayButton`], and [`CalendarViewSwitcher`] are independent
//! controls, so an application can arrange and switch them without adopting a
//! predefined shell.

mod calendar;
mod controls;
mod date;
mod date_picker;
mod event;
mod layout;
mod locale;
mod projections;
mod state;
mod time;
mod timeline_interaction;

pub use calendar::{CalendarAppearance, DayRenderContext, EventRenderContext};
pub use controls::{CalendarPager, CalendarTitle, CalendarTodayButton, CalendarViewSwitcher};
pub use date::{DateRange, YearMonth};
pub use date_picker::{
    CalendarDatePicker, DatePickerAppearance, DatePickerDayContext, DatePickerDayLabel,
};
pub use event::{
    CalendarEvent, CalendarEventMerge, CalendarEventTime, CalendarMergePolicy,
    CalendarSegmentLabelPolicy,
};
pub use locale::{CalendarLocale, CalendarLocaleLabels};
pub use projections::{DayCalendar, MonthCalendar, WeekCalendar, YearCalendar};
pub use state::{
    CalendarSelection, CalendarSelectionMode, CalendarState, CalendarStateEvent, CalendarView,
    DatePickerView,
};
pub use time::{CalendarClockRange, CalendarTimeSelection, CalendarTimeSelectionMergePolicy};
