//! Integration tests for reading and writing LiveSplit `.lss` files.

use std::path::PathBuf;

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use chrono::{TimeZone, Utc};
use livesplit_core::xml::{child, children_named, parse_xml, text_trimmed};
use livesplit_core::{
    load_run, load_run_from_str, save_run_to_string, Attempt, LssError, Run, Time, TimeSpan,
    PERSONAL_BEST,
};
use xmltree::XMLNode;

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

fn ts(nanos: i64) -> TimeSpan {
    TimeSpan::from_nanos(nanos)
}

fn time(real: Option<i64>, game: Option<i64>) -> Time {
    Time::new(real.map(ts), game.map(ts))
}

fn fixture_png() -> Vec<u8> {
    STANDARD
        .decode("iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAYAAABytg0kAAAAFElEQVR42mP4z8DwHwyBNBAw/AcAR8oI+FuapL4AAAAASUVORK5CYII=")
        .unwrap()
}

fn datetime(year: i32, month: u32, day: u32, hour: u32, minute: u32, second: u32) -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(year, month, day, hour, minute, second)
        .single()
        .unwrap()
}

/// A whitespace-insensitive XML signature used for comparing raw inner XML.
fn xml_signature(fragment: &str) -> String {
    fn walk(element: &livesplit_core::xml::XmlElement, output: &mut String) {
        output.push('<');
        output.push_str(&element.name);
        let mut attributes = element.attributes.iter().collect::<Vec<_>>();
        attributes.sort_by(|left, right| left.0.cmp(right.0));
        for (name, value) in attributes {
            output.push_str(&format!(" {name}={value:?}"));
        }
        output.push('>');
        for node in &element.children {
            match node {
                XMLNode::Element(child) => walk(child, output),
                XMLNode::Text(value) | XMLNode::CData(value) => {
                    let value = value.trim();
                    if !value.is_empty() {
                        output.push_str(value);
                    }
                }
                _ => {}
            }
        }
        output.push_str("</");
        output.push_str(&element.name);
        output.push('>');
    }

    let wrapped = format!("<root>{fragment}</root>");
    let root = parse_xml(wrapped.as_bytes()).unwrap();
    let mut output = String::new();
    for node in &root.children {
        match node {
            XMLNode::Element(element) => walk(element, &mut output),
            XMLNode::Text(value) | XMLNode::CData(value) => {
                let value = value.trim();
                if !value.is_empty() {
                    output.push_str(value);
                }
            }
            _ => {}
        }
    }
    output
}

fn assert_attempts_equal(expected: &[Attempt], actual: &[Attempt]) {
    assert_eq!(expected.len(), actual.len());
    for (expected, actual) in expected.iter().zip(actual) {
        assert_eq!(expected.index, actual.index);
        assert_eq!(expected.time, actual.time);
        assert_eq!(expected.started, actual.started);
        assert_eq!(expected.ended, actual.ended);
        assert_eq!(expected.pause_time, actual.pause_time);
    }
}

fn assert_runs_semantically_equal(expected: &Run, actual: &Run) {
    assert_eq!(expected.game_name, actual.game_name);
    assert_eq!(expected.category_name, actual.category_name);
    assert_eq!(expected.game_icon, actual.game_icon);
    assert_eq!(expected.layout_path, actual.layout_path);
    assert_eq!(expected.offset, actual.offset);
    assert_eq!(expected.attempt_count, actual.attempt_count);
    assert_eq!(expected.custom_comparisons, actual.custom_comparisons);
    assert_eq!(expected.metadata.run_id, actual.metadata.run_id);
    assert_eq!(expected.metadata.platform_name, actual.metadata.platform_name);
    assert_eq!(expected.metadata.uses_emulator, actual.metadata.uses_emulator);
    assert_eq!(expected.metadata.region_name, actual.metadata.region_name);
    assert_eq!(expected.metadata.variables, actual.metadata.variables);
    assert_eq!(
        expected.metadata.custom_variables,
        actual.metadata.custom_variables
    );
    assert_attempts_equal(&expected.attempts, &actual.attempts);

    assert_eq!(expected.segments.len(), actual.segments.len());
    for (expected_segment, actual_segment) in expected.segments.iter().zip(&actual.segments) {
        assert_eq!(expected_segment.name, actual_segment.name);
        assert_eq!(expected_segment.icon, actual_segment.icon);
        assert_eq!(
            expected_segment.best_segment_time,
            actual_segment.best_segment_time
        );
        assert_eq!(
            expected_segment.segment_history,
            actual_segment.segment_history
        );
        for comparison in &expected.custom_comparisons {
            assert_eq!(
                expected_segment.comparison(comparison),
                actual_segment.comparison(comparison),
                "comparison {comparison:?} differs for segment {:?}",
                expected_segment.name
            );
        }
    }

    match (
        &expected.auto_splitter_settings,
        &actual.auto_splitter_settings,
    ) {
        (Some(expected), Some(actual)) => assert_eq!(
            xml_signature(expected),
            xml_signature(actual),
            "auto splitter settings differ"
        ),
        (None, None) => {}
        (expected, actual) => panic!(
            "auto splitter settings presence differs: {expected:?} vs {actual:?}"
        ),
    }
}

