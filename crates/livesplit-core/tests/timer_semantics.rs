//! Integration tests for the timer state machine and the derived analysis
//! helpers.  They are written against the behaviour of LiveSplit's
//! `TimerModel` / `LiveSplitState` and use the deterministic [`TestClock`].

use std::sync::Arc;

use livesplit_core::analysis::{
    check_best_segment, check_live_delta, get_last_delta, get_live_segment_delta,
    get_live_segment_time, get_previous_segment_delta, get_previous_segment_time,
    possible_time_save, predicted_time, split_color, SplitColor,
};
use livesplit_core::{
    ComparisonSettings, Run, TestClock, Time, TimeSpan, Timer, TimerPhase, TimingMethod,
    BEST_SEGMENTS, CURRENT_COMPARISON, NONE, PERSONAL_BEST,
};

fn seconds(value: i64) -> TimeSpan {
    TimeSpan::from_seconds(value)
}

fn real(value: Option<i64>) -> Time {
    Time::new(value.map(seconds), None)
}

fn timer_with(run: Run) -> (TestClock, Timer) {
    let clock = TestClock::new();
    let timer = Timer::new(run, Arc::new(clock.clone()));
    (clock, timer)
}

fn run_with_pb(pb: &[Option<i64>]) -> Run {
    let mut run = Run::with_segments(
        "Game",
        "Any%",
        (0..pb.len()).map(|index| format!("Segment {}", index + 1)),
    );
    for (segment, time) in run.segments.iter_mut().zip(pb.iter().copied()) {
        segment.set_personal_best(real(time));
    }
    run
}

fn run_with_best(pb: &[Option<i64>], best: &[Option<i64>]) -> Run {
    let mut run = run_with_pb(pb);
    for (segment, time) in run.segments.iter_mut().zip(best.iter().copied()) {
        segment.best_segment_time = real(time);
    }
    run
}

#[test]
fn full_run_start_split_pause_and_reset() {
    let (clock, mut timer) = timer_with(run_with_pb(&[Some(10), Some(20), Some(30)]));

    assert_eq!(timer.phase, TimerPhase::NotRunning);
    assert_eq!(timer.current_split_index, -1);
    assert_eq!(timer.current_time().real_time, Some(TimeSpan::ZERO));
    assert_eq!(timer.current_time().game_time, None);

    timer.start();
    assert_eq!(timer.phase, TimerPhase::Running);
    assert_eq!(timer.current_split_index, 0);
    assert_eq!(timer.run.attempt_count, 1);
    assert_eq!(timer.current_attempt_duration(), Some(TimeSpan::ZERO));

    clock.advance(seconds(10));
    timer.split();
    assert_eq!(timer.run.segments[0].split_time.real_time, Some(seconds(10)));
    assert_eq!(timer.current_split_index, 1);

    // A pause freezes the running time and is tracked separately.
    clock.advance(seconds(2));
    timer.pause();
    assert_eq!(timer.phase, TimerPhase::Paused);
    assert_eq!(timer.current_time().real_time, Some(seconds(12)));
    assert_eq!(timer.pause_time(), Some(TimeSpan::ZERO));
    clock.advance(seconds(6));
    assert_eq!(timer.current_time().real_time, Some(seconds(12)));
    assert_eq!(timer.pause_time(), Some(seconds(6)));
    // Pausing does not stop the wall clock attempt duration.
    assert_eq!(timer.current_attempt_duration(), Some(seconds(18)));

    timer.pause();
    assert_eq!(timer.phase, TimerPhase::Running);
    assert_eq!(timer.current_time().real_time, Some(seconds(12)));
    clock.advance(seconds(8));
    assert_eq!(timer.current_time().real_time, Some(seconds(20)));
    timer.split();
    assert_eq!(timer.run.segments[1].split_time.real_time, Some(seconds(20)));

    clock.advance(seconds(10));
    assert_eq!(timer.current_time().real_time, Some(seconds(30)));
    timer.split();
    assert_eq!(timer.phase, TimerPhase::Ended);
    assert_eq!(timer.current_split_index, 3);
    assert_eq!(timer.current_time().real_time, Some(seconds(30)));
    assert_eq!(timer.current_attempt_duration(), Some(seconds(36)));
    // Splitting while ended is a no-op.
    timer.split();
    assert_eq!(timer.phase, TimerPhase::Ended);

    timer.reset();
    assert_eq!(timer.phase, TimerPhase::NotRunning);
    assert_eq!(timer.current_split_index, -1);
    assert_eq!(timer.current_attempt_duration(), Some(TimeSpan::ZERO));
    for segment in &timer.run.segments {
        assert_eq!(segment.split_time, Time::default());
    }

    assert_eq!(timer.run.attempts.len(), 1);
    let attempt = &timer.run.attempts[0];
    assert_eq!(attempt.index, 1);
    assert_eq!(attempt.time.real_time, Some(seconds(30)));
    assert_eq!(attempt.pause_time, Some(seconds(6)));
    assert!(attempt.started.is_some());
    assert!(attempt.ended.is_some());

    assert_eq!(timer.run.segments[0].personal_best().real_time, Some(seconds(10)));
    assert_eq!(timer.run.segments[1].personal_best().real_time, Some(seconds(20)));
    assert_eq!(timer.run.segments[2].personal_best().real_time, Some(seconds(30)));
    for segment in &timer.run.segments {
        assert_eq!(segment.best_segment_time.real_time, Some(seconds(10)));
        let history = segment.segment_history.get(&1).expect("attempt 1 history");
        assert_eq!(history.real_time, Some(seconds(10)));
    }
}

