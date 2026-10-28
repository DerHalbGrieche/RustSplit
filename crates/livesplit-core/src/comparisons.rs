//! Comparison generators (best segments, average, median, ...).
//!
//! This is a port of LiveSplit's comparison generators.  The important pieces
//! are the Sum of Best / Sum of Worst calculations, which need to consider
//! segment history branches (segments that were skipped turn into combined
//! segments), the current run and the personal best run.  The remaining
//! generators either aggregate the history (average / median) or project a
//! single attempt (latest run, best split times).
//!
//! All calculations are done independently for real time and game time, and
//! the results are stored in each segment's `comparisons` map under the
//! standard names from [`crate::model`].

use std::collections::BTreeSet;

use crate::model::{
    ComparisonSettings, IndexedTime, Run, TimingMethod, AVERAGE_SEGMENTS, BEST_SEGMENTS,
    BEST_SPLIT_TIMES, LATEST_RUN, MEDIAN_SEGMENTS, PERCENTILE, WORST_SEGMENTS,
};
use crate::time::{Time, TimeSpan};

/// Weight used by the Average Segments generator.  More recent attempts get a
/// larger weight; each older attempt is multiplied by this factor.
const AVERAGE_WEIGHT: f64 = 0.75;

/// Weight used by the Median Segments generator.
const MEDIAN_WEIGHT: f64 = 0.75;

/// Weight used by the Balanced PB (percentile) generator.
const PERCENTILE_WEIGHT: f64 = 0.9375;

const TICKS_PER_SECOND: f64 = 10_000_000.0;

/// Recompute all generated comparisons in place.
pub fn generate_all(run: &mut Run, settings: &ComparisonSettings) {
    generate_best_segments(run, settings);
    generate_best_split_times(run);
    generate_latest_run(run);
    generate_average_segments(run);
    generate_median_segments(run);
    generate_worst_segments(run);
    generate_balanced_pb(run);
}

// ---------------------------------------------------------------------------
// Storing comparison times
// ---------------------------------------------------------------------------

/// Overwrite both timing methods of a generated comparison.
fn set_comparison_both(
    run: &mut Run,
    name: &str,
    index: usize,
    real: Option<TimeSpan>,
    game: Option<TimeSpan>,
) {
    if let Some(segment) = run.segments.get_mut(index) {
        let mut time = segment.comparison(name);
        time.real_time = real;
        time.game_time = game;
        segment.set_comparison(name, time);
    }
}

/// Overwrite one timing method, leaving the other one untouched.
fn set_comparison_method(
    run: &mut Run,
    name: &str,
    index: usize,
    method: TimingMethod,
    value: Option<TimeSpan>,
) {
    if let Some(segment) = run.segments.get_mut(index) {
        let mut time = segment.comparison(name);
        time.set(method, value);
        segment.set_comparison(name, time);
    }
}

// ---------------------------------------------------------------------------
// Sum of Best / Sum of Worst
//
// The algorithm keeps one prediction per split boundary (there are
// `segments.len() + 1` boundaries, boundary 0 being the start of the run).
// A prediction is the fastest (or slowest) way to *reach* that boundary.  The
// final boundary is the sum we are interested in.
//
// For every segment that has been reached, we add the best segment time and
// then try every alternative route through the segment history:
//   * skipped segments in a historical attempt produce combined segments,
//     represented by a `None` timing value.  `track_branch` follows such an
//     attempt until it finds the next split that was actually recorded.
//   * the current run (if requested) can provide a route through the remaining
//     splits.
//   * the personal best run always provides a route.
//
// A prediction is only accepted if it is better (best) or worse (worst) than
// the currently known prediction for that boundary.  The C# implementation
// stores this in a flat array; only the times are needed for the comparisons.
// ---------------------------------------------------------------------------

/// Insert a candidate prediction into the prediction array.
fn populate_prediction(
    predictions: &mut [Option<TimeSpan>],
    predicted_time: Option<TimeSpan>,
    index: i32,
    take_minimum: bool,
) {
    let Some(predicted_time) = predicted_time else {
        return;
    };
    let Ok(index) = usize::try_from(index) else {
        return;
    };
    let Some(slot) = predictions.get_mut(index) else {
        return;
    };
    let replace = match *slot {
        None => true,
        Some(existing) => {
            if take_minimum {
                predicted_time < existing
            } else {
                predicted_time > existing
            }
        }
    };
    if replace {
        *slot = Some(predicted_time);
    }
}

/// Whether a history entry can start a continuous branch at `segment_index`.
///
/// If the previous segment has a history entry for the same attempt, that
/// entry must contain a time for the timing method.  An entry that is present
/// but has a `None` time means the previous segment was skipped, so the
/// current segment's time is a combined segment and must not be used on its
/// own (the branch started at the skipped segment already accounts for it).
fn branch_starts_here(
    run: &Run,
    segment_index: usize,
    attempt: i32,
    method: TimingMethod,
) -> bool {
    let Some(previous) = segment_index
        .checked_sub(1)
        .and_then(|index| run.segments.get(index))
    else {
        return true;
    };

    match previous.segment_history.get(&attempt) {
        None => true,
        Some(time) => time.get(method).is_some(),
    }
}

/// Expand the predictions reachable from the prediction at `segment_index`.
#[allow(clippy::too_many_arguments)]
fn populate_predictions(
    run: &Run,
    segment_index: usize,
    predictions: &mut [Option<TimeSpan>],
    simple_calculation: bool,
    use_current_run: bool,
    method: TimingMethod,
    take_minimum: bool,
) {
    let Some(current_time) = predictions[segment_index] else {
        return;
    };
    let Some(segment) = run.segments.get(segment_index) else {
        return;
    };

    // Always use the standalone best segment time first.
    let best_segment_prediction = segment
        .best_segment_time
        .get(method)
        .map(|time| current_time + time);
    populate_prediction(
        predictions,
        best_segment_prediction,
        segment_index as i32 + 1,
        take_minimum,
    );

    if take_minimum {
        // Sum of Best only needs history entries for *skipped* segments: a
        // skipped segment combines itself with the following segment(s), and
        // that combined time can be faster than the sum of the individual best
        // segment times.
        if !simple_calculation {
            for (&attempt, history_time) in &segment.segment_history {
                if history_time.get(method).is_some() {
                    continue;
                }
                if branch_starts_here(run, segment_index, attempt, method) {
                    // The branch begins *after* the skipped segment.
                    let prediction =
                        track_branch(run, Some(current_time), segment_index + 1, attempt, method);
                    populate_prediction(
                        predictions,
                        prediction.time.get(method),
                        prediction.index,
                        take_minimum,
                    );
                }
            }
        }
    } else {
        // Sum of Worst considers every recorded attempt, including entries
        // that have a time for this segment.  `track_branch` starts at this
        // segment and immediately returns the recorded segment time (or skips
        // a missing entry and continues with the combined segment).
        for (&attempt, _) in &segment.segment_history {
            if branch_starts_here(run, segment_index, attempt, method) {
                let prediction =
                    track_branch(run, Some(current_time), segment_index, attempt, method);
                populate_prediction(
                    predictions,
                    prediction.time.get(method),
                    prediction.index,
                    take_minimum,
                );
            }
        }
    }

    if use_current_run {
        let prediction = track_current_run(run, Some(current_time), segment_index, method);
        populate_prediction(
            predictions,
            prediction.time.get(method),
            prediction.index,
            take_minimum,
        );
    }

    // The personal best route is always considered, independently of the
    // `use_current_run` setting.
    let prediction = track_personal_best_run(run, Some(current_time), segment_index, method);
    populate_prediction(
        predictions,
        prediction.time.get(method),
        prediction.index,
        take_minimum,
    );
}

