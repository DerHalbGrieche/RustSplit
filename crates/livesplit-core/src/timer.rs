//! Timer state machine.
//!
//! The state machine follows LiveSplit's `TimerModel` / `LiveSplitState`
//! semantics.  Time is driven by a [`Clock`] so the timer can be tested
//! deterministically.

use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::{DateTime, TimeZone, Utc};

use crate::model::{
    Attempt, ComparisonSettings, Run, TimerPhase, TimingMethod, PERSONAL_BEST,
};
use crate::time::{Time, TimeSpan};

/// Supplies monotonic time and wall clock time to a timer.
pub trait Clock: Send + Sync {
    fn monotonic(&self) -> Duration;
    fn utc_now(&self) -> DateTime<Utc>;
}

/// The normal system clock.
pub struct SystemClock {
    origin: Instant,
}

impl SystemClock {
    pub fn new() -> Self {
        Self {
            origin: Instant::now(),
        }
    }
}

impl Default for SystemClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for SystemClock {
    fn monotonic(&self) -> Duration {
        self.origin.elapsed()
    }

    fn utc_now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

#[derive(Debug)]
struct TestClockState {
    nanos: AtomicI64,
    utc: Mutex<DateTime<Utc>>,
}

/// A manually advanced clock for tests.
#[derive(Clone, Debug)]
pub struct TestClock {
    state: Arc<TestClockState>,
}

impl TestClock {
    pub fn new() -> Self {
        Self::with_utc(Utc.with_ymd_and_hms(2024, 1, 1, 12, 0, 0).unwrap())
    }

    pub fn with_utc(utc: DateTime<Utc>) -> Self {
        Self {
            state: Arc::new(TestClockState {
                nanos: AtomicI64::new(0),
                utc: Mutex::new(utc),
            }),
        }
    }

    pub fn advance(&self, duration: TimeSpan) {
        self.state
            .nanos
            .fetch_add(duration.as_nanos(), Ordering::SeqCst);
        let mut utc = self.state.utc.lock().unwrap();
        *utc += chrono::Duration::nanoseconds(duration.as_nanos());
    }

    pub fn set_elapsed(&self, duration: TimeSpan) {
        self.state
            .nanos
            .store(duration.as_nanos(), Ordering::SeqCst);
    }

    pub fn utc(&self) -> DateTime<Utc> {
        *self.state.utc.lock().unwrap()
    }
}

impl Default for TestClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for TestClock {
    fn monotonic(&self) -> Duration {
        let nanos = self.state.nanos.load(Ordering::SeqCst).max(0) as u64;
        Duration::from_nanos(nanos)
    }

