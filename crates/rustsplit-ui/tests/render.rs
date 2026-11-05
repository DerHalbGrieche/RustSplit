//! Headless rendering tests for complete layouts.
//!
//! These drive the real renderer through `egui_kittest` without a GPU and
//! assert on egui's tessellation input: the text that was laid out, the
//! shapes that were painted and where. Pixel snapshots would need a wgpu
//! adapter (and stable font rasterization) on every CI runner; the shape
//! output catches the same regressions this crate has had (missing text,
//! panics, NaN geometry, wrong clipping) and runs anywhere `cargo test` does.
//!
//! The harness panics on characters the bundled fonts cannot draw, so every
//! test also checks that no component shows tofu.

use std::sync::Arc;

use egui::{Color32, Rect, Shape, Vec2};
use egui_kittest::Harness;
use rustsplit_core::{
    load_run_from_str, Run, TestClock, TimeSpan, Timer, TimerPhase, TimingMethod,
};
use rustsplit_ui::components::{default_component_for_path, Component, ADDABLE_COMPONENTS};
use rustsplit_ui::layout::{Layout, LayoutComponent, LayoutMode};
use rustsplit_ui::xml::XmlElement;
use rustsplit_ui::{configure_fonts, Renderer};

const DEMO_RUN: &str = include_str!("../../../examples/demo.lss");

/// The result of rendering a scene.
struct Frame {
    texts: Vec<String>,
    shapes: Vec<(Rect, Shape)>,
    component_rects: Vec<Rect>,
}

impl Frame {
    fn contains(&self, needle: &str) -> bool {
        self.texts.iter().any(|text| text.contains(needle))
    }

    fn assert_contains(&self, needle: &str) {
        assert!(
            self.contains(needle),
            "expected {needle:?} among the rendered text: {:?}",
            self.texts
        );
    }

    fn has_rect_filled_with(&self, color: Color32) -> bool {
        self.shapes
            .iter()
            .any(|(_, shape)| matches!(shape, Shape::Rect(rect) if rect.fill == color))
    }
}

fn render(layout: &Layout, timer: &Timer, size: Vec2) -> Frame {
    let mut renderer = Renderer::default();
    let mut shapes = Vec::new();
    {
        let mut harness = Harness::builder()
            .with_size(size)
            .with_context_setup(configure_fonts)
            .build_ui(|ui| {
                // The harness adds a margin around its ui; the app draws the
                // layout edge to edge, so do the same.
                let window = ui.ctx().content_rect();
                let mut ui = ui.new_child(egui::UiBuilder::new().max_rect(window));
                ui.set_clip_rect(window);
                renderer.draw(&mut ui, layout, timer);
            });
        // The first frame loads fonts and textures; the second is a steady
        // state.
        harness.step();
        harness.step();
        for clipped in &harness.output().shapes {
            flatten(clipped.clip_rect, &clipped.shape, &mut shapes);
        }
    }
    let texts = shapes
        .iter()
        .filter_map(|(_, shape)| match shape {
            Shape::Text(text) => Some(text.galley.text().to_string()),
            _ => None,
        })
        .collect();
    for (clip_rect, shape) in &shapes {
        assert!(!clip_rect.any_nan(), "NaN clip rect for {shape:?}");
        assert!(
            !shape.visual_bounding_rect().any_nan(),
            "NaN geometry in {shape:?}"
        );
    }
    let component_rects = renderer.last_component_rects;
    for rect in &component_rects {
        assert!(!rect.any_nan(), "NaN component rect");
    }
    Frame {
        texts,
        shapes,
        component_rects,
    }
}

fn flatten(clip_rect: Rect, shape: &Shape, out: &mut Vec<(Rect, Shape)>) {
    match shape {
        Shape::Vec(shapes) => {
            for shape in shapes {
                flatten(clip_rect, shape, out);
            }
        }
        shape => out.push((clip_rect, shape.clone())),
    }
}

fn default_layout() -> Layout {
    Layout::parse_default().expect("embedded default layout must parse")
}

fn demo_timer() -> (Timer, TestClock) {
    let run = load_run_from_str(DEMO_RUN).expect("demo run must parse");
    let clock = TestClock::new();
    (Timer::new(run, Arc::new(clock.clone())), clock)
}

/// A layout with one of every component, using their default settings.
fn every_component_layout(mode: LayoutMode) -> Layout {
    let mut layout = default_layout();
    layout.mode = mode;
    layout.components = ADDABLE_COMPONENTS
        .iter()
        .map(|(path, _)| LayoutComponent::new(default_component_for_path(path)))
        .collect();
    layout
        .components
        .push(LayoutComponent::new(Component::Unknown {
            path: "LiveSplit.SomethingElse.dll".to_string(),
            settings: XmlElement::new("Settings"),
        }));
    layout
}

