//! Timer actions from keys, hotkeys, menus and the control socket.
//!
//! Every action carries the moment it was triggered. Input from other
//! threads is handled up to a frame later, or every 100 ms while the window is
//! hidden, so [`apply_timer_action`] runs it through [`Timer::at`] and the
//! split lands at the moment the key was pressed, not when the UI thread got
//! to it.

use std::time::Instant;

use rustsplit_core::{Timer, TimerPhase, TimingMethod};

/// A change to the timer's state.
#[derive(Clone, Debug, PartialEq)]
pub enum TimerAction {
    StartSplit,
    Reset,
    ResetAndSetPb,
    Undo,
    Skip,
    Pause,
    UndoAllPauses,
    SwitchComparisonNext,
    SwitchComparisonPrevious,
    ToggleTimingMethod,
    SetComparison(String),
    SetTimingMethod(TimingMethod),
}

/// What the app should do after a [`TimerAction`].
#[derive(Debug, Default, PartialEq, Eq)]
pub struct TimerActionOutcome {
    /// The attempt finished or was reset, so the splits should be autosaved.
    pub autosave: bool,
    pub status: Option<String>,
}

/// Applies `action` as if it happened at `received`.
pub fn apply_timer_action(
    timer: &mut Timer,
    action: &TimerAction,
    received: Instant,
) -> TimerActionOutcome {
    let at = timer.timestamp_of(received);
    timer.at(at, |timer| apply(timer, action))
}

fn apply(timer: &mut Timer, action: &TimerAction) -> TimerActionOutcome {
    let status = |message: String| TimerActionOutcome {
        autosave: false,
        status: Some(message),
    };
    match action {
        TimerAction::StartSplit => match timer.phase {
            TimerPhase::NotRunning => timer.start(),
            TimerPhase::Running | TimerPhase::Paused => {
                timer.split();
                return TimerActionOutcome {
                    autosave: timer.phase == TimerPhase::Ended,
                    status: None,
                };
            }
            TimerPhase::Ended => {}
        },
        TimerAction::Reset => {
            if timer.phase != TimerPhase::NotRunning {
                timer.reset();
                let message = if timer.run.file_path.is_some() {
                    "Run reset."
                } else {
                    "Run reset. Use Ctrl+S to save your splits."
                };
                return TimerActionOutcome {
                    autosave: true,
                    status: Some(message.to_string()),
                };
            }
        }
        TimerAction::ResetAndSetPb => {
            if timer.phase != TimerPhase::NotRunning {
                timer.reset_and_set_attempt_as_pb();
                return TimerActionOutcome {
                    autosave: true,
                    status: Some("Run reset and set as personal best.".to_string()),
                };
            }
        }
        TimerAction::Undo => timer.undo_split(),
        TimerAction::Skip => timer.skip_split(),
        TimerAction::Pause => {
            timer.pause();
            return status(
                match timer.phase {
                    TimerPhase::Paused => "Paused.",
                    TimerPhase::Running => "Resumed.",
                    _ => "Timer running.",
                }
                .to_string(),
            );
        }
        TimerAction::UndoAllPauses => {
            timer.undo_all_pauses();
            return status("Undid all pauses.".to_string());
        }
        TimerAction::SwitchComparisonNext => timer.switch_comparison_next(),
        TimerAction::SwitchComparisonPrevious => timer.switch_comparison_previous(),
        TimerAction::ToggleTimingMethod => {
            timer.toggle_timing_method();
            return status(format!("Timing method: {}", timer.current_timing_method));
        }
        TimerAction::SetComparison(comparison) => {
            timer.set_current_comparison(comparison.clone());
        }
        TimerAction::SetTimingMethod(method) => {
            timer.set_timing_method(*method);
            return status(format!("Timing method: {method}"));
        }
    }
    TimerActionOutcome::default()
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use rustsplit_core::{Run, TestClock, TimeSpan};

    use super::*;

    fn timer() -> (TestClock, Timer) {
        let clock = TestClock::new();
        let run = Run::with_segments("Game", "Any%", ["One", "Two"]);
        (clock.clone(), Timer::new(run, Arc::new(clock)))
    }

    #[test]
    fn start_split_and_finish() {
        let (clock, mut timer) = timer();
        let now = Instant::now();
        assert_eq!(
            apply_timer_action(&mut timer, &TimerAction::StartSplit, now),
            TimerActionOutcome::default()
        );
        assert_eq!(timer.phase, TimerPhase::Running);
        clock.advance(TimeSpan::from_seconds(5));
        assert!(
            !apply_timer_action(&mut timer, &TimerAction::StartSplit, now).autosave,
            "only finishing the run autosaves"
        );
        clock.advance(TimeSpan::from_seconds(5));
        assert!(apply_timer_action(&mut timer, &TimerAction::StartSplit, now).autosave);
        assert_eq!(timer.phase, TimerPhase::Ended);
        let outcome = apply_timer_action(&mut timer, &TimerAction::Reset, now);
        assert!(outcome.autosave);
        assert_eq!(
            outcome.status.as_deref(),
            Some("Run reset. Use Ctrl+S to save your splits.")
        );
        assert_eq!(timer.run.attempts.len(), 1);
        // Resetting a timer that isn't running does nothing.
        assert_eq!(
            apply_timer_action(&mut timer, &TimerAction::Reset, now),
            TimerActionOutcome::default()
        );
    }

    #[test]
    fn hotkey_split_handled_late_keeps_its_time() {
        let mut timer = Timer::with_system_clock(Run::with_segments("Game", "Any%", ["One"]));
        let pressed = Instant::now();
        // The hotkey thread queued the press, but a hidden window only runs
        // its logic every 100 ms.
        std::thread::sleep(Duration::from_millis(100));
        apply_timer_action(&mut timer, &TimerAction::StartSplit, pressed);
        let running = timer.current_time().real_time.unwrap();
        assert!(running >= TimeSpan::from_millis(100), "{running:?}");

        let before = timer.current_time().real_time.unwrap();
        let pressed = Instant::now();
        std::thread::sleep(Duration::from_millis(100));
        apply_timer_action(&mut timer, &TimerAction::StartSplit, pressed);
        let split = timer.run.segments[0].split_time.real_time.unwrap();
        assert!(
            split >= before - TimeSpan::from_millis(1)
                && split < before + TimeSpan::from_millis(50),
            "split at {split:?}, pressed at {before:?}"
        );
    }
}
