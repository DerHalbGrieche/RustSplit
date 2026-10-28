//! Typed settings for all built-in layout components.
//!
//! Field names follow LiveSplit's serialized settings as closely as possible.
//! The XML readers/writers live next to the renderers in the component
//! modules; these types are the shared contract.

use egui::Color32;

use livesplit_core::{DigitsFormat, TimeAccuracy};

use crate::layout::{BackgroundStyle, FontSpec, GradientType};

#[derive(Clone, Debug)]
pub struct TitleSettings {
    pub show_game_name: bool,
    pub show_category_name: bool,
    pub show_attempt_count: bool,
    pub show_finished_runs_count: bool,
    pub text_alignment: u8,
    pub single_line: bool,
    pub display_game_icon: bool,
    pub show_region: bool,
    pub show_platform: bool,
    pub show_variables: bool,
    pub title_color: Color32,
    pub override_title_color: bool,
    pub background: BackgroundStyle,
    pub title_font: FontSpec,
    pub override_title_font: bool,
}

impl Default for TitleSettings {
    fn default() -> Self {
        Self {
            show_game_name: true,
            show_category_name: true,
            show_attempt_count: true,
            show_finished_runs_count: false,
            text_alignment: 0,
            single_line: false,
            display_game_icon: true,
            show_region: false,
            show_platform: false,
            show_variables: true,
            title_color: Color32::WHITE,
            override_title_color: false,
            background: BackgroundStyle::vertical(Color32::from_rgb(42, 42, 42), Color32::from_rgb(19, 19, 19)),
            title_font: FontSpec::text(),
            override_title_font: false,
        }
    }
}

#[derive(Clone, Debug)]
pub struct SplitsSettings {
    pub current_split_top_color: Color32,
    pub current_split_bottom_color: Color32,
    pub visual_split_count: i32,
    pub split_preview_count: i32,
    pub display_icons: bool,
    pub show_thin_separators: bool,
    pub always_show_last_split: bool,
    pub split_width: f32,
    pub show_split_times: bool,
    pub split_times_accuracy: TimeAccuracy,
    pub before_names_color: Color32,
    pub current_names_color: Color32,
    pub after_names_color: Color32,
    pub override_text_color: bool,
    pub before_times_color: Color32,
    pub current_times_color: Color32,
    pub after_times_color: Color32,
    pub override_times_color: bool,
    pub show_blank_splits: bool,
    pub lock_last_split: bool,
    pub icon_size: f32,
    pub icon_shadows: bool,
    /// LiveSplit stores this in units of 10 pixels.
    pub split_height: f32,
    pub current_split_gradient: GradientType,
    pub background: BackgroundStyle,
    pub separator_last_split: bool,
    pub deltas_accuracy: TimeAccuracy,
    pub drop_decimals: bool,
    pub override_deltas_color: bool,
    pub deltas_color: Color32,
    pub comparison: String,
    pub display_2_rows: bool,
    pub show_column_labels: bool,
    pub labels_color: Color32,
    pub automatic_abbreviations: bool,
}

impl Default for SplitsSettings {
    fn default() -> Self {
        Self {
            current_split_top_color: Color32::from_rgb(51, 115, 244),
            current_split_bottom_color: Color32::from_rgb(21, 53, 116),
            visual_split_count: 16,
            split_preview_count: 1,
            display_icons: true,
            show_thin_separators: true,
            always_show_last_split: true,
            split_width: 20.0,
            show_split_times: true,
            split_times_accuracy: TimeAccuracy::Seconds,
            before_names_color: Color32::WHITE,
            current_names_color: Color32::WHITE,
            after_names_color: Color32::WHITE,
            override_text_color: false,
            before_times_color: Color32::WHITE,
            current_times_color: Color32::WHITE,
            after_times_color: Color32::WHITE,
            override_times_color: false,
            show_blank_splits: true,
            lock_last_split: true,
            icon_size: 24.0,
            icon_shadows: true,
            split_height: 3.6,
            current_split_gradient: GradientType::Vertical,
            background: BackgroundStyle {
                color: Color32::TRANSPARENT,
                color2: Color32::from_rgba_unmultiplied(255, 255, 255, 1),
                gradient: GradientType::Alternating,
            },
            separator_last_split: true,
            deltas_accuracy: TimeAccuracy::Tenths,
            drop_decimals: true,
            override_deltas_color: false,
            deltas_color: Color32::WHITE,
            comparison: "Current Comparison".to_string(),
            display_2_rows: false,
            show_column_labels: false,
            labels_color: Color32::WHITE,
            automatic_abbreviations: false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimingMethodSetting {
    Current,
    RealTime,
    GameTime,
}

impl TimingMethodSetting {
    pub fn label(self) -> &'static str {
        match self {
            Self::Current => "Current Timing Method",
            Self::RealTime => "Real Time",
            Self::GameTime => "Game Time",
        }
    }

    pub fn parse(value: &str, default: Self) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "current timing method" | "current" => Self::Current,
            "real time" | "realtime" | "rta" => Self::RealTime,
            "game time" | "gametime" | "igt" => Self::GameTime,
            _ => default,
        }
    }
}

