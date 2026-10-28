//! Built-in layout components.

pub mod basic;
pub mod settings;
pub mod splits;
pub mod timing;

use std::sync::Arc;

use egui::{FontFamily, FontId, Rect, Ui};

use livesplit_core::{Timer, TimingMethod};

pub use settings::*;

use crate::icon::IconCache;
use crate::layout::{FontSpec, LayoutSettings};
use crate::xml::XmlElement;

/// Everything a component renderer needs beyond its own settings.
pub struct ComponentContext<'a> {
    pub timer: &'a Timer,
    pub theme: &'a LayoutSettings,
    pub icons: &'a mut IconCache,
    pub scale: f32,
}

impl<'a> ComponentContext<'a> {
    pub fn new(
        timer: &'a Timer,
        theme: &'a LayoutSettings,
        icons: &'a mut IconCache,
        scale: f32,
    ) -> Self {
        Self {
            timer,
            theme,
            icons,
            scale: scale.max(0.1),
        }
    }

    pub fn font(&self, spec: &FontSpec) -> FontId {
        let family = if spec.bold {
            FontFamily::Name(Arc::from("livesplit_bold"))
        } else {
            FontFamily::Name(Arc::from("livesplit_text"))
        };
        FontId::new((spec.size * self.scale).max(4.0), family)
    }

    pub fn timer_font(&self, size: f32, bold: bool) -> FontId {
        let family = if bold {
            FontFamily::Name(Arc::from("livesplit_timer_bold"))
        } else {
            FontFamily::Name(Arc::from("livesplit_timer"))
        };
        FontId::new((size * self.scale).max(4.0), family)
    }

    pub fn text_font(&self) -> FontId {
        self.font(&self.theme.text_font)
    }

    pub fn times_font(&self) -> FontId {
        self.font(&self.theme.times_font)
    }

    pub fn timer_font_default(&self) -> FontId {
        self.timer_font(self.theme.timer_font.size, self.theme.timer_font.bold)
    }

    pub fn resolved_timing_method(&self, setting: TimingMethodSetting) -> TimingMethod {
        match setting {
            TimingMethodSetting::Current => self.timer.current_timing_method,
            TimingMethodSetting::RealTime => TimingMethod::RealTime,
            TimingMethodSetting::GameTime => TimingMethod::GameTime,
        }
    }

    pub fn resolve_comparison<'b>(&'b self, comparison: &'b str) -> &'b str {
        if comparison == livesplit_core::CURRENT_COMPARISON {
            &self.timer.current_comparison
        } else {
            comparison
        }
    }
}

pub enum Component {
    Title(TitleSettings),
    Splits(SplitsSettings),
    Timer(TimerSettings),
    PreviousSegment(PreviousSegmentSettings),
    Delta(DeltaSettings),
    DetailedTimer(DetailedTimerSettings),
    SumOfBest(SumOfBestSettings),
    PossibleTimeSave(PossibleTimeSaveSettings),
    CurrentComparison(CurrentComparisonSettings),
    CurrentPace(CurrentPaceSettings),
    RunPrediction(RunPredictionSettings),
    Graph(GraphSettings),
    Text(TextSettings),
    Separator(SeparatorSettings),
    ThinSeparator(ThinSeparatorSettings),
    BlankSpace(BlankSpaceSettings),
    TotalPlaytime(TotalPlaytimeSettings),
    Counter(CounterSettings),
    HotkeyIndicator(HotkeyIndicatorSettings),
    TheRun(TheRunSettings),
    ComparisonTime(ComparisonTimeSettings),
    SumOfSegments(SumOfSegmentsSettings),
    WorldRecord(WorldRecordSettings),
    Subsplits(SubsplitsSettings),
    ManualGameTime(ManualGameTimeSettings),
    Unknown { path: String, settings: XmlElement },
}

impl std::fmt::Debug for Component {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Component")
            .field("name", &self.name())
            .field("path", &self.path())
            .finish()
    }
}