#[test]
fn parses_and_roundtrips_a_realistic_livesplit_file() {
    let path = fixture_path("realistic.lss");
    let run = load_run(&path).unwrap();
    assert_eq!(run.file_path.as_deref(), Some(path.as_path()));

    assert_eq!(run.game_name, "RustSplit Test Game");
    assert_eq!(run.category_name, "Any%");
    assert_eq!(run.layout_path.as_deref(), Some("Layouts/Any%.lsl"));
    assert_eq!(run.game_icon.as_deref(), Some(fixture_png().as_slice()));
    assert_eq!(run.offset, ts(-1_234_567_800));
    assert_eq!(run.attempt_count, 3);

    // Metadata, including empty elements and escaped values.
    assert_eq!(run.metadata.run_id.as_deref(), Some("9d3x8k2m"));
    assert_eq!(run.metadata.platform_name.as_deref(), Some("PC"));
    assert!(run.metadata.uses_emulator);
    assert_eq!(run.metadata.region_name.as_deref(), Some("Europe"));
    assert_eq!(
        run.metadata.variables.get("Game Version").map(String::as_str),
        Some("1.2.1.5")
    );
    assert_eq!(
        run.metadata
            .variables
            .get("Category Type")
            .map(String::as_str),
        Some("Per Game")
    );
    assert_eq!(
        run.metadata
            .custom_variables
            .get("Notes")
            .map(String::as_str),
        Some("A & B")
    );

    // Attempt timestamps use LiveSplit's invariant culture format and UTC.
    assert_eq!(run.attempts.len(), 3);
    assert_eq!(run.attempts[0].index, 1);
    assert_eq!(
        run.attempts[0].time,
        time(Some(3_871_170_000_000), Some(3_642_573_000_000))
    );
    assert_eq!(
        run.attempts[0].started,
        Some(datetime(2018, 2, 4, 19, 7, 3))
    );
    assert_eq!(
        run.attempts[0].ended,
        Some(datetime(2018, 2, 4, 20, 11, 33))
    );
    assert_eq!(run.attempts[0].pause_time, Some(ts(12_345_678_900)));

    assert_eq!(run.attempts[1].index, 2);
    assert_eq!(
        run.attempts[1].started,
        Some(datetime(2018, 2, 5, 16, 29, 26))
    );
    assert_eq!(run.attempts[1].ended, None);
    assert_eq!(run.attempts[1].time, Time::default());

    assert_eq!(run.attempts[2].index, 3);
    assert_eq!(run.attempts[2].started, None);
    assert_eq!(run.attempts[2].ended, None);
    assert_eq!(run.attempts[2].time, Time::default());

    // Generated comparisons are read as split times but are not stored as
    // custom comparisons.
    assert_eq!(
        run.custom_comparisons,
        vec![
            PERSONAL_BEST.to_string(),
            "Any% No SLA".to_string(),
            "Glitchless".to_string(),
        ]
    );
    assert_eq!(run.segments.len(), 3);
    assert_eq!(run.segments[0].name, "Start");
    assert_eq!(run.segments[1].name, "Level 1");
    assert_eq!(run.segments[2].name, "Level 2");
    assert_eq!(run.segments[0].icon.as_deref(), Some(fixture_png().as_slice()));
    assert_eq!(run.segments[1].icon, None);
    assert_eq!(run.segments[2].icon.as_deref(), Some(fixture_png().as_slice()));

    assert_eq!(
        run.segments[0].personal_best(),
        time(Some(10_123_456_700), Some(9_765_432_100))
    );
    assert_eq!(
        run.segments[0].comparison("Any% No SLA"),
        time(Some(11_000_000_100), Some(10_500_000_000))
    );
    assert_eq!(
        run.segments[0].comparison("Glitchless"),
        Time::default()
    );
    assert_eq!(
        run.segments[0].comparison("Best Segments"),
        time(Some(9_999_999_900), None)
    );
    assert_eq!(
        run.segments[0].best_segment_time,
        time(Some(9_876_543_200), Some(9_111_111_100))
    );
    assert_eq!(
        run.segments[0].segment_history.get(&1),
        Some(&time(Some(10_500_000_000), Some(10_000_000_000)))
    );
    assert_eq!(
        run.segments[0].segment_history.get(&3),
        Some(&time(Some(11_250_000_000), None))
    );
    assert_eq!(
        run.segments[1].personal_best(),
        time(Some(60_000_000_100), Some(58_000_000_100))
    );
    assert_eq!(run.segments[1].best_segment_time, Time::default());
    assert!(run.segments[1].segment_history.is_empty());
    assert_eq!(
        run.segments[2].personal_best(),
        time(Some(120_000_000_000), None)
    );
    assert_eq!(
        run.segments[2].best_segment_time,
        time(Some(30_000_000_100), None)
    );
    assert_eq!(
        run.segments[2].segment_history.get(&7),
        Some(&time(Some(31_000_000_000), None))
    );

    let saved = save_run_to_string(&run).unwrap();
    assert!(saved.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n"));
    assert!(saved.contains("<Run version=\"1.7.0\">"));
    assert!(saved.contains("<GameIcon>"));
    assert!(saved.contains("<LayoutPath>Layouts/Any%.lsl</LayoutPath>"));
    assert!(saved.contains("name=\"Personal Best\""));
    assert!(!saved.contains("name=\"Best Segments\""));
    assert!(!saved.contains("name=\"Latest Run\""));
    assert!(!saved.contains("name=\"None\""));
    assert!(saved.contains("<PauseTime>00:00:12.3456789</PauseTime>"));

    // Icons are written using the exact BinaryFormatter wrapper LiveSplit's
    // C# saver uses, so both LiveSplit and RustSplit can read them.
    let root = parse_xml(saved.as_bytes()).unwrap();
    let icon_text = text_trimmed(child(&root, "GameIcon").unwrap());
    let blob = STANDARD.decode(icon_text).unwrap();
    assert_eq!(&blob[161..blob.len() - 1], fixture_png().as_slice());
    assert_eq!(
        u32::from_le_bytes(blob[156..160].try_into().unwrap()) as usize,
        fixture_png().len()
    );

    // LiveSplit expects <RealTime>/<GameTime> directly inside <Attempt>.
    let attempt_history = child(&root, "AttemptHistory").unwrap();
    let first_attempt = children_named(attempt_history, "Attempt").next().unwrap();
    assert!(child(first_attempt, "RealTime").is_some());
    assert!(child(first_attempt, "GameTime").is_some());
    assert!(child(first_attempt, "Time").is_none());
    assert_eq!(
        text_trimmed(child(first_attempt, "RealTime").unwrap()),
        "01:04:31.1700000"
    );

    let reparsed = load_run_from_str(&saved).unwrap();
    assert_runs_semantically_equal(&run, &reparsed);
}

#[test]
fn parses_a_minimal_file_and_writes_all_required_elements() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<Run>
  <GameName>Minimal Game</GameName>
  <CategoryName />
  <Offset></Offset>
  <AttemptCount></AttemptCount>
  <Segments>
    <Segment>
      <Name>Only Segment</Name>
      <SplitTimes />
      <BestSegmentTime />
      <SegmentHistory />
    </Segment>
  </Segments>
</Run>"#;

    let run = load_run_from_str(xml).unwrap();
    assert_eq!(run.game_name, "Minimal Game");

    // A BOM in front of the XML declaration must be tolerated as well.
    let with_bom = format!("\u{feff}{xml}");
    let run_with_bom = load_run_from_str(&with_bom).unwrap();
    assert_eq!(run_with_bom.game_name, "Minimal Game");
    assert_eq!(run_with_bom.segments[0].name, "Only Segment");
    assert_eq!(run.category_name, "");
    assert_eq!(run.game_icon, None);
    assert_eq!(run.layout_path, None);
    assert_eq!(run.offset, TimeSpan::ZERO);
    assert_eq!(run.attempt_count, 0);
    assert!(run.attempts.is_empty());
    assert_eq!(run.custom_comparisons, vec![PERSONAL_BEST.to_string()]);
    assert_eq!(run.segments.len(), 1);
    assert_eq!(run.segments[0].name, "Only Segment");
    assert_eq!(run.segments[0].personal_best(), Time::default());
    assert_eq!(run.segments[0].best_segment_time, Time::default());
    assert!(run.segments[0].segment_history.is_empty());
    assert_eq!(run.auto_splitter_settings, None);
    assert_eq!(run.metadata.run_id, None);

    let saved = save_run_to_string(&run).unwrap();
    for element in [
        "GameIcon",
        "GameName",
        "CategoryName",
        "LayoutPath",
        "Metadata",
        "Offset",
        "AttemptCount",
        "AttemptHistory",
        "Segments",
        "AutoSplitterSettings",
    ] {
        assert!(
            saved.contains(&format!("<{element}")),
            "saved file is missing <{element}>:\n{saved}"
        );
    }
    assert!(saved.contains("<Run version=\"1.7.0\">"));

    let reparsed = load_run_from_str(&saved).unwrap();
    assert_runs_semantically_equal(&run, &reparsed);
}

