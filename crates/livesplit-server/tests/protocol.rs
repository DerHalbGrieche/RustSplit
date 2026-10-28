use std::sync::Arc;

use livesplit_core::model::{TimingMethod, BEST_SEGMENTS, PERSONAL_BEST};
use livesplit_core::{Run, TestClock, Time, TimeSpan, Timer};
use livesplit_server::{
    handle_command, parse_command, ServerCommand, ServerCommandError, DEFAULT_PORT,
};

fn test_timer() -> (TestClock, Timer) {
    let clock = TestClock::new();
    let mut run = Run::with_segments("Test Game", "Any%", ["One", "Two", "Three"]);
    run.segments[0].set_personal_best(Time::from_method(
        TimingMethod::RealTime,
        Some(TimeSpan::from_seconds(10)),
    ));
    run.segments[1].set_personal_best(Time::from_method(
        TimingMethod::RealTime,
        Some(TimeSpan::from_seconds(20)),
    ));
    run.segments[2].set_personal_best(Time::from_method(
        TimingMethod::RealTime,
        Some(TimeSpan::from_seconds(30)),
    ));
    for segment in &mut run.segments {
        segment.best_segment_time =
            Time::from_method(TimingMethod::RealTime, Some(TimeSpan::from_seconds(9)));
    }
    let timer = Timer::new(run, Arc::new(clock.clone()));
    (clock, timer)
}

fn response(timer: &mut Timer, command: ServerCommand) -> String {
    handle_command(timer, &command)
        .response
        .expect("command should produce a response")
}

#[test]
fn default_port_matches_livesplit() {
    assert_eq!(DEFAULT_PORT, 16834);
}