#[derive(Clone, Debug)]
pub struct TimerSettings {
    pub timer_height: f32,
    pub timer_width: f32,
    pub decimals_size: f32,
    pub digits_format: DigitsFormat,
    pub accuracy: TimeAccuracy,
    pub mode: crate::layout::LayoutMode,
    pub timer_color: Color32,
    pub override_split_colors: bool,
    pub center_timer: bool,
    pub show_gradient: bool,
    pub timing_method: TimingMethodSetting,
    pub background: BackgroundStyle,
}

impl Default for TimerSettings {
    fn default() -> Self {
        Self {
            timer_height: 69.0,
            timer_width: 225.0,
            decimals_size: 35.0,
            digits_format: DigitsFormat::SingleDigitSeconds,
            accuracy: TimeAccuracy::Hundredths,
            mode: crate::layout::LayoutMode::Vertical,
            timer_color: Color32::from_rgb(170, 170, 170),
            override_split_colors: false,
            center_timer: false,
            show_gradient: true,
            timing_method: TimingMethodSetting::Current,
            background: BackgroundStyle {
                color: Color32::TRANSPARENT,
                color2: Color32::from_rgb(34, 34, 34),
                gradient: GradientType::Plain,
            },
        }
    }
}

#[derive(Clone, Debug)]
pub struct PreviousSegmentSettings {
    pub text_color: Color32,
    pub override_text_color: bool,
    pub background: BackgroundStyle,
    pub delta_accuracy: TimeAccuracy,
    pub time_save_accuracy: TimeAccuracy,
    pub drop_decimals: bool,
    pub comparison: String,
    pub display_2_rows: bool,
    pub show_possible_time_save: bool,
}

impl Default for PreviousSegmentSettings {
    fn default() -> Self {
        Self {
            text_color: Color32::WHITE,
            override_text_color: false,
            background: BackgroundStyle::vertical(Color32::from_rgb(28, 28, 28), Color32::from_rgb(13, 13, 13)),
            delta_accuracy: TimeAccuracy::Tenths,
            time_save_accuracy: TimeAccuracy::Tenths,
            drop_decimals: true,
            comparison: "Current Comparison".to_string(),
            display_2_rows: false,
            show_possible_time_save: false,
        }
    }
}

#[derive(Clone, Debug)]
pub struct DeltaSettings {
    pub text_color: Color32,
    pub override_text_color: bool,
    pub accuracy: TimeAccuracy,
    pub background: BackgroundStyle,
    pub comparison: String,
    pub display_2_rows: bool,
    pub drop_decimals: bool,
    pub override_text: bool,
    pub custom_text: String,
    pub differential_text: bool,
}

impl Default for DeltaSettings {
    fn default() -> Self {
        Self {
            text_color: Color32::WHITE,
            override_text_color: false,
            accuracy: TimeAccuracy::Tenths,
            background: BackgroundStyle::transparent(),
            comparison: "Current Comparison".to_string(),
            display_2_rows: false,
            drop_decimals: true,
            override_text: false,
            custom_text: String::new(),
            differential_text: false,
        }
    }
}

#[derive(Clone, Debug)]
pub struct DetailedTimerSettings {
    pub height: f32,
    pub width: f32,
    pub timer_color: Color32,
    pub segment_timer_color: Color32,
    pub background: BackgroundStyle,
    pub timer_format: (DigitsFormat, TimeAccuracy),
    pub segment_timer_format: (DigitsFormat, TimeAccuracy),
    pub segment_times_accuracy: TimeAccuracy,
    pub segment_times_color: Color32,
    pub show_split_name: bool,
    pub split_name_color: Color32,
    pub split_name_font: FontSpec,
    pub comparison: String,
    pub comparison2: String,
    pub hide_comparison: bool,
    pub timing_method: TimingMethodSetting,
    pub display_icon: bool,
    pub icon_size: f32,
}

