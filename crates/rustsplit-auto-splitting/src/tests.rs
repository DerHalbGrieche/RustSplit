//! Tests of the timer adapter, command application and stored settings.
//! None of these need a WebAssembly module; see `tests/wasm.rs` for that.

use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use livesplit_auto_splitting::settings::{List, Map, Value};
use livesplit_auto_splitting::{time, Timer as _, TimerState};
use rustsplit_core::{Run, TestClock, TimeSpan, Timer, TimerPhase};

use crate::{apply_command, AutoSplitterCommand, AutoSplitterHost, StoredSettings, TimerSnapshot};

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

const PATH: &str = "/splitters/game.wasm";

#[test]
fn prediction_matches_the_timer_state_machine() {
    use AutoSplitterCommand::*;
    let sequences: &[&[AutoSplitterCommand]] = &[
        &[
            Split, Start, Start, Split, Split, Split, Split, UndoSplit, Split,
        ],
        &[
            Start, SkipSplit, SkipSplit, SkipSplit, Split, Split, Reset, Reset,
        ],
        &[
            UndoSplit, Start, UndoSplit, Split, UndoSplit, UndoSplit, SkipSplit,
        ],
        &[
            Start,
            PauseGameTime,
            PauseGameTime,
            Split,
            ResumeGameTime,
            Reset,
        ],
        &[
            PauseGameTime,
            Start,
            PauseGameTime,
            Reset,
            ResumeGameTime,
            Start,
        ],
        &[Start, Split, Split, Split, Reset, Start, Split],
    ];
    for sequence in sequences {
        let (mut timer, clock) = timer(3);
        let mut predicted = TimerSnapshot::from_timer(&timer);
        for command in *sequence {
            clock.advance(TimeSpan::from_seconds(1));
            let before = TimerSnapshot::from_timer(&timer);
            let changes = predicted.predict(command);
            apply_command(&mut timer, command);
            let actual = TimerSnapshot::from_timer(&timer);
            assert!(
                predicted.same_timer_state(&actual),
                "after {command:?} in {sequence:?}: predicted {predicted:?}, actual {actual:?}"
            );
            if !changes {
                assert!(
                    before.same_timer_state(&actual),
                    "{command:?} was predicted to be a no-op but changed the timer"
                );
            }
        }
    }
}

#[test]
fn commands_go_through_the_channel_and_wake_the_ui() {
    let (mut host, wakes) = host();
    let (mut timer, clock) = timer(2);
    let mut adapter = host.attach_test_timer(Path::new(PATH), &timer);

    assert_eq!(adapter.state(), TimerState::NotRunning);
    assert_eq!(adapter.current_split_index(), None);
    adapter.start();
    // Before the UI applied it, the adapter already reports the new state, so
    // a second start in the same tick is not sent again.
    assert_eq!(adapter.state(), TimerState::Running);
    assert_eq!(adapter.current_split_index(), Some(0));
    adapter.start();
    assert_eq!(wakes.load(Ordering::SeqCst), 1);
    assert_eq!(
        timer.phase,
        TimerPhase::NotRunning,
        "the timer is untouched"
    );

    let outcome = host.poll(&mut timer);
    assert_eq!(outcome.commands, 1);
    assert_eq!(timer.phase, TimerPhase::Running);

    clock.advance(TimeSpan::from_seconds(5));
    adapter.split();
    assert_eq!(adapter.segment_splitted(0), Some(true));
    assert_eq!(adapter.segment_splitted(1), None);
    adapter.split();
    assert_eq!(adapter.state(), TimerState::Ended);
    assert_eq!(adapter.current_split_index(), Some(2));

    let outcome = host.poll(&mut timer);
    assert_eq!(outcome.commands, 2);
    assert!(outcome.autosave, "finishing a run autosaves");
    assert_eq!(timer.phase, TimerPhase::Ended);
    assert_eq!(wakes.load(Ordering::SeqCst), 3);

    adapter.reset();
    let outcome = host.poll(&mut timer);
    assert!(outcome.autosave, "a reset autosaves");
    assert_eq!(timer.phase, TimerPhase::NotRunning);
    assert_eq!(timer.run.attempt_count, 1);
}