#[test]
fn parses_every_command() {
    let cases: Vec<(&str, ServerCommand)> = vec![
        ("startorsplit", ServerCommand::StartOrSplit),
        ("start", ServerCommand::Start),
        ("starttimer", ServerCommand::Start),
        ("split", ServerCommand::Split),
        ("undosplit", ServerCommand::Undo),
        ("unsplit", ServerCommand::Undo),
        ("skipsplit", ServerCommand::Skip),
        ("reset", ServerCommand::Reset),
        ("pause", ServerCommand::Pause),
        ("resume", ServerCommand::Resume),
        ("togglepause", ServerCommand::TogglePause),
        ("undoallpauses", ServerCommand::UndoAllPauses),
        (
            "resetandsetattemptaspb",
            ServerCommand::ResetAndSetAttemptAsPb,
        ),
        (
            "initializegametime",
            ServerCommand::InitializeGameTime,
        ),
        (
            "setgametime 01:02:03.5",
            ServerCommand::SetGameTime(Some(TimeSpan::from_nanos(
                3_723_500_000_000,
            ))),
        ),
        ("setgametime -", ServerCommand::SetGameTime(None)),
        (
            "setloadingtimes 0:01.5",
            ServerCommand::SetLoadingTimes(TimeSpan::from_nanos(1_500_000_000)),
        ),
        (
            "addloadingtimes 0:02",
            ServerCommand::AddLoadingTimes(TimeSpan::from_seconds(2)),
        ),
        ("pausegametime", ServerCommand::PauseGameTime),
        ("unpausegametime", ServerCommand::ResumeGameTime),
        ("resumegametime", ServerCommand::ResumeGameTime),
        ("alwayspausegametime", ServerCommand::AlwaysPauseGameTime),
        ("switchto realtime", ServerCommand::SwitchToRealTime),
        ("switchto gametime", ServerCommand::SwitchToGameTime),
        ("switchtorealtime", ServerCommand::SwitchToRealTime),
        ("switchtogametime", ServerCommand::SwitchToGameTime),
        ("toggletimingmethod", ServerCommand::ToggleTimingMethod),
        (
            "setcomparison Best Segments",
            ServerCommand::SwitchComparison("Best Segments".to_string()),
        ),
        (
            "switchcomparison Personal Best",
            ServerCommand::SwitchComparison("Personal Best".to_string()),
        ),
        (
            "switchcomparisonnext",
            ServerCommand::SwitchComparisonNext,
        ),
        (
            "switchcomparisonprevious",
            ServerCommand::SwitchComparisonPrevious,
        ),
        ("getcurrenttime", ServerCommand::GetCurrentTime),
        ("getcurrentrealtime", ServerCommand::GetCurrentRealTime),
        ("getcurrentgametime", ServerCommand::GetCurrentGameTime),
        ("getlastsplittime", ServerCommand::GetLastSplitTime),
        (
            "getprevioussplittime",
            ServerCommand::GetLastSplitTime,
        ),
        (
            "getcurrentsplittime",
            ServerCommand::GetCurrentSplitTime(None),
        ),
        (
            "getcomparisonsplittime Best Segments",
            ServerCommand::GetCurrentSplitTime(Some("Best Segments".to_string())),
        ),
        ("getcomparisons", ServerCommand::GetComparisons),
        (
            "getcomparisonname",
            ServerCommand::GetCurrentComparison,
        ),
        (
            "getcurrenttimerphase",
            ServerCommand::GetCurrentTimerPhase,
        ),
        ("gettimerphase", ServerCommand::GetCurrentTimerPhase),
        ("getsplitindex", ServerCommand::GetSplitIndex),
        ("getsplitnames", ServerCommand::GetSplitNames),
        ("getsplitcount", ServerCommand::GetSplitCount),
        (
            "getsplitname 1",
            ServerCommand::GetSplitName { index: 0 },
        ),
        (
            "getsplitname 0",
            ServerCommand::GetSplitName { index: 0 },
        ),
        (
            "getsplitname -1",
            ServerCommand::GetSplitName { index: -1 },
        ),
        (
            "setsplitname 2 Second split",
            ServerCommand::SetSplitName {
                index: 1,
                name: "Second split".to_string(),
            },
        ),
        (
            "setsplitname -1 Last split",
            ServerCommand::SetSplitName {
                index: -1,
                name: "Last split".to_string(),
            },
        ),
        (
            "setcurrentsplitname A custom name",
            ServerCommand::SetCurrentSplitName("A custom name".to_string()),
        ),
        (
            "getcurrentsplitname",
            ServerCommand::GetCurrentSplitName,
        ),
        (
            "getprevioussplitname",
            ServerCommand::GetPreviousSplitName,
        ),
        (
            "getupcomingsplitname",
            ServerCommand::GetUpcomingSplitName,
        ),
        ("gettimerstate", ServerCommand::GetTimerState),
        ("getstate", ServerCommand::GetTimerState),
        ("getdelta", ServerCommand::GetDelta(None)),
        (
            "getdelta Best Segments",
            ServerCommand::GetDelta(Some("Best Segments".to_string())),
        ),
        ("getfinaltime", ServerCommand::GetFinalTime(None)),
        (
            "getfinaltime Personal Best",
            ServerCommand::GetFinalTime(Some("Personal Best".to_string())),
        ),
        (
            "getpredictedtime Personal Best",
            ServerCommand::GetPredictedTime(Some("Personal Best".to_string())),
        ),
        ("getbestpossibletime", ServerCommand::GetBestPossibleTime),
        ("getpausedrealtime", ServerCommand::GetPausedRealTime),
        ("getpausedgametime", ServerCommand::GetPausedGameTime),
        ("getoffset", ServerCommand::GetOffset),
        ("gettimingmethod", ServerCommand::GetTimingMethod),
        ("getgamename", ServerCommand::GetGameName),
        ("getcategoryname", ServerCommand::GetCategoryName),
        (
            "getcategoryvariables",
            ServerCommand::GetCategoryVariables,
        ),
        ("getattemptcount", ServerCommand::GetAttemptCount),
        ("getcompletedcount", ServerCommand::GetCompletedCount),
        (
            "getcustomvariablevalue Region",
            ServerCommand::GetCustomVariableValue("Region".to_string()),
        ),
        (
            "setcustomvariable [\"Region\", \"Europe\"]",
            ServerCommand::SetCustomVariable {
                name: "Region".to_string(),
                value: "Europe".to_string(),
            },
        ),
        ("getlayoutpath", ServerCommand::GetLayoutPath),
        ("savelayout", ServerCommand::SaveLayout),
        (
            "savelayoutas /tmp/test.lsl",
            ServerCommand::SaveLayoutAs("/tmp/test.lsl".to_string()),
        ),
        ("getsplitspath", ServerCommand::GetSplitsPath),
        ("savesplits", ServerCommand::SaveSplits),
        (
            "savesplitsas /tmp/test.lss",
            ServerCommand::SaveSplitsAs("/tmp/test.lss".to_string()),
        ),
        (
            "switchlayout /tmp/test.lsl",
            ServerCommand::SwitchLayout("/tmp/test.lsl".to_string()),
        ),
        (
            "switchsplits /tmp/test.lss",
            ServerCommand::SwitchSplits("/tmp/test.lss".to_string()),
        ),
        (
            "getsplitsscreenshot",
            ServerCommand::GetSplitsScreenshot,
        ),
        (
            "savesplitsscreenshot /tmp/test.png",
            ServerCommand::SaveSplitsScreenshot("/tmp/test.png".to_string()),
        ),
        (
            "globalhotkeysenabled",
            ServerCommand::GlobalHotkeysEnabled,
        ),
        ("enableglobalhotkeys", ServerCommand::EnableGlobalHotkeys),
        (
            "disableglobalhotkeys",
            ServerCommand::DisableGlobalHotkeys,
        ),
        (
            "switchhotkeyprofile Default",
            ServerCommand::SwitchHotkeyProfile("Default".to_string()),
        ),
        ("gethotkeyprofile", ServerCommand::GetHotkeyProfile),
        (
            "getautosplitterpath",
            ServerCommand::GetAutoSplitterPath,
        ),
        (
            "autosplitteractivated",
            ServerCommand::AutoSplitterActivated,
        ),
        (
            "getlivesplitversion",
            ServerCommand::GetLiveSplitVersion,
        ),
        ("getlivesplitpath", ServerCommand::GetLiveSplitPath),
        ("getservertype", ServerCommand::GetServerType),
        ("ping", ServerCommand::Ping),
        ("quit", ServerCommand::Quit),
        ("exit", ServerCommand::Quit),
    ];

    for (line, expected) in cases {
        assert_eq!(
            parse_command(line).expect("command should parse"),
            expected,
            "parsing {line:?}"
        );
    }
}