#[test]
fn pb_and_best_segments_across_attempts() {
    let (clock, mut timer) = timer_with(run_with_pb(&[None, None]));

    // Attempt 1: 10 + 10 = 20, first PB.
    timer.start();
    clock.advance(seconds(10));
    timer.split();
    clock.advance(seconds(10));
    timer.split();
    timer.reset();
    assert_eq!(timer.run.segments[0].personal_best().real_time, Some(seconds(10)));
    assert_eq!(timer.run.segments[1].personal_best().real_time, Some(seconds(20)));

    // Attempt 2: 9 + 11 = 20 (equal to PB, not a new PB), new best segment.
    timer.start();
    clock.advance(seconds(9));
    timer.split();
    clock.advance(seconds(11));
    timer.split();
    timer.reset();
    assert_eq!(timer.run.segments[1].personal_best().real_time, Some(seconds(20)));
    assert_eq!(timer.run.segments[0].best_segment_time.real_time, Some(seconds(9)));
    assert_eq!(timer.run.segments[1].best_segment_time.real_time, Some(seconds(10)));

    // Attempt 3: 8 + 10 = 18, a new PB.
    timer.start();
    clock.advance(seconds(8));
    timer.split();
    clock.advance(seconds(10));
    timer.split();
    timer.reset();
    assert_eq!(timer.run.segments[0].personal_best().real_time, Some(seconds(8)));
    assert_eq!(timer.run.segments[1].personal_best().real_time, Some(seconds(18)));
    assert_eq!(timer.run.segments[0].best_segment_time.real_time, Some(seconds(8)));
    assert_eq!(timer.run.segments[1].best_segment_time.real_time, Some(seconds(10)));

    // Attempt 4: 7 + 12 = 19. A new first best segment, no PB.
    timer.start();
    clock.advance(seconds(7));
    timer.split();
    clock.advance(seconds(12));
    timer.split();
    timer.reset();
    assert_eq!(timer.run.segments[0].personal_best().real_time, Some(seconds(8)));
    assert_eq!(timer.run.segments[1].personal_best().real_time, Some(seconds(18)));
    assert_eq!(timer.run.segments[0].best_segment_time.real_time, Some(seconds(7)));
    assert_eq!(timer.run.segments[1].best_segment_time.real_time, Some(seconds(10)));

    let indices: Vec<i32> = timer.run.attempts.iter().map(|attempt| attempt.index).collect();
    assert_eq!(indices, vec![1, 2, 3, 4]);
    let totals: Vec<Option<TimeSpan>> = timer
        .run
        .attempts
        .iter()
        .map(|attempt| attempt.time.real_time)
        .collect();
    assert_eq!(
        totals,
        vec![
            Some(seconds(20)),
            Some(seconds(20)),
            Some(seconds(18)),
            Some(seconds(19))
        ]
    );

    let segment_times = |segment: usize| -> Vec<Option<TimeSpan>> {
        (1..=4)
            .map(|index| {
                timer.run.segments[segment]
                    .segment_history
                    .get(&index)
                    .and_then(|time| time.real_time)
            })
            .collect()
    };
    assert_eq!(
        segment_times(0),
        vec![
            Some(seconds(10)),
            Some(seconds(9)),
            Some(seconds(8)),
            Some(seconds(7))
        ]
    );
    assert_eq!(
        segment_times(1),
        vec![
            Some(seconds(10)),
            Some(seconds(11)),
            Some(seconds(10)),
            Some(seconds(12))
        ]
    );
}

