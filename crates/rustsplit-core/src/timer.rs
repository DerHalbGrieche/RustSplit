//! Timer state machine.
//!
//! The state machine follows LiveSplit's `TimerModel` / `LiveSplitState`
//! semantics.  Time is driven by a [`Clock`] so the timer can be tested
//! deterministically.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::{DateTime, TimeZone, Utc};

use crate::model::{Attempt, ComparisonSettings, Run, TimerPhase, TimingMethod, PERSONAL_BEST};
use crate::time::{Time, TimeSpan};

/// Supplies monotonic time and wall clock time to a timer.
pub trait Clock: Send + Sync {
    fn monotonic(&self) -> Duration;
    fn utc_now(&self) -> DateTime<Utc>;

    /// How long ago `instant` (captured with `Instant::now()`, possibly on
    /// another thread) was on this clock. An instant in the future was zero
    /// time ago.
    fn age_of(&self, instant: Instant) -> Duration {
        instant.elapsed()
    }
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

    /// Real instants don't map onto a manually advanced clock, so they all
    /// count as now. Tests of delayed handling capture [`Timer::now`] or use
    /// [`Timer::timestamp_ago`] instead.
    fn age_of(&self, _instant: Instant) -> Duration {
        Duration::ZERO
    }
}

/// A moment on a timer's clock, captured when an input arrived.
///
/// Input from other threads (global hotkeys, the server, the control socket,
/// an auto splitter) reaches the UI thread some time after it happened, up to
/// a frame later, or 100 ms while the window is hidden. [`Timer::at`]
/// applies an action as if it ran at the captured moment, so a split
/// received at T is recorded at T no matter when it is handled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Timestamp {
    monotonic: Duration,
    utc: DateTime<Utc>,
}

impl Timestamp {
    /// Moves the timestamp `delta` into the future, keeping both clocks in
    /// step.
    fn later_by(self, delta: Duration) -> Self {
        Self {
            monotonic: self.monotonic.saturating_add(delta),
            utc: chrono::Duration::from_std(delta)
                .ok()
                .and_then(|delta| self.utc.checked_add_signed(delta))
                .unwrap_or(self.utc),
        }
    }
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
    /// The clock reading actions use instead of "now", set by [`Timer::at`].
    time_override: Option<Timestamp>,
    /// When the latest start, split, skip, undo, reset or pause happened.
    /// Timestamped actions are never applied before it.
    last_action: Duration,
    /// Custom variables that only live in memory, like the ones an auto
    /// splitter sets. They are shown like the run's custom variables (and
    /// take precedence over them) but are never saved.
    pub temporary_variables: BTreeMap<String, String>,
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
            time_override: None,
            last_action: Duration::ZERO,
            temporary_variables: BTreeMap::new(),
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

    /// The current moment on the timer's clock.
    pub fn now(&self) -> Timestamp {
        self.time_override.unwrap_or_else(|| Timestamp {
            monotonic: self.clock.monotonic(),
            utc: self.clock.utc_now(),
        })
    }

    /// The moment `age` ago.
    pub fn timestamp_ago(&self, age: Duration) -> Timestamp {
        let now = self.now();
        Timestamp {
            monotonic: now.monotonic.saturating_sub(age),
            utc: chrono::Duration::from_std(age)
                .ok()
                .and_then(|age| now.utc.checked_sub_signed(age))
                .unwrap_or(now.utc),
        }
    }

    /// The moment `instant` refers to. The instant can be captured on any
    /// thread; one in the future counts as now.
    pub fn timestamp_of(&self, instant: Instant) -> Timestamp {
        self.timestamp_ago(self.clock.age_of(instant))
    }