/// Timers that are missing every kind of data a component may look up.
fn timers_with_missing_data() -> Vec<(&'static str, Timer)> {
    let clock = TestClock::new();
    let timer = |run: Run| Timer::new(run, Arc::new(clock.clone()));

    let empty = timer(Run::with_segments("", "", Vec::<String>::new()));
    let mut empty_started = timer(Run::with_segments("", "", Vec::<String>::new()));
    empty_started.start();

    let no_times = timer(Run::with_segments("", "", ["One", "Two", "Three"]));
    let mut running = timer(Run::with_segments("", "", ["One", "Two", "Three"]));
    running.start();
    clock.advance(TimeSpan::from_seconds(5));
    let mut skipped = timer(Run::with_segments("", "", ["One", "Two", "Three"]));
    skipped.start();
    skipped.skip_split();
    let mut finished = timer(Run::with_segments("", "", ["One"]));
    finished.start();
    clock.advance(TimeSpan::from_seconds(1));
    finished.split();

    let mut game_time = timer(Run::with_segments("Game", "Category", ["One", "Two"]));
    game_time.set_timing_method(TimingMethod::GameTime);
    game_time.start();

    let (mut missing_comparison, _) = demo_timer();
    missing_comparison.set_current_comparison("No Such Comparison");
    missing_comparison.start();

    vec![
        ("empty run", empty),
        ("empty run started", empty_started),
        ("no times", no_times),
        ("running without history", running),
        ("skipped split", skipped),
        ("finished without history", finished),
        ("game time without game time", game_time),
        ("missing comparison", missing_comparison),
    ]
}

#[test]
fn default_layout_renders_a_fresh_run() {
    let layout = default_layout();
    let (timer, _) = demo_timer();
    let frame = render(&layout, &timer, Vec2::new(286.0, 519.0));

    frame.assert_contains("RustSplit Demo");
    frame.assert_contains("Any%");
    for segment in ["Intro", "Forest", "Ruins", "Boss", "Escape"] {
        frame.assert_contains(segment);
    }
    assert!(frame.has_rect_filled_with(layout.settings.background_color));
    assert_eq!(frame.component_rects.len(), layout.components.len());
}

#[test]
fn default_layout_renders_a_run_in_progress() {
    let layout = default_layout();
    let (mut timer, clock) = demo_timer();
    let fresh = render(&layout, &timer, Vec2::new(286.0, 519.0));
    timer.start();
    clock.advance(TimeSpan::from_seconds(70));
    timer.split();
    clock.advance(TimeSpan::from_seconds(65));
    assert_eq!(timer.phase, TimerPhase::Running);

    let frame = render(&layout, &timer, Vec2::new(286.0, 519.0));
    // The main timer shows the 2:15 that has elapsed.
    frame.assert_contains("2:15");
    // Splitting produced a delta against the personal best.
    assert!(
        frame.texts.iter().any(|text| text.starts_with('+')
            || text.starts_with('-')
            || text.starts_with('\u{2212}')),
        "expected a delta among {:?}",
        frame.texts
    );
    assert_ne!(fresh.texts, frame.texts);
}

#[test]
fn default_layout_renders_a_finished_run() {
    let layout = default_layout();
    let (mut timer, clock) = demo_timer();
    timer.start();
    let segments = timer.run.segments.len();
    for _ in 0..segments {
        clock.advance(TimeSpan::from_seconds(60));
        timer.split();
    }
    assert_eq!(timer.phase, TimerPhase::Ended);

    let frame = render(&layout, &timer, Vec2::new(286.0, 519.0));
    frame.assert_contains("5:00");
    frame.assert_contains("Escape");
}

#[test]
fn transparent_mode_draws_text_without_a_background() {
    let mut layout = default_layout();
    let (timer, _) = demo_timer();
    let opaque = render(&layout, &timer, Vec2::new(286.0, 519.0));
    layout.settings.transparent_background = true;
    let frame = render(&layout, &timer, Vec2::new(286.0, 519.0));

    assert!(!frame.has_rect_filled_with(layout.settings.background_color));
    frame.assert_contains("RustSplit Demo");
    // Every label keeps its text; outlines and shadows only add shapes.
    for text in &opaque.texts {
        frame.assert_contains(text);
    }
    assert!(frame.texts.len() >= opaque.texts.len());
}