#[test]
fn skipped_segments_keep_null_history_markers() {
    let (clock, mut timer) = timer_with(run_with_pb(&[Some(10), Some(20), Some(30)]));

    timer.start();
    clock.advance(seconds(10));
    timer.split();
    timer.skip_split();
    assert_eq!(timer.current_split_index, 2);
    assert_eq!(timer.run.segments[1].split_time, Time::default());
    clock.advance(seconds(15));
    timer.split();
    assert_eq!(timer.phase, TimerPhase::Ended);
    assert_eq!(timer.run.segments[2].split_time.real_time, Some(seconds(25)));

    timer.reset();
    assert_eq!(timer.run.segments[0].personal_best().real_time, Some(seconds(10)));
    assert_eq!(timer.run.segments[1].personal_best().real_time, None);
    assert_eq!(timer.run.segments[2].personal_best().real_time, Some(seconds(25)));
    assert_eq!(timer.run.segments[0].best_segment_time.real_time, Some(seconds(10)));
    // The best segments for the skipped and following segment are kept from
    // the old PB, which had a 10s segment for the last split as well.
    assert_eq!(timer.run.segments[1].best_segment_time.real_time, Some(seconds(10)));
    assert_eq!(timer.run.segments[2].best_segment_time.real_time, Some(seconds(10)));

    let first = timer.run.segments[0].segment_history.get(&1).unwrap();
    assert_eq!(first.real_time, Some(seconds(10)));
    let skipped = timer.run.segments[1]
        .segment_history
        .get(&1)
        .expect("skipped segment history marker");
    assert!(skipped.is_empty());
    let last = timer.run.segments[2].segment_history.get(&1).unwrap();
    assert_eq!(last.real_time, Some(seconds(15)));
}

#[test]
fn undo_split_after_the_run_ended() {
    let (clock, mut timer) = timer_with(run_with_pb(&[Some(10), Some(20)]));

    timer.start();
    clock.advance(seconds(10));
    timer.split();
    clock.advance(seconds(10));
    timer.split();
    assert_eq!(timer.phase, TimerPhase::Ended);
    assert_eq!(timer.current_time().real_time, Some(seconds(20)));

    timer.undo_split();
    assert_eq!(timer.phase, TimerPhase::Running);
    assert_eq!(timer.current_split_index, 1);
    assert_eq!(timer.run.segments[1].split_time, Time::default());
    assert_eq!(timer.current_time().real_time, Some(seconds(20)));

    clock.advance(seconds(2));
    timer.split();
    assert_eq!(timer.phase, TimerPhase::Ended);
    assert_eq!(timer.run.segments[1].split_time.real_time, Some(seconds(22)));

    timer.reset();
    assert_eq!(timer.run.attempts.len(), 1);
    assert_eq!(timer.run.attempts[0].time.real_time, Some(seconds(22)));
    // The slower final segment does not become a new best segment; the PB
    // segment for the second split is still 10 seconds.
    assert_eq!(timer.run.segments[1].best_segment_time.real_time, Some(seconds(10)));
}

#[test]
fn game_time_with_loading_times_and_pauses() {
    let (clock, mut timer) = timer_with(run_with_pb(&[None]));

    timer.start();
    timer.initialize_game_time();
    assert!(timer.is_game_time_initialized);
    assert_eq!(timer.loading_times, TimeSpan::ZERO);
    assert_eq!(timer.current_time().game_time, Some(TimeSpan::ZERO));

    clock.advance(seconds(5));
    assert_eq!(
        timer.current_time(),
        Time::new(Some(seconds(5)), Some(seconds(5)))
    );

    timer.set_loading_times(seconds(2));
    assert_eq!(timer.loading_times, seconds(2));
    assert_eq!(timer.current_time().game_time, Some(seconds(3)));

    clock.advance(seconds(5));
    assert_eq!(timer.current_time().real_time, Some(seconds(10)));
    assert_eq!(timer.current_time().game_time, Some(seconds(8)));

    // Initializing an already initialized game time must not discard the
    // loading times that have been accumulated.
    timer.initialize_game_time();
    assert_eq!(timer.loading_times, seconds(2));
    assert_eq!(timer.current_time().game_time, Some(seconds(8)));

    timer.set_game_time_paused(true);
    assert!(timer.is_game_time_paused());
    assert_eq!(timer.game_time_pause_time, Some(seconds(8)));
    clock.advance(seconds(4));
    assert_eq!(timer.current_time().real_time, Some(seconds(14)));
    assert_eq!(timer.current_time().game_time, Some(seconds(8)));

    timer.set_game_time_paused(false);
    assert!(!timer.is_game_time_paused());
    assert_eq!(timer.loading_times, seconds(6));
    assert_eq!(timer.current_time().game_time, Some(seconds(8)));

    clock.advance(seconds(2));
    assert_eq!(timer.current_time().real_time, Some(seconds(16)));
    assert_eq!(timer.current_time().game_time, Some(seconds(10)));

    // Setting loading times while game time is paused keeps the current game
    // time at `real time - loading times`.
    timer.set_game_time_paused(true);
    clock.advance(seconds(1));
    timer.set_loading_times(seconds(3));
    assert_eq!(timer.current_time().real_time, Some(seconds(17)));
    assert_eq!(timer.current_time().game_time, Some(seconds(14)));
    assert_eq!(timer.game_time_pause_time, Some(seconds(14)));
    timer.set_game_time_paused(false);
    assert_eq!(timer.loading_times, seconds(3));
    assert_eq!(timer.current_time().game_time, Some(seconds(14)));

    timer.split();
    assert_eq!(timer.phase, TimerPhase::Ended);
    assert_eq!(
        timer.run.segments[0].split_time,
        Time::new(Some(seconds(17)), Some(seconds(14)))
    );
    assert_eq!(
        timer.current_time(),
        Time::new(Some(seconds(17)), Some(seconds(14)))
    );

    // Game time is reset to an initialized zero loading time and is
    // uninitialized again for the next attempt.
    timer.reset();
    assert!(timer.is_game_time_initialized);
    assert_eq!(timer.loading_times, TimeSpan::ZERO);
    assert_eq!(timer.current_time().game_time, Some(TimeSpan::ZERO));
    timer.start();
    assert!(!timer.is_game_time_initialized);
    assert_eq!(timer.current_time().game_time, None);
}