    fn utc_now(&self) -> DateTime<Utc> {
        self.utc()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TimerEvent {
    Started,
    Split { index: i32 },
    SkippedSplit { index: i32 },
    UndidSplit { index: i32 },
    Reset { previous_phase: TimerPhase },
    Paused,
    Resumed,
    UndoAllPauses,
    GameTimeInitialized,
    ComparisonChanged(String),
    TimingMethodChanged(TimingMethod),
    RunChanged,
}

pub struct Timer {
    pub run: Run,
    pub phase: TimerPhase,
    /// -1 before the timer is started, otherwise the index of the segment the
    /// runner is currently on.
    pub current_split_index: i32,
    pub current_comparison: String,
    pub current_timing_method: TimingMethod,
    pub is_game_time_initialized: bool,
    pub is_game_time_paused: bool,
    pub game_time_pause_time: Option<TimeSpan>,
    pub loading_times: TimeSpan,
    pub attempt_started: Option<DateTime<Utc>>,
    pub attempt_ended: Option<DateTime<Utc>>,
    pub comparison_settings: ComparisonSettings,

    clock: Arc<dyn Clock>,
    /// Monotonic time at which the current run started, adjusted by offset.
    adjusted_start_time: i128,
    /// Monotonic time at which the run was originally started.
    start_time_with_offset: i128,
    time_paused_at: TimeSpan,
    events: Vec<TimerEvent>,
}

impl Timer {
    pub fn new(run: Run, clock: Arc<dyn Clock>) -> Self {
        let now = clock.monotonic().as_nanos() as i128;
        let mut timer = Self {
            run,
            phase: TimerPhase::NotRunning,
            current_split_index: -1,
            current_comparison: PERSONAL_BEST.to_string(),
            current_timing_method: TimingMethod::RealTime,
            is_game_time_initialized: false,
            is_game_time_paused: false,
            game_time_pause_time: None,
            loading_times: TimeSpan::ZERO,
            attempt_started: None,
            attempt_ended: None,
            comparison_settings: ComparisonSettings::default(),
            clock,
            adjusted_start_time: now,
            start_time_with_offset: now,
            time_paused_at: TimeSpan::ZERO,
            events: Vec::new(),
        };
        timer.run.refresh_comparisons(&timer.comparison_settings);
        timer
    }

    pub fn with_system_clock(run: Run) -> Self {
        Self::new(run, Arc::new(SystemClock::new()))
    }

    pub fn clock(&self) -> &Arc<dyn Clock> {
        &self.clock
    }

    pub fn take_events(&mut self) -> Vec<TimerEvent> {
        std::mem::take(&mut self.events)
    }

    pub fn current_time(&self) -> Time {
        let real_time = match self.phase {
            TimerPhase::NotRunning => Some(TimeSpan::ZERO),
            TimerPhase::Running => {
                let elapsed = self.clock.monotonic().as_nanos() as i128 - self.adjusted_start_time;
                Some(TimeSpan::from_nanos(
                    elapsed.clamp(i64::MIN as i128, i64::MAX as i128) as i64,
                ))
            }
            TimerPhase::Paused => Some(self.time_paused_at),
            TimerPhase::Ended => self
                .run
                .segments
                .last()
                .and_then(|segment| segment.split_time.real_time),
        };

        let game_time = if self.phase == TimerPhase::Ended {
            self.run
                .segments
                .last()
                .and_then(|segment| segment.split_time.game_time)
        } else if self.is_game_time_paused {
            self.game_time_pause_time
        } else if self.is_game_time_initialized {
            real_time.map(|real| real - self.loading_times)
        } else {
            None
        };

        Time {
            real_time,
            game_time,
        }
    }

    pub fn current_split(&self) -> Option<&crate::model::Segment> {
        if self.current_split_index >= 0 {
            self.run.segments.get(self.current_split_index as usize)
        } else {
            None
        }
    }

    pub fn current_split_mut(&mut self) -> Option<&mut crate::model::Segment> {
        if self.current_split_index >= 0 {
            self.run
                .segments
                .get_mut(self.current_split_index as usize)
        } else {
            None
        }
    }

    pub fn pause_time(&self) -> Option<TimeSpan> {
        match self.phase {
            TimerPhase::Paused => Some(TimeSpan::from_nanos(
                (self.clock.monotonic().as_nanos() as i128 - self.start_time_with_offset
                    - self.time_paused_at.as_nanos() as i128)
                    .clamp(i64::MIN as i128, i64::MAX as i128) as i64,
            )),
            phase if phase != TimerPhase::NotRunning
                && self.adjusted_start_time != self.start_time_with_offset =>
            {
                Some(TimeSpan::from_nanos(
                    (self.adjusted_start_time - self.start_time_with_offset)
                        .clamp(i64::MIN as i128, i64::MAX as i128) as i64,
                ))
            }
            _ => None,
        }
    }

    /// The wall clock duration of the current attempt.
    ///
    /// Like LiveSplit's `LiveSplitState.CurrentAttemptDuration` this is not the
    /// timer value: the start offset is not applied and time spent paused is
    /// included.  A timer that has not been started reports a duration of zero.
    pub fn current_attempt_duration(&self) -> Option<TimeSpan> {
        match self.phase {
            TimerPhase::Running | TimerPhase::Paused => Some(
                self.attempt_started
                    .map(|started| {
                        let duration = self.clock.utc_now().signed_duration_since(started);
                        TimeSpan::from_nanos(duration.num_nanoseconds().unwrap_or(0))
                    })
                    .unwrap_or_default(),
            ),
            TimerPhase::Ended => match (self.attempt_started, self.attempt_ended) {
                (Some(started), Some(ended)) => {
                    let duration = ended.signed_duration_since(started);
                    Some(TimeSpan::from_nanos(duration.num_nanoseconds().unwrap_or(0)))
                }
                _ => None,
            },
            TimerPhase::NotRunning => Some(TimeSpan::ZERO),
        }
    }

    pub fn start(&mut self) {
        if self.phase != TimerPhase::NotRunning {
            return;
        }
        let now = self.clock.monotonic().as_nanos() as i128;
        self.phase = TimerPhase::Running;
        self.current_split_index = 0;
        self.attempt_started = Some(self.clock.utc_now());
        self.attempt_ended = None;
        self.adjusted_start_time = now - self.run.offset.as_nanos() as i128;
        self.start_time_with_offset = self.adjusted_start_time;
        self.time_paused_at = self.run.offset;
        self.is_game_time_initialized = false;
        self.is_game_time_paused = false;
        self.game_time_pause_time = None;
        self.loading_times = TimeSpan::ZERO;
        self.run.attempt_count += 1;
        self.run.has_changed = true;
        self.events.push(TimerEvent::Started);
    }

    pub fn split(&mut self) {
        if self.phase != TimerPhase::Running {
            return;
        }
        let current_time = self.current_time();
        let Some(real) = current_time.real_time else {
            return;
        };
        if real <= TimeSpan::ZERO {
            return;
        }
        let index = self.current_split_index;
        if index < 0 || index as usize >= self.run.segments.len() {
            return;
        }
        let custom_variables = self.run.metadata.custom_variables.clone();
        if let Some(segment) = self.run.segments.get_mut(index as usize) {
            segment.split_time = current_time;
            for (name, value) in custom_variables {
                segment.custom_variable_values.insert(name, value);
            }
        }
        self.current_split_index += 1;
        if self.current_split_index as usize >= self.run.segments.len() {
            self.phase = TimerPhase::Ended;
            self.attempt_ended = Some(self.clock.utc_now());
        }
        self.run.has_changed = true;
        self.events.push(TimerEvent::Split { index });
        self.refresh_comparisons();
    }

    pub fn skip_split(&mut self) {
        if !self.phase.is_running() {
            return;
        }
        if self.current_split_index < 0
            || self.current_split_index as usize >= self.run.segments.len().saturating_sub(1)
        {
            return;
        }
        let index = self.current_split_index;
        if let Some(segment) = self.run.segments.get_mut(index as usize) {
            segment.split_time = Time::default();
            segment.custom_variable_values.clear();
        }
        self.current_split_index += 1;
        self.run.has_changed = true;
        self.events.push(TimerEvent::SkippedSplit { index });
        self.refresh_comparisons();
    }

    pub fn undo_split(&mut self) {
        if self.phase == TimerPhase::NotRunning || self.current_split_index <= 0 {
            return;
        }
        if self.phase == TimerPhase::Ended {
            self.phase = TimerPhase::Running;
            self.attempt_ended = None;
        }
        self.current_split_index -= 1;
        let index = self.current_split_index;
        if let Some(segment) = self.run.segments.get_mut(index as usize) {
            segment.split_time = Time::default();
            segment.custom_variable_values.clear();
        }
        self.run.has_changed = true;
        self.events.push(TimerEvent::UndidSplit { index });
        self.refresh_comparisons();
    }

    pub fn reset(&mut self) {
        self.reset_internal(true);
    }

    pub fn reset_without_updating_times(&mut self) {
        self.reset_internal(false);
    }

    fn reset_internal(&mut self, update_times: bool) {
        if self.phase == TimerPhase::NotRunning {
            return;
        }
        self.reset_state(update_times);
        self.reset_splits();
    }

    fn reset_state(&mut self, update_times: bool) {
        if self.phase != TimerPhase::Ended {
            self.attempt_ended = Some(self.clock.utc_now());
        }
        self.is_game_time_paused = false;
        self.game_time_pause_time = None;
        self.loading_times = TimeSpan::ZERO;
        // LiveSplit's `LoadingTimes = TimeSpan.Zero` also marks the game time
        // as initialized (with zero loading time).  `start()` deinitializes it
        // again for the next attempt.
        self.is_game_time_initialized = true;
        if update_times {
            self.update_times();
        }
    }

    fn reset_splits(&mut self) {
        let previous_phase = self.phase;
        self.phase = TimerPhase::NotRunning;
        self.current_split_index = -1;
        for segment in &mut self.run.segments {
            segment.split_time = Time::default();
            segment.custom_variable_values.clear();
        }
        self.events.push(TimerEvent::Reset { previous_phase });
        self.run.fix_splits();
        self.refresh_comparisons();
    }

    pub fn reset_and_set_attempt_as_pb(&mut self) {
        if self.phase == TimerPhase::NotRunning {
            return;
        }
        self.reset_state(true);
        self.set_run_as_pb();
        self.reset_splits();
    }

    pub fn pause(&mut self) {
        match self.phase {
            TimerPhase::Running => {
                self.time_paused_at = self.current_time().real_time.unwrap_or_default();
                self.phase = TimerPhase::Paused;
                self.events.push(TimerEvent::Paused);
            }
            TimerPhase::Paused => {
                self.adjusted_start_time = self.clock.monotonic().as_nanos() as i128
                    - self.time_paused_at.as_nanos() as i128;
                self.phase = TimerPhase::Running;
                self.events.push(TimerEvent::Resumed);
            }
            TimerPhase::NotRunning => self.start(),
            TimerPhase::Ended => {}
        }
    }

    pub fn undo_all_pauses(&mut self) {
        if self.phase == TimerPhase::Paused {
            self.pause();
        }
        let pause_time = self.pause_time().unwrap_or_default();
        if self.phase == TimerPhase::Ended {
            if let Some(segment) = self.run.segments.last_mut() {
                segment.split_time.real_time = segment
                    .split_time
                    .real_time
                    .map(|time| time + pause_time);
                segment.split_time.game_time = segment
                    .split_time
                    .game_time
                    .map(|time| time + pause_time);
            }
        }
        self.adjusted_start_time = self.start_time_with_offset;
        self.events.push(TimerEvent::UndoAllPauses);
    }

    pub fn is_game_time_paused(&self) -> bool {
        self.is_game_time_paused
    }

    pub fn initialize_game_time(&mut self) {
        if self.is_game_time_initialized {
            return;
        }
        self.is_game_time_initialized = true;
        self.loading_times = TimeSpan::ZERO;
        self.events.push(TimerEvent::GameTimeInitialized);
    }

    pub fn set_loading_times(&mut self, loading_times: TimeSpan) {
        self.loading_times = loading_times;
        self.is_game_time_initialized = true;
        if self.is_game_time_paused {
            self.game_time_pause_time = self
                .current_time()
                .real_time
                .map(|real| real - loading_times);
        }
    }

    pub fn set_game_time(&mut self, game_time: Option<TimeSpan>) {
        let current = self.current_time();
        if let (Some(real), Some(game_time)) = (current.real_time, game_time) {
            self.loading_times = real - game_time;
            self.is_game_time_initialized = true;
            if self.is_game_time_paused {
                self.game_time_pause_time = Some(game_time);
            }
        }
    }

    pub fn set_game_time_paused(&mut self, paused: bool) {
        if paused == self.is_game_time_paused {
            return;
        }
        let current = self.current_time();
        if paused {
            self.game_time_pause_time = current.game_time.or(current.real_time);
        } else {
            let real = current.real_time.unwrap_or_default();
            let game = current.game_time.unwrap_or(real);
            self.loading_times = real - game;
            self.is_game_time_initialized = true;
        }
        self.is_game_time_paused = paused;
    }

    pub fn set_offset(&mut self, offset: TimeSpan) {
        self.run.offset = offset;
        if self.phase == TimerPhase::NotRunning {
            self.time_paused_at = offset;
        }
        self.run.has_changed = true;
    }

    pub fn switch_comparison_next(&mut self) {
        let comparisons = self.run.comparisons();
        if comparisons.is_empty() {
            return;
        }
        let index = comparisons
            .iter()
            .position(|comparison| comparison == &self.current_comparison)
            .map(|index| (index + 1) % comparisons.len())
            .unwrap_or(0);
        self.set_current_comparison(comparisons[index].clone());
    }

    pub fn switch_comparison_previous(&mut self) {
        let comparisons = self.run.comparisons();
        if comparisons.is_empty() {
            return;
        }
        let index = comparisons
            .iter()
            .position(|comparison| comparison == &self.current_comparison)
            .map(|index| (index + comparisons.len() - 1) % comparisons.len())
            .unwrap_or(comparisons.len().saturating_sub(2));
        self.set_current_comparison(comparisons[index].clone());
    }

    pub fn set_current_comparison(&mut self, comparison: impl Into<String>) {
        let comparison = comparison.into();
        if comparison != self.current_comparison {
            self.current_comparison = comparison.clone();
            self.events.push(TimerEvent::ComparisonChanged(comparison));
        }
    }

    pub fn set_timing_method(&mut self, method: TimingMethod) {
        if method != self.current_timing_method {
            self.current_timing_method = method;
            self.events.push(TimerEvent::TimingMethodChanged(method));
        }
    }

    pub fn toggle_timing_method(&mut self) {
        self.set_timing_method(self.current_timing_method.other());
    }

    pub fn refresh_comparisons(&mut self) {
        self.run.refresh_comparisons(&self.comparison_settings);
        self.run.has_changed = true;
    }

    pub fn set_run(&mut self, run: Run) {
        self.run = run;
        self.phase = TimerPhase::NotRunning;
        self.current_split_index = -1;
        self.is_game_time_initialized = false;
        self.is_game_time_paused = false;
        self.game_time_pause_time = None;
        self.loading_times = TimeSpan::ZERO;
        self.attempt_started = None;
        self.attempt_ended = None;
        if !self.run.contains_comparison(&self.current_comparison) {
            self.current_comparison = PERSONAL_BEST.to_string();
        }
        self.refresh_comparisons();
        self.events.push(TimerEvent::RunChanged);
    }

    fn update_times(&mut self) {
        self.update_attempt_history();
        self.update_best_segments();
        self.update_pb_splits();
        self.update_segment_history();
    }

    fn update_attempt_history(&mut self) {
        let time = if self.phase == TimerPhase::Ended {
            self.current_time()
        } else {
            Time::default()
        };
        let max_index = self
            .run
            .attempts
            .iter()
            .map(|attempt| attempt.index)
            .max()
            .unwrap_or(0);
        // Mirrors `Math.Max(0, maxIndex + 1)` in TimerModel.UpdateAttemptHistory.
        let new_index = max_index.saturating_add(1).max(0);
        self.run.attempts.push(Attempt {
            index: new_index,
            time,
            started: self.attempt_started,
            ended: self.attempt_ended,
            pause_time: self.pause_time(),
        });
    }

    fn update_best_segments(&mut self) {
        let mut previous_real = Some(TimeSpan::ZERO);
        let mut previous_game = Some(TimeSpan::ZERO);
        for segment in &mut self.run.segments {
            let mut new_best = segment.best_segment_time;
            if let Some(split) = segment.split_time.real_time {
                let current = split - previous_real.unwrap_or_default();
                previous_real = Some(split);
                if new_best.real_time.is_none()
                    || new_best.real_time.is_some_and(|best| current < best)
                {
                    new_best.real_time = Some(current);
                }
            }
            if let Some(split) = segment.split_time.game_time {
                let current = split - previous_game.unwrap_or_default();
                previous_game = Some(split);
                if new_best.game_time.is_none()
                    || new_best.game_time.is_some_and(|best| current < best)
                {
                    new_best.game_time = Some(current);
                }
            }
            segment.best_segment_time = new_best;
        }
    }

    fn update_pb_splits(&mut self) {
        let method = self.current_timing_method;
        let Some(last) = self.run.segments.last() else {
            return;
        };
        let split = last.split_time.get(method);
        let pb = last.personal_best().get(method);
        // Same condition as TimerModel.UpdatePBSplits: a finished attempt is a
        // new PB when there either is no PB yet or the attempt was faster.
        if let Some(split) = split {
            if pb.is_none_or(|pb| split < pb) {
                self.set_run_as_pb();
            }
        }
    }

    fn update_segment_history(&mut self) {
        let attempt_index = self
            .run
            .attempts
            .last()
            .map(|attempt| attempt.index)
            .unwrap_or(1);
        let count = self.current_split_index.max(0) as usize;
        let mut total_real = TimeSpan::ZERO;
        let mut total_game = TimeSpan::ZERO;
        for index in 0..count.min(self.run.segments.len()) {
            let segment = &mut self.run.segments[index];
            let real = segment.split_time.real_time.map(|time| time - total_real);
            let game = segment.split_time.game_time.map(|time| time - total_game);
            segment
                .segment_history
                .insert(attempt_index, Time { real_time: real, game_time: game });
            if let Some(time) = segment.split_time.real_time {
                total_real = time;
            }
            if let Some(time) = segment.split_time.game_time {
                total_game = time;
            }
        }
    }

    fn set_run_as_pb(&mut self) {
        self.run.import_segment_history();
        self.run.fix_splits();
        let split_times = self
            .run
            .segments
            .iter()
            .map(|segment| segment.split_time)
            .collect::<Vec<_>>();
        for (segment, split_time) in self.run.segments.iter_mut().zip(split_times) {
            segment.set_personal_best(split_time);
        }
        self.run.metadata.run_id = None;
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Run, TimerPhase};
    use crate::time::TimeSpan;

    fn test_timer() -> (TestClock, Timer) {
        let clock = TestClock::new();
        let run = Run::with_segments("Game", "Any%", ["One", "Two"]);
        let timer = Timer::new(run, Arc::new(clock.clone()));
        (clock, timer)
    }

    #[test]
    fn start_and_split() {
        let (clock, mut timer) = test_timer();
        timer.start();
        assert_eq!(timer.phase, TimerPhase::Running);
        assert_eq!(timer.run.attempt_count, 1);
        clock.advance(TimeSpan::from_nanos(10_500_000_000));
        assert_eq!(
            timer.current_time().real_time.unwrap(),
            TimeSpan::from_nanos(10_500_000_000)
        );
        timer.split();
        assert_eq!(timer.current_split_index, 1);
        clock.advance(TimeSpan::from_nanos(2_000_000_000));
        timer.split();
        assert_eq!(timer.phase, TimerPhase::Ended);
        assert_eq!(
            timer.run.segments[0].split_time.real_time.unwrap(),
            TimeSpan::from_nanos(10_500_000_000)
        );
        assert_eq!(
            timer.run.segments[1].split_time.real_time.unwrap(),
            TimeSpan::from_nanos(12_500_000_000)
        );
    }

    #[test]
    fn pause_freezes_time() {
        let (clock, mut timer) = test_timer();
        timer.start();
        clock.advance(TimeSpan::from_seconds(5));
        timer.pause();
        assert_eq!(timer.phase, TimerPhase::Paused);
        clock.advance(TimeSpan::from_seconds(10));
        assert_eq!(timer.current_time().real_time.unwrap(), TimeSpan::from_seconds(5));
        timer.pause();
        assert_eq!(timer.current_time().real_time.unwrap(), TimeSpan::from_seconds(5));
        clock.advance(TimeSpan::from_seconds(1));
        assert_eq!(timer.current_time().real_time.unwrap(), TimeSpan::from_seconds(6));
        assert_eq!(timer.pause_time(), Some(TimeSpan::from_seconds(10)));
    }

    #[test]
    fn reset_updates_pb_and_best_segments() {
        let (clock, mut timer) = test_timer();
        timer.start();
        clock.advance(TimeSpan::from_seconds(4));
        timer.split();
        clock.advance(TimeSpan::from_seconds(6));
        timer.split();
        timer.reset();
        assert_eq!(timer.phase, TimerPhase::NotRunning);
        assert_eq!(timer.current_split_index, -1);
        assert_eq!(
            timer.run.segments[1].personal_best().real_time.unwrap(),
            TimeSpan::from_seconds(10)
        );
        assert_eq!(
            timer.run.segments[0].best_segment_time.real_time.unwrap(),
            TimeSpan::from_seconds(4)
        );
        assert_eq!(
            timer.run.segments[1].best_segment_time.real_time.unwrap(),
            TimeSpan::from_seconds(6)
        );
        assert_eq!(timer.run.attempts.len(), 1);
    }

    #[test]
    fn offset_shifts_current_time() {
        let clock = TestClock::new();
        let mut run = Run::with_segments("Game", "Any%", ["One"]);
        run.offset = TimeSpan::from_seconds(2);
        let mut timer = Timer::new(run, Arc::new(clock.clone()));
        timer.start();
        assert_eq!(timer.current_time().real_time.unwrap(), TimeSpan::from_seconds(2));
        clock.advance(TimeSpan::from_seconds(3));
        assert_eq!(timer.current_time().real_time.unwrap(), TimeSpan::from_seconds(5));
    }
}