impl Clone for Component {
    fn clone(&self) -> Self {
        match self {
            Self::Title(s) => Self::Title(s.clone()),
            Self::Splits(s) => Self::Splits(s.clone()),
            Self::Timer(s) => Self::Timer(s.clone()),
            Self::PreviousSegment(s) => Self::PreviousSegment(s.clone()),
            Self::Delta(s) => Self::Delta(s.clone()),
            Self::DetailedTimer(s) => Self::DetailedTimer(s.clone()),
            Self::SumOfBest(s) => Self::SumOfBest(s.clone()),
            Self::PossibleTimeSave(s) => Self::PossibleTimeSave(s.clone()),
            Self::CurrentComparison(s) => Self::CurrentComparison(s.clone()),
            Self::CurrentPace(s) => Self::CurrentPace(s.clone()),
            Self::RunPrediction(s) => Self::RunPrediction(s.clone()),
            Self::Graph(s) => Self::Graph(s.clone()),
            Self::Text(s) => Self::Text(s.clone()),
            Self::Separator(s) => Self::Separator(s.clone()),
            Self::ThinSeparator(s) => Self::ThinSeparator(s.clone()),
            Self::BlankSpace(s) => Self::BlankSpace(s.clone()),
            Self::TotalPlaytime(s) => Self::TotalPlaytime(s.clone()),
            Self::Counter(s) => Self::Counter(s.clone()),
            Self::HotkeyIndicator(s) => Self::HotkeyIndicator(s.clone()),
            Self::TheRun(s) => Self::TheRun(s.clone()),
            Self::ComparisonTime(s) => Self::ComparisonTime(s.clone()),
            Self::SumOfSegments(s) => Self::SumOfSegments(s.clone()),
            Self::WorldRecord(s) => Self::WorldRecord(s.clone()),
            Self::Subsplits(s) => Self::Subsplits(s.clone()),
            Self::ManualGameTime(s) => Self::ManualGameTime(s.clone()),
            Self::Unknown { path, settings } => Self::Unknown {
                path: path.clone(),
                settings: settings.clone(),
            },
        }
    }
}

