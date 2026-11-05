//! The read-only view of the timer that the auto splitter thread sees.

use rustsplit_core::{Timer, TimerPhase};

use crate::AutoSplitterCommand;

/// The parts of the timer state an auto splitter can query.
///
/// The UI thread publishes a fresh snapshot (through an `ArcSwap`, so neither
/// side ever blocks the other) whenever the timer changes. The snapshot also
/// records which of the auto splitter's commands have been applied, so the
/// auto splitter can tell whether it is looking at stale state.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TimerSnapshot {
    /// The auto splitter instance whose commands `applied_seq` refers to.
    pub generation: u64,
    /// Sequence number of the last command of `generation` that the UI thread
    /// has applied.
    pub applied_seq: u64,
    pub phase: TimerPhase,
    /// LiveSplit's split index: -1 before the timer is started, the segment
    /// count once the run has ended.
    pub current_split_index: i32,
    pub segment_count: usize,
    /// For every segment before the current one, whether it was split
    /// (`true`) or skipped (`false`).
    pub splitted: Vec<bool>,
    pub game_time_paused: bool,
}

impl TimerSnapshot {
    pub fn from_timer(timer: &Timer) -> Self {
        let completed = usize::try_from(timer.current_split_index)
            .unwrap_or(0)
            .min(timer.run.segments.len());
        Self {
            generation: 0,
            applied_seq: 0,
            phase: timer.phase,
            current_split_index: timer.current_split_index,
            segment_count: timer.run.segments.len(),
            splitted: timer.run.segments[..completed]
                .iter()
                .map(|segment| segment.split_time.real_time.is_some())
                .collect(),
            game_time_paused: timer.is_game_time_paused,
        }
    }

    /// Whether the timer parts (ignoring the bookkeeping fields) are equal.
    pub fn same_timer_state(&self, other: &Self) -> bool {
        self.phase == other.phase
            && self.current_split_index == other.current_split_index
            && self.segment_count == other.segment_count
            && self.splitted == other.splitted
            && self.game_time_paused == other.game_time_paused
    }

    fn attempt_active(&self) -> bool {
        matches!(self.phase, TimerPhase::Running | TimerPhase::Paused)
    }

    /// Applies the effect `command` will have once the UI thread executes
    /// it, mirroring [`rustsplit_core::Timer`]'s state machine and
    /// [`crate::apply_command`]. Returns `false` if the command would not
    /// change anything, in which case it does not need to be sent at all.
    pub fn predict(&mut self, command: &AutoSplitterCommand) -> bool {
        let index = self.current_split_index;
        match command {
            AutoSplitterCommand::Start => {
                if self.phase != TimerPhase::NotRunning {
                    return false;
                }
                self.phase = TimerPhase::Running;
                self.current_split_index = 0;
                self.splitted.clear();
                self.game_time_paused = false;
            }
            AutoSplitterCommand::Split => {
                if self.phase != TimerPhase::Running
                    || index < 0
                    || index as usize >= self.segment_count
                {
                    return false;
                }
                self.splitted.push(true);
                self.current_split_index += 1;
                if self.current_split_index as usize >= self.segment_count {
                    self.phase = TimerPhase::Ended;
                }
            }
            AutoSplitterCommand::SkipSplit => {
                if !self.attempt_active()
                    || index < 0
                    || index as usize >= self.segment_count.saturating_sub(1)
                {
                    return false;
                }
                self.splitted.push(false);
                self.current_split_index += 1;
            }
            AutoSplitterCommand::UndoSplit => {
                if self.phase == TimerPhase::NotRunning || index <= 0 {
                    return false;
                }
                if self.phase == TimerPhase::Ended {
                    self.phase = TimerPhase::Running;
                }
                self.current_split_index -= 1;
                self.splitted.truncate(self.current_split_index as usize);
            }
            AutoSplitterCommand::Reset => {
                if self.phase == TimerPhase::NotRunning {
                    return false;
                }
                self.phase = TimerPhase::NotRunning;
                self.current_split_index = -1;
                self.splitted.clear();
                self.game_time_paused = false;
            }
            AutoSplitterCommand::SetGameTime(_) => return self.attempt_active(),
            AutoSplitterCommand::PauseGameTime => {
                if !self.attempt_active() || self.game_time_paused {
                    return false;
                }
                self.game_time_paused = true;
            }
            AutoSplitterCommand::ResumeGameTime => {
                if !self.attempt_active() || !self.game_time_paused {
                    return false;
                }
                self.game_time_paused = false;
            }
            AutoSplitterCommand::SetVariable { .. } => {}
        }
        true
    }
}