    /// Runs `action` as if the clock read `at`: splits, starts, pauses,
    /// attempt start/end times and game time changes are recorded at that
    /// moment instead of now.
    ///
    /// A timestamp older than the timer's latest action is moved up to it,
    /// so inputs from different sources that are handled out of order can't
    /// make time run backwards (for example a negative segment).
    pub fn at<R>(&mut self, at: Timestamp, action: impl FnOnce(&mut Self) -> R) -> R {
        let at = match self.last_action.checked_sub(at.monotonic) {
            Some(behind) if !behind.is_zero() => at.later_by(behind),
            _ => at,
        };
        let previous = self.time_override.replace(at);
        let result = action(self);
        self.time_override = previous;
        result
    }

    fn monotonic_nanos(&self) -> i128 {
        self.now().monotonic.as_nanos() as i128
    }

    fn utc_now(&self) -> DateTime<Utc> {
        self.now().utc
    }

    fn mark_action(&mut self) {
        self.last_action = self.last_action.max(self.now().monotonic);
    }

    /// The value of a custom variable: a temporary one if set, otherwise the
    /// run's permanent one.
    pub fn custom_variable(&self, name: &str) -> Option<&str> {
        self.temporary_variables
            .get(name)
            .or_else(|| self.run.metadata.custom_variables.get(name))
            .map(String::as_str)
    }

    /// Sets a temporary (unsaved) custom variable. Returns whether the value
    /// changed.
    pub fn set_temporary_variable(&mut self, name: &str, value: &str) -> bool {
        if self.temporary_variables.get(name).map(String::as_str) == Some(value) {
            return false;
        }
        self.temporary_variables
            .insert(name.to_string(), value.to_string());
        true
    }

