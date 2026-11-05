//! Timer commands issued by an auto splitter and their application to the
//! RustSplit [`Timer`] on the UI thread.

use rustsplit_core::{TimeSpan, Timer, TimerPhase};

/// A timer mutation requested by an auto splitter.
///
/// The auto splitter thread never touches the [`Timer`]. It sends these over
/// a channel and the UI thread applies them with [`apply_command`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AutoSplitterCommand {
    Start,
    Split,
    SkipSplit,
    UndoSplit,
    Reset,
    SetGameTime(TimeSpan),
    PauseGameTime,
    ResumeGameTime,
    SetVariable { key: String, value: String },
}

impl AutoSplitterCommand {
    /// Whether the UI should be woken up right away for this command.
    ///
    /// Game time and variables are typically updated on every tick, so they
    /// are picked up on the next regular repaint instead of forcing 120
    /// repaints per second. Anything visible to the run's state wakes the UI.
    pub fn is_urgent(&self) -> bool {
        !matches!(self, Self::SetGameTime(_) | Self::SetVariable { .. })
    }
}

/// What the UI should do after applying a command.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CommandEffect {
    /// The attempt finished or got reset, so the splits should be
    /// autosaved, exactly like a manual split/reset would.
    pub autosave: bool,
}

/// Applies an auto splitter command to the timer.
///
/// This mirrors what livesplit-core does for its auto splitting runtime:
/// game time commands are ignored while no attempt is in progress, and the
/// timer's own state machine rejects transitions that are not possible.
pub fn apply_command(timer: &mut Timer, command: &AutoSplitterCommand) -> CommandEffect {
    let attempt_active = matches!(timer.phase, TimerPhase::Running | TimerPhase::Paused);
    match command {
        AutoSplitterCommand::Start => {
            timer.start();
            CommandEffect::default()
        }
        AutoSplitterCommand::Split => {
            let before = timer.current_split_index;
            timer.split();
            CommandEffect {
                autosave: timer.current_split_index != before && timer.phase == TimerPhase::Ended,
            }
        }
        AutoSplitterCommand::SkipSplit => {
            timer.skip_split();
            CommandEffect::default()
        }
        AutoSplitterCommand::UndoSplit => {
            timer.undo_split();
            CommandEffect::default()
        }
        AutoSplitterCommand::Reset => {
            if timer.phase == TimerPhase::NotRunning {
                return CommandEffect::default();
            }
            timer.reset();
            CommandEffect { autosave: true }
        }
        AutoSplitterCommand::SetGameTime(time) => {
            if attempt_active {
                timer.set_game_time(Some(*time));
            }
            CommandEffect::default()
        }
        AutoSplitterCommand::PauseGameTime => {
            if attempt_active {
                timer.set_game_time_paused(true);
            }
            CommandEffect::default()
        }
        AutoSplitterCommand::ResumeGameTime => {
            if attempt_active {
                timer.set_game_time_paused(false);
            }
            CommandEffect::default()
        }
        AutoSplitterCommand::SetVariable { key, value } => {
            // Like LiveSplit's server `setcustomvariable`, but without marking
            // the run as changed: auto splitters update variables constantly
            // and that must not trigger saves or "unsaved changes" state.
            let variables = &mut timer.run.metadata.custom_variables;
            if variables.get(key) != Some(value) {
                variables.insert(key.clone(), value.clone());
            }
            CommandEffect::default()
        }
    }
}