/// Calculate all Sum of Best predictions, including the start and end
/// boundaries.
fn calculate_sum_of_best_predictions(
    run: &Run,
    simple_calculation: bool,
    use_current_run: bool,
    method: TimingMethod,
) -> Vec<Option<TimeSpan>> {
    let mut predictions = vec![None; run.segments.len() + 1];
    // The start of the run is always reached at zero.  Keeping this value set
    // also prevents branch predictions from writing into index 0.
    predictions[0] = Some(TimeSpan::ZERO);

    for segment_index in 0..run.segments.len() {
        populate_predictions(
            run,
            segment_index,
            &mut predictions,
            simple_calculation,
            use_current_run,
            method,
            true,
        );
    }

    predictions
}

/// Calculate all Sum of Worst predictions.
fn calculate_sum_of_worst_predictions(
    run: &Run,
    use_current_run: bool,
    method: TimingMethod,
) -> Vec<Option<TimeSpan>> {
    let mut predictions = vec![None; run.segments.len() + 1];
    predictions[0] = Some(TimeSpan::ZERO);

    for segment_index in 0..run.segments.len() {
        populate_predictions(
            run,
            segment_index,
            &mut predictions,
            false,
            use_current_run,
            method,
            false,
        );
    }

    predictions
}

/// Calculate the Sum of Best for the whole run and timing method provided.
///
/// `simple` excludes the segment history from the calculation (only best
/// segment times, the current run and the personal best are used), while
/// `use_current_run` controls whether the current attempt is taken into
/// account.
pub fn calculate_sum_of_best(
    run: &Run,
    simple: bool,
    use_current_run: bool,
    method: TimingMethod,
) -> Option<TimeSpan> {
    calculate_sum_of_best_predictions(run, simple, use_current_run, method)
        .pop()
        .flatten()
}

/// Calculate the Sum of Worst for the whole run and timing method provided.
pub fn calculate_sum_of_worst(
    run: &Run,
    use_current_run: bool,
    method: TimingMethod,
) -> Option<TimeSpan> {
    calculate_sum_of_worst_predictions(run, use_current_run, method)
        .pop()
        .flatten()
}

// ---------------------------------------------------------------------------
// Route tracking helpers (same semantics as LiveSplit's SumOfSegmentsHelper)
// ---------------------------------------------------------------------------

/// The empty prediction returned when a route cannot be followed.
fn no_prediction() -> IndexedTime {
    IndexedTime {
        time: Time::default(),
        index: 0,
    }
}

/// Follow the current run from `segment_index` to the next split that has a
/// time.  Returns the combined segment time anchored at `current_time`.
///
/// The index returned is the *boundary* after the split that was found, i.e.
/// `segment_index + 1` for the usual case.
pub fn track_current_run(
    run: &Run,
    current_time: Option<TimeSpan>,
    segment_index: usize,
    method: TimingMethod,
) -> IndexedTime {
    // If the segment before the one we are tracking has no split time, the
    // current run cannot provide a continuous route.
    let first_split_time = if segment_index == 0 {
        Some(TimeSpan::ZERO)
    } else {
        match run
            .segments
            .get(segment_index - 1)
            .and_then(|segment| segment.split_time.get(method))
        {
            Some(time) => Some(time),
            None => return no_prediction(),
        }
    };

    for (index, segment) in run.segments.iter().enumerate().skip(segment_index) {
        if let Some(second_split_time) = segment.split_time.get(method) {
            return IndexedTime {
                time: Time::from_method(
                    method,
                    current_time.map(|time| second_split_time - first_split_time.unwrap_or_default() + time),
                ),
                index: index as i32 + 1,
            };
        }
    }

    no_prediction()
}

/// Follow the personal best run from `segment_index` to the next split that has
/// a time.
pub fn track_personal_best_run(
    run: &Run,
    current_time: Option<TimeSpan>,
    segment_index: usize,
    method: TimingMethod,
) -> IndexedTime {
    let first_split_time = if segment_index == 0 {
        Some(TimeSpan::ZERO)
    } else {
        match run
            .segments
            .get(segment_index - 1)
            .and_then(|segment| segment.personal_best().get(method))
        {
            Some(time) => Some(time),
            None => return no_prediction(),
        }
    };

    for (index, segment) in run.segments.iter().enumerate().skip(segment_index) {
        if let Some(second_split_time) = segment.personal_best().get(method) {
            return IndexedTime {
                time: Time::from_method(
                    method,
                    current_time.map(|time| second_split_time - first_split_time.unwrap_or_default() + time),
                ),
                index: index as i32 + 1,
            };
        }
    }

    no_prediction()
}

/// Follow a branch of a historical attempt starting at `segment_index`.
///
/// Missing timing values are skipped (they represent combined segments).  If
/// the attempt has no entry at all for a segment, the branch ends there and a
/// default (empty) [`IndexedTime`] is returned.
pub fn track_branch(
    run: &Run,
    current_time: Option<TimeSpan>,
    segment_index: usize,
    run_index: i32,
    method: TimingMethod,
) -> IndexedTime {
    for (index, segment) in run.segments.iter().enumerate().skip(segment_index) {
        match segment.segment_history.get(&run_index) {
            Some(segment_time) => {
                if let Some(segment_time) = segment_time.get(method) {
                    return IndexedTime {
                        time: Time::from_method(
                            method,
                            current_time.map(|time| segment_time + time),
                        ),
                        index: index as i32 + 1,
                    };
                }
                // Present but empty for this method: the segment was skipped
                // in this attempt, keep following the branch.
            }
            // The attempt did not reach this segment at all.
            None => break,
        }
    }

    no_prediction()
}

// ---------------------------------------------------------------------------
// Shared history helpers
// ---------------------------------------------------------------------------

/// All attempt indices that represent actual runs (index >= 1).
///
/// The C# implementation iterates the attempt history, while upstream
/// livesplit-core iterates the segment histories.  Using both keeps the
/// generators correct for run files whose attempt list is incomplete, and
/// still filters out imported pseudo attempts (index <= 0).
fn actual_attempt_indices(run: &Run) -> Vec<i32> {
    let mut indices = BTreeSet::new();
    for attempt in &run.attempts {
        if attempt.index >= 1 {
            indices.insert(attempt.index);
        }
    }
    for segment in &run.segments {
        for &index in segment.segment_history.keys() {
            if index >= 1 {
                indices.insert(index);
            }
        }
    }
    indices.into_iter().collect()
}