#[test]
fn undo_all_pauses_while_paused() {
    let (clock, mut timer) = timer_with(run_with_pb(&[Some(10)]));

    timer.start();
    clock.advance(seconds(5));
    timer.pause();
    clock.advance(seconds(10));
    assert_eq!(timer.current_time().real_time, Some(seconds(5)));
    assert_eq!(timer.pause_time(), Some(seconds(10)));

    timer.undo_all_pauses();
    assert_eq!(timer.phase, TimerPhase::Running);
    // Undoing all pauses removes the pauses from the timer value: the timer
    // now shows the full wall clock time since the start.
    assert_eq!(timer.current_time().real_time, Some(seconds(15)));
    assert_eq!(timer.pause_time(), None);

    clock.advance(seconds(2));
    assert_eq!(timer.current_time().real_time, Some(seconds(17)));
    timer.split();
    assert_eq!(timer.run.segments[0].split_time.real_time, Some(seconds(17)));
}

#[test]
fn undo_all_pauses_while_ended_adjusts_the_final_split() {
    let (clock, mut timer) = timer_with(run_with_pb(&[Some(10)]));

    timer.start();
    clock.advance(seconds(5));
    timer.pause();
    clock.advance(seconds(10));
    timer.pause();
    clock.advance(seconds(5));
    assert_eq!(timer.current_time().real_time, Some(seconds(10)));
    timer.split();
    assert_eq!(timer.phase, TimerPhase::Ended);
    assert_eq!(timer.run.segments[0].split_time.real_time, Some(seconds(10)));
    assert_eq!(timer.pause_time(), Some(seconds(10)));

    timer.undo_all_pauses();
    assert_eq!(timer.phase, TimerPhase::Ended);
    assert_eq!(timer.run.segments[0].split_time.real_time, Some(seconds(20)));
    assert_eq!(timer.current_time().real_time, Some(seconds(20)));
    assert_eq!(timer.pause_time(), None);
}

#[test]
fn comparison_switching_wraps_and_handles_unknown_comparisons() {
    let mut run = run_with_pb(&[Some(10)]);
    run.custom_comparisons.push("My Comparison".to_string());
    let (_clock, mut timer) = timer_with(run);

    let comparisons = timer.run.comparisons();
    assert_eq!(comparisons[0], PERSONAL_BEST);
    assert_eq!(comparisons[1], "My Comparison");
    assert_eq!(comparisons.last().unwrap(), NONE);

    timer.switch_comparison_next();
    assert_eq!(timer.current_comparison, "My Comparison");
    timer.switch_comparison_previous();
    assert_eq!(timer.current_comparison, PERSONAL_BEST);
    timer.switch_comparison_previous();
    assert_eq!(timer.current_comparison, NONE);
    timer.switch_comparison_next();
    assert_eq!(timer.current_comparison, PERSONAL_BEST);

    // An unknown current comparison falls back to the LiveSplit formula:
    // next => first comparison, previous => second to last comparison.
    timer.set_current_comparison("Bogus");
    timer.switch_comparison_next();
    assert_eq!(timer.current_comparison, PERSONAL_BEST);

    timer.set_current_comparison("Bogus");
    timer.switch_comparison_previous();
    assert_eq!(timer.current_comparison, comparisons[comparisons.len() - 2]);
}

