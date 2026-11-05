//! The auto splitter thread: compiles the module, then calls its `update`
//! export at the tick rate the auto splitter requests (120 Hz by default).
//! A watchdog interrupts an `update` call that hangs for too long.

use std::error::Error;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use livesplit_auto_splitting::{settings::Map, AutoSplitter, Config, Runtime};

use crate::adapter::{ChannelTimer, Message, Waker};

/// How long a single `update` call may run before it is interrupted. This is
/// the same timeout livesplit-core uses.
const WATCHDOG_TIMEOUT: Duration = Duration::from_secs(5);

/// Ticks that are this late are dropped instead of being run back to back.
const MAX_TICK_LAG: Duration = Duration::from_millis(250);

pub(crate) struct Runner {
    pub generation: u64,
    pub path: PathBuf,
    pub settings: Option<Map>,
    pub timer: ChannelTimer,
    pub sender: Sender<Message>,
    pub waker: Waker,
    pub stop: Arc<AtomicBool>,
}

fn error_chain(error: &dyn Error) -> String {
    let mut text = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        text.push_str(": ");
        text.push_str(&cause.to_string());
        source = cause.source();
    }
    text
}

fn instantiate(
    path: &Path,
    timer: ChannelTimer,
    settings: Option<Map>,
) -> Result<AutoSplitter<ChannelTimer>, String> {
    let module = std::fs::read(path)
        .map_err(|error| format!("could not read {}: {error}", path.display()))?;
    // One engine per auto splitter: interrupting it (watchdog, unload) then
    // can never affect an auto splitter that was loaded afterwards.
    let runtime = Runtime::new(Config::default()).map_err(|error| error_chain(&error))?;
    let compiled = runtime
        .compile(&module)
        .map_err(|error| error_chain(&error))?;
    compiled
        .instantiate(timer, settings, None)
        .map_err(|error| error_chain(&error))
}

impl Runner {
    pub fn spawn(self) -> std::io::Result<thread::Thread> {
        let handle = thread::Builder::new()
            .name("auto-splitter".to_string())
            .spawn(move || self.run())?;
        Ok(handle.thread().clone())
    }

    fn run(self) {
        let Self {
            generation,
            path,
            settings,
            timer,
            sender,
            waker,
            stop,
        } = self;
        let report = |message: Message| {
            if sender.send(message).is_ok() {
                waker();
            }
        };
        let auto_splitter = match instantiate(&path, timer, settings) {
            Ok(auto_splitter) => Arc::new(auto_splitter),
            Err(error) => {
                log::error!(target: "auto_splitter", "{error}");
                report(Message::Stopped { generation, error });
                return;
            }
        };
        if stop.load(Ordering::Acquire) {
            return;
        }
        log::info!(target: "auto_splitter", "loaded {}", path.display());
        report(Message::Loaded {
            generation,
            auto_splitter: auto_splitter.clone(),
        });

        // Milliseconds since `epoch` (+1, so 0 means idle) at which the
        // current `update` call started.
        let busy_since = Arc::new(AtomicU64::new(0));
        let finished = Arc::new(AtomicBool::new(false));
        let epoch = Instant::now();
        let watchdog = {
            let busy_since = busy_since.clone();
            let finished = finished.clone();
            let interrupt = auto_splitter.interrupt_handle();
            thread::Builder::new()
                .name("auto-splitter-watchdog".to_string())
                .spawn(move || loop {
                    thread::park_timeout(Duration::from_millis(500));
                    if finished.load(Ordering::Acquire) {
                        return;
                    }
                    let started = busy_since.load(Ordering::Acquire);
                    if started == 0 {
                        continue;
                    }
                    let now = epoch.elapsed().as_millis() as u64 + 1;
                    if now.saturating_sub(started) > WATCHDOG_TIMEOUT.as_millis() as u64 {
                        log::error!(
                            target: "auto_splitter",
                            "update did not return within {} seconds, interrupting",
                            WATCHDOG_TIMEOUT.as_secs()
                        );
                        interrupt.interrupt();
                    }
                })
        };

        let mut next_tick = Instant::now();
        let error = loop {
            if stop.load(Ordering::Acquire) {
                break None;
            }
            busy_since.store(epoch.elapsed().as_millis() as u64 + 1, Ordering::Release);
            let result = auto_splitter.lock().update();
            busy_since.store(0, Ordering::Release);
            if let Err(error) = result {
                if stop.load(Ordering::Acquire) {
                    break None;
                }
                break Some(format!("the auto splitter crashed: {error:#}"));
            }

            next_tick += auto_splitter.tick_rate();
            let now = Instant::now();
            if next_tick + MAX_TICK_LAG < now {
                next_tick = now;
            }
            while !stop.load(Ordering::Acquire) {
                let now = Instant::now();
                if now >= next_tick {
                    break;
                }
                thread::park_timeout(next_tick - now);
            }
        };

        finished.store(true, Ordering::Release);
        if let Ok(watchdog) = &watchdog {
            watchdog.thread().unpark();
        }
        if let Some(error) = error {
            log::error!(target: "auto_splitter", "{error}");
            report(Message::Stopped { generation, error });
        }
    }
}