#[test]
fn the_adapter_follows_changes_made_on_the_ui_thread() {
    let (mut host, _) = host();
    let (mut timer, clock) = timer(3);
    let mut adapter = host.attach_test_timer(Path::new(PATH), &timer);

    adapter.start();
    host.poll(&mut timer);
    // The user splits and then resets with a hotkey.
    clock.advance(TimeSpan::from_seconds(1));
    timer.split();
    host.publish(&timer);
    assert_eq!(adapter.current_split_index(), Some(1));
    assert_eq!(adapter.segment_splitted(0), Some(true));
    timer.reset();
    host.publish(&timer);
    assert_eq!(adapter.state(), TimerState::NotRunning);

    // And the auto splitter can start the next attempt.
    adapter.start();
    host.poll(&mut timer);
    assert_eq!(timer.phase, TimerPhase::Running);
    assert_eq!(timer.run.attempt_count, 2);
}

#[test]
fn game_time_and_variables() {
    let (mut host, wakes) = host();
    let (mut timer, clock) = timer(2);
    let mut adapter = host.attach_test_timer(Path::new(PATH), &timer);

    // Ignored while no attempt is in progress.
    adapter.pause_game_time();
    adapter.set_game_time(time::Duration::seconds(3));
    host.poll(&mut timer);
    assert!(!timer.is_game_time_paused);
    assert_eq!(timer.current_time().game_time, None);

    adapter.start();
    host.poll(&mut timer);
    clock.advance(TimeSpan::from_seconds(10));
    adapter.set_game_time(time::Duration::seconds(4));
    adapter.pause_game_time();
    adapter.pause_game_time();
    adapter.set_variable("Area", "Forest");
    adapter.set_variable("Area", "Forest");
    let wakes_before = wakes.load(Ordering::SeqCst);
    host.poll(&mut timer);
    assert!(timer.is_game_time_paused);
    assert_eq!(
        timer.current_time().game_time,
        Some(TimeSpan::from_seconds(4))
    );
    assert_eq!(timer.custom_variable("Area"), Some("Forest"));
    assert!(timer.run.metadata.custom_variables.is_empty());

    clock.advance(TimeSpan::from_seconds(10));
    assert_eq!(
        timer.current_time().game_time,
        Some(TimeSpan::from_seconds(4))
    );
    adapter.resume_game_time();
    host.poll(&mut timer);
    assert!(!timer.is_game_time_paused);
    clock.advance(TimeSpan::from_seconds(1));
    assert_eq!(
        timer.current_time().game_time,
        Some(TimeSpan::from_seconds(5))
    );
    // Only pause/resume woke the UI; game time and variables wait for the
    // next regular repaint.
    assert_eq!(wakes.load(Ordering::SeqCst), wakes_before + 1);
}

#[test]
fn commands_are_applied_when_they_were_issued() {
    let (mut host, _) = host();
    let mut timer = Timer::with_system_clock(Run::with_segments("Game", "Any%", ["One", "Two"]));
    let mut adapter = host.attach_test_timer(Path::new(PATH), &timer);
    adapter.start();
    host.poll(&mut timer);
    std::thread::sleep(std::time::Duration::from_millis(20));
    let before = timer.current_time().real_time.unwrap();
    adapter.split();
    let after = timer.current_time().real_time.unwrap();
    // The UI only gets to the split much later, e.g. while minimized.
    std::thread::sleep(std::time::Duration::from_millis(150));
    host.poll(&mut timer);
    let split = timer.run.segments[0].split_time.real_time.unwrap();
    let tolerance = TimeSpan::from_millis(1);
    assert!(
        before - tolerance <= split && split <= after + tolerance,
        "the split was not recorded when it was issued: {split:?} not in {before:?}..{after:?}"
    );
}

#[test]
fn set_variable_does_not_mark_the_run_as_changed() {
    let (mut timer, _) = timer(1);
    timer.run.has_changed = false;
    let command = AutoSplitterCommand::SetVariable {
        key: "Deaths".into(),
        value: "3".into(),
    };
    assert!(!apply_command(&mut timer, &command).autosave);
    assert!(!timer.run.has_changed);
    assert_eq!(timer.custom_variable("Deaths"), Some("3"));
    // Auto splitter variables are not saved into the split file.
    assert!(timer.run.metadata.custom_variables.is_empty());
}

