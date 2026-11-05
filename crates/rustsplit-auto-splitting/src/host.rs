//! The UI-thread side of the auto splitter: owns the command channel and the
//! published timer snapshot, applies commands and keeps the run's stored
//! settings in sync.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::Arc;
use std::thread::Thread;

use arc_swap::ArcSwap;
use livesplit_auto_splitting::settings::{Map, Value, Widget};
use livesplit_auto_splitting::AutoSplitter;
use rustsplit_core::{Run, Timer};

use crate::adapter::{ChannelTimer, LogLine, Message, Waker};
use crate::runner::Runner;
use crate::{apply_command, StoredSettings, TimerSnapshot};

/// Log lines kept for display.
const MAX_LOG_LINES: usize = 200;

/// Upper bound of messages handled per [`AutoSplitterHost::poll`], so a
/// misbehaving auto splitter cannot stall a frame.
const MAX_MESSAGES_PER_POLL: usize = 16_384;

/// The lifecycle state of the auto splitter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostStatus<'a> {
    Unloaded,
    /// The module is being compiled on the auto splitter thread.
    Loading(&'a Path),
    Running(&'a Path),
    /// Loading failed or the auto splitter crashed (trapped).
    Failed(&'a Path, &'a str),
}

/// Something the UI may want to tell the user about.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HostEvent {
    Loaded(PathBuf),
    Failed(PathBuf, String),
}

#[derive(Debug, Default)]
pub struct PollOutcome {
    /// A command finished or reset the attempt; autosave like a manual
    /// split/reset would.
    pub autosave: bool,
    /// Number of timer commands applied.
    pub commands: usize,
    pub events: Vec<HostEvent>,
}

struct Active {
    path: PathBuf,
    stop: Arc<AtomicBool>,
    thread: Thread,
    /// The settings handed to the auto splitter, used until it is running.
    initial_settings: Map,
    /// The settings were edited while the module was still compiling.
    settings_edited: bool,
    auto_splitter: Option<Arc<AutoSplitter<ChannelTimer>>>,
}

impl Active {
    fn shut_down(&self) {
        self.stop.store(true, Ordering::Release);
        if let Some(auto_splitter) = &self.auto_splitter {
            // Breaks out of an `update` call that is still running.
            auto_splitter.interrupt_handle().interrupt();
        }
        self.thread.unpark();
    }
}

enum State {
    Unloaded,
    Active(Active),
    Failed { path: PathBuf, error: String },
}

pub struct AutoSplitterHost {
    sender: Sender<Message>,
    receiver: Receiver<Message>,
    waker: Waker,
    shared: Arc<ArcSwap<TimerSnapshot>>,
    published: TimerSnapshot,
    generation: u64,
    applied_seq: u64,
    state: State,
    logs: VecDeque<LogLine>,
    /// The settings map most recently written into the run, to skip
    /// re-serializing when nothing changed.
    synced: Option<(Map, Option<String>)>,
}

impl AutoSplitterHost {
    /// `waker` is called from the auto splitter thread whenever a command
    /// arrives that the UI should apply promptly.
    pub fn new(waker: impl Fn() + Send + Sync + 'static) -> Self {
        let (sender, receiver) = mpsc::channel();
        Self {
            sender,
            receiver,
            waker: Arc::new(waker),
            shared: Arc::new(ArcSwap::from_pointee(TimerSnapshot::default())),
            published: TimerSnapshot::default(),
            generation: 0,
            applied_seq: 0,
            state: State::Unloaded,
            logs: VecDeque::new(),
            synced: None,
        }
    }

    /// Loads (compiles and starts) the auto splitter at `path` on a new
    /// thread, replacing the current one. `settings` is the stored settings
    /// map to start with.
    pub fn load(&mut self, path: PathBuf, settings: Option<Map>, timer: &Timer) {
        self.unload();
        self.generation += 1;
        self.applied_seq = 0;
        self.logs.clear();
        self.synced = None;
        // The new auto splitter must see a snapshot of its own generation
        // before it sends anything.
        self.publish_now(timer);

        let stop = Arc::new(AtomicBool::new(false));
        let runner = Runner {
            generation: self.generation,
            path: path.clone(),
            settings: settings.clone(),
            timer: ChannelTimer::new(
                self.generation,
                self.shared.clone(),
                self.sender.clone(),
                self.waker.clone(),
            ),
            sender: self.sender.clone(),
            waker: self.waker.clone(),
            stop: stop.clone(),
        };
        match runner.spawn() {
            Ok(thread) => {
                self.state = State::Active(Active {
                    path,
                    stop,
                    thread,
                    initial_settings: settings.unwrap_or_default(),
                    settings_edited: false,
                    auto_splitter: None,
                });
            }
            Err(error) => {
                self.state = State::Failed {
                    path,
                    error: format!("could not start the auto splitter thread: {error}"),
                };
            }
        }
    }

