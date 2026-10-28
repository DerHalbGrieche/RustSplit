//! Derived timing values used by the layout components.

use crate::model::{Segment, TimerPhase, TimingMethod, BEST_SEGMENTS, PERSONAL_BEST};
use crate::time::TimeSpan;
use crate::timer::Timer;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SplitColor {
    AheadGaining,
    AheadLosing,
    BehindGaining,
    BehindLosing,
    BestSegment,
    NotRunning,
    Paused,
}

/// Last completed delta at or before `split_number`.
pub fn get_last_delta(
    timer: &Timer,
    split_number: i32,
    comparison: &str,
    method: TimingMethod,
) -> Option<TimeSpan> {
    let segment_count = timer.run.segments.len();
    if split_number < 0 || segment_count == 0 {
        return None;
    }
    // Components can ask for the current split index after the run ended, in
    // which case it points one past the last segment.  Clamp instead of
    // panicking / returning `None` for an otherwise valid delta.
    let split_number = (split_number as usize).min(segment_count - 1);
    for index in (0..=split_number).rev() {
        let segment = &timer.run.segments[index];
        let comparison_time = segment.comparison(comparison).get(method);
        let split_time = segment.split_time.get(method);
        if let (Some(comparison_time), Some(split_time)) = (comparison_time, split_time) {
            return Some(split_time - comparison_time);
        }
    }
    None
}

fn segment_time_or_delta(
    timer: &Timer,
    split_number: i32,
    use_current_time: bool,
    segment_time: bool,
    comparison: &str,
    method: TimingMethod,
) -> Option<TimeSpan> {
    if split_number < 0 {
        return None;
    }
    let split_number_usize = split_number as usize;
    let segment = timer.run.segments.get(split_number_usize)?;
    let current_time = if use_current_time {
        timer.current_time().get(method)?
    } else {
        segment.split_time.get(method)?
    };

    for index in (0..split_number_usize).rev() {
        let previous = &timer.run.segments[index];
        if let Some(previous_split) = previous.split_time.get(method) {
            if segment_time {
                return Some(current_time - previous_split);
            }
            if let Some(previous_comparison) = previous.comparison(comparison).get(method) {
                let current_comparison = segment.comparison(comparison).get(method)?;
                return Some(
                    current_time - current_comparison - (previous_split - previous_comparison),
                );
            }
        }
    }

    if segment_time {
        Some(current_time)
    } else {
        Some(current_time - segment.comparison(comparison).get(method)?)
    }
}

pub fn get_previous_segment_time(
    timer: &Timer,
    split_number: i32,
    method: TimingMethod,
) -> Option<TimeSpan> {
    segment_time_or_delta(timer, split_number, false, true, PERSONAL_BEST, method)
}

pub fn get_live_segment_time(
    timer: &Timer,
    split_number: i32,
    method: TimingMethod,
) -> Option<TimeSpan> {
    segment_time_or_delta(timer, split_number, true, true, PERSONAL_BEST, method)
}

pub fn get_previous_segment_delta(
    timer: &Timer,
    split_number: i32,
    comparison: &str,
    method: TimingMethod,
) -> Option<TimeSpan> {
    segment_time_or_delta(timer, split_number, false, false, comparison, method)
}

pub fn get_live_segment_delta(
    timer: &Timer,
    split_number: i32,
    comparison: &str,
    method: TimingMethod,
) -> Option<TimeSpan> {
    segment_time_or_delta(timer, split_number, true, false, comparison, method)
}

/// Returns the live delta that should be displayed, if any.
pub fn check_live_delta(
    timer: &Timer,
    split_delta: bool,
    comparison: &str,
    method: TimingMethod,
    show_best_segments: bool,
) -> Option<TimeSpan> {
    if !matches!(timer.phase, TimerPhase::Running | TimerPhase::Paused) {
        return None;
    }
    let index = timer.current_split_index;
    if index < 0 {
        return None;
    }
    let segment = timer.run.segments.get(index as usize)?;
    let current_time = timer.current_time().get(method)?;
    let comparison_time = segment.comparison(comparison).get(method);
    let current_segment = get_live_segment_time(timer, index, method)?;
    let best_segment = segment.best_segment_time.get(method);
    let best_segment_delta = get_live_segment_delta(timer, index, BEST_SEGMENTS, method);
    let comparison_delta = get_live_segment_delta(timer, index, comparison, method);

    let split_behind = comparison_time.is_some_and(|time| current_time > time);
    let behind_best = show_best_segments
        && best_segment.is_some_and(|best| current_segment > best)
        && best_segment_delta.is_some_and(|delta| delta > TimeSpan::ZERO);

    if (split_delta && split_behind)
        || behind_best
        || comparison_delta.is_some_and(|delta| delta > TimeSpan::ZERO)
    {
        if split_delta {
            return comparison_time.map(|time| current_time - time);
        }
        // LiveSplit returns the (possibly null) comparison delta here, not a
        // forced zero when the comparison has no time for this segment.
        return comparison_delta;
    }
    None
}