#[test]
fn parser_is_case_sensitive_and_rejects_unknown_commands() {
    assert!(matches!(
        parse_command("PING"),
        Err(ServerCommandError::UnknownCommand(command)) if command == "PING"
    ));
    assert!(matches!(
        parse_command("StartOrSplit"),
        Err(ServerCommandError::UnknownCommand(_))
    ));
    assert!(matches!(
        parse_command("doesnotexist"),
        Err(ServerCommandError::UnknownCommand(_))
    ));
    assert_eq!(parse_command(""), Err(ServerCommandError::Empty));
}

#[test]
fn parser_reports_missing_and_invalid_arguments() {
    for command in [
        "getsplitname",
        "setsplitname 1",
        "setgametime",
        "setloadingtimes",
        "addloadingtimes",
        "switchto",
        "switchto sideways",
        "getcustomvariablevalue",
        "setcustomvariable",
        "savesplitsas",
        "getsplitname nope",
        "setsplitname nope Name",
        "setgametime nope",
        "setloadingtimes nope",
        "savelayoutas",
        "switchhotkeyprofile",
    ] {
        assert!(
            parse_command(command).is_err(),
            "{command:?} should not parse"
        );
    }

    assert!(matches!(
        parse_command("setcustomvariable [\"only\"]"),
        Err(ServerCommandError::InvalidArgument { .. })
    ));
    assert!(matches!(
        parse_command("setcustomvariable not-json"),
        Err(ServerCommandError::InvalidArgument { .. })
    ));
    assert!(matches!(
        parse_command("getsplitname 999999999999999999999999"),
        Err(ServerCommandError::InvalidArgument { .. })
    ));
}

