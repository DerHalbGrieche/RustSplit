//! End-to-end tests with tiny auto splitters written in WebAssembly text
//! format, run on the real `livesplit-auto-splitting` runtime thread.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use rustsplit_auto_splitting::settings::{Map, Value, WidgetKind};
use rustsplit_auto_splitting::{AutoSplitterHost, HostEvent, HostStatus, StoredSettings};
use rustsplit_core::{Run, TestClock, TimeSpan, Timer, TimerPhase};

/// Starts the timer on the first tick and splits on every later tick unless
/// the "split" setting is turned off. Also publishes a variable.
const SPLITTER: &str = r#"
(module
  (import "env" "timer_get_state" (func $state (result i32)))
  (import "env" "timer_start" (func $start))
  (import "env" "timer_split" (func $split))
  (import "env" "timer_set_variable" (func $set_variable (param i32 i32 i32 i32)))
  (import "env" "user_settings_add_bool"
    (func $add_bool (param i32 i32 i32 i32 i32) (result i32)))
  (import "env" "runtime_set_tick_rate" (func $set_tick_rate (param f64)))
  (memory (export "memory") 1)
  (data (i32.const 0) "split")
  (data (i32.const 16) "Split automatically")
  (data (i32.const 48) "Phase")
  (data (i32.const 64) "ticking")
  (global $initialized (mut i32) (i32.const 0))
  (global $split_enabled (mut i32) (i32.const 0))
  (func (export "update")
    (if (i32.eqz (global.get $initialized))
      (then
        (global.set $split_enabled
          (call $add_bool (i32.const 0) (i32.const 5) (i32.const 16) (i32.const 19) (i32.const 1)))
        (call $set_tick_rate (f64.const 1000))
        (global.set $initialized (i32.const 1))))
    (call $set_variable (i32.const 48) (i32.const 5) (i32.const 64) (i32.const 7))
    (if (i32.eqz (call $state))
      (then (call $start))
      (else (if (global.get $split_enabled) (then (call $split)))))))
"#;

const TRAPS: &str = r#"
(module
  (memory (export "memory") 1)
  (func (export "update") unreachable))
"#;

const LOOPS_FOREVER: &str = r#"
(module
  (memory (export "memory") 1)
  (func (export "update") (loop $forever (br $forever))))
"#;

fn write_module(name: &str, wat: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "rustsplit-auto-splitting-{}-{name}.wasm",
        std::process::id()
    ));
    std::fs::write(&path, wat::parse_str(wat).expect("valid WAT")).unwrap();
    path
}

fn timer(segments: usize) -> (Timer, Arc<TestClock>) {
    let clock = Arc::new(TestClock::new());
    let names = (1..=segments).map(|index| format!("Segment {index}"));
    let run = Run::with_segments("Game", "Any%", names);
    (Timer::new(run, clock.clone()), clock)
}

fn host() -> (AutoSplitterHost, Arc<AtomicUsize>) {
    let wakes = Arc::new(AtomicUsize::new(0));
    let counter = wakes.clone();
    let host = AutoSplitterHost::new(move || {
        counter.fetch_add(1, Ordering::SeqCst);
    });
    (host, wakes)
}