pub fn check_best_segment(
    timer: &Timer,
    split_number: i32,
    method: TimingMethod,
) -> bool {
    if split_number < 0 {
        return false;
    }
    let Some(segment) = timer.run.segments.get(split_number as usize) else {
        return false;
    };
    if segment.split_time.get(method).is_none() {
        return false;
    }
    let delta = get_previous_segment_delta(timer, split_number, BEST_SEGMENTS, method);
    let current_segment = get_previous_segment_time(timer, split_number, method);
    let best_segment = segment.best_segment_time.get(method);
    best_segment.is_none()
        || current_segment.is_some_and(|current| current < best_segment.unwrap())
        || delta.is_some_and(|delta| delta < TimeSpan::ZERO)
}

/// Choose the semantic color for a split delta.  The actual colors come from
/// the layout settings.
pub fn split_color(
    timer: &Timer,
    time_difference: Option<TimeSpan>,
    split_number: i32,
    show_segment_deltas: bool,
    show_best_segments: bool,
    comparison: &str,
    method: TimingMethod,
) -> Option<SplitColor> {
    if split_number < 0 || split_number as usize >= timer.run.segments.len() {
        return None;
    }
    let mut color = None;
    if let Some(time_difference) = time_difference {
        if time_difference <= TimeSpan::ZERO {
            color = Some(SplitColor::AheadGaining);
            let last_delta = get_last_delta(timer, split_number - 1, comparison, method);
            if show_segment_deltas
                && split_number > 0
                && last_delta.is_some_and(|last| time_difference > last)
            {
                color = Some(SplitColor::AheadLosing);
            }
        } else {
            color = Some(SplitColor::BehindLosing);
            let last_delta = get_last_delta(timer, split_number - 1, comparison, method);
            if show_segment_deltas
                && split_number > 0
                && last_delta.is_some_and(|last| time_difference < last)
            {
                color = Some(SplitColor::BehindGaining);
            }
        }
    }
    if show_best_segments && check_best_segment(timer, split_number, method) {
        color = Some(SplitColor::BestSegment);
    }
    color
}

pub fn comparison_time(
    segment: &Segment,
    comparison: &str,
    method: TimingMethod,
) -> Option<TimeSpan> {
    segment.comparison(comparison).get(method)
}

/// Resolve the comparison name, substituting the timer's current comparison
/// when a component is configured to follow it.
pub fn resolve_comparison<'a>(timer: &'a Timer, comparison: &'a str) -> &'a str {
    if comparison == crate::model::CURRENT_COMPARISON {
        &timer.current_comparison
    } else {
        comparison
    }
}

pub fn comparison_time_resolved(
    timer: &Timer,
    segment_index: i32,
    comparison: &str,
    method: TimingMethod,
) -> Option<TimeSpan> {
    if segment_index < 0 {
        return None;
    }
    let segment = timer.run.segments.get(segment_index as usize)?;
    let comparison = resolve_comparison(timer, comparison);
    segment.comparison(comparison).get(method)
}

pub fn comparison_time_for_split(
    timer: &Timer,
    split_number: i32,
    comparison: &str,
    method: TimingMethod,
) -> Option<TimeSpan> {
    if split_number < 0 {
        return None;
    }
    let segment = timer.run.segments.get(split_number as usize)?;
    let comparison = resolve_comparison(timer, comparison);
    segment.comparison(comparison).get(method)
}