/// Largest attempt index known to either the attempt list or the segment
/// histories.
fn max_attempt_index(run: &Run) -> i32 {
    let mut max = run.max_attempt_index();
    for segment in &run.segments {
        if let Some((&index, _)) = segment.segment_history.iter().next_back() {
            max = max.max(index);
        }
    }
    max
}

/// Collect the per-segment history values for the Average Segments generator.
///
/// A `None` timing value means the segment was skipped.  The following segment
/// is then a combined segment and must be ignored, because the skipped segment
/// already contributes its time through the combined segment.  Missing history
/// entries simply end the relevant part of the attempt.
fn collect_average_history(run: &Run, method: TimingMethod) -> Vec<Vec<TimeSpan>> {
    let mut all_history = vec![Vec::new(); run.segments.len()];

    for attempt in actual_attempt_indices(run) {
        let mut ignore_next_history = false;
        for (index, segment) in run.segments.iter().enumerate() {
            match segment.segment_history.get(&attempt) {
                Some(history) => match history.get(method) {
                    Some(time) => {
                        if ignore_next_history {
                            // This is a combined segment; the skipped segment
                            // is already accounted for.
                            ignore_next_history = false;
                        } else {
                            all_history[index].push(time);
                        }
                    }
                    None => ignore_next_history = true,
                },
                None => ignore_next_history = false,
            }
        }
    }

    all_history
}

/// Collect the per-segment history values for the Median Segments generator.
///
/// Unlike the average generator, the C# median generator stops processing an
/// attempt as soon as a segment history entry is missing entirely.
fn collect_median_history(run: &Run, method: TimingMethod) -> Vec<Vec<TimeSpan>> {
    let mut all_history = vec![Vec::new(); run.segments.len()];

    for attempt in actual_attempt_indices(run) {
        let mut ignore_next_history = false;
        for (index, segment) in run.segments.iter().enumerate() {
            match segment.segment_history.get(&attempt) {
                Some(history) => match history.get(method) {
                    Some(time) => {
                        if ignore_next_history {
                            ignore_next_history = false;
                        } else {
                            all_history[index].push(time);
                        }
                    }
                    None => ignore_next_history = true,
                },
                None => break,
            }
        }
    }

    all_history
}

/// Weighted arithmetic mean used by the Average Segments generator.
fn weighted_average(values: &[TimeSpan]) -> TimeSpan {
    let count = values.len();
    debug_assert!(count > 0);

    // The weights are based on recency (oldest first), not on the time value.
    // Sorting by value first mirrors the C# implementation; the result of a
    // weighted mean does not depend on the order anyway.
    let mut weighted = values
        .iter()
        .enumerate()
        .map(|(index, value)| (AVERAGE_WEIGHT.powi((count - index - 1) as i32), *value))
        .collect::<Vec<_>>();
    weighted.sort_by_key(|&(_, value)| value);

    let total_weight: f64 = weighted.iter().map(|(weight, _)| *weight).sum();
    let average_seconds = weighted
        .iter()
        .map(|(weight, value)| *weight * value.as_secs_f64())
        .sum::<f64>()
        / total_weight;

    // LiveSplit truncates the result to 100 ns ticks.
    TimeSpan::from_ticks((average_seconds * TICKS_PER_SECOND) as i64)
}

/// Weighted median used by the Median Segments generator.
fn weighted_median(values: &[TimeSpan]) -> TimeSpan {
    let count = values.len();
    debug_assert!(count > 0);

    let mut weighted = values
        .iter()
        .enumerate()
        .map(|(index, value)| (MEDIAN_WEIGHT.powi((count - index - 1) as i32), *value))
        .collect::<Vec<_>>();
    weighted.sort_by_key(|&(_, value)| value);

    let total_weight: f64 = weighted.iter().map(|(weight, _)| *weight).sum();
    let half_total_weight = total_weight / 2.0;
    let mut current_weight = 0.0;
    for (weight, value) in weighted {
        current_weight += weight;
        if current_weight >= half_total_weight {
            return value;
        }
    }

    TimeSpan::ZERO
}

/// Accumulate the per-segment aggregate into a comparison, starting at zero.
/// Once a segment has no history, the rest of the comparison is empty.
fn generate_aggregate_comparison(
    run: &mut Run,
    name: &str,
    method: TimingMethod,
    history: &[Vec<TimeSpan>],
    aggregate: fn(&[TimeSpan]) -> TimeSpan,
) {
    let mut total_time: Option<TimeSpan> = Some(TimeSpan::ZERO);

    for index in 0..run.segments.len() {
        let values = &history[index];
        if values.is_empty() {
            total_time = None;
        }

        if let Some(total) = &mut total_time {
            *total += aggregate(values);
        }

        set_comparison_method(run, name, index, method, total_time);
    }
}

// ---------------------------------------------------------------------------
// Generators
// ---------------------------------------------------------------------------

fn generate_best_segments(run: &mut Run, settings: &ComparisonSettings) {
    let real = calculate_sum_of_best_predictions(
        run,
        settings.simple_sum_of_best,
        false,
        TimingMethod::RealTime,
    );
    let game = calculate_sum_of_best_predictions(
        run,
        settings.simple_sum_of_best,
        false,
        TimingMethod::GameTime,
    );

    for index in 0..run.segments.len() {
        // Predictions are indexed by split boundary; segment `index` ends at
        // boundary `index + 1`.
        set_comparison_both(
            run,
            BEST_SEGMENTS,
            index,
            real.get(index + 1).copied().flatten(),
            game.get(index + 1).copied().flatten(),
        );
    }
}

fn generate_best_split_times(run: &mut Run) {
    if run.segments.is_empty() {
        return;
    }

    // The initial values are the personal best split times.  The first
    // segment uses its best segment time instead, as there is no previous
    // split to base a personal best split time on.
    let initial = run
        .segments
        .iter()
        .enumerate()
        .map(|(index, segment)| {
            if index == 0 {
                segment.best_segment_time
            } else {
                segment.personal_best()
            }
        })
        .collect::<Vec<_>>();
    for (segment, time) in run.segments.iter_mut().zip(initial) {
        segment.set_comparison(BEST_SPLIT_TIMES, time);
    }

    let min_index = run.min_segment_history_index();
    let max_index = max_attempt_index(run);

    for method in [TimingMethod::RealTime, TimingMethod::GameTime] {
        for attempt in min_index..=max_index {
            let mut total_time = TimeSpan::ZERO;

            for index in 0..run.segments.len() {
                // An attempt ends at the first segment without a history
                // entry, so stop processing this attempt.
                let Some(history_time) = run.segments[index]
                    .segment_history
                    .get(&attempt)
                    .copied()
                else {
                    break;
                };

                if let Some(segment_time) = history_time.get(method) {
                    total_time += segment_time;

                    let current = run.segments[index]
                        .comparison(BEST_SPLIT_TIMES)
                        .get(method);
                    if current.is_none_or(|current| total_time < current) {
                        set_comparison_method(
                            run,
                            BEST_SPLIT_TIMES,
                            index,
                            method,
                            Some(total_time),
                        );
                    }
                }
                // A present but empty history value is a skipped segment.  The
                // split comparison for this segment stays unchanged, but the
                // attempt can still continue (and provide combined segments).
            }
        }
    }
}