/// Runs the "UI loop" until `done` holds or the timeout expires.
fn pump(
    host: &mut AutoSplitterHost,
    timer: &mut Timer,
    clock: &TestClock,
    timeout: Duration,
    mut done: impl FnMut(&AutoSplitterHost, &Timer, &[HostEvent]) -> bool,
) -> Vec<HostEvent> {
    let deadline = Instant::now() + timeout;
    let mut events = Vec::new();
    loop {
        clock.advance(TimeSpan::from_millis(100));
        let outcome = host.poll(timer);
        events.extend(outcome.events);
        if done(host, timer, &events) {
            return events;
        }
        assert!(
            Instant::now() < deadline,
            "timed out; status {:?}, phase {:?}, events {events:?}",
            host.status(),
            timer.phase
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn auto_splitter_starts_and_splits_the_timer() {
    let path = write_module("splits", SPLITTER);
    let (mut host, wakes) = host();
    let (mut timer, clock) = timer(3);
    host.load(path.clone(), None, &timer);
    assert_eq!(host.status(), HostStatus::Loading(&path));

    let events = pump(
        &mut host,
        &mut timer,
        &clock,
        Duration::from_secs(30),
        |_, timer, _| timer.phase == TimerPhase::Ended,
    );
    assert_eq!(events.first(), Some(&HostEvent::Loaded(path.clone())));
    assert_eq!(host.status(), HostStatus::Running(&path));
    assert_eq!(timer.run.attempt_count, 1);
    assert!(timer
        .run
        .segments
        .iter()
        .all(|segment| segment.split_time.real_time.is_some()));
    assert!(
        wakes.load(Ordering::SeqCst) >= 4,
        "start and 3 splits wake the UI"
    );
    assert_eq!(
        timer
            .run
            .metadata
            .custom_variables
            .get("Phase")
            .map(String::as_str),
        Some("ticking")
    );

    let widgets = host.settings_widgets().unwrap();
    assert_eq!(widgets.len(), 1);
    assert_eq!(&*widgets[0].key, "split");
    assert_eq!(&*widgets[0].description, "Split automatically");
    assert!(matches!(
        widgets[0].kind,
        WidgetKind::Bool {
            default_value: true
        }
    ));

    let stored =
        StoredSettings::parse(timer.run.auto_splitter_settings.as_deref().unwrap()).unwrap();
    assert_eq!(stored.script_path.as_deref(), path.to_str());

    // Unloading stops the commands; a reset stays a reset.
    host.unload();
    timer.reset();
    std::thread::sleep(Duration::from_millis(50));
    host.poll(&mut timer);
    assert_eq!(timer.phase, TimerPhase::NotRunning);
    assert_eq!(host.status(), HostStatus::Unloaded);
    let _ = std::fs::remove_file(path);
}

#[test]
fn stored_settings_are_passed_to_the_auto_splitter() {
    let path = write_module("settings", SPLITTER);
    let (mut host, _) = host();
    let (mut timer, clock) = timer(3);
    let mut settings = Map::new();
    settings.insert("split".into(), Value::Bool(false));
    host.load(path.clone(), Some(settings), &timer);

    pump(
        &mut host,
        &mut timer,
        &clock,
        Duration::from_secs(30),
        |host, timer, _| {
            matches!(host.status(), HostStatus::Running(_)) && timer.phase == TimerPhase::Running
        },
    );
    // Give it plenty of ticks to (not) split.
    let started = Instant::now();
    while started.elapsed() < Duration::from_millis(200) {
        clock.advance(TimeSpan::from_millis(100));
        host.poll(&mut timer);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(timer.current_split_index, 0);
    assert_eq!(
        host.settings_map().unwrap().get("split"),
        Some(&Value::Bool(false))
    );
    let _ = std::fs::remove_file(path);
}

#[test]
fn a_trapping_auto_splitter_is_unloaded_with_an_error() {
    let path = write_module("traps", TRAPS);
    let (mut host, _) = host();
    let (mut timer, clock) = timer(1);
    host.load(path.clone(), None, &timer);
    let events = pump(
        &mut host,
        &mut timer,
        &clock,
        Duration::from_secs(30),
        |_, _, events| {
            events
                .iter()
                .any(|event| matches!(event, HostEvent::Failed(..)))
        },
    );
    assert!(matches!(host.status(), HostStatus::Failed(failed, _) if failed == path));
    assert!(events.contains(&HostEvent::Loaded(path.clone())));
    let _ = std::fs::remove_file(path);
}

#[test]
fn an_invalid_module_reports_an_error() {
    let path = std::env::temp_dir().join(format!(
        "rustsplit-auto-splitting-{}-invalid.wasm",
        std::process::id()
    ));
    std::fs::write(&path, b"definitely not wasm").unwrap();
    let (mut host, _) = host();
    let (mut timer, clock) = timer(1);
    host.load(path.clone(), None, &timer);
    let events = pump(
        &mut host,
        &mut timer,
        &clock,
        Duration::from_secs(30),
        |_, _, events| !events.is_empty(),
    );
    assert!(matches!(&events[0], HostEvent::Failed(failed, _) if *failed == path));

    host.load(
        PathBuf::from("/nonexistent/auto-splitter.wasm"),
        None,
        &timer,
    );
    let events = pump(
        &mut host,
        &mut timer,
        &clock,
        Duration::from_secs(30),
        |_, _, events| !events.is_empty(),
    );
    assert!(matches!(&events[0], HostEvent::Failed(_, error) if error.contains("could not read")));
    let _ = std::fs::remove_file(path);
}

#[test]
fn the_watchdog_interrupts_a_hung_update() {
    let path = write_module("hangs", LOOPS_FOREVER);
    let (mut host, _) = host();
    let (mut timer, clock) = timer(1);
    host.load(path.clone(), None, &timer);
    let events = pump(
        &mut host,
        &mut timer,
        &clock,
        Duration::from_secs(30),
        |_, _, events| {
            events
                .iter()
                .any(|event| matches!(event, HostEvent::Failed(..)))
        },
    );
    assert!(events.contains(&HostEvent::Loaded(path.clone())));
    let _ = std::fs::remove_file(path);
}

#[test]
fn unloading_interrupts_a_hung_update() {
    let path = write_module("hangs-unload", LOOPS_FOREVER);
    let (mut host, _) = host();
    let (mut timer, clock) = timer(1);
    host.load(path.clone(), None, &timer);
    pump(
        &mut host,
        &mut timer,
        &clock,
        Duration::from_secs(30),
        |host, _, _| matches!(host.status(), HostStatus::Running(_)),
    );
    let started = Instant::now();
    host.unload();
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "unload never blocks"
    );
    assert_eq!(host.status(), HostStatus::Unloaded);
    let _ = std::fs::remove_file(path);
}