/// The predicted final time based on the current run and the comparison.
///
/// This mirrors LiveSplit's Run Prediction component: while running, the
/// prediction is the comparison's final split time plus the worst delta the
/// runner has accumulated (either up to the previous split or live on the
/// current one).  A finished run simply shows its final time, and a timer that
/// has not started yet shows the comparison's final split time.
pub fn predicted_time(timer: &Timer, comparison: &str, method: TimingMethod) -> Option<TimeSpan> {
    let comparison = resolve_comparison(timer, comparison);
    match timer.phase {
        TimerPhase::Running | TimerPhase::Paused => {
            let last_comparison = timer
                .run
                .segments
                .last()
                .and_then(|segment| segment.comparison(comparison).get(method))?;
            let mut delta =
                get_last_delta(timer, timer.current_split_index, comparison, method)
                    .unwrap_or(TimeSpan::ZERO);

            let live_delta = timer.current_time().get(method).zip(
                timer
                    .current_split()
                    .and_then(|segment| segment.comparison(comparison).get(method)),
            );
            if let Some((current_time, comparison_time)) = live_delta {
                let live_delta = current_time - comparison_time;
                if live_delta > delta {
                    delta = live_delta;
                }
            }

            Some(last_comparison + delta)
        }
        TimerPhase::Ended => timer
            .run
            .segments
            .last()
            .and_then(|segment| segment.split_time.get(method))
            .or_else(|| timer.current_time().get(method)),
        TimerPhase::NotRunning => timer
            .run
            .segments
            .last()
            .and_then(|segment| segment.comparison(comparison).get(method)),
    }
}

/// Total time that can still be saved compared to the sum of best.
///
/// This is LiveSplit's "Total Possible Time Save": the time save is only
/// computed for the segments that are still ahead of the runner.  A finished
/// run has no time left to save, and a timer that has not started yet considers
/// the whole run.
pub fn possible_time_save(
    timer: &Timer,
    method: TimingMethod,
    _comparison_settings: &crate::model::ComparisonSettings,
) -> Option<TimeSpan> {
    if timer.phase == TimerPhase::Ended || timer.run.segments.is_empty() {
        return Some(TimeSpan::ZERO);
    }
    let segment_count = timer.run.segments.len();
    let start = timer.current_split_index.max(0) as usize;
    if start >= segment_count {
        return Some(TimeSpan::ZERO);
    }
    let comparison = resolve_comparison(timer, &timer.current_comparison);

    let mut total = TimeSpan::ZERO;
    for segment_index in start..segment_count {
        if let Some(save) =
            possible_time_save_for_segment(timer, segment_index, comparison, method)
        {
            total += save;
        }
    }
    Some(total)
}

fn possible_time_save_for_segment(
    timer: &Timer,
    segment_index: usize,
    comparison: &str,
    method: TimingMethod,
) -> Option<TimeSpan> {
    let segments = &timer.run.segments;
    let segment = segments.get(segment_index)?;
    let mut best_segments = segment.best_segment_time.get(method)?;
    let mut previous_segment_index = segment_index;
    let mut previous_comparison_time = TimeSpan::ZERO;

    // Skipped segments do not have a comparison split.  Their best segment
    // times are combined with this segment's best segment time until a split
    // with a comparison time is found.
    while previous_segment_index > 0 {
        let previous = &segments[previous_segment_index - 1];
        if let Some(time) = previous.comparison(comparison).get(method) {
            previous_comparison_time = time;
            break;
        }
        previous_segment_index -= 1;
        best_segments += segments[previous_segment_index]
            .best_segment_time
            .get(method)?;
    }

    let mut time = segment.comparison(comparison).get(method)?
        - previous_comparison_time
        - best_segments;

    if timer.current_split_index >= 0
        && previous_segment_index == timer.current_split_index as usize
    {
        if let Some(segment_delta) =
            get_live_segment_delta(timer, segment_index as i32, comparison, method)
        {
            let segment_delta = TimeSpan::ZERO - segment_delta;
            if segment_delta < time {
                time = segment_delta;
            }
        }
    }

    Some(time.max(TimeSpan::ZERO))
}

pub fn live_delta(timer: &Timer, method: TimingMethod) -> Option<TimeSpan> {
    let index = timer.current_split_index;
    let current = timer.current_time().get(method)?;
    let comparison = comparison_time_for_split(timer, index, &timer.current_comparison, method)?;
    Some(current - comparison)
}