#[test]
fn malformed_input_is_reported_or_ignored_gracefully() {
    assert!(matches!(
        load_run_from_str("<Run><Segments>"),
        Err(LssError::Xml(_))
    ));
    assert!(matches!(
        load_run_from_str("<Run version=\"1.7.0\"><GameName>No Segments</GameName></Run>"),
        Err(LssError::NoSegments)
    ));
    assert!(matches!(
        load_run_from_str("<Run version=\"1.7.0\"><Segments /></Run>"),
        Err(LssError::NoSegments)
    ));
    assert!(matches!(
        load_run_from_str(
            "<Timer><Segments><Segment><Name>Wrong Root</Name></Segment></Segments></Timer>"
        ),
        Err(LssError::Invalid(_))
    ));

    // Broken optional values are ignored instead of failing the whole run.
    let xml = r#"<Run version="1.7.0">
      <GameIcon>!!! definitely not base64 !!!</GameIcon>
      <Offset>not a time</Offset>
      <AttemptCount>three</AttemptCount>
      <Segments>
        <Segment>
          <Name>Still Loads</Name>
          <SplitTimes>
            <SplitTime name="Personal Best">
              <RealTime>also not a time</RealTime>
            </SplitTime>
          </SplitTimes>
          <BestSegmentTime />
          <SegmentHistory>
            <Time id="not-a-number"><RealTime>00:00:01.0000000</RealTime></Time>
          </SegmentHistory>
        </Segment>
      </Segments>
    </Run>"#;
    let run = load_run_from_str(xml).unwrap();
    assert_eq!(run.game_icon, None);
    assert_eq!(run.offset, TimeSpan::ZERO);
    assert_eq!(run.attempt_count, 0);
    assert_eq!(run.segments[0].personal_best(), Time::default());
    assert!(run.segments[0].segment_history.is_empty());
}