#[test]
fn commands_of_an_unloaded_auto_splitter_are_dropped() {
    let (mut host, _) = host();
    let (mut timer, _) = timer(2);
    let mut old = host.attach_test_timer(Path::new(PATH), &timer);
    let _new = host.attach_test_timer(Path::new("/splitters/other.wasm"), &timer);
    old.start();
    assert_eq!(host.poll(&mut timer).commands, 0);
    assert_eq!(timer.phase, TimerPhase::NotRunning);

    let mut current = host.attach_test_timer(Path::new(PATH), &timer);
    host.unload();
    current.start();
    assert_eq!(host.poll(&mut timer).commands, 0);
    assert_eq!(timer.phase, TimerPhase::NotRunning);
}

#[test]
fn settings_are_written_into_the_run() {
    let (mut host, _) = host();
    let (mut timer, _) = timer(1);
    timer.run.has_changed = false;
    let _adapter = host.attach_test_timer(Path::new(PATH), &timer);
    host.set_setting("start", Value::Bool(false));
    host.poll(&mut timer);
    assert!(timer.run.has_changed);
    let stored =
        StoredSettings::parse(timer.run.auto_splitter_settings.as_deref().unwrap()).unwrap();
    assert_eq!(stored.script_path.as_deref(), Some(PATH));
    assert_eq!(stored.settings.get("start"), Some(&Value::Bool(false)));

    // Nothing changed, nothing is rewritten.
    timer.run.has_changed = false;
    host.poll(&mut timer);
    assert!(!timer.run.has_changed);
}

#[test]
fn stored_settings_roundtrip() {
    let mut nested = Map::new();
    nested.insert("level".into(), Value::I64(-3));
    let mut list = List::new();
    list.push(Value::F64(1.5));
    list.push(Value::String("a \"quoted\" <value> & more".into()));
    let mut settings = Map::new();
    settings.insert("start".into(), Value::Bool(true));
    settings.insert("reset".into(), Value::Bool(false));
    settings.insert("route".into(), Value::String("glitchless".into()));
    settings.insert("nested".into(), Value::Map(nested));
    settings.insert("list".into(), Value::List(list));
    let stored = StoredSettings {
        script_path: Some("C:\\Auto Splitters\\Game & Co.wasm".into()),
        settings,
    };
    let xml = stored.to_inner_xml();
    assert!(xml.contains("<Version>1.0</Version>"), "{xml}");
    // Attribute order is not stable (xmltree uses a HashMap), content is.
    assert!(
        xml.contains(r#"type="bool""#) && xml.contains(">True</Setting>"),
        "{xml}"
    );
    assert_eq!(StoredSettings::parse(&xml).unwrap(), stored);

    // Survives a full .lss save and load.
    let (mut timer, _) = timer(1);
    timer.run.auto_splitter_settings = Some(xml);
    let lss = rustsplit_core::save_run_to_string(&timer.run).unwrap();
    let run = rustsplit_core::load_run_from_str(&lss).unwrap();
    let parsed = StoredSettings::parse(run.auto_splitter_settings.as_deref().unwrap()).unwrap();
    assert_eq!(parsed, stored);
}

#[test]
fn foreign_settings_are_parsed_leniently() {
    // A LiveSplit ASL component payload: no script path, extra elements.
    let asl = r#"<Version>1.5</Version><AutoReset>True</AutoReset>
        <CustomSettings>
          <Setting id="any" type="bool">True</Setting>
          <Setting id="weird" type="color">Red</Setting>
          <Setting type="bool">True</Setting>
        </CustomSettings>"#;
    let stored = StoredSettings::parse(asl).unwrap();
    assert_eq!(stored.script_path, None);
    assert_eq!(stored.settings.len(), 1);
    assert_eq!(stored.settings.get("any"), Some(&Value::Bool(true)));
    assert!(StoredSettings::parse("").unwrap().settings.is_empty());
    assert!(StoredSettings::parse("<Unclosed>").is_err());
}

#[test]
fn stored_settings_only_apply_to_their_script() {
    let mut settings = Map::new();
    settings.insert("start".into(), Value::Bool(true));
    let stored = StoredSettings {
        script_path: Some(PATH.into()),
        settings: settings.clone(),
    };
    assert_eq!(stored.settings_for(Path::new(PATH)), Some(settings.clone()));
    assert_eq!(stored.settings_for(Path::new("/other.wasm")), None);
    let pathless = StoredSettings {
        script_path: None,
        settings: settings.clone(),
    };
    assert_eq!(
        pathless.settings_for(Path::new("/other.wasm")),
        Some(settings)
    );
}