#[test]
fn positive_and_negative_offsets() {
    // Positive offset: the timer starts ahead of the run.
    let (clock, mut timer) = {
        let mut run = run_with_pb(&[Some(10)]);
        run.offset = seconds(2);
        timer_with(run)
    };
    timer.start();
    assert_eq!(timer.current_time().real_time, Some(seconds(2)));
    clock.advance(seconds(3));
    assert_eq!(timer.current_time().real_time, Some(seconds(5)));
    timer.split();
    assert_eq!(timer.run.segments[0].split_time.real_time, Some(seconds(5)));

    // Negative offset: the timer starts before zero. Splitting at a
    // non-positive time is rejected.
    let (clock, mut timer) = {
        let mut run = run_with_pb(&[Some(10), Some(20)]);
        run.offset = seconds(-4);
        timer_with(run)
    };
    timer.start();
    assert_eq!(timer.current_time().real_time, Some(seconds(-4)));
    clock.advance(seconds(2));
    assert_eq!(timer.current_time().real_time, Some(seconds(-2)));
    timer.split();
    assert_eq!(timer.current_split_index, 0);
    assert_eq!(timer.run.segments[0].split_time, Time::default());

    clock.advance(seconds(2));
    assert_eq!(timer.current_time().real_time, Some(TimeSpan::ZERO));
    timer.split();
    assert_eq!(timer.current_split_index, 0);

    clock.advance(seconds(1));
    assert_eq!(timer.current_time().real_time, Some(seconds(1)));
    timer.split();
    assert_eq!(timer.run.segments[0].split_time.real_time, Some(seconds(1)));

    // Pause math must not be thrown off by the negative offset.
    timer.pause();
    assert_eq!(timer.pause_time(), Some(TimeSpan::ZERO));
    clock.advance(seconds(3));
    assert_eq!(timer.current_time().real_time, Some(seconds(1)));
    assert_eq!(timer.pause_time(), Some(seconds(3)));
    timer.undo_all_pauses();
    assert_eq!(timer.current_time().real_time, Some(seconds(4)));
    assert_eq!(timer.pause_time(), None);
    clock.advance(seconds(1));
    timer.split();
    assert_eq!(timer.run.segments[1].split_time.real_time, Some(seconds(5)));
}

#[test]
fn reset_while_paused_records_the_partial_attempt() {
    let (clock, mut timer) = timer_with(run_with_pb(&[Some(10), Some(20)]));

    timer.start();
    clock.advance(seconds(5));
    timer.split();
    clock.advance(seconds(2));
    timer.pause();
    clock.advance(seconds(3));
    timer.reset();

    assert_eq!(timer.phase, TimerPhase::NotRunning);
    assert_eq!(timer.current_time().real_time, Some(TimeSpan::ZERO));
    assert_eq!(timer.pause_time(), None);
    assert_eq!(timer.run.segments[0].split_time, Time::default());
    assert_eq!(timer.run.segments[1].split_time, Time::default());

    assert_eq!(timer.run.attempts.len(), 1);
    let attempt = &timer.run.attempts[0];
    assert_eq!(attempt.index, 1);
    // A running attempt that is reset has no final time.
    assert_eq!(attempt.time, Time::default());
    assert_eq!(attempt.pause_time, Some(seconds(3)));
    assert!(attempt.ended.is_some());

    // Only the completed segment is stored in the segment history.
    assert_eq!(
        timer.run.segments[0]
            .segment_history
            .get(&1)
            .and_then(|time| time.real_time),
        Some(seconds(5))
    );
    assert!(!timer.run.segments[1].segment_history.contains_key(&1));
}

#[test]
fn skip_split_is_allowed_while_paused() {
    let (clock, mut timer) = timer_with(run_with_pb(&[Some(10), Some(20)]));

    timer.start();
    clock.advance(seconds(5));
    timer.pause();
    timer.skip_split();
    assert_eq!(timer.phase, TimerPhase::Paused);
    assert_eq!(timer.current_split_index, 1);
    assert_eq!(timer.run.segments[0].split_time, Time::default());

    clock.advance(seconds(2));
    timer.pause();
    clock.advance(seconds(5));
    assert_eq!(timer.current_time().real_time, Some(seconds(10)));
    timer.split();
    assert_eq!(timer.phase, TimerPhase::Ended);
    assert_eq!(timer.run.segments[0].split_time, Time::default());
    assert_eq!(timer.run.segments[1].split_time.real_time, Some(seconds(10)));
}

#[test]
fn ended_state_ignores_skip_pause_and_split() {
    let (clock, mut timer) = timer_with(run_with_pb(&[Some(10)]));

    timer.start();
    clock.advance(seconds(10));
    timer.split();
    assert_eq!(timer.phase, TimerPhase::Ended);
    let ended = timer.attempt_ended;

    timer.skip_split();
    timer.pause();
    timer.split();
    assert_eq!(timer.phase, TimerPhase::Ended);
    assert_eq!(timer.current_split_index, 1);
    assert_eq!(timer.attempt_ended, ended);
    assert!(!timer.is_game_time_paused());

    // Undoing the final split puts the timer back into the running phase.
    timer.undo_split();
    assert_eq!(timer.phase, TimerPhase::Running);
    assert_eq!(timer.current_split_index, 0);
    assert_eq!(timer.current_time().real_time, Some(seconds(10)));
}