/// Find the attempt that should be used for the Latest Run comparison.
///
/// The comparison shows the attempt that got the furthest; among attempts
/// that reached the same segment, the most recent one wins.  Imported pseudo
/// attempts (index <= 0) are ignored.
fn latest_run_attempt(run: &Run) -> Option<i32> {
    for segment in run.segments.iter().rev() {
        if let Some((&index, _)) = segment.segment_history.iter().next_back() {
            if index >= 1 {
                return Some(index);
            }
        }
    }
    None
}

fn generate_latest_run_for_method(run: &mut Run, method: TimingMethod) {
    let attempt = latest_run_attempt(run);

    // Clear the comparison first so stale values from an older attempt are
    // not left behind.
    for index in 0..run.segments.len() {
        set_comparison_method(run, LATEST_RUN, index, method, None);
    }

    let Some(attempt) = attempt else {
        return;
    };

    let mut total_time = TimeSpan::ZERO;
    let mut attempt_continues = true;

    for index in 0..run.segments.len() {
        if !attempt_continues {
            continue;
        }

        match run.segments[index]
            .segment_history
            .get(&attempt)
            .copied()
        {
            Some(history_time) => match history_time.get(method) {
                Some(segment_time) => {
                    total_time += segment_time;
                    set_comparison_method(
                        run,
                        LATEST_RUN,
                        index,
                        method,
                        Some(total_time),
                    );
                }
                // A skipped segment has no split time.  The running total is
                // deliberately left untouched: LiveSplit resumes it at the
                // next recorded segment.
                None => {}
            },
            // The attempt ended before this segment.
            None => attempt_continues = false,
        }
    }
}

fn generate_latest_run(run: &mut Run) {
    generate_latest_run_for_method(run, TimingMethod::RealTime);
    generate_latest_run_for_method(run, TimingMethod::GameTime);
}

fn generate_average_segments(run: &mut Run) {
    for method in [TimingMethod::RealTime, TimingMethod::GameTime] {
        let history = collect_average_history(run, method);
        generate_aggregate_comparison(
            run,
            AVERAGE_SEGMENTS,
            method,
            &history,
            weighted_average,
        );
    }
}

fn generate_median_segments(run: &mut Run) {
    for method in [TimingMethod::RealTime, TimingMethod::GameTime] {
        let history = collect_median_history(run, method);
        generate_aggregate_comparison(
            run,
            MEDIAN_SEGMENTS,
            method,
            &history,
            weighted_median,
        );
    }
}

fn generate_worst_segments(run: &mut Run) {
    let real = calculate_sum_of_worst_predictions(run, false, TimingMethod::RealTime);
    let game = calculate_sum_of_worst_predictions(run, false, TimingMethod::GameTime);

    for index in 0..run.segments.len() {
        set_comparison_both(
            run,
            WORST_SEGMENTS,
            index,
            real.get(index + 1).copied().flatten(),
            game.get(index + 1).copied().flatten(),
        );
    }
}

// ---------------------------------------------------------------------------
// Balanced PB (PercentileComparisonGenerator)
// ---------------------------------------------------------------------------

/// A recorded segment time together with the index of the segment that
/// preceded it in the same attempt (-1 when it is the first segment).
#[derive(Clone, Copy)]
struct IndexedTimeSpan {
    time: TimeSpan,
    index: i32,
}

/// Linear interpolation used by the percentile generator.
fn percentile_calculate(
    percentile: f64,
    value1: TimeSpan,
    key1: f64,
    value2: TimeSpan,
    key2: f64,
) -> TimeSpan {
    let denominator = key1 - key2;
    if denominator == 0.0 {
        return value1;
    }

    // This is the same formula as the C# implementation:
    //   value = value2 + (percentile - key2) * (value1 - value2) / (key1 - key2)
    // written as two interpolated parts and rounded to ticks like
    // Convert.ToInt64 does in .NET.
    let lower = (key1 - percentile) * value2.ticks() as f64 / denominator;
    let upper = (percentile - key2) * value1.ticks() as f64 / denominator;
    TimeSpan::from_ticks((upper + lower).round_ties_even() as i64)
}

/// Generate the Balanced PB comparison for one timing method.
fn generate_balanced_pb_for_method(run: &mut Run, method: TimingMethod) {
    let segment_count = run.segments.len();
    if segment_count == 0 {
        return;
    }

    // 1. Collect the segment times of all attempts, tagging each time with
    //    the segment that preceded it.  This is used to reconstruct the
    //    actual path through the run.
    let mut all_history = vec![Vec::<IndexedTimeSpan>::new(); segment_count];

    for attempt in actual_attempt_indices(run) {
        let mut history_starting_index: i32 = -1;

        for (current_index, segment) in run.segments.iter().enumerate() {
            match segment.segment_history.get(&attempt) {
                Some(history) => {
                    if let Some(time) = history.get(method) {
                        all_history[current_index].push(IndexedTimeSpan {
                            time,
                            index: history_starting_index,
                        });
                        history_starting_index = current_index as i32;
                    }
                    // A skipped segment leaves the starting index untouched,
                    // so the next recorded time is treated as a combined
                    // segment belonging to the last non-skipped segment.
                }
                None => history_starting_index = current_index as i32,
            }
        }
    }

    // 2. Turn the collected history into weighted value lists.  For segments
    //    without usable history we fall back to the personal best segment
    //    time (only when the previous segment also has a personal best time).
    let mut weighted_lists = Vec::<Option<Vec<(f64, TimeSpan)>>>::with_capacity(segment_count);
    let mut overall_starting_index: i32 = -1;

    for (current_index, current_list) in all_history.iter().enumerate() {
        let current_pb = run.segments[current_index].personal_best().get(method);
        let previous_pb = if overall_starting_index >= 0 {
            run.segments[overall_starting_index as usize]
                .personal_best()
                .get(method)
        } else {
            None
        };

        let mut final_list = current_list
            .iter()
            .filter(|entry| entry.index == overall_starting_index)
            .map(|entry| entry.time)
            .collect::<Vec<_>>();

        let mut null_segment = false;
        if !final_list.is_empty() {
            overall_starting_index = current_index as i32;
        } else if let (Some(current_pb), Some(previous_pb)) = (current_pb, previous_pb) {
            final_list.push(current_pb - previous_pb);
            overall_starting_index = current_index as i32;
        } else {
            null_segment = true;
        }

        if null_segment {
            weighted_lists.push(None);
            continue;
        }

        let count = final_list.len();
        let mut temp_list = final_list
            .iter()
            .enumerate()
            .map(|(index, time)| {
                (PERCENTILE_WEIGHT.powi((count - index - 1) as i32), *time)
            })
            .collect::<Vec<_>>();

        let weighted_list = if temp_list.len() > 1 {
            // The weights are assigned by recency, then sorted by time.  The
            // cumulative weights are rescaled so that they span 0..=1.
            temp_list.sort_by_key(|&(_, time)| time);
            let total_weight: f64 = temp_list.iter().map(|(weight, _)| *weight).sum();
            let smallest_weight = temp_list[0].0;
            let range_weight = total_weight - smallest_weight;

            let mut aggregated_weight = 0.0;
            let mut list = Vec::with_capacity(temp_list.len());
            for (weight, time) in temp_list {
                aggregated_weight += weight;
                list.push((
                    (aggregated_weight - smallest_weight) / range_weight,
                    time,
                ));
            }
            list.sort_by_key(|&(_, time)| time);
            list
        } else {
            vec![(1.0, temp_list[0].1)]
        };

        weighted_lists.push(Some(weighted_list));
    }

    // 3. The caller's goal is the personal best final split time.  We search
    //    for the percentile on the skill curve that produces exactly that
    //    final time.
    let goal_time = run
        .segments
        .last()
        .and_then(|segment| segment.personal_best().get(method));

    let mut output_splits = Vec::with_capacity(segment_count);
    let mut percentile_max = 1.0;
    let mut percentile_min = 0.0;

    for _ in 0..50 {
        output_splits.clear();
        let mut run_sum = TimeSpan::ZERO;
        let percentile = 0.5 * (percentile_max - percentile_min) + percentile_min;

        for weighted_list in &weighted_lists {
            let Some(weighted_list) = weighted_list else {
                output_splits.push(TimeSpan::ZERO);
                continue;
            };

            let current_value = if weighted_list.len() > 1 {
                let mut value = TimeSpan::ZERO;
                for index in 0..weighted_list.len() {
                    let (key, time) = weighted_list[index];
                    if key > percentile {
                        let (previous_key, previous_time) = weighted_list[index - 1];
                        value = percentile_calculate(
                            percentile,
                            time,
                            key,
                            previous_time,
                            previous_key,
                        );
                        break;
                    }
                    if key == percentile {
                        value = time;
                        break;
                    }
                }
                value
            } else {
                weighted_list[0].1
            };

            output_splits.push(current_value);
            run_sum += current_value;
        }

        let Some(goal_time) = goal_time else {
            // Without a personal best there is no target to search for.  One
            // iteration at the median is all LiveSplit does in this case.
            break;
        };

        if run_sum > goal_time {
            percentile_max = percentile;
        } else {
            percentile_min = percentile;
        }

        if run_sum == goal_time {
            break;
        }
    }

    // 4. Store the cumulative split times.  A zero segment time marks a
    //    missing split (same convention as the C# generator).
    let mut total_time = TimeSpan::ZERO;
    for index in 0..segment_count {
        let segment_time = output_splits.get(index).copied().unwrap_or(TimeSpan::ZERO);
        total_time += segment_time;
        let value = if segment_time == TimeSpan::ZERO {
            None
        } else {
            Some(total_time)
        };
        set_comparison_method(run, PERCENTILE, index, method, value);
    }
}