impl Component {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Title(_) => "Title",
            Self::Splits(_) => "Splits",
            Self::Timer(_) => "Timer",
            Self::PreviousSegment(_) => "Previous Segment",
            Self::Delta(_) => "Delta",
            Self::DetailedTimer(_) => "Detailed Timer",
            Self::SumOfBest(_) => "Sum of Best",
            Self::PossibleTimeSave(_) => "Possible Time Save",
            Self::CurrentComparison(_) => "Current Comparison",
            Self::CurrentPace(_) => "Current Pace",
            Self::RunPrediction(_) => "Run Prediction",
            Self::Graph(_) => "Graph",
            Self::Text(_) => "Text",
            Self::Separator(_) => "Separator",
            Self::ThinSeparator(_) => "Thin Separator",
            Self::BlankSpace(_) => "Blank Space",
            Self::TotalPlaytime(_) => "Total Playtime",
            Self::Counter(_) => "Counter",
            Self::HotkeyIndicator(_) => "Hotkey Indicator",
            Self::TheRun(_) => "The Run",
            Self::ComparisonTime(_) => "Comparison Time",
            Self::SumOfSegments(_) => "Sum of Segments",
            Self::WorldRecord(_) => "World Record",
            Self::Subsplits(_) => "Subsplits",
            Self::ManualGameTime(_) => "Manual Game Time",
            Self::Unknown { .. } => "Unknown",
        }
    }

    pub fn path(&self) -> &str {
        match self {
            Self::Title(_) => "LiveSplit.Title.dll",
            Self::Splits(_) => "LiveSplit.Splits.dll",
            Self::Timer(_) => "LiveSplit.Timer.dll",
            Self::PreviousSegment(_) => "LiveSplit.PreviousSegment.dll",
            Self::Delta(_) => "LiveSplit.Delta.dll",
            Self::DetailedTimer(_) => "LiveSplit.DetailedTimer.dll",
            Self::SumOfBest(_) => "LiveSplit.SumOfBest.dll",
            Self::PossibleTimeSave(_) => "LiveSplit.PossibleTimeSave.dll",
            Self::CurrentComparison(_) => "LiveSplit.CurrentComparison.dll",
            Self::CurrentPace(_) => "LiveSplit.CurrentPace.dll",
            Self::RunPrediction(_) => "LiveSplit.RunPrediction.dll",
            Self::Graph(_) => "LiveSplit.Graph.dll",
            Self::Text(_) => "LiveSplit.Text.dll",
            Self::Separator(_) => "LiveSplit.Separator.dll",
            Self::ThinSeparator(_) => "LiveSplit.ThinSeparator.dll",
            Self::BlankSpace(_) => "LiveSplit.BlankSpace.dll",
            Self::TotalPlaytime(_) => "LiveSplit.TotalPlaytime.dll",
            Self::Counter(_) => "LiveSplit.Counter.dll",
            Self::HotkeyIndicator(_) => "LiveSplit.HotkeyIndicator.dll",
            Self::TheRun(_) => "LiveSplit.TheRun.dll",
            Self::ComparisonTime(_) => "LiveSplit.ComparisonTime.dll",
            Self::SumOfSegments(_) => "LiveSplit.SumOfSegments.dll",
            Self::WorldRecord(_) => "LiveSplit.WorldRecord.dll",
            Self::Subsplits(_) => "LiveSplit.Subsplits.dll",
            Self::ManualGameTime(_) => "LiveSplit.ManualGameTime.dll",
            Self::Unknown { path, .. } => path.as_str(),
        }
    }

    pub fn from_xml(path: &str, settings: &XmlElement) -> Self {
        let normalized = normalize_path(path);
        match normalized.as_str() {
            "title" => Self::Title(basic::title_from_xml(settings)),
            "splits" => Self::Splits(splits::splits_from_xml(settings)),
            "timer" => Self::Timer(timing::timer_from_xml(settings)),
            "previoussegment" => {
                Self::PreviousSegment(timing::previous_segment_from_xml(settings))
            }
            "delta" => Self::Delta(timing::delta_from_xml(settings)),
            "detailedtimer" => Self::DetailedTimer(timing::detailed_timer_from_xml(settings)),
            "sumofbest" => Self::SumOfBest(basic::sum_of_best_from_xml(settings)),
            "possibletimesave" => {
                Self::PossibleTimeSave(timing::possible_time_save_from_xml(settings))
            }
            "currentcomparison" => {
                Self::CurrentComparison(timing::current_comparison_from_xml(settings))
            }
            "currentpace" => Self::CurrentPace(timing::current_pace_from_xml(settings)),
            "runprediction" => Self::RunPrediction(timing::run_prediction_from_xml(settings)),
            "graph" => Self::Graph(splits::graph_from_xml(settings)),
            "text" => Self::Text(basic::text_from_xml(settings)),
            "separator" => Self::Separator(SeparatorSettings),
            "thinseparator" => Self::ThinSeparator(ThinSeparatorSettings),
            "blankspace" => Self::BlankSpace(basic::blank_space_from_xml(settings)),
            "totalplaytime" => Self::TotalPlaytime(basic::total_playtime_from_xml(settings)),
            "counter" => Self::Counter(basic::counter_from_xml(settings)),
            "hotkeyindicator" => {
                Self::HotkeyIndicator(basic::hotkey_indicator_from_xml(settings))
            }
            "therun" => Self::TheRun(TheRunSettings),
            "comparisontime" => {
                Self::ComparisonTime(basic::comparison_time_from_xml(settings))
            }
            "sumofsegments" => Self::SumOfSegments(SumOfSegmentsSettings),
            "worldrecord" => Self::WorldRecord(basic::world_record_from_xml(settings)),
            "subsplits" => Self::Subsplits(splits::subsplits_from_xml(settings)),
            "manualgametime" => {
                Self::ManualGameTime(timing::manual_game_time_from_xml(settings))
            }
            _ => Self::Unknown {
                path: path.to_string(),
                settings: settings.clone(),
            },
        }
    }

    pub fn to_xml(&self) -> XmlElement {
        match self {
            Self::Title(s) => basic::title_to_xml(s),
            Self::Splits(s) => splits::splits_to_xml(s),
            Self::Timer(s) => timing::timer_to_xml(s),
            Self::PreviousSegment(s) => timing::previous_segment_to_xml(s),
            Self::Delta(s) => timing::delta_to_xml(s),
            Self::DetailedTimer(s) => timing::detailed_timer_to_xml(s),
            Self::SumOfBest(s) => basic::sum_of_best_to_xml(s),
            Self::PossibleTimeSave(s) => timing::possible_time_save_to_xml(s),
            Self::CurrentComparison(s) => timing::current_comparison_to_xml(s),
            Self::CurrentPace(s) => timing::current_pace_to_xml(s),
            Self::RunPrediction(s) => timing::run_prediction_to_xml(s),
            Self::Graph(s) => splits::graph_to_xml(s),
            Self::Text(s) => basic::text_to_xml(s),
            Self::Separator(_) => XmlElement::new("Settings"),
            Self::ThinSeparator(_) => XmlElement::new("Settings"),
            Self::BlankSpace(s) => basic::blank_space_to_xml(s),
            Self::TotalPlaytime(s) => basic::total_playtime_to_xml(s),
            Self::Counter(s) => basic::counter_to_xml(s),
            Self::HotkeyIndicator(s) => basic::hotkey_indicator_to_xml(s),
            Self::TheRun(_) => XmlElement::new("Settings"),
            Self::ComparisonTime(s) => basic::comparison_time_to_xml(s),
            Self::SumOfSegments(_) => XmlElement::new("Settings"),
            Self::WorldRecord(s) => basic::world_record_to_xml(s),
            Self::Subsplits(s) => splits::subsplits_to_xml(s),
            Self::ManualGameTime(s) => timing::manual_game_time_to_xml(s),
            Self::Unknown { settings, .. } => settings.clone(),
        }
    }

    pub fn natural_height(&self, timer: &Timer, scale: f32) -> f32 {
        let scale = scale.max(0.1);
        match self {
            Self::Title(_) => 42.0 * scale,
            Self::Splits(s) => splits::splits_height(timer, s, scale),
            Self::Timer(s) => s.timer_height * scale,
            Self::PreviousSegment(_) => 26.0 * scale,
            Self::Delta(_) => 40.0 * scale,
            Self::DetailedTimer(s) => s.height * scale,
            Self::SumOfBest(_) => 26.0 * scale,
            Self::PossibleTimeSave(_) => 26.0 * scale,
            Self::CurrentComparison(_) => 26.0 * scale,
            Self::CurrentPace(_) => 26.0 * scale,
            Self::RunPrediction(_) => 26.0 * scale,
            Self::Graph(s) => s.height * scale,
            Self::Text(_) => 26.0 * scale,
            Self::Separator(_) => 5.0 * scale,
            Self::ThinSeparator(_) => 1.0 * scale,
            Self::BlankSpace(s) => s.space_height * scale,
            Self::TotalPlaytime(_) => 26.0 * scale,
            Self::Counter(_) => 26.0 * scale,
            Self::HotkeyIndicator(s) => s.indicator_height * scale,
            Self::TheRun(_) => 26.0 * scale,
            Self::ComparisonTime(_) => 26.0 * scale,
            Self::SumOfSegments(_) => 26.0 * scale,
            Self::WorldRecord(_) => 26.0 * scale,
            Self::Subsplits(s) => splits::splits_height(timer, &s.splits, scale),
            Self::ManualGameTime(_) => 26.0 * scale,
            Self::Unknown { .. } => 0.0,
        }
    }

    pub fn natural_width(&self, _timer: &Timer, scale: f32) -> f32 {
        let scale = scale.max(0.1);
        match self {
            Self::Timer(s) => s.timer_width * scale,
            Self::DetailedTimer(s) => s.width * scale,
            Self::Graph(s) => s.width * scale,
            Self::BlankSpace(s) => s.space_width * scale,
            Self::HotkeyIndicator(s) => s.indicator_width * scale,
            Self::Separator(_) => 5.0 * scale,
            Self::ThinSeparator(_) => 1.0 * scale,
            Self::Splits(_) | Self::Subsplits(_) => 220.0 * scale,
            Self::Title(_) => 350.0 * scale,
            Self::Unknown { .. } => 0.0,
            _ => 200.0 * scale,
        }
    }

    pub fn draw(&self, ui: &mut Ui, rect: Rect, ctx: &mut ComponentContext) {
        match self {
            Self::Title(s) => basic::draw_title(ui, rect, ctx, s),
            Self::Splits(s) => splits::draw_splits(ui, rect, ctx, s),
            Self::Timer(s) => timing::draw_timer(ui, rect, ctx, s),
            Self::PreviousSegment(s) => timing::draw_previous_segment(ui, rect, ctx, s),
            Self::Delta(s) => timing::draw_delta(ui, rect, ctx, s),
            Self::DetailedTimer(s) => timing::draw_detailed_timer(ui, rect, ctx, s),
            Self::SumOfBest(s) => basic::draw_sum_of_best(ui, rect, ctx, s),
            Self::PossibleTimeSave(s) => timing::draw_possible_time_save(ui, rect, ctx, s),
            Self::CurrentComparison(s) => timing::draw_current_comparison(ui, rect, ctx, s),
            Self::CurrentPace(s) => timing::draw_current_pace(ui, rect, ctx, s),
            Self::RunPrediction(s) => timing::draw_run_prediction(ui, rect, ctx, s),
            Self::Graph(s) => splits::draw_graph(ui, rect, ctx, s),
            Self::Text(s) => basic::draw_text(ui, rect, ctx, s),
            Self::Separator(_) => basic::draw_separator(ui, rect, ctx, false),
            Self::ThinSeparator(_) => basic::draw_separator(ui, rect, ctx, true),
            Self::BlankSpace(s) => basic::draw_blank_space(ui, rect, ctx, s),
            Self::TotalPlaytime(s) => basic::draw_total_playtime(ui, rect, ctx, s),
            Self::Counter(s) => basic::draw_counter(ui, rect, ctx, s),
            Self::HotkeyIndicator(s) => basic::draw_hotkey_indicator(ui, rect, ctx, s),
            Self::TheRun(_) => basic::draw_the_run(ui, rect, ctx),
            Self::ComparisonTime(s) => basic::draw_comparison_time(ui, rect, ctx, s),
            Self::SumOfSegments(_) => timing::draw_sum_of_segments(ui, rect, ctx),
            Self::WorldRecord(s) => basic::draw_world_record(ui, rect, ctx, s),
            Self::Subsplits(s) => splits::draw_subsplits(ui, rect, ctx, s),
            Self::ManualGameTime(s) => timing::draw_manual_game_time(ui, rect, ctx, s),
            Self::Unknown { .. } => {}
        }
    }

    pub fn settings_ui(&mut self, ui: &mut Ui, theme: &mut LayoutSettings) {
        match self {
            Self::Title(s) => basic::title_settings_ui(ui, s),
            Self::Splits(s) => splits::splits_settings_ui(ui, s),
            Self::Timer(s) => timing::timer_settings_ui(ui, s),
            Self::PreviousSegment(s) => timing::previous_segment_settings_ui(ui, s),
            Self::Delta(s) => timing::delta_settings_ui(ui, s),
            Self::DetailedTimer(s) => timing::detailed_timer_settings_ui(ui, s),
            Self::SumOfBest(s) => basic::sum_of_best_settings_ui(ui, s),
            Self::PossibleTimeSave(s) => timing::possible_time_save_settings_ui(ui, s),
            Self::CurrentComparison(s) => timing::current_comparison_settings_ui(ui, s),
            Self::CurrentPace(s) => timing::current_pace_settings_ui(ui, s),
            Self::RunPrediction(s) => timing::run_prediction_settings_ui(ui, s),
            Self::Graph(s) => splits::graph_settings_ui(ui, s),
            Self::Text(s) => basic::text_settings_ui(ui, s),
            Self::Separator(_) | Self::ThinSeparator(_) | Self::TheRun(_) | Self::SumOfSegments(_) => {
                ui.label("This component has no settings.");
            }
            Self::BlankSpace(s) => basic::blank_space_settings_ui(ui, s),
            Self::TotalPlaytime(s) => basic::total_playtime_settings_ui(ui, s),
            Self::Counter(s) => basic::counter_settings_ui(ui, s),
            Self::HotkeyIndicator(s) => basic::hotkey_indicator_settings_ui(ui, s),
            Self::ComparisonTime(s) => basic::comparison_time_settings_ui(ui, s),
            Self::WorldRecord(s) => basic::world_record_settings_ui(ui, s),
            Self::Subsplits(s) => splits::subsplits_settings_ui(ui, s),
            Self::ManualGameTime(s) => timing::manual_game_time_settings_ui(ui, s),
            Self::Unknown { path, .. } => {
                ui.label(format!("Unknown component: {path}"));
            }
        }
        let _ = theme;
    }
}

