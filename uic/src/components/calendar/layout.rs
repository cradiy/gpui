use std::{collections::HashMap, sync::Arc};

use chrono::{Duration, FixedOffset, NaiveDate};
use gpui::SharedString;

use super::{
    CalendarEvent, CalendarEventMerge, CalendarMergePolicy, CalendarSegmentLabelPolicy, DateRange,
};

pub(crate) type MergeKeyFn<T> = Arc<dyn Fn(&CalendarEvent<T>) -> Option<SharedString>>;

pub(crate) struct EventGroup<T> {
    pub id: SharedString,
    pub title: SharedString,
    pub range: DateRange,
    pub color: Option<gpui::Hsla>,
    pub events: Arc<[CalendarEvent<T>]>,
}

impl<T> Clone for EventGroup<T> {
    fn clone(&self) -> Self {
        Self {
            id: self.id.clone(),
            title: self.title.clone(),
            range: self.range,
            color: self.color,
            events: self.events.clone(),
        }
    }
}

pub(crate) struct EventSegment<T> {
    pub group: EventGroup<T>,
    pub range: DateRange,
    pub week_index: usize,
    pub start_column: usize,
    pub span_columns: usize,
    pub lane: usize,
    pub continues_before: bool,
    pub continues_after: bool,
    pub show_label: bool,
}

pub(crate) struct MonthLayout<T> {
    pub segments: Vec<EventSegment<T>>,
    pub overflow: HashMap<NaiveDate, usize>,
}

pub(crate) fn resolve_event_groups<T>(
    events: &[CalendarEvent<T>],
    policy: CalendarMergePolicy,
    key_fn: Option<&MergeKeyFn<T>>,
    offset: FixedOffset,
) -> Vec<EventGroup<T>> {
    let mut individual = Vec::new();
    let mut keyed: HashMap<SharedString, Vec<EventGroup<T>>> = HashMap::new();

    for event in events {
        let range = event.date_range(offset);
        if policy == CalendarMergePolicy::None {
            for day in 0..range.days() {
                let day = range.start + Duration::days(day);
                individual.push(group_for_event(event, DateRange::single(day)));
            }
            continue;
        }

        let group = group_for_event(event, range);
        let key = match &event.merge {
            CalendarEventMerge::Never => None,
            CalendarEventMerge::Key(key) => Some(key.clone()),
            CalendarEventMerge::Inherit if policy == CalendarMergePolicy::AdjacentByKey => {
                key_fn.and_then(|key_fn| key_fn(event))
            }
            CalendarEventMerge::Inherit => None,
        };

        if policy == CalendarMergePolicy::AdjacentByKey
            && event.time.is_all_day()
            && let Some(key) = key
        {
            keyed.entry(key).or_default().push(group);
            continue;
        }
        individual.push(group);
    }

    for (_, mut groups) in keyed {
        groups.sort_by_key(|group| (group.range.start, group.range.end_exclusive));
        let mut merged: Vec<EventGroup<T>> = Vec::new();
        for group in groups {
            if let Some(previous) = merged.last_mut()
                && previous.range.end_exclusive >= group.range.start
            {
                previous.range.end_exclusive =
                    previous.range.end_exclusive.max(group.range.end_exclusive);
                let mut events = previous.events.to_vec();
                events.extend(group.events.iter().cloned());
                previous.events = events.into();
            } else {
                merged.push(group);
            }
        }
        individual.extend(merged);
    }

    individual.sort_by(|a, b| {
        a.range
            .start
            .cmp(&b.range.start)
            .then_with(|| b.range.days().cmp(&a.range.days()))
            .then_with(|| a.id.cmp(&b.id))
    });
    individual
}

fn group_for_event<T>(event: &CalendarEvent<T>, range: DateRange) -> EventGroup<T> {
    EventGroup {
        id: event.id.clone(),
        title: event.title.clone(),
        range,
        color: event.color,
        events: Arc::from([event.clone()]),
    }
}

pub(crate) fn layout_month<T>(
    groups: &[EventGroup<T>],
    visible: DateRange,
    max_lanes: usize,
    label_policy: CalendarSegmentLabelPolicy,
) -> MonthLayout<T> {
    let mut segments = Vec::new();

    for group in groups {
        let Some(clipped) = group.range.intersection(visible) else {
            continue;
        };
        let first_week = ((clipped.start - visible.start).num_days() / 7) as usize;
        let last_date = clipped.end_exclusive - Duration::days(1);
        let last_week = ((last_date - visible.start).num_days() / 7) as usize;
        let first_segment_index = segments.len();

        for week_index in first_week..=last_week {
            let week_start = visible.start + Duration::days((week_index * 7) as i64);
            let week = DateRange::new(week_start, week_start + Duration::days(7));
            let range = clipped.intersection(week).expect("intersecting week");
            segments.push(EventSegment {
                group: group.clone(),
                range,
                week_index,
                start_column: (range.start - week_start).num_days() as usize,
                span_columns: range.days() as usize,
                lane: 0,
                continues_before: group.range.start < range.start,
                continues_after: group.range.end_exclusive > range.end_exclusive,
                show_label: true,
            });
        }

        let event_segments = &mut segments[first_segment_index..];
        match label_policy {
            CalendarSegmentLabelPolicy::Each => {}
            CalendarSegmentLabelPolicy::First => {
                for segment in event_segments.iter_mut().skip(1) {
                    segment.show_label = false;
                }
            }
            CalendarSegmentLabelPolicy::Longest => {
                let longest = event_segments
                    .iter()
                    .enumerate()
                    .max_by_key(|(_, segment)| segment.span_columns)
                    .map(|(index, _)| index)
                    .unwrap_or(0);
                for (index, segment) in event_segments.iter_mut().enumerate() {
                    segment.show_label = index == longest;
                }
            }
        }
    }

    for week_index in 0..6 {
        let mut indexes: Vec<_> = segments
            .iter()
            .enumerate()
            .filter_map(|(index, segment)| (segment.week_index == week_index).then_some(index))
            .collect();
        indexes.sort_by_key(|index| {
            let segment = &segments[*index];
            (segment.start_column, usize::MAX - segment.span_columns)
        });

        let mut lane_ends: Vec<usize> = Vec::new();
        for index in indexes {
            let segment = &segments[index];
            let lane = lane_ends
                .iter()
                .position(|end| *end <= segment.start_column)
                .unwrap_or(lane_ends.len());
            let end = segment.start_column + segment.span_columns;
            if lane == lane_ends.len() {
                lane_ends.push(end);
            } else {
                lane_ends[lane] = end;
            }
            segments[index].lane = lane;
        }
    }

    let mut overflow = HashMap::new();
    for segment in &segments {
        if segment.lane < max_lanes {
            continue;
        }
        for day in 0..segment.range.days() {
            *overflow
                .entry(segment.range.start + Duration::days(day))
                .or_insert(0) += 1;
        }
    }

    MonthLayout { segments, overflow }
}