#[test]
fn custom_variable_json_handles_escapes() {
    let command =
        parse_command(r#"setcustomvariable ["Note", "line\nquote\" slash\\"]"#).unwrap();
    assert_eq!(
        command,
        ServerCommand::SetCustomVariable {
            name: "Note".to_string(),
            value: "line\nquote\" slash\\".to_string(),
        }
    );
}

#[test]
fn start_split_reset_and_getters_use_test_clock() {
    let (clock, mut timer) = test_timer();

    assert_eq!(
        response(&mut timer, ServerCommand::GetCurrentTimerPhase),
        "NotRunning"
    );
    assert_eq!(response(&mut timer, ServerCommand::GetSplitIndex), "-1");
    assert_eq!(response(&mut timer, ServerCommand::GetSplitCount), "3");
    assert_eq!(
        response(&mut timer, ServerCommand::GetSplitNames),
        "One\nTwo\nThree"
    );

    handle_command(&mut timer, &ServerCommand::Start);
    assert_eq!(
        response(&mut timer, ServerCommand::GetCurrentTimerPhase),
        "Running"
    );
    assert_eq!(response(&mut timer, ServerCommand::GetSplitIndex), "1");

    clock.advance(TimeSpan::from_seconds(5));
    assert_eq!(
        response(&mut timer, ServerCommand::GetCurrentTime),
        "00:00:05"
    );
    assert_eq!(
        response(&mut timer, ServerCommand::GetCurrentRealTime),
        "00:00:05"
    );
    assert_eq!(
        response(&mut timer, ServerCommand::GetCurrentSplitName),
        "One"
    );
    assert_eq!(
        response(&mut timer, ServerCommand::GetUpcomingSplitName),
        "Two"
    );

    handle_command(&mut timer, &ServerCommand::Split);
    assert_eq!(response(&mut timer, ServerCommand::GetSplitIndex), "2");
    assert_eq!(
        response(&mut timer, ServerCommand::GetLastSplitTime),
        "00:00:05"
    );
    assert_eq!(
        response(&mut timer, ServerCommand::GetPreviousSplitName),
        "One"
    );
    assert_eq!(
        response(&mut timer, ServerCommand::GetCurrentSplitName),
        "Two"
    );

    clock.advance(TimeSpan::from_seconds(5));
    handle_command(&mut timer, &ServerCommand::Split);
    assert_eq!(
        response(&mut timer, ServerCommand::GetCurrentTimerPhase),
        "Running"
    );
    assert_eq!(response(&mut timer, ServerCommand::GetSplitIndex), "3");

    clock.advance(TimeSpan::from_seconds(5));
    handle_command(&mut timer, &ServerCommand::Split);
    assert_eq!(
        response(&mut timer, ServerCommand::GetCurrentTimerPhase),
        "Ended"
    );
    assert_eq!(response(&mut timer, ServerCommand::GetSplitIndex), "4");
    assert_eq!(
        response(&mut timer, ServerCommand::GetFinalTime(None)),
        "00:00:15"
    );
    assert_eq!(
        response(&mut timer, ServerCommand::GetLastSplitTime),
        "00:00:15"
    );

    handle_command(&mut timer, &ServerCommand::Undo);
    assert_eq!(
        response(&mut timer, ServerCommand::GetCurrentTimerPhase),
        "Running"
    );
    assert_eq!(response(&mut timer, ServerCommand::GetSplitIndex), "3");
    assert_eq!(
        response(&mut timer, ServerCommand::GetLastSplitTime),
        "00:00:10"
    );

    handle_command(&mut timer, &ServerCommand::Reset);
    assert_eq!(
        response(&mut timer, ServerCommand::GetCurrentTimerPhase),
        "NotRunning"
    );
    assert_eq!(response(&mut timer, ServerCommand::GetSplitIndex), "-1");

    handle_command(&mut timer, &ServerCommand::StartOrSplit);
    assert_eq!(
        response(&mut timer, ServerCommand::GetCurrentTimerPhase),
        "Running"
    );
    clock.advance(TimeSpan::from_seconds(1));
    handle_command(&mut timer, &ServerCommand::StartOrSplit);
    assert_eq!(response(&mut timer, ServerCommand::GetSplitIndex), "2");
}

#[test]
fn pause_resume_and_toggle_pause_freeze_time() {
    let (clock, mut timer) = test_timer();
    handle_command(&mut timer, &ServerCommand::Start);
    clock.advance(TimeSpan::from_seconds(5));

    handle_command(&mut timer, &ServerCommand::Pause);
    assert_eq!(
        response(&mut timer, ServerCommand::GetCurrentTimerPhase),
        "Paused"
    );
    clock.advance(TimeSpan::from_seconds(5));
    assert_eq!(
        response(&mut timer, ServerCommand::GetCurrentTime),
        "00:00:05"
    );

    handle_command(&mut timer, &ServerCommand::Resume);
    assert_eq!(
        response(&mut timer, ServerCommand::GetCurrentTimerPhase),
        "Running"
    );
    clock.advance(TimeSpan::from_seconds(1));
    assert_eq!(
        response(&mut timer, ServerCommand::GetCurrentTime),
        "00:00:06"
    );

    handle_command(&mut timer, &ServerCommand::TogglePause);
    assert_eq!(
        response(&mut timer, ServerCommand::GetCurrentTimerPhase),
        "Paused"
    );
    handle_command(&mut timer, &ServerCommand::TogglePause);
    assert_eq!(
        response(&mut timer, ServerCommand::GetCurrentTimerPhase),
        "Running"
    );
}

#[test]
fn game_time_and_loading_time_commands() {
    let (clock, mut timer) = test_timer();
    handle_command(&mut timer, &ServerCommand::Start);
    clock.advance(TimeSpan::from_seconds(10));

    handle_command(
        &mut timer,
        &ServerCommand::SetLoadingTimes(TimeSpan::from_seconds(2)),
    );
    assert_eq!(
        response(&mut timer, ServerCommand::GetCurrentGameTime),
        "00:00:08"
    );

    handle_command(
        &mut timer,
        &ServerCommand::SwitchToGameTime,
    );
    assert_eq!(
        response(&mut timer, ServerCommand::GetCurrentTime),
        "00:00:08"
    );
    assert_eq!(
        response(&mut timer, ServerCommand::GetTimingMethod),
        "GameTime"
    );

    handle_command(&mut timer, &ServerCommand::ToggleTimingMethod);
    assert_eq!(
        response(&mut timer, ServerCommand::GetTimingMethod),
        "RealTime"
    );

    handle_command(
        &mut timer,
        &ServerCommand::SetGameTime(Some(TimeSpan::from_seconds(3))),
    );
    assert_eq!(
        response(&mut timer, ServerCommand::GetCurrentGameTime),
        "00:00:03"
    );

    handle_command(
        &mut timer,
        &ServerCommand::AddLoadingTimes(TimeSpan::from_seconds(1)),
    );
    assert_eq!(
        response(&mut timer, ServerCommand::GetCurrentGameTime),
        "00:00:02"
    );

    handle_command(&mut timer, &ServerCommand::PauseGameTime);
    clock.advance(TimeSpan::from_seconds(5));
    assert_eq!(
        response(&mut timer, ServerCommand::GetCurrentGameTime),
        "00:00:02"
    );
    assert_eq!(
        response(&mut timer, ServerCommand::GetPausedGameTime),
        "00:00:02"
    );

    handle_command(&mut timer, &ServerCommand::ResumeGameTime);
    clock.advance(TimeSpan::from_seconds(1));
    assert_eq!(
        response(&mut timer, ServerCommand::GetCurrentGameTime),
        "00:00:03"
    );
}

#[test]
fn split_names_are_one_based_and_bounds_checked() {
    let (_, mut timer) = test_timer();

    let command = parse_command("setsplitname 2 Renamed").unwrap();
    assert_eq!(
        command,
        ServerCommand::SetSplitName {
            index: 1,
            name: "Renamed".to_string()
        }
    );
    assert_eq!(handle_command(&mut timer, &command).response, None);
    assert_eq!(timer.run.segments[1].name, "Renamed");

    let command = parse_command("setsplitname 99 Out of range").unwrap();
    let outcome = handle_command(&mut timer, &command);
    assert_eq!(outcome.response.as_deref(), Some("-"));
    assert_eq!(timer.run.segments[1].name, "Renamed");

    let command = parse_command("setsplitname -1 Last").unwrap();
    handle_command(&mut timer, &command);
    assert_eq!(timer.run.segments[2].name, "Last");

    assert_eq!(
        response(
            &mut timer,
            ServerCommand::GetSplitName { index: 0 }
        ),
        "One"
    );
    assert_eq!(
        response(
            &mut timer,
            ServerCommand::GetSplitName { index: -2 }
        ),
        "Renamed"
    );
    assert_eq!(
        response(
            &mut timer,
            ServerCommand::GetSplitName { index: 3 }
        ),
        "-"
    );

    // Extreme values must not panic or overflow.
    let command = parse_command("setsplitname -9223372036854775808 Name").unwrap();
    assert_eq!(
        handle_command(&mut timer, &command).response.as_deref(),
        Some("-")
    );
    assert_eq!(
        response(
            &mut timer,
            ServerCommand::GetSplitName {
                index: i64::MAX
            }
        ),
        "-"
    );

    handle_command(&mut timer, &ServerCommand::Start);
    handle_command(
        &mut timer,
        &ServerCommand::SetCurrentSplitName("Live".to_string()),
    );
    assert_eq!(timer.run.segments[0].name, "Live");
    assert_eq!(
        response(&mut timer, ServerCommand::GetCurrentSplitName),
        "Live"
    );
}

#[test]
fn comparisons_delta_and_predicted_times() {
    let (clock, mut timer) = test_timer();

    let comparisons = response(&mut timer, ServerCommand::GetComparisons);
    let comparisons = comparisons.lines().collect::<Vec<_>>();
    assert_eq!(comparisons[0], PERSONAL_BEST);
    assert_eq!(comparisons[1], BEST_SEGMENTS);
    assert_eq!(
        response(
            &mut timer,
            ServerCommand::GetCurrentComparison
        ),
        PERSONAL_BEST
    );

    handle_command(&mut timer, &ServerCommand::Start);
    clock.advance(TimeSpan::from_seconds(10));
    handle_command(&mut timer, &ServerCommand::Split);

    assert_eq!(
        response(
            &mut timer,
            ServerCommand::GetCurrentSplitTime(None)
        ),
        "00:00:20"
    );
    assert_eq!(
        response(
            &mut timer,
            ServerCommand::GetCurrentSplitTime(Some("Personal Best".to_string()))
        ),
        "00:00:20"
    );
    assert_eq!(
        response(&mut timer, ServerCommand::GetDelta(None)),
        "00:00:00"
    );
    assert_eq!(
        response(
            &mut timer,
            ServerCommand::GetPredictedTime(None)
        ),
        "00:00:30"
    );

    clock.advance(TimeSpan::from_seconds(5));
    handle_command(&mut timer, &ServerCommand::Split);
    assert_eq!(
        response(
            &mut timer,
            ServerCommand::GetDelta(None)
        ),
        "-00:00:05"
    );
    assert_eq!(
        response(
            &mut timer,
            ServerCommand::GetFinalTime(None)
        ),
        "00:00:30"
    );

    clock.advance(TimeSpan::from_seconds(5));
    handle_command(&mut timer, &ServerCommand::Split);
    assert_eq!(
        response(
            &mut timer,
            ServerCommand::GetFinalTime(None)
        ),
        "00:00:20"
    );
    assert_eq!(
        response(
            &mut timer,
            ServerCommand::GetFinalTime(Some("Personal Best".to_string()))
        ),
        "00:00:20"
    );

    handle_command(&mut timer, &ServerCommand::Reset);
    assert_eq!(
        response(&mut timer, ServerCommand::GetBestPossibleTime),
        "00:00:19"
    );
    assert_eq!(
        response(
            &mut timer,
            ServerCommand::GetFinalTime(None)
        ),
        "00:00:20"
    );

    handle_command(
        &mut timer,
        &ServerCommand::SwitchComparison("Best Segments".to_string()),
    );
    assert_eq!(timer.current_comparison, "Best Segments");
    handle_command(&mut timer, &ServerCommand::SwitchComparisonNext);
    assert_eq!(timer.current_comparison, "Best Split Times");
    handle_command(&mut timer, &ServerCommand::SwitchComparisonPrevious);
    assert_eq!(timer.current_comparison, "Best Segments");
}

#[test]
fn reset_and_set_attempt_as_pb_updates_personal_best() {
    let (clock, mut timer) = test_timer();
    handle_command(&mut timer, &ServerCommand::Start);
    clock.advance(TimeSpan::from_seconds(5));
    handle_command(&mut timer, &ServerCommand::Split);
    clock.advance(TimeSpan::from_seconds(5));
    handle_command(&mut timer, &ServerCommand::Split);
    clock.advance(TimeSpan::from_seconds(5));
    handle_command(&mut timer, &ServerCommand::Split);
    assert_eq!(
        response(&mut timer, ServerCommand::GetCurrentTimerPhase),
        "Ended"
    );

    handle_command(&mut timer, &ServerCommand::ResetAndSetAttemptAsPb);
    assert_eq!(
        response(&mut timer, ServerCommand::GetCurrentTimerPhase),
        "NotRunning"
    );
    assert_eq!(
        timer.run.segments[2].personal_best().real_time,
        Some(TimeSpan::from_seconds(15))
    );
    assert_eq!(response(&mut timer, ServerCommand::GetAttemptCount), "1");
}

#[test]
fn metadata_custom_variables_and_category_variables() {
    let (_, mut timer) = test_timer();

    handle_command(
        &mut timer,
        &ServerCommand::SetCustomVariable {
            name: "Region".to_string(),
            value: "Europe".to_string(),
        },
    );
    assert_eq!(
        response(
            &mut timer,
            ServerCommand::GetCustomVariableValue("Region".to_string())
        ),
        "Europe"
    );
    assert_eq!(
        response(
            &mut timer,
            ServerCommand::GetCustomVariableValue("Missing".to_string())
        ),
        "-"
    );

    timer
        .run
        .metadata
        .custom_variables
        .insert("Note".to_string(), "line one\nline two".to_string());
    assert_eq!(
        response(
            &mut timer,
            ServerCommand::GetCustomVariableValue("Note".to_string())
        ),
        "line one line two"
    );

    timer.run.metadata.platform_name = Some("PC".to_string());
    timer.run.metadata.region_name = Some("EU".to_string());
    timer
        .run
        .metadata
        .variables
        .insert("Difficulty".to_string(), "Hard".to_string());
    timer.run.metadata.uses_emulator = true;
    let json = response(&mut timer, ServerCommand::GetCategoryVariables);
    assert!(json.contains("\"region\":\"EU\""));
    assert!(json.contains("\"platform\":\"PC\""));
    assert!(json.contains("\"usesEmulator\":true"));
    assert!(json.contains("\"Difficulty\":\"Hard\""));

    assert_eq!(
        response(&mut timer, ServerCommand::GetGameName),
        "Test Game"
    );
    assert_eq!(
        response(&mut timer, ServerCommand::GetCategoryName),
        "Any%"
    );
    assert_eq!(response(&mut timer, ServerCommand::GetOffset), "00:00:00");
}

#[test]
fn misc_commands_produce_stable_responses() {
    let (_, mut timer) = test_timer();

    assert_eq!(response(&mut timer, ServerCommand::Ping), "pong");
    assert_eq!(
        response(&mut timer, ServerCommand::GetServerType),
        "TCP"
    );
    assert!(!response(&mut timer, ServerCommand::GetLiveSplitVersion).is_empty());
    assert_eq!(
        response(&mut timer, ServerCommand::GlobalHotkeysEnabled),
        "False"
    );
    assert_eq!(
        response(&mut timer, ServerCommand::GetHotkeyProfile),
        "Default"
    );
    assert_eq!(
        response(&mut timer, ServerCommand::GetAutoSplitterPath),
        "-"
    );
    assert_eq!(
        response(&mut timer, ServerCommand::AutoSplitterActivated),
        "False"
    );
    assert_eq!(
        response(&mut timer, ServerCommand::GetSplitsScreenshot),
        "-"
    );
    assert_eq!(
        response(
            &mut timer,
            ServerCommand::SaveSplitsScreenshot("/tmp/x.png".to_string())
        ),
        "False"
    );
    assert_eq!(
        response(
            &mut timer,
            ServerCommand::SwitchLayout("/tmp/x.lsl".to_string())
        ),
        "False"
    );
    assert_eq!(
        response(
            &mut timer,
            ServerCommand::SwitchSplits("/definitely/not/a/run.lss".to_string())
        ),
        "False"
    );

    let save_layout = handle_command(&mut timer, &ServerCommand::SaveLayout);
    assert_eq!(save_layout.response.as_deref(), Some("True"));
    assert!(!save_layout.save);

    let save_splits = handle_command(&mut timer, &ServerCommand::SaveSplits);
    assert_eq!(save_splits.response.as_deref(), Some("True"));
    assert!(save_splits.save);

    handle_command(
        &mut timer,
        &ServerCommand::SaveLayoutAs("/tmp/layout.lsl".to_string()),
    );
    assert_eq!(
        response(&mut timer, ServerCommand::GetLayoutPath),
        "/tmp/layout.lsl"
    );

    handle_command(
        &mut timer,
        &ServerCommand::SaveSplitsAs("/tmp/run.lss".to_string()),
    );
    assert_eq!(
        response(&mut timer, ServerCommand::GetSplitsPath),
        "/tmp/run.lss"
    );

    let quit = handle_command(&mut timer, &ServerCommand::Quit);
    assert!(quit.quit);
    assert_eq!(quit.response, None);
}

#[test]
fn timer_state_contains_phase_time_and_split_information() {
    let (clock, mut timer) = test_timer();
    handle_command(&mut timer, &ServerCommand::Start);
    clock.advance(TimeSpan::from_seconds(3));

    let state = response(&mut timer, ServerCommand::GetTimerState);
    assert!(state.contains("phase=Running"));
    assert!(state.contains("current_time=00:00:03"));
    assert!(state.contains("current_real_time=00:00:03"));
    assert!(state.contains("split_index=1"));
    assert!(state.contains("split_count=3"));
    assert!(state.contains("current_comparison=Personal Best"));
    assert!(state.contains("timing_method=RealTime"));
}