impl Default for DetailedTimerSettings {
    fn default() -> Self {
        Self {
            height: 75.0,
            width: 200.0,
            timer_color: Color32::from_rgb(170, 170, 170),
            segment_timer_color: Color32::from_rgb(170, 170, 170),
            background: BackgroundStyle::transparent(),
            timer_format: (DigitsFormat::SingleDigitSeconds, TimeAccuracy::Hundredths),
            segment_timer_format: (DigitsFormat::SingleDigitSeconds, TimeAccuracy::Hundredths),
            segment_times_accuracy: TimeAccuracy::Hundredths,
            segment_times_color: Color32::WHITE,
            show_split_name: true,
            split_name_color: Color32::WHITE,
            split_name_font: FontSpec {
                name: "Segoe UI".to_string(),
                size: 15.0,
                bold: false,
                italic: false,
                raw: None,
            },
            comparison: "Current Comparison".to_string(),
            comparison2: "Best Segments".to_string(),
            hide_comparison: false,
            timing_method: TimingMethodSetting::Current,
            display_icon: true,
            icon_size: 24.0,
        }
    }
}

#[derive(Clone, Debug)]
pub struct SumOfBestSettings {
    pub text_color: Color32,
    pub time_color: Color32,
    pub override_text_color: bool,
    pub override_time_color: bool,
    pub accuracy: TimeAccuracy,
    pub background: BackgroundStyle,
    pub display_2_rows: bool,
}

impl Default for SumOfBestSettings {
    fn default() -> Self {
        Self {
            text_color: Color32::WHITE,
            time_color: Color32::WHITE,
            override_text_color: false,
            override_time_color: false,
            accuracy: TimeAccuracy::Seconds,
            background: BackgroundStyle::transparent(),
            display_2_rows: false,
        }
    }
}

#[derive(Clone, Debug)]
pub struct PossibleTimeSaveSettings {
    pub text_color: Color32,
    pub time_color: Color32,
    pub override_text_color: bool,
    pub override_time_color: bool,
    pub accuracy: TimeAccuracy,
    pub drop_decimals: bool,
    pub background: BackgroundStyle,
    pub comparison: String,
    pub display_2_rows: bool,
    pub total_time_save: bool,
}

impl Default for PossibleTimeSaveSettings {
    fn default() -> Self {
        Self {
            text_color: Color32::WHITE,
            time_color: Color32::WHITE,
            override_text_color: false,
            override_time_color: false,
            accuracy: TimeAccuracy::Hundredths,
            drop_decimals: false,
            background: BackgroundStyle::transparent(),
            comparison: "Current Comparison".to_string(),
            display_2_rows: false,
            total_time_save: false,
        }
    }
}

#[derive(Clone, Debug)]
pub struct CurrentComparisonSettings {
    pub text_color: Color32,
    pub time_color: Color32,
    pub override_text_color: bool,
    pub override_time_color: bool,
    pub background: BackgroundStyle,
    pub display_2_rows: bool,
}

impl Default for CurrentComparisonSettings {
    fn default() -> Self {
        Self {
            text_color: Color32::WHITE,
            time_color: Color32::WHITE,
            override_text_color: false,
            override_time_color: false,
            background: BackgroundStyle::transparent(),
            display_2_rows: false,
        }
    }
}

#[derive(Clone, Debug)]
pub struct CurrentPaceSettings {
    pub text_color: Color32,
    pub time_color: Color32,
    pub override_text_color: bool,
    pub override_time_color: bool,
    pub background: BackgroundStyle,
    pub comparison: String,
    pub accuracy: TimeAccuracy,
    pub display_2_rows: bool,
}

impl Default for CurrentPaceSettings {
    fn default() -> Self {
        Self {
            text_color: Color32::WHITE,
            time_color: Color32::WHITE,
            override_text_color: false,
            override_time_color: false,
            background: BackgroundStyle::transparent(),
            comparison: "Current Comparison".to_string(),
            accuracy: TimeAccuracy::Seconds,
            display_2_rows: false,
        }
    }
}

#[derive(Clone, Debug)]
pub struct RunPredictionSettings {
    pub text_color: Color32,
    pub time_color: Color32,
    pub override_text_color: bool,
    pub override_time_color: bool,
    pub accuracy: TimeAccuracy,
    pub background: BackgroundStyle,
    pub comparison: String,
    pub display_2_rows: bool,
}