fn generate_balanced_pb(run: &mut Run) {
    generate_balanced_pb_for_method(run, TimingMethod::RealTime);
    generate_balanced_pb_for_method(run, TimingMethod::GameTime);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Attempt, PERSONAL_BEST};

    fn seconds(value: i64) -> TimeSpan {
        TimeSpan::from_seconds(value)
    }

    fn time(real: Option<i64>, game: Option<i64>) -> Time {
        Time {
            real_time: real.map(seconds),
            game_time: game.map(seconds),
        }
    }

    fn set_pb(run: &mut Run, index: usize, real: Option<i64>, game: Option<i64>) {
        run.segments[index].set_personal_best(time(real, game));
    }

    fn set_best(run: &mut Run, index: usize, real: Option<i64>, game: Option<i64>) {
        run.segments[index].best_segment_time = time(real, game);
    }

    fn set_history(
        run: &mut Run,
        segment: usize,
        attempt: i32,
        real: Option<i64>,
        game: Option<i64>,
    ) {
        run.segments[segment]
            .segment_history
            .insert(attempt, time(real, game));
    }

    fn add_attempt(run: &mut Run, index: i32) {
        run.attempts.push(Attempt {
            index,
            ..Attempt::default()
        });
    }

    fn run_with_segments(names: &[&str]) -> Run {
        Run::with_segments("Game", "Any%", names.iter().copied())
    }

    #[test]
    fn sum_of_best_without_history_uses_best_segments() {
        let mut run = run_with_segments(&["A", "B", "C"]);
        set_best(&mut run, 0, Some(5), None);
        set_best(&mut run, 1, Some(10), None);
        set_best(&mut run, 2, Some(15), None);

        assert_eq!(
            calculate_sum_of_best(&run, false, true, TimingMethod::RealTime),
            Some(seconds(30))
        );
        assert_eq!(
            calculate_sum_of_best(&run, true, true, TimingMethod::RealTime),
            Some(seconds(30))
        );
        assert_eq!(
            calculate_sum_of_best(&run, false, true, TimingMethod::GameTime),
            None
        );
    }

    #[test]
    fn sum_of_best_follows_personal_best_without_best_segments() {
        let mut run = run_with_segments(&["A", "B", "C"]);
        set_pb(&mut run, 0, Some(10), Some(8));
        set_pb(&mut run, 1, Some(20), Some(16));
        set_pb(&mut run, 2, Some(30), Some(24));

        assert_eq!(
            calculate_sum_of_best(&run, false, true, TimingMethod::RealTime),
            Some(seconds(30))
        );
        assert_eq!(
            calculate_sum_of_best(&run, true, true, TimingMethod::RealTime),
            Some(seconds(30))
        );
        assert_eq!(
            calculate_sum_of_best(&run, false, true, TimingMethod::GameTime),
            Some(seconds(24))
        );
    }

    #[test]
    fn sum_of_best_uses_skipped_segment_branches() {
        let mut run = run_with_segments(&["A", "B", "C"]);
        set_best(&mut run, 0, Some(10), None);
        set_best(&mut run, 1, Some(10), None);
        set_best(&mut run, 2, Some(5), None);

        add_attempt(&mut run, 1);
        add_attempt(&mut run, 2);

        // Attempt 1: every segment was run.
        set_history(&mut run, 0, 1, Some(10), None);
        set_history(&mut run, 1, 1, Some(10), None);
        set_history(&mut run, 2, 1, Some(10), None);

        // Attempt 2 skipped the first segment, so segment B is a combined
        // segment of A+B.  This should allow a 20s sum of best instead of the
        // 25s achieved by the best segment times alone.
        set_history(&mut run, 0, 2, None, None);
        set_history(&mut run, 1, 2, Some(15), None);
        set_history(&mut run, 2, 2, Some(5), None);

        assert_eq!(
            calculate_sum_of_best(&run, true, false, TimingMethod::RealTime),
            Some(seconds(25))
        );
        assert_eq!(
            calculate_sum_of_best(&run, false, false, TimingMethod::RealTime),
            Some(seconds(20))
        );

        let branch = track_branch(&run, Some(seconds(3)), 1, 2, TimingMethod::RealTime);
        assert_eq!(branch.index, 2);
        assert_eq!(branch.time.real_time, Some(seconds(18)));

        let branch = track_branch(&run, Some(seconds(0)), 2, 2, TimingMethod::RealTime);
        assert_eq!(branch.index, 3);
        assert_eq!(branch.time.real_time, Some(seconds(5)));
    }

    #[test]
    fn sum_of_best_handles_missing_history_without_using_zero() {
        let mut run = run_with_segments(&["A", "B", "C"]);
        set_best(&mut run, 0, Some(10), None);
        set_best(&mut run, 1, Some(10), None);
        set_best(&mut run, 2, Some(10), None);

        add_attempt(&mut run, 1);
        add_attempt(&mut run, 2);
        add_attempt(&mut run, 3);

        // Attempt 1 skipped A and combined it with B.
        set_history(&mut run, 0, 1, None, None);
        set_history(&mut run, 1, 1, Some(5), None);
        set_history(&mut run, 2, 1, Some(20), None);

        // Attempt 2 skipped A and B; it ended after that.
        set_history(&mut run, 0, 2, None, None);
        set_history(&mut run, 1, 2, None, None);

        // Attempt 3 only reached segment A.
        set_history(&mut run, 0, 3, Some(7), None);

        // The combined A+B branch (5) is anchored at the start of the run and
        // is followed by C (10).  Missing history must not be interpreted as a
        // zero-length segment.
        assert_eq!(
            calculate_sum_of_best(&run, false, false, TimingMethod::RealTime),
            Some(seconds(15))
        );
        assert_eq!(
            calculate_sum_of_best(&run, true, false, TimingMethod::RealTime),
            Some(seconds(30))
        );

        assert_eq!(
            track_branch(&run, Some(seconds(0)), 1, 2, TimingMethod::RealTime),
            no_prediction()
        );
    }

    #[test]
    fn sum_of_best_uses_current_run() {
        let mut run = run_with_segments(&["A", "B", "C"]);
        // The run is currently at segment C; segment B was skipped.
        run.segments[0].split_time = time(Some(8), None);
        run.segments[2].split_time = time(Some(20), None);

        assert_eq!(
            calculate_sum_of_best(&run, false, true, TimingMethod::RealTime),
            Some(seconds(20))
        );
        assert_eq!(
            calculate_sum_of_best(&run, false, false, TimingMethod::RealTime),
            None
        );

        let prediction = track_current_run(&run, Some(seconds(5)), 0, TimingMethod::RealTime);
        assert_eq!(prediction.index, 1);
        assert_eq!(prediction.time.real_time, Some(seconds(13)));

        let prediction = track_current_run(&run, Some(seconds(5)), 1, TimingMethod::RealTime);
        assert_eq!(prediction.index, 3);
        assert_eq!(prediction.time.real_time, Some(seconds(17)));

        // The previous segment has no split time, so no continuous route.
        assert_eq!(
            track_current_run(&run, Some(seconds(5)), 2, TimingMethod::RealTime),
            no_prediction()
        );

        // The Best Segments generator deliberately does not use the current
        // run (only the sum displayed by the timer does).
        generate_all(&mut run, &ComparisonSettings::default());
        assert_eq!(
            run.segments[2].comparison(BEST_SEGMENTS).real_time,
            None
        );
    }

    #[test]
    fn track_personal_best_run_uses_pb_segments() {
        let mut run = run_with_segments(&["A", "B", "C"]);
        set_pb(&mut run, 0, Some(10), None);
        set_pb(&mut run, 1, Some(20), None);
        set_pb(&mut run, 2, Some(30), None);

        let prediction =
            track_personal_best_run(&run, Some(seconds(5)), 1, TimingMethod::RealTime);
        assert_eq!(prediction.index, 2);
        assert_eq!(prediction.time.real_time, Some(seconds(15)));

        let prediction =
            track_personal_best_run(&run, Some(seconds(0)), 0, TimingMethod::RealTime);
        assert_eq!(prediction.index, 1);
        assert_eq!(prediction.time.real_time, Some(seconds(10)));

        let prediction = track_personal_best_run(&run, None, 0, TimingMethod::RealTime);
        assert_eq!(prediction.index, 1);
        assert!(prediction.time.is_empty());
    }

    #[test]
    fn sum_of_best_is_calculated_per_timing_method() {
        let mut run = run_with_segments(&["A", "B"]);
        set_best(&mut run, 0, Some(7), Some(5));
        set_best(&mut run, 1, Some(12), Some(10));

        add_attempt(&mut run, 1);
        // For game time the first segment was skipped, combining it with the
        // second segment.  For real time the first segment was played.
        set_history(&mut run, 0, 1, Some(3), None);
        set_history(&mut run, 1, 1, None, Some(8));

        // Game time: best A (5) or combined A+B (8), then B (10) -> 8 is best.
        assert_eq!(
            calculate_sum_of_best(&run, false, false, TimingMethod::GameTime),
            Some(seconds(8))
        );
        // Real time: A best (7) + B best (12) = 19.
        assert_eq!(
            calculate_sum_of_best(&run, false, false, TimingMethod::RealTime),
            Some(seconds(19))
        );
    }

    #[test]
    fn sum_of_worst_uses_worst_attempt() {
        let mut run = run_with_segments(&["A", "B", "C"]);
        for index in 0..3 {
            set_best(&mut run, index, Some(10), None);
        }

        for attempt in 1..=3 {
            add_attempt(&mut run, attempt);
            for index in 0..3 {
                let time = match attempt {
                    1 => 10,
                    2 => 20,
                    _ => 5,
                };
                set_history(&mut run, index, attempt, Some(time), None);
            }
        }

        assert_eq!(
            calculate_sum_of_worst(&run, false, TimingMethod::RealTime),
            Some(seconds(60))
        );
        assert_eq!(
            calculate_sum_of_worst(&run, true, TimingMethod::RealTime),
            Some(seconds(60))
        );
        assert_eq!(
            calculate_sum_of_worst(&run, false, TimingMethod::GameTime),
            None
        );
    }

    #[test]
    fn sum_of_worst_skips_combined_history_segments() {
        let mut run = run_with_segments(&["A", "B", "C"]);
        for index in 0..3 {
            set_best(&mut run, index, Some(10), None);
        }

        add_attempt(&mut run, 1);
        add_attempt(&mut run, 2);
        add_attempt(&mut run, 3);

        for index in 0..3 {
            set_history(&mut run, index, 1, Some(10), None);
            set_history(&mut run, index, 3, Some(5), None);
        }
        // Attempt 2 skipped the first segment.
        set_history(&mut run, 0, 2, None, None);
        set_history(&mut run, 1, 2, Some(20), None);
        set_history(&mut run, 2, 2, Some(20), None);

        // Attempt 2's combined segment must not be counted twice (which would
        // produce 50); its actual total is 40.
        assert_eq!(
            calculate_sum_of_worst(&run, false, TimingMethod::RealTime),
            Some(seconds(40))
        );
    }

    #[test]
    fn sum_of_worst_uses_current_run() {
        let mut run = run_with_segments(&["A", "B"]);
        set_best(&mut run, 0, Some(5), None);
        set_best(&mut run, 1, Some(5), None);
        run.segments[0].split_time = time(Some(10), None);

        assert_eq!(
            calculate_sum_of_worst(&run, false, TimingMethod::RealTime),
            Some(seconds(10))
        );
        assert_eq!(
            calculate_sum_of_worst(&run, true, TimingMethod::RealTime),
            Some(seconds(15))
        );

        // The Worst Segments generator does not include the current run.
        generate_all(&mut run, &ComparisonSettings::default());
        assert_eq!(
            run.segments[1].comparison(WORST_SEGMENTS).real_time,
            Some(seconds(10))
        );
    }

    #[test]
    fn average_segments_are_weighted_and_skip_combined_segments() {
        let mut run = run_with_segments(&["A", "B", "C"]);
        add_attempt(&mut run, 1);
        add_attempt(&mut run, 2);

        set_history(&mut run, 0, 1, Some(5), None);
        set_history(&mut run, 1, 1, Some(4), None);
        set_history(&mut run, 2, 1, Some(8), None);

        // Segment B was skipped; its 100s value is a combined segment and must
        // not be counted as B's time.
        set_history(&mut run, 0, 2, Some(12), None);
        set_history(&mut run, 1, 2, None, None);
        set_history(&mut run, 2, 2, Some(100), None);

        generate_all(&mut run, &ComparisonSettings::default());

        // A: (0.75 * 5 + 12) / 1.75 = 9
        assert_eq!(
            run.segments[0].comparison(AVERAGE_SEGMENTS).real_time,
            Some(seconds(9))
        );
        // B: only attempt 1 contributes (4), so the cumulative time is 13.
        assert_eq!(
            run.segments[1].comparison(AVERAGE_SEGMENTS).real_time,
            Some(seconds(13))
        );
        // C: only attempt 1 contributes (8), so the cumulative time is 21.
        assert_eq!(
            run.segments[2].comparison(AVERAGE_SEGMENTS).real_time,
            Some(seconds(21))
        );
        assert_eq!(
            run.segments[2].comparison(AVERAGE_SEGMENTS).game_time,
            None
        );
    }

    #[test]
    fn median_segments_are_weighted() {
        let mut run = run_with_segments(&["A", "B"]);
        for (attempt, (a, b)) in [(1, (10, 2)), (2, (20, 4)), (3, (30, 6))] {
            add_attempt(&mut run, attempt);
            set_history(&mut run, 0, attempt, Some(a), None);
            set_history(&mut run, 1, attempt, Some(b), None);
        }

        generate_all(&mut run, &ComparisonSettings::default());

        // The weighted median of [10, 20, 30] is 20 and the one of [2, 4, 6]
        // is 4, giving cumulative splits of 20 and 24.
        assert_eq!(
            run.segments[0].comparison(MEDIAN_SEGMENTS).real_time,
            Some(seconds(20))
        );
        assert_eq!(
            run.segments[1].comparison(MEDIAN_SEGMENTS).real_time,
            Some(seconds(24))
        );
    }

    #[test]
    fn best_split_times_use_pb_and_all_attempts() {
        let mut run = run_with_segments(&["A", "B", "C"]);
        set_best(&mut run, 0, Some(8), None);
        set_pb(&mut run, 0, Some(10), None);
        set_pb(&mut run, 1, Some(20), None);
        set_pb(&mut run, 2, Some(30), None);

        for attempt in 1..=3 {
            add_attempt(&mut run, attempt);
        }
        set_history(&mut run, 0, 1, Some(5), None);
        set_history(&mut run, 1, 1, Some(10), None);
        set_history(&mut run, 2, 1, Some(15), None);

        set_history(&mut run, 0, 2, Some(4), None);
        set_history(&mut run, 1, 2, Some(12), None);
        set_history(&mut run, 2, 2, Some(10), None);

        // The third attempt skipped B but continued with C.
        set_history(&mut run, 0, 3, Some(3), None);
        set_history(&mut run, 1, 3, None, None);
        set_history(&mut run, 2, 3, Some(5), None);

        generate_all(&mut run, &ComparisonSettings::default());

        assert_eq!(
            run.segments[0].comparison(BEST_SPLIT_TIMES).real_time,
            Some(seconds(3))
        );
        assert_eq!(
            run.segments[1].comparison(BEST_SPLIT_TIMES).real_time,
            Some(seconds(15))
        );
        // Attempt 3 accumulates 3 + 5 = 8 across the skipped segment.
        assert_eq!(
            run.segments[2].comparison(BEST_SPLIT_TIMES).real_time,
            Some(seconds(8))
        );
    }

    #[test]
    fn latest_run_prefers_the_attempt_that_got_furthest() {
        let mut run = run_with_segments(&["A", "B", "C"]);
        add_attempt(&mut run, 1);
        add_attempt(&mut run, 2);

        // Attempt 1 finished the run.
        set_history(&mut run, 0, 1, Some(10), Some(9));
        set_history(&mut run, 1, 1, Some(20), Some(18));
        set_history(&mut run, 2, 1, Some(30), Some(27));

        // Attempt 2 is more recent but only reached B.
        set_history(&mut run, 0, 2, Some(5), Some(4));
        set_history(&mut run, 1, 2, Some(10), Some(8));

        generate_all(&mut run, &ComparisonSettings::default());

        assert_eq!(
            run.segments[0].comparison(LATEST_RUN).real_time,
            Some(seconds(10))
        );
        assert_eq!(
            run.segments[1].comparison(LATEST_RUN).real_time,
            Some(seconds(30))
        );
        assert_eq!(
            run.segments[2].comparison(LATEST_RUN).real_time,
            Some(seconds(60))
        );
        assert_eq!(
            run.segments[2].comparison(LATEST_RUN).game_time,
            Some(seconds(54))
        );

        // Now the second attempt reaches the final segment; it wins.
        set_history(&mut run, 2, 2, Some(40), Some(36));
        generate_all(&mut run, &ComparisonSettings::default());

        assert_eq!(
            run.segments[0].comparison(LATEST_RUN).real_time,
            Some(seconds(5))
        );
        assert_eq!(
            run.segments[1].comparison(LATEST_RUN).real_time,
            Some(seconds(15))
        );
        assert_eq!(
            run.segments[2].comparison(LATEST_RUN).real_time,
            Some(seconds(55))
        );
    }

    #[test]
    fn latest_run_keeps_total_across_a_missing_timing_method() {
        let mut run = run_with_segments(&["A", "B", "C"]);
        add_attempt(&mut run, 1);
        set_history(&mut run, 0, 1, Some(10), Some(8));
        set_history(&mut run, 1, 1, Some(20), None);
        set_history(&mut run, 2, 1, Some(30), Some(24));

        generate_all(&mut run, &ComparisonSettings::default());

        // Real time is a normal cumulative run.
        assert_eq!(
            run.segments[2].comparison(LATEST_RUN).real_time,
            Some(seconds(60))
        );
        // Game time has no split for B, but C resumes from A's total.
        assert_eq!(
            run.segments[1].comparison(LATEST_RUN).game_time,
            None
        );
        assert_eq!(
            run.segments[2].comparison(LATEST_RUN).game_time,
            Some(seconds(32))
        );
    }

    #[test]
    fn balanced_pb_balances_to_the_personal_best_goal() {
        let mut run = run_with_segments(&["A", "B", "C"]);
        set_pb(&mut run, 0, Some(10), None);
        set_pb(&mut run, 1, Some(20), None);
        set_pb(&mut run, 2, Some(30), None);

        // Three attempts with different pacing, all finishing at 30s.
        let attempts = [
            (1, [8, 15, 7]),
            (2, [12, 10, 8]),
            (3, [9, 12, 9]),
        ];
        for (attempt, times) in attempts {
            add_attempt(&mut run, attempt);
            for (index, time) in times.into_iter().enumerate() {
                set_history(&mut run, index, attempt, Some(time), None);
            }
        }

        generate_all(&mut run, &ComparisonSettings::default());

        let balanced = run.segments[2].comparison(PERCENTILE).real_time;
        assert!(balanced.is_some());
        let difference = (balanced.unwrap() - seconds(30)).abs();
        assert!(
            difference <= TimeSpan::from_millis(1),
            "Balanced PB ended at {:?} instead of 30s",
            balanced
        );

        // Every split must be present and cumulative.
        let mut previous = TimeSpan::ZERO;
        for segment in &run.segments {
            let time = segment
                .comparison(PERCENTILE)
                .real_time
                .expect("Balanced PB should have a time");
            assert!(time >= previous);
            previous = time;
        }
    }

    #[test]
    fn generate_all_populates_every_comparison_for_both_methods() {
        let mut run = run_with_segments(&["A", "B", "C"]);
        set_pb(&mut run, 0, Some(10), Some(9));
        set_pb(&mut run, 1, Some(20), Some(17));
        set_pb(&mut run, 2, Some(30), Some(26));
        set_best(&mut run, 0, Some(5), Some(4));
        set_best(&mut run, 1, Some(9), Some(8));
        set_best(&mut run, 2, Some(11), Some(10));

        for attempt in 1..=2 {
            add_attempt(&mut run, attempt);
            for index in 0..3 {
                let real = 8 + index as i64 + attempt as i64;
                let game = real - 1;
                set_history(&mut run, index, attempt, Some(real), Some(game));
            }
        }

        generate_all(&mut run, &ComparisonSettings::default());

        for comparison in [
            BEST_SEGMENTS,
            BEST_SPLIT_TIMES,
            LATEST_RUN,
            AVERAGE_SEGMENTS,
            MEDIAN_SEGMENTS,
            WORST_SEGMENTS,
            PERCENTILE,
        ] {
            for (index, segment) in run.segments.iter().enumerate() {
                assert!(
                    segment.comparison(comparison).real_time.is_some(),
                    "{comparison} is missing real time for segment {index}"
                );
                assert!(
                    segment.comparison(comparison).game_time.is_some(),
                    "{comparison} is missing game time for segment {index}"
                );
            }
        }

        // Personal best is a stored comparison and must not be touched.
        assert_eq!(
            run.segments[2].comparison(PERSONAL_BEST).real_time,
            Some(seconds(30))
        );
        assert_eq!(
            run.segments[2].comparison(PERSONAL_BEST).game_time,
            Some(seconds(26))
        );
        assert!(run
            .custom_comparisons
            .iter()
            .any(|name| name == PERSONAL_BEST));

        // The generated Best Segments / Worst Segments totals must match the
        // standalone Sum of Best / Sum of Worst calculations.
        assert_eq!(
            run.segments[2].comparison(BEST_SEGMENTS).real_time,
            calculate_sum_of_best(&run, false, false, TimingMethod::RealTime)
        );
        assert_eq!(
            run.segments[2].comparison(WORST_SEGMENTS).real_time,
            calculate_sum_of_worst(&run, false, TimingMethod::RealTime)
        );
    }

    #[test]
    fn simple_sum_of_best_setting_is_respected_by_best_segments() {
        let mut run = run_with_segments(&["A", "B", "C"]);
        set_best(&mut run, 0, Some(10), None);
        set_best(&mut run, 1, Some(10), None);
        set_best(&mut run, 2, Some(5), None);

        add_attempt(&mut run, 1);
        set_history(&mut run, 0, 1, None, None);
        set_history(&mut run, 1, 1, Some(15), None);
        set_history(&mut run, 2, 1, Some(5), None);

        generate_all(
            &mut run,
            &ComparisonSettings {
                simple_sum_of_best: true,
                use_current_run: true,
            },
        );
        assert_eq!(
            run.segments[2].comparison(BEST_SEGMENTS).real_time,
            Some(seconds(25))
        );

        generate_all(
            &mut run,
            &ComparisonSettings {
                simple_sum_of_best: false,
                use_current_run: true,
            },
        );
        assert_eq!(
            run.segments[2].comparison(BEST_SEGMENTS).real_time,
            Some(seconds(20))
        );
    }

    #[test]
    fn single_attempt_generators_use_that_attempt() {
        let mut run = run_with_segments(&["A", "B"]);
        add_attempt(&mut run, 1);
        set_history(&mut run, 0, 1, Some(5), None);
        set_history(&mut run, 1, 1, Some(6), None);
        set_best(&mut run, 0, Some(4), None);
        set_best(&mut run, 1, Some(6), None);

        generate_all(&mut run, &ComparisonSettings::default());

        for comparison in [AVERAGE_SEGMENTS, MEDIAN_SEGMENTS, LATEST_RUN] {
            assert_eq!(
                run.segments[0].comparison(comparison).real_time,
                Some(seconds(5)),
                "{comparison} segment A"
            );
            assert_eq!(
                run.segments[1].comparison(comparison).real_time,
                Some(seconds(11)),
                "{comparison} segment B"
            );
        }

        assert_eq!(
            calculate_sum_of_worst(&run, false, TimingMethod::RealTime),
            Some(seconds(11))
        );
    }

    #[test]
    fn no_history_generators_are_empty() {
        let mut run = run_with_segments(&["A", "B"]);
        set_pb(&mut run, 0, Some(10), None);
        set_pb(&mut run, 1, Some(20), None);
        set_best(&mut run, 0, Some(5), None);
        set_best(&mut run, 1, Some(8), None);

        generate_all(&mut run, &ComparisonSettings::default());

        for comparison in [AVERAGE_SEGMENTS, MEDIAN_SEGMENTS, LATEST_RUN] {
            for segment in &run.segments {
                assert_eq!(
                    segment.comparison(comparison).real_time,
                    None,
                    "{comparison} should be empty without history"
                );
            }
        }

        // Best Segments only needs the best segment times.
        assert_eq!(
            run.segments[1].comparison(BEST_SEGMENTS).real_time,
            Some(seconds(13))
        );
        // Worst Segments also considers the personal best route, which is
        // slower than the best segment times here.
        assert_eq!(
            run.segments[1].comparison(WORST_SEGMENTS).real_time,
            Some(seconds(20))
        );
    }

    #[test]
    fn empty_run_is_handled() {
        let mut run = Run::with_segments("Game", "Any%", std::iter::empty::<&str>());

        assert_eq!(
            calculate_sum_of_best(&run, false, true, TimingMethod::RealTime),
            Some(TimeSpan::ZERO)
        );
        assert_eq!(
            calculate_sum_of_worst(&run, true, TimingMethod::RealTime),
            Some(TimeSpan::ZERO)
        );

        generate_all(&mut run, &ComparisonSettings::default());
        assert!(run.segments.is_empty());
    }
}