pub fn normalize_path(path: &str) -> String {
    let path = path.split('?').next().unwrap_or(path);
    let file = path.rsplit(['/', '\\']).next().unwrap_or(path);
    let file = file.strip_suffix(".dll").unwrap_or(file);
    file.trim_start_matches("LiveSplit.").to_ascii_lowercase()
}

pub fn default_component_for_path(path: &str) -> Component {
    let normalized = normalize_path(path);
    match normalized.as_str() {
        "title" => Component::Title(TitleSettings::default()),
        "splits" => Component::Splits(SplitsSettings::default()),
        "timer" => Component::Timer(TimerSettings::default()),
        "previoussegment" => Component::PreviousSegment(PreviousSegmentSettings::default()),
        "delta" => Component::Delta(DeltaSettings::default()),
        "detailedtimer" => Component::DetailedTimer(DetailedTimerSettings::default()),
        "sumofbest" => Component::SumOfBest(SumOfBestSettings::default()),
        "possibletimesave" => Component::PossibleTimeSave(PossibleTimeSaveSettings::default()),
        "currentcomparison" => Component::CurrentComparison(CurrentComparisonSettings::default()),
        "currentpace" => Component::CurrentPace(CurrentPaceSettings::default()),
        "runprediction" => Component::RunPrediction(RunPredictionSettings::default()),
        "graph" => Component::Graph(GraphSettings::default()),
        "text" => Component::Text(TextSettings::default()),
        "separator" => Component::Separator(SeparatorSettings),
        "thinseparator" => Component::ThinSeparator(ThinSeparatorSettings),
        "blankspace" => Component::BlankSpace(BlankSpaceSettings::default()),
        "totalplaytime" => Component::TotalPlaytime(TotalPlaytimeSettings::default()),
        "counter" => Component::Counter(CounterSettings::default()),
        "hotkeyindicator" => Component::HotkeyIndicator(HotkeyIndicatorSettings::default()),
        "therun" => Component::TheRun(TheRunSettings),
        "comparisontime" => Component::ComparisonTime(ComparisonTimeSettings::default()),
        "sumofsegments" => Component::SumOfSegments(SumOfSegmentsSettings),
        "worldrecord" => Component::WorldRecord(WorldRecordSettings::default()),
        "subsplits" => Component::Subsplits(SubsplitsSettings::default()),
        "manualgametime" => Component::ManualGameTime(ManualGameTimeSettings::default()),
        _ => Component::Unknown {
            path: path.to_string(),
            settings: XmlElement::new("Settings"),
        },
    }
}