#[test]
fn accepts_legacy_attempt_and_time_elements() {
    let xml = r#"<Run version="1.4.1">
      <RunHistory>
        <Time id="1">00:00:16.8767376</Time>
        <Time id="2" />
      </RunHistory>
      <Segments>
        <Segment>
          <Name>Old Layout</Name>
          <PersonalBestSplitTime>00:01:39.3900000</PersonalBestSplitTime>
          <SplitTimes>
            <SplitTime name="Personal Best">00:01:39.3900000</SplitTime>
          </SplitTimes>
          <BestSegmentTime />
          <SegmentHistory />
        </Segment>
      </Segments>
    </Run>"#;

    let run = load_run_from_str(xml).unwrap();
    assert_eq!(run.attempts.len(), 2);
    assert_eq!(run.attempts[0].index, 1);
    assert_eq!(run.attempts[0].time, time(Some(16_876_737_600), None));
    assert_eq!(run.attempts[1].index, 2);
    assert_eq!(run.attempts[1].time, Time::default());
    assert_eq!(
        run.segments[0].personal_best(),
        time(Some(99_390_000_000), None)
    );

    // The old, nested attempt time and the <= 1.7 ended-before-started bug
    // are handled as well.
    let xml = r#"<Run version="1.7.0">
      <AttemptHistory>
        <Attempt id="5" started="03/01/2018 10:00:00" ended="03/01/2018 09:59:59">
          <Time><RealTime>00:00:05.0000000</RealTime></Time>
        </Attempt>
        <Attempt id="6" started="2/4/2018 19:07:03" />
      </AttemptHistory>
      <Segments><Segment><Name>Start</Name></Segment></Segments>
    </Run>"#;
    let run = load_run_from_str(xml).unwrap();
    assert_eq!(run.attempts[0].index, 5);
    assert_eq!(run.attempts[0].time, time(Some(5_000_000_000), None));
    assert_eq!(run.attempts[0].started, Some(datetime(2018, 3, 1, 10, 0, 0)));
    assert_eq!(run.attempts[0].ended, None);
    assert_eq!(
        run.attempts[1].started,
        Some(datetime(2018, 2, 4, 19, 7, 3))
    );
}

