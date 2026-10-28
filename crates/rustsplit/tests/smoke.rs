//! Smoketests for the packaged example files and layout parser.

use std::path::Path;

use livesplit_core::{load_run, PERSONAL_BEST};
use livesplit_ui::layout::DEFAULT_LAYOUT_LSL;
use livesplit_ui::parse_layout;

#[test]
fn demo_run_loads_and_has_expected_data() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/demo.lss");
    let run = load_run(&path).expect("demo.lss should parse");
    assert_eq!(run.game_name, "RustSplit Demo");
    assert_eq!(run.category_name, "Any%");
    assert_eq!(run.segments.len(), 5);
    assert_eq!(run.attempts.len(), 3);
    assert!(run
        .custom_comparisons
        .iter()
        .any(|comparison| comparison == PERSONAL_BEST));
    assert!(run
        .custom_comparisons
        .iter()
        .any(|comparison| comparison == "Glitchless"));
    assert!(run.segments[0].personal_best().real_time.is_some());
    assert!(run.segments[0].best_segment_time.real_time.is_some());
    assert_eq!(run.segments[0].segment_history.len(), 3);
}

#[test]
fn bundled_layout_parses() {
    let layout = parse_layout(DEFAULT_LAYOUT_LSL.as_bytes()).expect("default layout parses");
    assert_eq!(layout.components.len(), 4);
    assert_eq!(layout.components[0].component.name(), "Title");
    assert_eq!(layout.components[1].component.name(), "Splits");
    assert_eq!(layout.components[2].component.name(), "Timer");
    assert_eq!(layout.components[3].component.name(), "Previous Segment");
    assert_eq!(layout.vertical_width, 286);
    assert!(layout.settings.always_on_top);
}
