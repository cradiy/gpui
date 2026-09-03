use chrono::{DateTime, Utc};

/// A recurring wall-clock range used to fold part of a day timeline.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct CalendarClockRange {
    pub start_minute: u16,
    pub end_minute: u16,
}

impl CalendarClockRange {
    pub fn new(start_hour: u8, start_minute: u8, end_hour: u8, end_minute: u8) -> Self {
        assert!(start_hour <= 24 && end_hour <= 24);
        assert!(start_minute < 60 && end_minute < 60);
        assert!(start_hour < 24 || start_minute == 0);
        assert!(end_hour < 24 || end_minute == 0);
        let start = u16::from(start_hour) * 60 + u16::from(start_minute);
        let end = u16::from(end_hour) * 60 + u16::from(end_minute);
        assert!(start < end);
        Self {
            start_minute: start,
            end_minute: end,
        }
    }

    pub fn hours(start_hour: u8, end_hour: u8) -> Self {
        Self::new(start_hour, 0, end_hour, 0)
    }
}

/// The UTC range selected from a week or day time grid.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CalendarTimeSelection {
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
}

/// Controls whether separately activated, adjacent time selections become one
/// logical range.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum CalendarTimeSelectionMergePolicy {
    /// Preserve adjacent selections as independently cancellable ranges.
    #[default]
    SeparateAdjacent,
    /// Join adjacent selections into one continuous range.
    MergeAdjacent,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct TimeAxisSegment {
    pub start_minute: u32,
    pub end_minute: u32,
    pub top: f32,
    pub height: f32,
    pub collapsed: bool,
}

#[derive(Clone, Debug)]
pub(super) struct TimeAxis {
    pub segments: Vec<TimeAxisSegment>,
    pub height: f32,
}

impl TimeAxis {
    pub fn new(
        start_minute: u32,
        end_minute: u32,
        collapsed: &[CalendarClockRange],
        hour_height: f32,
        collapsed_height: f32,
    ) -> Self {
        let mut ranges: Vec<(u32, u32)> = collapsed
            .iter()
            .filter_map(|range| {
                let start = u32::from(range.start_minute).max(start_minute);
                let end = u32::from(range.end_minute).min(end_minute);
                (start < end).then_some((start, end))
            })
            .collect();
        ranges.sort_unstable();
        let mut merged: Vec<(u32, u32)> = Vec::with_capacity(ranges.len());
        for (start, end) in ranges {
            if let Some((_, previous_end)) = merged.last_mut()
                && start <= *previous_end
            {
                *previous_end = (*previous_end).max(end);
            } else {
                merged.push((start, end));
            }
        }

        let mut segments = Vec::with_capacity(merged.len() * 2 + 1);
        let mut minute = start_minute;
        let mut top = 0.;
        for (start, end) in merged {
            if minute < start {
                let height = (start - minute) as f32 / 60. * hour_height;
                segments.push(TimeAxisSegment {
                    start_minute: minute,
                    end_minute: start,
                    top,
                    height,
                    collapsed: false,
                });
                top += height;
            }
            segments.push(TimeAxisSegment {
                start_minute: start,
                end_minute: end,
                top,
                height: collapsed_height,
                collapsed: true,
            });
            top += collapsed_height;
            minute = end;
        }
        if minute < end_minute {
            let height = (end_minute - minute) as f32 / 60. * hour_height;
            segments.push(TimeAxisSegment {
                start_minute: minute,
                end_minute,
                top,
                height,
                collapsed: false,
            });
            top += height;
        }
        Self {
            segments,
            height: top,
        }
    }

    pub fn y_for_minute(&self, minute: u32) -> f32 {
        let minute = minute.clamp(
            self.segments
                .first()
                .map_or(0, |segment| segment.start_minute),
            self.segments.last().map_or(0, |segment| segment.end_minute),
        );
        let segment = self
            .segments
            .iter()
            .find(|segment| minute <= segment.end_minute)
            .or_else(|| self.segments.last())
            .expect("time axis has a visible segment");
        let duration = (segment.end_minute - segment.start_minute).max(1) as f32;
        segment.top + (minute - segment.start_minute) as f32 / duration * segment.height
    }

    pub fn minute_for_y(&self, y: f32) -> Option<u32> {
        let y = y.clamp(0., self.height.max(0.));
        let segment = self
            .segments
            .iter()
            .find(|segment| y < segment.top + segment.height)
            .or_else(|| self.segments.last())?;
        if segment.collapsed {
            return None;
        }
        let ratio = ((y - segment.top) / segment.height.max(1.)).clamp(0., 1.);
        Some(
            segment.start_minute
                + (ratio * (segment.end_minute - segment.start_minute) as f32).floor() as u32,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collapsed_ranges_compress_the_axis_and_are_not_selectable() {
        let axis = TimeAxis::new(0, 24 * 60, &[CalendarClockRange::hours(0, 6)], 60., 24.);
        assert_eq!(axis.y_for_minute(6 * 60), 24.);
        assert_eq!(axis.y_for_minute(7 * 60), 84.);
        assert_eq!(axis.minute_for_y(12.), None);
        assert_eq!(axis.minute_for_y(84.), Some(7 * 60));
    }

    #[test]
    fn overlapping_collapsed_ranges_form_one_fold() {
        let axis = TimeAxis::new(
            0,
            12 * 60,
            &[
                CalendarClockRange::hours(0, 4),
                CalendarClockRange::hours(3, 6),
            ],
            60.,
            24.,
        );
        assert_eq!(axis.segments.len(), 2);
        assert!(axis.segments[0].collapsed);
        assert_eq!(axis.segments[0].end_minute, 6 * 60);
    }
}