    /// Makes the host act as if an auto splitter at `path` were running,
    /// returning the timer adapter it would have been instantiated with.
    #[cfg(test)]
    pub(crate) fn attach_test_timer(&mut self, path: &Path, timer: &Timer) -> ChannelTimer {
        self.unload();
        self.generation += 1;
        self.applied_seq = 0;
        self.publish_now(timer);
        self.state = State::Active(Active {
            path: path.to_path_buf(),
            stop: Arc::new(AtomicBool::new(false)),
            thread: std::thread::current(),
            initial_settings: Map::new(),
            settings_edited: false,
            auto_splitter: None,
        });
        ChannelTimer::new(
            self.generation,
            self.shared.clone(),
            self.sender.clone(),
            self.waker.clone(),
        )
    }

    /// Stops the current auto splitter. Never blocks: the thread exits on its
    /// own once its current `update` call returns or gets interrupted.
    pub fn unload(&mut self) {
        if let State::Active(active) = &self.state {
            active.shut_down();
            log::info!(target: "auto_splitter", "unloaded {}", active.path.display());
        }
        self.state = State::Unloaded;
        self.synced = None;
    }

    pub fn status(&self) -> HostStatus<'_> {
        match &self.state {
            State::Unloaded => HostStatus::Unloaded,
            State::Active(Active {
                path,
                auto_splitter: None,
                ..
            }) => HostStatus::Loading(path),
            State::Active(Active { path, .. }) => HostStatus::Running(path),
            State::Failed { path, error } => HostStatus::Failed(path, error),
        }
    }

    /// The path of the auto splitter that is loading or running.
    pub fn active_path(&self) -> Option<&Path> {
        match &self.state {
            State::Active(active) => Some(&active.path),
            _ => None,
        }
    }

    fn running(&self) -> Option<&Arc<AutoSplitter<ChannelTimer>>> {
        match &self.state {
            State::Active(active) => active.auto_splitter.as_ref(),
            _ => None,
        }
    }

    /// The settings widgets the auto splitter registered, once it runs.
    pub fn settings_widgets(&self) -> Option<Arc<Vec<Widget>>> {
        self.running()
            .map(|auto_splitter| auto_splitter.settings_widgets())
    }

    /// The current settings map (the initial one while still loading).
    pub fn settings_map(&self) -> Option<Map> {
        match &self.state {
            State::Active(Active {
                auto_splitter: Some(auto_splitter),
                ..
            }) => Some(auto_splitter.settings_map()),
            State::Active(active) => Some(active.initial_settings.clone()),
            _ => None,
        }
    }

    /// Replaces the whole settings map of the running auto splitter.
    pub fn set_settings_map(&mut self, map: Map) {
        match &mut self.state {
            State::Active(Active {
                auto_splitter: Some(auto_splitter),
                ..
            }) => auto_splitter.set_settings_map(map),
            State::Active(active) => {
                active.initial_settings = map;
                active.settings_edited = true;
            }
            _ => {}
        }
    }

    /// Changes a single setting. The auto splitter may modify its settings
    /// map concurrently, so this retries on top of its latest version.
    pub fn set_setting(&mut self, key: &str, value: Value) {
        if let Some(auto_splitter) = self.running() {
            loop {
                let old = auto_splitter.settings_map();
                let mut new = old.clone();
                new.insert(key.into(), value.clone());
                if auto_splitter.set_settings_map_if_unchanged(&old, new) {
                    break;
                }
            }
        } else if let Some(mut map) = self.settings_map() {
            map.insert(key.into(), value);
            self.set_settings_map(map);
        }
    }

    /// Recent messages of the auto splitter and the runtime.
    pub fn logs(&self) -> &VecDeque<LogLine> {
        &self.logs
    }

    /// Applies pending auto splitter commands to the timer, processes state
    /// changes, stores the settings into the run and republishes the timer
    /// snapshot. Call this on the UI thread every frame.
    pub fn poll(&mut self, timer: &mut Timer) -> PollOutcome {
        let mut outcome = PollOutcome::default();
        for _ in 0..MAX_MESSAGES_PER_POLL {
            let message = match self.receiver.try_recv() {
                Ok(message) => message,
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
            };
            self.handle(message, timer, &mut outcome);
        }
        self.sync_settings(&mut timer.run);
        self.publish(timer);
        outcome
    }

    fn handle(&mut self, message: Message, timer: &mut Timer, outcome: &mut PollOutcome) {
        let current = self.generation;
        match message {
            Message::Command {
                generation,
                seq,
                command,
            } if generation == current => {
                if matches!(self.state, State::Active(_)) {
                    let effect = apply_command(timer, &command);
                    outcome.autosave |= effect.autosave;
                    outcome.commands += 1;
                }
                self.applied_seq = seq;
            }
            Message::Log { generation, line } if generation == current => {
                if self.logs.len() >= MAX_LOG_LINES {
                    self.logs.pop_front();
                }
                self.logs.push_back(line);
            }
            Message::Loaded {
                generation,
                auto_splitter,
            } if generation == current => {
                if let State::Active(active) = &mut self.state {
                    // Settings edited in the UI while compiling replace the
                    // ones the module was instantiated with.
                    if active.settings_edited {
                        auto_splitter.set_settings_map(active.initial_settings.clone());
                    }
                    active.auto_splitter = Some(auto_splitter);
                    outcome.events.push(HostEvent::Loaded(active.path.clone()));
                } else {
                    auto_splitter.interrupt_handle().interrupt();
                }
            }
            Message::Stopped { generation, error } if generation == current => {
                if let State::Active(active) = &self.state {
                    let path = active.path.clone();
                    active.shut_down();
                    outcome
                        .events
                        .push(HostEvent::Failed(path.clone(), error.clone()));
                    self.state = State::Failed { path, error };
                    self.synced = None;
                }
            }
            // Leftovers of an auto splitter that was unloaded.
            Message::Loaded { auto_splitter, .. } => auto_splitter.interrupt_handle().interrupt(),
            Message::Command { .. } | Message::Log { .. } | Message::Stopped { .. } => {}
        }
    }

    /// Writes the active auto splitter's path and settings into the run's
    /// `<AutoSplitterSettings>`, marking the run as changed when they differ.
    fn sync_settings(&mut self, run: &mut Run) {
        let Some(path) = self.active_path() else {
            return;
        };
        let Some(map) = self.settings_map() else {
            return;
        };
        if let Some((synced_map, synced_xml)) = &self.synced {
            if synced_map.is_unchanged(&map) && *synced_xml == run.auto_splitter_settings {
                return;
            }
        }
        let desired = StoredSettings {
            script_path: Some(path.to_string_lossy().into_owned()),
            settings: map.clone(),
        };
        let current = run
            .auto_splitter_settings
            .as_deref()
            .and_then(|xml| StoredSettings::parse(xml).ok());
        if current.as_ref() != Some(&desired) {
            run.auto_splitter_settings = Some(desired.to_inner_xml());
            run.has_changed = true;
        }
        self.synced = Some((map, run.auto_splitter_settings.clone()));
    }

    /// Republishes the timer snapshot if the timer changed. Call this after
    /// anything else (keys, server, menus) may have mutated the timer, e.g.
    /// at the end of every frame.
    pub fn publish(&mut self, timer: &Timer) {
        if !matches!(self.state, State::Active(_)) {
            return;
        }
        let snapshot = TimerSnapshot::from_timer(timer);
        if snapshot.same_timer_state(&self.published)
            && self.published.generation == self.generation
            && self.published.applied_seq == self.applied_seq
        {
            return;
        }
        self.publish_now(timer);
    }

    fn publish_now(&mut self, timer: &Timer) {
        let mut snapshot = TimerSnapshot::from_timer(timer);
        snapshot.generation = self.generation;
        snapshot.applied_seq = self.applied_seq;
        self.shared.store(Arc::new(snapshot.clone()));
        self.published = snapshot;
    }
}

impl Drop for AutoSplitterHost {
    fn drop(&mut self) {
        self.unload();
    }
}
