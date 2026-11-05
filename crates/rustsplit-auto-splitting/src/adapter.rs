//! The [`livesplit_auto_splitting::Timer`] implementation handed to the
//! runtime. It runs on the auto splitter thread and only talks to the UI
//! thread through a channel (commands) and an `ArcSwap` (timer snapshot).

use std::collections::HashMap;
use std::fmt;
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::time::Instant;

use arc_swap::ArcSwap;
use livesplit_auto_splitting::{time, AutoSplitter, LogLevel, Timer, TimerState};
use rustsplit_core::{TimeSpan, TimerPhase};

use crate::{AutoSplitterCommand, TimerSnapshot};

/// Callback that wakes the UI thread (for example `egui::Context::request_repaint`).
pub type Waker = Arc<dyn Fn() + Send + Sync>;

/// Upper bound for commands that were sent but not yet applied. Past this,
/// non-urgent commands (game time, variables) are dropped so a UI that is not
/// repainting (e.g. a minimized window) does not let the queue grow forever.
const MAX_BACKLOG: u64 = 4096;

/// Severity of a message from the auto splitter or the runtime.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogSeverity {
    Trace,
    Debug,
    Info,
    Warning,
    Error,
}

/// A message printed by the auto splitter or logged by the runtime.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogLine {
    /// `true` for `runtime_print_message` output of the auto splitter itself.
    pub from_auto_splitter: bool,
    pub severity: LogSeverity,
    pub text: String,
}

pub(crate) enum Message {
    Command {
        generation: u64,
        seq: u64,
        command: AutoSplitterCommand,
        /// When the auto splitter issued the command. The UI applies it at
        /// this moment, however long it takes to get to it.
        issued: Instant,
    },
    Log {
        generation: u64,
        line: LogLine,
    },
    Loaded {
        generation: u64,
        auto_splitter: Arc<AutoSplitter<ChannelTimer>>,
    },
    Stopped {
        generation: u64,
        error: String,
    },
}

pub struct ChannelTimer {
    generation: u64,
    shared: Arc<ArcSwap<TimerSnapshot>>,
    sender: Sender<Message>,
    waker: Waker,
    sent_seq: u64,
    /// The state the timer will be in once all sent commands are applied.
    predicted: Arc<TimerSnapshot>,
    variables: HashMap<String, String>,
}

impl ChannelTimer {
    pub(crate) fn new(
        generation: u64,
        shared: Arc<ArcSwap<TimerSnapshot>>,
        sender: Sender<Message>,
        waker: Waker,
    ) -> Self {
        let predicted = shared.load_full();
        Self {
            generation,
            shared,
            sender,
            waker,
            sent_seq: 0,
            predicted,
            variables: HashMap::new(),
        }
    }

    /// The timer state as the auto splitter should see it: the UI's snapshot
    /// once it has caught up with every command sent so far, otherwise the
    /// locally predicted state. Without this, an auto splitter that calls
    /// `timer_start` would still read "not running" until the next frame and
    /// might start or split twice.
    fn view(&self) -> Arc<TimerSnapshot> {
        let shared = self.shared.load_full();
        if self.sent_seq == 0
            || (shared.generation == self.generation && shared.applied_seq >= self.sent_seq)
        {
            shared
        } else {
            self.predicted.clone()
        }
    }

    fn backlog(&self) -> u64 {
        let shared = self.shared.load();
        if shared.generation == self.generation {
            self.sent_seq.saturating_sub(shared.applied_seq)
        } else {
            self.sent_seq
        }
    }

    fn send(&mut self, command: AutoSplitterCommand) {
        let urgent = command.is_urgent();
        if !urgent && self.backlog() >= MAX_BACKLOG {
            return;
        }
        let mut next = (*self.view()).clone();
        if !next.predict(&command) {
            return;
        }
        self.sent_seq += 1;
        self.predicted = Arc::new(next);
        let message = Message::Command {
            generation: self.generation,
            seq: self.sent_seq,
            command,
            issued: Instant::now(),
        };
        if self.sender.send(message).is_ok() && urgent {
            (self.waker)();
        }
    }

    fn log(&mut self, from_auto_splitter: bool, severity: LogSeverity, message: fmt::Arguments) {
        let text = message.to_string();
        let level = match severity {
            LogSeverity::Trace => log::Level::Trace,
            LogSeverity::Debug => log::Level::Debug,
            LogSeverity::Info => log::Level::Info,
            LogSeverity::Warning => log::Level::Warn,
            LogSeverity::Error => log::Level::Error,
        };
        log::log!(target: "auto_splitter", level, "{text}");
        if severity == LogSeverity::Trace || severity == LogSeverity::Debug {
            return;
        }
        let _ = self.sender.send(Message::Log {
            generation: self.generation,
            line: LogLine {
                from_auto_splitter,
                severity,
                text,
            },
        });
    }
}

fn duration_to_time_span(duration: time::Duration) -> TimeSpan {
    let nanos = duration
        .whole_nanoseconds()
        .clamp(i64::MIN as i128, i64::MAX as i128) as i64;
    TimeSpan::from_nanos(nanos)
}

impl Timer for ChannelTimer {
    fn state(&self) -> TimerState {
        match self.view().phase {
            TimerPhase::NotRunning => TimerState::NotRunning,
            TimerPhase::Running => TimerState::Running,
            TimerPhase::Paused => TimerState::Paused,
            TimerPhase::Ended => TimerState::Ended,
        }
    }

    fn current_split_index(&self) -> Option<usize> {
        let view = self.view();
        if view.phase == TimerPhase::NotRunning {
            return None;
        }
        usize::try_from(view.current_split_index).ok()
    }

    fn segment_splitted(&self, idx: usize) -> Option<bool> {
        self.view().splitted.get(idx).copied()
    }

    fn start(&mut self) {
        self.send(AutoSplitterCommand::Start);
    }

    fn split(&mut self) {
        self.send(AutoSplitterCommand::Split);
    }

    fn skip_split(&mut self) {
        self.send(AutoSplitterCommand::SkipSplit);
    }

    fn undo_split(&mut self) {
        self.send(AutoSplitterCommand::UndoSplit);
    }

    fn reset(&mut self) {
        self.send(AutoSplitterCommand::Reset);
    }

    fn set_game_time(&mut self, time: time::Duration) {
        self.send(AutoSplitterCommand::SetGameTime(duration_to_time_span(
            time,
        )));
    }

    fn pause_game_time(&mut self) {
        self.send(AutoSplitterCommand::PauseGameTime);
    }

    fn resume_game_time(&mut self) {
        self.send(AutoSplitterCommand::ResumeGameTime);
    }

    fn set_variable(&mut self, key: &str, value: &str) {
        if self.variables.get(key).map(String::as_str) == Some(value) {
            return;
        }
        self.variables.insert(key.to_string(), value.to_string());
        self.send(AutoSplitterCommand::SetVariable {
            key: key.to_string(),
            value: value.to_string(),
        });
    }

    fn log_auto_splitter(&mut self, message: fmt::Arguments) {
        self.log(true, LogSeverity::Info, message);
    }

    fn log_runtime(&mut self, message: fmt::Arguments, log_level: LogLevel) {
        let severity = match log_level {
            LogLevel::Trace => LogSeverity::Trace,
            LogLevel::Debug => LogSeverity::Debug,
            LogLevel::Info => LogSeverity::Info,
            LogLevel::Warning => LogSeverity::Warning,
            LogLevel::Error => LogSeverity::Error,
        };
        self.log(false, severity, message);
    }
}