#[test]
fn only_custom_comparisons_are_saved_and_personal_best_is_forced() {
    let mut run = Run::with_segments("Game", "Category", ["First", "Second"]);
    run.custom_comparisons = vec![
        "Custom".to_string(),
        "Best Segments".to_string(),
        "Latest Run".to_string(),
        "None".to_string(),
        "Balanced PB".to_string(),
        "Custom".to_string(),
    ];
    for (index, segment) in run.segments.iter_mut().enumerate() {
        let base = (index as i64 + 1) * 1_000_000_000;
        segment.set_personal_best(Time::new(Some(ts(base)), None));
        segment.set_comparison("Custom", Time::new(Some(ts(base + 500_000_000)), None));
        segment.set_comparison("Best Segments", Time::new(Some(ts(base - 100_000_000)), None));
        segment.best_segment_time = Time::new(Some(ts(400_000_000)), None);
    }

    let saved = save_run_to_string(&run).unwrap();
    assert!(saved.contains("<GameIcon />") || saved.contains("<GameIcon></GameIcon>"));
    assert!(saved.contains("<LayoutPath />") || saved.contains("<LayoutPath></LayoutPath>"));

    let root = parse_xml(saved.as_bytes()).unwrap();
    let segments = child(&root, "Segments").unwrap();
    for segment in children_named(segments, "Segment") {
        let split_times = child(segment, "SplitTimes").unwrap();
        let names = children_named(split_times, "SplitTime")
            .map(|split_time| {
                split_time
                    .attributes
                    .get("name")
                    .expect("split time without a name")
                    .as_str()
            })
            .collect::<Vec<_>>();
        assert_eq!(names, vec![PERSONAL_BEST, "Custom"]);
    }

    let reparsed = load_run_from_str(&saved).unwrap();
    assert_eq!(
        reparsed.custom_comparisons,
        vec![PERSONAL_BEST.to_string(), "Custom".to_string()]
    );
    assert_eq!(
        reparsed.segments[1].personal_best(),
        Time::new(Some(ts(2_000_000_000)), None)
    );
    assert_eq!(
        reparsed.segments[1].comparison("Custom"),
        Time::new(Some(ts(2_500_000_000)), None)
    );
}

#[test]
fn auto_splitter_settings_roundtrip_preserves_text_and_elements() {
    let xml = r#"<Run version="1.7.0">
      <Segments><Segment><Name>Start</Name></Segment></Segments>
      <AutoSplitterSettings>Some &amp; text<Version>1.7</Version><Splits><Split>Start</Split></Splits></AutoSplitterSettings>
    </Run>"#;
    let run = load_run_from_str(xml).unwrap();
    let settings = run.auto_splitter_settings.as_deref().unwrap();
    assert!(xml_signature(settings).contains("Some & text"));

    let saved = save_run_to_string(&run).unwrap();
    assert!(saved.contains("Some &amp; text"));
    let reparsed = load_run_from_str(&saved).unwrap();
    assert_runs_semantically_equal(&run, &reparsed);
}

#[test]
fn negative_offsets_and_nanosecond_times_roundtrip_exactly() {
    let mut run = Run::with_segments("Game", "Category", ["Only"]);
    run.offset = TimeSpan::parse("-1.02:03:04.5678901").unwrap();
    assert_eq!(run.offset.to_precise_string(), "-1.02:03:04.5678901");
    run.segments[0].set_personal_best(Time::new(
        Some(TimeSpan::from_nanos(12_345_678_900)),
        Some(TimeSpan::from_nanos(-1_234_567_800)),
    ));
    run.segments[0].best_segment_time =
        Time::new(Some(TimeSpan::from_nanos(987_654_300)), None);
    run.segments[0].segment_history.insert(
        1,
        Time::new(Some(TimeSpan::from_nanos(1_000_000_100)), None),
    );

    let saved = save_run_to_string(&run).unwrap();
    assert!(saved.contains("-1.02:03:04.5678901"));
    assert!(saved.contains("00:00:12.3456789"));
    assert!(saved.contains("-00:00:01.2345678"));
    assert!(saved.contains("00:00:00.9876543"));
    assert!(saved.contains("00:00:01.0000001"));

    let reparsed = load_run_from_str(&saved).unwrap();
    assert_eq!(reparsed.offset, run.offset);
    assert_eq!(reparsed.offset.as_nanos(), -93_784_567_890_100);
    assert_eq!(reparsed.segments[0].personal_best(), run.segments[0].personal_best());
    assert_eq!(
        reparsed.segments[0].best_segment_time,
        run.segments[0].best_segment_time
    );
    assert_eq!(
        reparsed.segments[0].segment_history,
        run.segments[0].segment_history
    );
}