pub const ADDABLE_COMPONENTS: &[(&str, &str)] = &[
    ("LiveSplit.Title.dll", "Title"),
    ("LiveSplit.Splits.dll", "Splits"),
    ("LiveSplit.Timer.dll", "Timer"),
    ("LiveSplit.DetailedTimer.dll", "Detailed Timer"),
    ("LiveSplit.PreviousSegment.dll", "Previous Segment"),
    ("LiveSplit.Delta.dll", "Delta"),
    ("LiveSplit.SumOfBest.dll", "Sum of Best"),
    ("LiveSplit.PossibleTimeSave.dll", "Possible Time Save"),
    ("LiveSplit.CurrentComparison.dll", "Current Comparison"),
    ("LiveSplit.CurrentPace.dll", "Current Pace"),
    ("LiveSplit.RunPrediction.dll", "Run Prediction"),
    ("LiveSplit.Graph.dll", "Graph"),
    ("LiveSplit.Text.dll", "Text"),
    ("LiveSplit.Separator.dll", "Separator"),
    ("LiveSplit.ThinSeparator.dll", "Thin Separator"),
    ("LiveSplit.BlankSpace.dll", "Blank Space"),
    ("LiveSplit.TotalPlaytime.dll", "Total Playtime"),
    ("LiveSplit.Counter.dll", "Counter"),
    ("LiveSplit.HotkeyIndicator.dll", "Hotkey Indicator"),
    ("LiveSplit.TheRun.dll", "The Run"),
    ("LiveSplit.ComparisonTime.dll", "Comparison Time"),
    ("LiveSplit.SumOfSegments.dll", "Sum of Segments"),
    ("LiveSplit.WorldRecord.dll", "World Record"),
    ("LiveSplit.Subsplits.dll", "Subsplits"),
    ("LiveSplit.ManualGameTime.dll", "Manual Game Time"),
];