#[test]
fn split_colors() {
    // Ahead of the PB, no segment delta information -> AheadGaining.
    let (clock, mut timer) = timer_with(run_with_pb(&[Some(10), Some(20)]));
    timer.start();
    clock.advance(seconds(9));
    timer.split();
    assert_eq!(
        split_color(
            &timer,
            Some(seconds(-1)),
            0,
            true,
            false,
            PERSONAL_BEST,
            TimingMethod::RealTime
        ),
        Some(SplitColor::AheadGaining)
    );

    // Segment 0 is 2s ahead, segment 1 only 1s ahead -> AheadLosing.
    let (clock, mut timer) = timer_with(run_with_pb(&[Some(10), Some(20)]));
    timer.start();
    clock.advance(seconds(8));
    timer.split();
    clock.advance(seconds(11));
    timer.split();
    assert_eq!(
        split_color(
            &timer,
            Some(seconds(-1)),
            1,
            true,
            false,
            PERSONAL_BEST,
            TimingMethod::RealTime
        ),
        Some(SplitColor::AheadLosing)
    );

    // Segment 0 is 2s behind, segment 1 only 1s behind -> BehindGaining.
    let (clock, mut timer) = timer_with(run_with_pb(&[Some(10), Some(20)]));
    timer.start();
    clock.advance(seconds(12));
    timer.split();
    clock.advance(seconds(9));
    timer.split();
    assert_eq!(
        split_color(
            &timer,
            Some(seconds(1)),
            1,
            true,
            false,
            PERSONAL_BEST,
            TimingMethod::RealTime
        ),
        Some(SplitColor::BehindGaining)
    );

    // Segment 0 is 2s behind, segment 1 3s behind -> BehindLosing.
    let (clock, mut timer) = timer_with(run_with_pb(&[Some(10), Some(20)]));
    timer.start();
    clock.advance(seconds(12));
    timer.split();
    clock.advance(seconds(11));
    timer.split();
    assert_eq!(
        split_color(
            &timer,
            Some(seconds(3)),
            1,
            true,
            false,
            PERSONAL_BEST,
            TimingMethod::RealTime
        ),
        Some(SplitColor::BehindLosing)
    );

    // A best segment overrides the delta color.
    let (clock, mut timer) = timer_with(run_with_best(
        &[Some(10), Some(20)],
        &[Some(10), Some(10)],
    ));
    timer.start();
    clock.advance(seconds(9));
    timer.split();
    assert!(check_best_segment(&timer, 0, TimingMethod::RealTime));
    assert_eq!(
        split_color(
            &timer,
            Some(seconds(-1)),
            0,
            true,
            true,
            PERSONAL_BEST,
            TimingMethod::RealTime
        ),
        Some(SplitColor::BestSegment)
    );
    assert_eq!(
        split_color(
            &timer,
            Some(seconds(-1)),
            0,
            true,
            false,
            PERSONAL_BEST,
            TimingMethod::RealTime
        ),
        Some(SplitColor::AheadGaining)
    );

    // Index bounds and null deltas.
    assert_eq!(
        split_color(
            &timer,
            Some(seconds(-1)),
            -1,
            true,
            true,
            PERSONAL_BEST,
            TimingMethod::RealTime
        ),
        None
    );
    assert_eq!(
        split_color(
            &timer,
            Some(seconds(-1)),
            99,
            true,
            true,
            PERSONAL_BEST,
            TimingMethod::RealTime
        ),
        None
    );
    assert_eq!(
        split_color(
            &timer,
            None,
            0,
            true,
            false,
            PERSONAL_BEST,
            TimingMethod::RealTime
        ),
        None
    );
}

#[test]
fn check_best_segment_uses_best_segments_delta() {
    let (clock, mut timer) = timer_with(run_with_best(
        &[Some(10), Some(20)],
        &[Some(10), Some(10)],
    ));

    // The split is not completed yet.
    assert!(!check_best_segment(&timer, 0, TimingMethod::RealTime));
    assert!(!check_best_segment(&timer, -1, TimingMethod::RealTime));
    assert!(!check_best_segment(&timer, 99, TimingMethod::RealTime));

    timer.start();
    clock.advance(seconds(10));
    timer.split();
    // Equal to the best segment and no negative delta against Best Segments.
    assert!(!check_best_segment(&timer, 0, TimingMethod::RealTime));

    clock.advance(seconds(9));
    timer.split();
    assert!(check_best_segment(&timer, 1, TimingMethod::RealTime));
}