impl Default for RunPredictionSettings {
    fn default() -> Self {
        Self {
            text_color: Color32::WHITE,
            time_color: Color32::WHITE,
            override_text_color: false,
            override_time_color: false,
            accuracy: TimeAccuracy::Seconds,
            background: BackgroundStyle::transparent(),
            comparison: "Current Comparison".to_string(),
            display_2_rows: false,
        }
    }
}

#[derive(Clone, Debug)]
pub struct GraphSettings {
    pub height: f32,
    pub width: f32,
    pub comparison: String,
    pub graph_color: Color32,
    pub graph_gold_color: Color32,
    pub ahead_graph_color: Color32,
    pub behind_graph_color: Color32,
    pub graph_lines_color: Color32,
    pub gridlines_color: Color32,
    pub partial_fill_color_ahead: Color32,
    pub complete_fill_color_ahead: Color32,
    pub partial_fill_color_behind: Color32,
    pub complete_fill_color_behind: Color32,
    pub flip_graph: bool,
    pub show_best_segments: bool,
    pub live_graph: bool,
    pub background: BackgroundStyle,
}

impl Default for GraphSettings {
    fn default() -> Self {
        Self {
            height: 120.0,
            width: 180.0,
            comparison: "Current Comparison".to_string(),
            graph_color: Color32::from_rgb(155, 155, 155),
            graph_gold_color: Color32::from_rgb(216, 175, 31),
            ahead_graph_color: Color32::from_rgb(41, 204, 84),
            behind_graph_color: Color32::from_rgb(204, 55, 41),
            graph_lines_color: Color32::from_rgb(255, 255, 255),
            gridlines_color: Color32::from_rgba_unmultiplied(255, 255, 255, 40),
            partial_fill_color_ahead: Color32::from_rgba_unmultiplied(41, 204, 84, 80),
            complete_fill_color_ahead: Color32::from_rgba_unmultiplied(41, 204, 84, 160),
            partial_fill_color_behind: Color32::from_rgba_unmultiplied(204, 55, 41, 80),
            complete_fill_color_behind: Color32::from_rgba_unmultiplied(204, 55, 41, 160),
            flip_graph: false,
            show_best_segments: true,
            live_graph: true,
            background: BackgroundStyle::transparent(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct TextSettings {
    pub text1: String,
    pub text2: String,
    pub text_color: Color32,
    pub time_color: Color32,
    pub override_text_color: bool,
    pub override_time_color: bool,
    pub background: BackgroundStyle,
    pub display_2_rows: bool,
    pub custom_variable: String,
}

impl Default for TextSettings {
    fn default() -> Self {
        Self {
            text1: "Text".to_string(),
            text2: String::new(),
            text_color: Color32::WHITE,
            time_color: Color32::WHITE,
            override_text_color: false,
            override_time_color: false,
            background: BackgroundStyle::transparent(),
            display_2_rows: false,
            custom_variable: String::new(),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct SeparatorSettings;

#[derive(Clone, Debug, Default)]
pub struct ThinSeparatorSettings;

#[derive(Clone, Debug)]
pub struct BlankSpaceSettings {
    pub space_height: f32,
    pub space_width: f32,
    pub background: BackgroundStyle,
}

impl Default for BlankSpaceSettings {
    fn default() -> Self {
        Self {
            space_height: 100.0,
            space_width: 100.0,
            background: BackgroundStyle::transparent(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct TotalPlaytimeSettings {
    pub text_color: Color32,
    pub time_color: Color32,
    pub override_text_color: bool,
    pub override_time_color: bool,
    pub background: BackgroundStyle,
    pub display_2_rows: bool,
    pub show_total_hours: bool,
}

impl Default for TotalPlaytimeSettings {
    fn default() -> Self {
        Self {
            text_color: Color32::WHITE,
            time_color: Color32::WHITE,
            override_text_color: false,
            override_time_color: false,
            background: BackgroundStyle::transparent(),
            display_2_rows: false,
            show_total_hours: false,
        }
    }
}

#[derive(Clone, Debug)]
pub struct CounterSettings {
    pub counter_text: String,
    pub counter_text_color: Color32,
    pub counter_value_color: Color32,
    pub initial_value: i32,
    pub increment: i32,
    pub override_text_color: bool,
    pub background: BackgroundStyle,
    pub display_2_rows: bool,
    pub increment_key: String,
    pub decrement_key: String,
    pub reset_key: String,
    pub global_hotkeys_enabled: bool,
}

impl Default for CounterSettings {
    fn default() -> Self {
        Self {
            counter_text: "Counter".to_string(),
            counter_text_color: Color32::WHITE,
            counter_value_color: Color32::WHITE,
            initial_value: 0,
            increment: 1,
            override_text_color: false,
            background: BackgroundStyle::transparent(),
            display_2_rows: false,
            increment_key: "NumPad+".to_string(),
            decrement_key: "NumPad-".to_string(),
            reset_key: "NumPad*".to_string(),
            global_hotkeys_enabled: false,
        }
    }
}

#[derive(Clone, Debug)]
pub struct HotkeyIndicatorSettings {
    pub indicator_width: f32,
    pub indicator_height: f32,
    pub on_color: Color32,
    pub off_color: Color32,
}

impl Default for HotkeyIndicatorSettings {
    fn default() -> Self {
        Self {
            indicator_width: 60.0,
            indicator_height: 20.0,
            on_color: Color32::from_rgb(41, 204, 84),
            off_color: Color32::from_rgb(80, 80, 80),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct TheRunSettings;

#[derive(Clone, Debug)]
pub struct ComparisonTimeSettings {
    pub text_color: Color32,
    pub time_color: Color32,
    pub override_text_color: bool,
    pub override_time_color: bool,
    pub accuracy: TimeAccuracy,
    pub background: BackgroundStyle,
    pub comparison: String,
    pub timing_method: TimingMethodSetting,
    pub display_2_rows: bool,
    pub kind: i32,
    pub split_index: usize,
}

impl Default for ComparisonTimeSettings {
    fn default() -> Self {
        Self {
            text_color: Color32::WHITE,
            time_color: Color32::WHITE,
            override_text_color: false,
            override_time_color: false,
            accuracy: TimeAccuracy::Seconds,
            background: BackgroundStyle::transparent(),
            comparison: "Current Comparison".to_string(),
            timing_method: TimingMethodSetting::Current,
            display_2_rows: false,
            kind: 0,
            split_index: 0,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct SumOfSegmentsSettings;

#[derive(Clone, Debug)]
pub struct WorldRecordSettings {
    pub text_color: Color32,
    pub time_color: Color32,
    pub override_text_color: bool,
    pub override_time_color: bool,
    pub background: BackgroundStyle,
    pub display_2_rows: bool,
    pub timing_method: TimingMethodSetting,
    pub centered_text: bool,
    pub precision_type: i32,
    pub filter_platform: String,
    pub filter_region: String,
    pub filter_subcategories: bool,
    pub filter_variables: bool,
}

impl Default for WorldRecordSettings {
    fn default() -> Self {
        Self {
            text_color: Color32::WHITE,
            time_color: Color32::WHITE,
            override_text_color: false,
            override_time_color: false,
            background: BackgroundStyle::transparent(),
            display_2_rows: false,
            timing_method: TimingMethodSetting::Current,
            centered_text: false,
            precision_type: 0,
            filter_platform: String::new(),
            filter_region: String::new(),
            filter_subcategories: false,
            filter_variables: false,
        }
    }
}

#[derive(Clone, Debug)]
pub struct SubsplitsSettings {
    pub splits: SplitsSettings,
    pub show_subsplits: bool,
    pub show_header: bool,
    pub header_text: String,
    pub header_top_color: Color32,
    pub header_bottom_color: Color32,
    pub header_gradient: GradientType,
    pub header_text_color: Color32,
    pub header_times_color: Color32,
    pub header_accuracy: TimeAccuracy,
    pub header_timing_method: TimingMethodSetting,
    pub header_comparison: String,
    pub current_section_only: bool,
    pub minimum_major_splits: i32,
}

impl Default for SubsplitsSettings {
    fn default() -> Self {
        Self {
            splits: SplitsSettings::default(),
            show_subsplits: false,
            show_header: false,
            header_text: String::new(),
            header_top_color: Color32::from_rgb(51, 115, 244),
            header_bottom_color: Color32::from_rgb(21, 53, 116),
            header_gradient: GradientType::Vertical,
            header_text_color: Color32::WHITE,
            header_times_color: Color32::WHITE,
            header_accuracy: TimeAccuracy::Tenths,
            header_timing_method: TimingMethodSetting::Current,
            header_comparison: "Current Comparison".to_string(),
            current_section_only: false,
            minimum_major_splits: 0,
        }
    }
}

#[derive(Clone, Debug)]
pub struct ManualGameTimeSettings {
    pub use_segment_times: bool,
}

impl Default for ManualGameTimeSettings {
    fn default() -> Self {
        Self {
            use_segment_times: true,
        }
    }
}