#[cfg(test)]
mod tests {
    use chrono::NaiveDate;

    use super::*;

    fn date(day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 9, day).unwrap()
    }

    #[test]
    fn explicit_multi_day_events_stay_connected() {
        let event = CalendarEvent::all_day_range("holiday", "Holiday", date(2), date(6), ());
        let groups = resolve_event_groups(
            &[event],
            CalendarMergePolicy::ExplicitOnly,
            None,
            FixedOffset::east_opt(0).unwrap(),
        );
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].range, DateRange::new(date(2), date(6)));
    }

    #[test]
    fn adjacent_events_only_merge_by_an_explicit_key() {
        let keyed = [
            CalendarEvent::all_day("a", "Holiday", date(2), ())
                .merge(CalendarEventMerge::Key("holiday".into())),
            CalendarEvent::all_day("b", "Different title", date(3), ())
                .merge(CalendarEventMerge::Key("holiday".into())),
        ];
        let groups = resolve_event_groups(
            &keyed,
            CalendarMergePolicy::AdjacentByKey,
            None,
            FixedOffset::east_opt(0).unwrap(),
        );
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].range, DateRange::new(date(2), date(4)));

        let same_title = [
            CalendarEvent::all_day("a", "Holiday", date(5), ()),
            CalendarEvent::all_day("b", "Holiday", date(6), ()),
        ];
        let groups = resolve_event_groups(
            &same_title,
            CalendarMergePolicy::AdjacentByKey,
            None,
            FixedOffset::east_opt(0).unwrap(),
        );
        assert_eq!(groups.len(), 2);
    }

    #[test]
    fn adjacent_events_can_merge_by_application_data() {
        let events: [CalendarEvent<&'static str>; 2] = [
            CalendarEvent::all_day("a", "First", date(5), "holiday"),
            CalendarEvent::all_day("b", "Second", date(6), "holiday"),
        ];
        let key: MergeKeyFn<&'static str> = Arc::new(|event| Some((*event.data).into()));
        let groups = resolve_event_groups(
            &events,
            CalendarMergePolicy::AdjacentByKey,
            Some(&key),
            FixedOffset::east_opt(0).unwrap(),
        );
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].events.len(), 2);
        assert_eq!(groups[0].range, DateRange::new(date(5), date(7)));
    }

    #[test]
    fn an_event_crossing_a_week_boundary_is_segmented_and_keeps_continuations() {
        let event = CalendarEvent::all_day_range("trip", "Trip", date(4), date(10), ());
        let groups = resolve_event_groups(
            &[event],
            CalendarMergePolicy::ExplicitOnly,
            None,
            FixedOffset::east_opt(0).unwrap(),
        );
        let visible = DateRange::new(date(1), date(15));
        let layout = layout_month(&groups, visible, 3, CalendarSegmentLabelPolicy::Each);
        assert_eq!(layout.segments.len(), 2);
        assert!(layout.segments[0].continues_after);
        assert!(layout.segments[1].continues_before);
        assert_eq!(layout.segments[0].span_columns, 4);
        assert_eq!(layout.segments[1].span_columns, 2);
    }

    #[test]
    fn overlapping_segments_are_assigned_different_lanes() {
        let events = [
            CalendarEvent::all_day_range("a", "A", date(1), date(5), ()),
            CalendarEvent::all_day_range("b", "B", date(3), date(6), ()),
            CalendarEvent::all_day_range("c", "C", date(6), date(7), ()),
        ];
        let groups = resolve_event_groups(
            &events,
            CalendarMergePolicy::ExplicitOnly,
            None,
            FixedOffset::east_opt(0).unwrap(),
        );
        let layout = layout_month(
            &groups,
            DateRange::new(date(1), date(8)),
            1,
            CalendarSegmentLabelPolicy::Each,
        );
        assert_eq!(layout.segments[0].lane, 0);
        assert_eq!(layout.segments[1].lane, 1);
        assert_eq!(layout.segments[2].lane, 0);
        assert_eq!(layout.overflow.get(&date(3)), Some(&1));
    }
}