#[test]
fn predicted_time_matches_livesplit_pace() {
    let (clock, mut timer) = timer_with(run_with_pb(&[Some(10), Some(20), Some(30)]));

    assert_eq!(
        predicted_time(&timer, PERSONAL_BEST, TimingMethod::RealTime),
        Some(seconds(30))
    );

    timer.start();
    assert_eq!(
        predicted_time(&timer, PERSONAL_BEST, TimingMethod::RealTime),
        Some(seconds(30))
    );

    // Being 5s behind on the first segment predicts 35s.
    clock.advance(seconds(15));
    assert_eq!(
        predicted_time(&timer, PERSONAL_BEST, TimingMethod::RealTime),
        Some(seconds(35))
    );

    // At the first split the live delta is neutralized by the previous delta.
    let (clock, mut timer) = timer_with(run_with_pb(&[Some(10), Some(20), Some(30)]));
    timer.start();
    clock.advance(seconds(15));
    timer.split();
    clock.advance(seconds(5));
    assert_eq!(
        predicted_time(&timer, PERSONAL_BEST, TimingMethod::RealTime),
        Some(seconds(35))
    );

    // The runner is 5 seconds behind the PB at both splits, so the prediction
    // stays at PB + 5s even while the current segment is being run.
    clock.advance(seconds(5));
    timer.split();
    assert_eq!(
        predicted_time(&timer, PERSONAL_BEST, TimingMethod::RealTime),
        Some(seconds(35))
    );
    clock.advance(seconds(2));
    assert_eq!(
        predicted_time(&timer, PERSONAL_BEST, TimingMethod::RealTime),
        Some(seconds(35))
    );

    clock.advance(seconds(3));
    timer.split();
    assert_eq!(timer.phase, TimerPhase::Ended);
    assert_eq!(
        predicted_time(&timer, PERSONAL_BEST, TimingMethod::RealTime),
        Some(seconds(30))
    );

    // "Current Comparison" follows the timer's selected comparison.
    timer.set_current_comparison(BEST_SEGMENTS);
    assert_eq!(
        predicted_time(&timer, CURRENT_COMPARISON, TimingMethod::RealTime),
        predicted_time(&timer, BEST_SEGMENTS, TimingMethod::RealTime)
    );
}

#[test]
fn possible_time_save_counts_the_remaining_segments() {
    let (clock, mut timer) = timer_with(run_with_best(
        &[Some(10), Some(20), Some(30)],
        &[Some(9), Some(9), Some(9)],
    ));

    assert_eq!(
        possible_time_save(
            &timer,
            TimingMethod::RealTime,
            &ComparisonSettings::default()
        ),
        Some(seconds(3))
    );

    timer.start();
    clock.advance(seconds(10));
    timer.split();
    // Remaining best: 9 + 9 = 18. Remaining comparison: 30 - 10 = 20.
    assert_eq!(
        possible_time_save(
            &timer,
            TimingMethod::RealTime,
            &ComparisonSettings::default()
        ),
        Some(seconds(2))
    );

    clock.advance(seconds(10));
    timer.split();
    assert_eq!(
        possible_time_save(
            &timer,
            TimingMethod::RealTime,
            &ComparisonSettings::default()
        ),
        Some(seconds(1))
    );

    clock.advance(seconds(10));
    timer.split();
    assert_eq!(timer.phase, TimerPhase::Ended);
    assert_eq!(
        possible_time_save(
            &timer,
            TimingMethod::RealTime,
            &ComparisonSettings::default()
        ),
        Some(TimeSpan::ZERO)
    );

    // Without best segment times there is no calculated time save, but the
    // total is still zero rather than an absent value.
    let (_clock, timer) = timer_with(run_with_pb(&[Some(10), Some(20)]));
    assert_eq!(
        possible_time_save(
            &timer,
            TimingMethod::RealTime,
            &ComparisonSettings::default()
        ),
        Some(TimeSpan::ZERO)
    );

    // A sum of best that is slower than the comparison is clamped to zero.
    let (_clock, timer) = timer_with(run_with_best(
        &[Some(10), Some(20)],
        &[Some(12), Some(12)],
    ));
    assert_eq!(
        possible_time_save(
            &timer,
            TimingMethod::RealTime,
            &ComparisonSettings::default()
        ),
        Some(TimeSpan::ZERO)
    );
}