    pub fn current_time(&self) -> Time {
        let real_time = match self.phase {
            TimerPhase::NotRunning => Some(TimeSpan::ZERO),
            TimerPhase::Running => {
                let elapsed = self.monotonic_nanos() - self.adjusted_start_time;
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
            self.run.segments.get_mut(self.current_split_index as usize)
        } else {
            None
        }
    }

    pub fn pause_time(&self) -> Option<TimeSpan> {
        match self.phase {
            TimerPhase::Paused => Some(TimeSpan::from_nanos(
                (self.monotonic_nanos()
                    - self.start_time_with_offset
                    - self.time_paused_at.as_nanos() as i128)
                    .clamp(i64::MIN as i128, i64::MAX as i128) as i64,
            )),
            phase
                if phase != TimerPhase::NotRunning
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
                        let duration = self.utc_now().signed_duration_since(started);
                        TimeSpan::from_nanos(duration.num_nanoseconds().unwrap_or(0))
                    })
                    .unwrap_or_default(),
            ),
            TimerPhase::Ended => match (self.attempt_started, self.attempt_ended) {
                (Some(started), Some(ended)) => {
                    let duration = ended.signed_duration_since(started);
                    Some(TimeSpan::from_nanos(
                        duration.num_nanoseconds().unwrap_or(0),
                    ))
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
        self.mark_action();
        let now = self.monotonic_nanos();
        self.phase = TimerPhase::Running;
        self.current_split_index = 0;
        self.attempt_started = Some(self.utc_now());
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
        self.mark_action();
        let mut custom_variables = self.run.metadata.custom_variables.clone();
        custom_variables.extend(self.temporary_variables.clone());
        if let Some(segment) = self.run.segments.get_mut(index as usize) {
            segment.split_time = current_time;
            for (name, value) in custom_variables {
                segment.custom_variable_values.insert(name, value);
            }
        }
        self.current_split_index += 1;
        if self.current_split_index as usize >= self.run.segments.len() {
            self.phase = TimerPhase::Ended;
            self.attempt_ended = Some(self.utc_now());
        }
        self.run.has_changed = true;
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
        self.mark_action();
        let index = self.current_split_index;
        if let Some(segment) = self.run.segments.get_mut(index as usize) {
            segment.split_time = Time::default();
            segment.custom_variable_values.clear();
        }
        self.current_split_index += 1;
        self.run.has_changed = true;
        self.refresh_comparisons();
    }

    pub fn undo_split(&mut self) {
        if self.phase == TimerPhase::NotRunning || self.current_split_index <= 0 {
            return;
        }
        self.mark_action();
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
        self.mark_action();
        if self.phase != TimerPhase::Ended {
            self.attempt_ended = Some(self.utc_now());
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
        self.phase = TimerPhase::NotRunning;
        self.current_split_index = -1;
        for segment in &mut self.run.segments {
            segment.split_time = Time::default();
            segment.custom_variable_values.clear();
        }
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
                self.mark_action();
                self.time_paused_at = self.current_time().real_time.unwrap_or_default();
                self.phase = TimerPhase::Paused;
            }
            TimerPhase::Paused => {
                self.mark_action();
                self.adjusted_start_time =
                    self.monotonic_nanos() - self.time_paused_at.as_nanos() as i128;
                self.phase = TimerPhase::Running;
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
                segment.split_time.real_time =
                    segment.split_time.real_time.map(|time| time + pause_time);
                segment.split_time.game_time =
                    segment.split_time.game_time.map(|time| time + pause_time);
            }
        }
        self.adjusted_start_time = self.start_time_with_offset;
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
        }
    }

    pub fn set_timing_method(&mut self, method: TimingMethod) {
        if method != self.current_timing_method {
            self.current_timing_method = method;
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
            segment.segment_history.insert(
                attempt_index,
                Time {
                    real_time: real,
                    game_time: game,
                },
            );
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
        assert_eq!(
            timer.current_time().real_time.unwrap(),
            TimeSpan::from_seconds(5)
        );
        timer.pause();
        assert_eq!(
            timer.current_time().real_time.unwrap(),
            TimeSpan::from_seconds(5)
        );
        clock.advance(TimeSpan::from_seconds(1));
        assert_eq!(
            timer.current_time().real_time.unwrap(),
            TimeSpan::from_seconds(6)
        );
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
        assert_eq!(
            timer.current_time().real_time.unwrap(),
            TimeSpan::from_seconds(2)
        );
        clock.advance(TimeSpan::from_seconds(3));
        assert_eq!(
            timer.current_time().real_time.unwrap(),
            TimeSpan::from_seconds(5)
        );
    }

    #[test]
    fn delayed_split_is_recorded_at_its_timestamp() {
        let (clock, mut timer) = test_timer();
        timer.start();
        clock.advance(TimeSpan::from_seconds(10));
        let received = timer.now();
        let received_utc = clock.utc();
        // The UI thread gets to the split 100 ms later.
        clock.advance(TimeSpan::from_millis(100));
        timer.at(received, Timer::split);
        assert_eq!(
            timer.run.segments[0].split_time.real_time,
            Some(TimeSpan::from_seconds(10))
        );
        clock.advance(TimeSpan::from_seconds(2));
        let received = timer.now();
        clock.advance(TimeSpan::from_millis(80));
        timer.at(received, Timer::split);
        assert_eq!(timer.phase, TimerPhase::Ended);
        assert_eq!(
            timer.run.segments[1].split_time.real_time,
            Some(TimeSpan::from_millis(12_100))
        );
        assert_eq!(
            timer.attempt_ended,
            Some(received_utc + chrono::Duration::milliseconds(2100))
        );
        // Outside of `at`, the clock is "now" again.
        assert_eq!(timer.now().monotonic, clock.monotonic());
    }

    #[test]
    fn delayed_start_and_pause_use_their_timestamps() {
        let (clock, mut timer) = test_timer();
        let started_utc = clock.utc();
        let received = timer.now();
        clock.advance(TimeSpan::from_millis(100));
        timer.at(received, Timer::start);
        assert_eq!(timer.attempt_started, Some(started_utc));
        assert_eq!(
            timer.current_time().real_time,
            Some(TimeSpan::from_millis(100))
        );

        clock.advance(TimeSpan::from_seconds(5));
        let paused = timer.timestamp_ago(Duration::from_millis(40));
        timer.at(paused, Timer::pause);
        assert_eq!(timer.phase, TimerPhase::Paused);
        assert_eq!(
            timer.current_time().real_time,
            Some(TimeSpan::from_millis(5060))
        );

        clock.advance(TimeSpan::from_seconds(1));
        let resumed = timer.timestamp_ago(Duration::from_millis(500));
        timer.at(resumed, Timer::pause);
        assert_eq!(timer.phase, TimerPhase::Running);
        assert_eq!(
            timer.current_time().real_time,
            Some(TimeSpan::from_millis(5560))
        );
        assert_eq!(timer.pause_time(), Some(TimeSpan::from_millis(540)));
    }

    #[test]
    fn delayed_reset_records_the_attempt_end_at_its_timestamp() {
        let (clock, mut timer) = test_timer();
        timer.start();
        clock.advance(TimeSpan::from_seconds(3));
        let received = timer.now();
        let received_utc = clock.utc();
        clock.advance(TimeSpan::from_millis(100));
        timer.at(received, Timer::reset);
        assert_eq!(timer.run.attempts.len(), 1);
        assert_eq!(timer.run.attempts[0].ended, Some(received_utc));
    }

    #[test]
    fn timestamps_never_go_before_the_latest_action() {
        let (clock, mut timer) = test_timer();
        timer.start();
        clock.advance(TimeSpan::from_seconds(10));
        let older = timer.timestamp_ago(Duration::from_millis(50));
        let older_utc = clock.utc() - chrono::Duration::milliseconds(50);
        // A split handled "now" first, then an older one from another source.
        timer.split();
        timer.at(older, Timer::split);
        assert_eq!(
            timer.run.segments[0].split_time.real_time,
            Some(TimeSpan::from_seconds(10))
        );
        assert_eq!(
            timer.run.segments[1].split_time.real_time,
            Some(TimeSpan::from_seconds(10))
        );
        // The wall clock moves along with the monotonic one.
        assert_eq!(
            timer.attempt_ended,
            Some(older_utc + chrono::Duration::milliseconds(50))
        );
    }

    #[test]
    fn instants_from_other_threads_become_timestamps() {
        let mut timer = Timer::with_system_clock(Run::with_segments("Game", "Any%", ["One"]));
        let received = std::thread::spawn(Instant::now).join().unwrap();
        std::thread::sleep(Duration::from_millis(30));
        let at = timer.timestamp_of(received);
        timer.at(at, Timer::start);
        let elapsed = timer.current_time().real_time.unwrap();
        assert!(elapsed >= TimeSpan::from_millis(30), "{elapsed:?}");
        // A future instant counts as now.
        let future = timer.timestamp_of(Instant::now() + Duration::from_secs(60));
        assert!(future.monotonic <= timer.now().monotonic);
    }

    #[test]
    fn temporary_variables_are_shown_but_not_saved() {
        let (clock, mut timer) = test_timer();
        timer
            .run
            .metadata
            .custom_variables
            .insert("Route".to_string(), "Glitchless".to_string());
        assert!(timer.set_temporary_variable("Deaths", "3"));
        assert!(!timer.set_temporary_variable("Deaths", "3"));
        assert!(timer.set_temporary_variable("Route", "Any%"));
        assert_eq!(timer.custom_variable("Deaths"), Some("3"));
        assert_eq!(timer.custom_variable("Route"), Some("Any%"));
        assert_eq!(timer.custom_variable("Missing"), None);
        assert!(!timer.run.has_changed);

        timer.start();
        clock.advance(TimeSpan::from_seconds(1));
        timer.split();
        assert_eq!(timer.run.segments[0].custom_variable_values["Deaths"], "3");
        let saved = crate::save_run_to_string(&timer.run).unwrap();
        assert!(!saved.contains("Deaths"));
        assert!(saved.contains("Glitchless"));
    }
}