#[test]
fn empty_run_renders_without_segments() {
    let layout = default_layout();
    for (name, timer) in timers_with_missing_data()
        .into_iter()
        .filter(|(name, _)| name.starts_with("empty"))
    {
        let frame = render(&layout, &timer, Vec2::new(286.0, 519.0));
        assert_eq!(
            frame.component_rects.len(),
            layout.components.len(),
            "{name}"
        );
        assert!(!frame.texts.is_empty(), "{name}: nothing was drawn");
    }
}

#[test]
fn extreme_window_sizes_do_not_break_rendering() {
    let (mut timer, clock) = demo_timer();
    timer.start();
    clock.advance(TimeSpan::from_seconds(42));
    timer.split();
    for mode in [LayoutMode::Vertical, LayoutMode::Horizontal] {
        let mut layout = default_layout();
        layout.mode = mode;
        for size in [
            Vec2::new(1.0, 1.0),
            Vec2::new(2.0, 2.0),
            Vec2::new(12.0, 7.0),
            Vec2::new(40.0, 4000.0),
            Vec2::new(4000.0, 40.0),
            Vec2::new(7680.0, 4320.0),
        ] {
            let frame = render(&layout, &timer, size);
            let screen = Rect::from_min_size(egui::Pos2::ZERO, size);
            for (clip_rect, _) in &frame.shapes {
                assert!(
                    screen.expand(0.5).contains_rect(*clip_rect),
                    "{mode:?} {size:?}: clip {clip_rect:?} leaves the window"
                );
            }
        }
    }
}

#[test]
fn huge_window_scales_the_layout_up() {
    let layout = default_layout();
    let (timer, _) = demo_timer();
    let normal = render(&layout, &timer, Vec2::new(286.0, 519.0));
    let huge = render(&layout, &timer, Vec2::new(2860.0, 5190.0));
    assert_eq!(normal.component_rects.len(), huge.component_rects.len());
    for (normal, huge) in normal.component_rects.iter().zip(&huge.component_rects) {
        if normal.height() > 0.0 {
            assert!((huge.height() / normal.height() - 10.0).abs() < 0.01);
        }
    }
    assert_eq!(normal.texts, huge.texts);
}

#[test]
fn horizontal_layout_places_components_side_by_side() {
    let mut layout = default_layout();
    layout.mode = LayoutMode::Horizontal;
    let (mut timer, clock) = demo_timer();
    timer.start();
    clock.advance(TimeSpan::from_seconds(30));
    let frame = render(&layout, &timer, Vec2::new(1200.0, 120.0));

    frame.assert_contains("RustSplit Demo");
    frame.assert_contains("Intro");
    let drawn: Vec<_> = frame
        .component_rects
        .iter()
        .filter(|rect| rect.is_positive())
        .collect();
    assert!(drawn.len() > 1);
    for pair in drawn.windows(2) {
        assert!(pair[0].max.x <= pair[1].min.x + 0.01, "{pair:?}");
        assert!((pair[0].height() - 120.0).abs() < 0.01, "{pair:?}");
    }
}

#[test]
fn every_component_renders_with_missing_data() {
    for mode in [LayoutMode::Vertical, LayoutMode::Horizontal] {
        let layout = every_component_layout(mode);
        // Make sure the list really covers every component type.
        assert_eq!(layout.components.len(), ADDABLE_COMPONENTS.len() + 1);
        for (name, timer) in timers_with_missing_data() {
            for size in [Vec2::new(300.0, 900.0), Vec2::new(30.0, 30.0)] {
                let frame = render(&layout, &timer, size);
                assert_eq!(
                    frame.component_rects.len(),
                    layout.components.len(),
                    "{mode:?} {name} {size:?}"
                );
                for (component, rect) in layout.components.iter().zip(&frame.component_rects) {
                    let unknown = matches!(component.component, Component::Unknown { .. });
                    assert_eq!(
                        rect.is_positive(),
                        !unknown,
                        "{mode:?} {name}: {} was not laid out",
                        component.name()
                    );
                }
            }
        }
    }
}

#[test]
fn every_component_renders_with_a_full_run() {
    let layout = every_component_layout(LayoutMode::Vertical);
    let (mut timer, clock) = demo_timer();
    let mut frames = vec![render(&layout, &timer, Vec2::new(300.0, 1600.0))];
    timer.start();
    clock.advance(TimeSpan::from_seconds(75));
    timer.split();
    clock.advance(TimeSpan::from_seconds(10));
    timer.pause();
    frames.push(render(&layout, &timer, Vec2::new(300.0, 1600.0)));
    timer.pause();
    while timer.phase != TimerPhase::Ended {
        clock.advance(TimeSpan::from_seconds(50));
        timer.split();
    }
    frames.push(render(&layout, &timer, Vec2::new(300.0, 1600.0)));
    for frame in frames {
        frame.assert_contains("RustSplit Demo");
    }
}