#[test]
fn analysis_helpers_handle_null_times_and_bounds() {
    let (clock, mut timer) = timer_with(run_with_pb(&[Some(10), Some(20), Some(30)]));

    assert_eq!(
        get_last_delta(&timer, -1, PERSONAL_BEST, TimingMethod::RealTime),
        None
    );
    assert_eq!(
        get_last_delta(&timer, 0, PERSONAL_BEST, TimingMethod::RealTime),
        None
    );

    timer.start();
    clock.advance(seconds(10));
    timer.split();
    clock.advance(seconds(10));
    timer.split();
    clock.advance(seconds(10));
    timer.split();

    // An out of bounds split number gets clamped to the last segment.
    assert_eq!(
        get_last_delta(&timer, 99, PERSONAL_BEST, TimingMethod::RealTime),
        Some(TimeSpan::ZERO)
    );
    assert_eq!(
        get_last_delta(&timer, -5, PERSONAL_BEST, TimingMethod::RealTime),
        None
    );

    assert_eq!(
        get_previous_segment_time(&timer, 99, TimingMethod::RealTime),
        None
    );
    assert_eq!(
        get_previous_segment_delta(&timer, 99, PERSONAL_BEST, TimingMethod::RealTime),
        None
    );
    assert_eq!(
        get_live_segment_time(&timer, -1, TimingMethod::RealTime),
        None
    );
    assert_eq!(
        get_live_segment_delta(&timer, -1, PERSONAL_BEST, TimingMethod::RealTime),
        None
    );

    // An uncompleted split has no previous segment time, but its live segment
    // time is the elapsed time.
    let (clock, mut timer) = timer_with(run_with_pb(&[Some(10), Some(20)]));
    timer.start();
    clock.advance(seconds(3));
    assert_eq!(
        get_previous_segment_time(&timer, 0, TimingMethod::RealTime),
        None
    );
    assert_eq!(
        get_live_segment_time(&timer, 0, TimingMethod::RealTime),
        Some(seconds(3))
    );

    // A missing comparison must not turn a live best-segment delta into 0.
    let mut run = Run::with_segments("Game", "Any%", ["One", "Two"]);
    run.segments[0].best_segment_time = real(Some(1));
    run.segments[1].best_segment_time = real(Some(1));
    let (clock, mut timer) = timer_with(run);
    timer.start();
    clock.advance(seconds(2));
    assert_eq!(
        check_live_delta(
            &timer,
            false,
            PERSONAL_BEST,
            TimingMethod::RealTime,
            true
        ),
        None
    );
    assert_eq!(
        check_live_delta(
            &timer,
            true,
            PERSONAL_BEST,
            TimingMethod::RealTime,
            true
        ),
        None
    );
}

#[test]
fn reset_without_updating_times_keeps_history_and_pb() {
    let (clock, mut timer) = timer_with(run_with_pb(&[Some(10), Some(20)]));

    timer.start();
    clock.advance(seconds(5));
    timer.split();
    clock.advance(seconds(5));
    timer.split();
    timer.reset_without_updating_times();

    assert_eq!(timer.phase, TimerPhase::NotRunning);
    assert!(timer.run.attempts.is_empty());
    assert_eq!(timer.run.segments[0].personal_best().real_time, Some(seconds(10)));
    assert_eq!(timer.run.segments[1].personal_best().real_time, Some(seconds(20)));
    for segment in &timer.run.segments {
        assert!(segment.segment_history.is_empty());
        assert_eq!(segment.split_time, Time::default());
    }

    // Resetting while not running is a no-op.
    timer.reset();
    assert_eq!(timer.phase, TimerPhase::NotRunning);
    assert!(timer.run.attempts.is_empty());
}

#[test]
fn reset_and_set_attempt_as_pb() {
    let (clock, mut timer) = timer_with(run_with_pb(&[Some(10), Some(20)]));
    timer.run.metadata.run_id = Some("run-id".to_string());

    timer.start();
    clock.advance(seconds(12));
    timer.split();
    clock.advance(seconds(12));
    timer.split();
    timer.reset_and_set_attempt_as_pb();

    assert_eq!(timer.phase, TimerPhase::NotRunning);
    assert_eq!(timer.run.segments[0].personal_best().real_time, Some(seconds(12)));
    assert_eq!(timer.run.segments[1].personal_best().real_time, Some(seconds(24)));
    // The old PB still has faster segments, so they remain the best segments.
    assert_eq!(timer.run.segments[0].best_segment_time.real_time, Some(seconds(10)));
    assert_eq!(timer.run.segments[1].best_segment_time.real_time, Some(seconds(10)));
    assert_eq!(timer.run.metadata.run_id, None);
    assert_eq!(timer.run.attempts.len(), 1);
    assert_eq!(timer.run.attempts[0].time.real_time, Some(seconds(24)));
}

#[test]
fn reset_and_set_pb_does_not_overwrite_the_ended_timestamp() {
    let (clock, mut timer) = timer_with(run_with_pb(&[Some(10)]));

    timer.start();
    clock.advance(seconds(10));
    timer.split();
    let ended = timer.attempt_ended;
    clock.advance(seconds(1));
    timer.reset_and_set_attempt_as_pb();
    assert_eq!(timer.attempt_ended, ended);
    assert_eq!(timer.run.attempts[0].ended, ended);
}
