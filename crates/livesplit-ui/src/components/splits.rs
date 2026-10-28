//! Splits table, graph and subsplits components.
//!
//! The XML readers/writers mirror LiveSplit's plugin settings.  Rendering is
//! done directly with egui's painter so that it can scale with the layout and
//! stay independent of the rest of the application.

use egui::{Align2, Color32, Pos2, Rect, Stroke, Ui, Vec2};

use livesplit_core::{
    analysis, DeltaTimeFormatter, DigitsFormat, GeneralTimeFormatter, NullFormat, TimeAccuracy,
    TimeSpan, TimerPhase, CURRENT_COMPARISON,
};

use crate::canvas::{fit_text, paint_background, paint_gradient, paint_text, text_width};
use crate::components::{
    ComponentContext, GraphSettings, SplitsSettings, SubsplitsSettings, TimingMethodSetting,
};
use crate::layout::{BackgroundStyle, GradientType};
use crate::xml::{
    child, child_text, children_named, element_with_text, parse_bool, parse_f32, parse_i32,
    XmlElement,
};
use xmltree::XMLNode;

/// Internal marker for LiveSplit's `HeaderText = false` setting.  The public
/// settings type stores header text as a string, so a control character that
/// cannot be typed into the settings dialog is used to keep the old boolean
/// information while still round-tripping.
const HEADER_TEXT_HIDDEN: &str = "\u{2063}";

// ---------------------------------------------------------------------------
// XML helpers
// ---------------------------------------------------------------------------

fn add_text(parent: &mut XmlElement, name: &str, value: impl Into<String>) {
    parent
        .children
        .push(XMLNode::Element(element_with_text(name, value)));
}

fn add_bool(parent: &mut XmlElement, name: &str, value: bool) {
    add_text(parent, name, bool_text(value));
}

fn bool_text(value: bool) -> &'static str {
    if value {
        "True"
    } else {
        "False"
    }
}

fn add_color(parent: &mut XmlElement, name: &str, color: Color32) {
    add_text(parent, name, color_hex(color));
}

/// Serialize a color as `AARRGGBB`, converting back from Color32's
/// premultiplied representation first.
fn color_hex(color: Color32) -> String {
    let [r, g, b, a] = color.to_srgba_unmultiplied();
    format!("{a:02X}{r:02X}{g:02X}{b:02X}")
}

fn parse_version(element: &XmlElement) -> (i32, i32) {
    let version = child_text(element, "Version").unwrap_or_default();
    let mut parts = version.trim().split('.');
    let major = parts
        .next()
        .and_then(|part| part.trim().parse::<i32>().ok())
        .unwrap_or(1);
    let minor = parts
        .next()
        .and_then(|part| part.trim().parse::<i32>().ok())
        .unwrap_or(0);
    (major, minor)
}

fn version_at_least(version: (i32, i32), required: (i32, i32)) -> bool {
    version.0 > required.0 || (version.0 == required.0 && version.1 >= required.1)
}

fn parse_accuracy(value: Option<&str>, default: TimeAccuracy) -> TimeAccuracy {
    match value
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "seconds" => TimeAccuracy::Seconds,
        "tenths" => TimeAccuracy::Tenths,
        "hundredths" => TimeAccuracy::Hundredths,
        "milliseconds" => TimeAccuracy::Milliseconds,
        _ => default,
    }
}

fn gradient_xml_label(gradient: GradientType) -> &'static str {
    match gradient {
        GradientType::Plain | GradientType::PlainWithDeltaColor => "Plain",
        GradientType::Vertical | GradientType::VerticalWithDeltaColor => "Vertical",
        GradientType::Horizontal | GradientType::HorizontalWithDeltaColor => "Horizontal",
        GradientType::Alternating => "Alternating",
    }
}

fn gradient_is_plain(gradient: GradientType) -> bool {
    matches!(
        gradient,
        GradientType::Plain | GradientType::PlainWithDeltaColor
    )
}

fn parse_background_style(element: &XmlElement, mut style: BackgroundStyle) -> BackgroundStyle {
    style.color = crate::layout::parse_color(
        child_text(element, "BackgroundColor").as_deref(),
        style.color,
    );
    style.color2 = crate::layout::parse_color(
        child_text(element, "BackgroundColor2").as_deref(),
        style.color2,
    );
    let gradient = child_text(element, "BackgroundGradient")
        .or_else(|| child_text(element, "BackgroundType"));
    style.gradient = GradientType::parse(gradient.as_deref().unwrap_or_default(), style.gradient);
    style
}

fn write_background_style(element: &mut XmlElement, style: &BackgroundStyle) {
    add_color(element, "BackgroundColor", style.color);
    add_color(element, "BackgroundColor2", style.color2);
    add_text(
        element,
        "BackgroundGradient",
        gradient_xml_label(style.gradient),
    );
}

// ---------------------------------------------------------------------------
// Splits settings XML
// ---------------------------------------------------------------------------

pub fn splits_from_xml(settings: &XmlElement) -> SplitsSettings {
    parse_splits_settings(settings, SplitsSettings::default())
}

fn parse_splits_settings(element: &XmlElement, mut settings: SplitsSettings) -> SplitsSettings {
    let version = parse_version(element);
    let value = |name: &str| child_text(element, name);

    settings.current_split_top_color = crate::layout::parse_color(
        value("CurrentSplitTopColor").as_deref(),
        settings.current_split_top_color,
    );
    settings.current_split_bottom_color = crate::layout::parse_color(
        value("CurrentSplitBottomColor").as_deref(),
        settings.current_split_bottom_color,
    );
    settings.visual_split_count = parse_i32(
        value("VisualSplitCount").as_deref(),
        settings.visual_split_count,
    );
    settings.split_preview_count = parse_i32(
        value("SplitPreviewCount").as_deref(),
        settings.split_preview_count,
    );
    settings.display_icons =
        parse_bool(value("DisplayIcons").as_deref(), settings.display_icons);
    settings.show_thin_separators = parse_bool(
        value("ShowThinSeparators").as_deref(),
        settings.show_thin_separators,
    );
    settings.always_show_last_split = parse_bool(
        value("AlwaysShowLastSplit").as_deref(),
        settings.always_show_last_split,
    );
    settings.split_width =
        parse_f32(value("SplitWidth").as_deref(), settings.split_width);
    settings.icon_size = parse_f32(value("IconSize").as_deref(), settings.icon_size);
    settings.icon_shadows =
        parse_bool(value("IconShadows").as_deref(), settings.icon_shadows);
    settings.split_height =
        parse_f32(value("SplitHeight").as_deref(), settings.split_height);
    settings.show_blank_splits = parse_bool(
        value("ShowBlankSplits").as_deref(),
        settings.show_blank_splits,
    );
    settings.lock_last_split = parse_bool(
        value("LockLastSplit").as_deref(),
        settings.lock_last_split,
    );
    settings.separator_last_split = parse_bool(
        value("SeparatorLastSplit").as_deref(),
        settings.separator_last_split,
    );
    settings.display_2_rows =
        parse_bool(value("Display2Rows").as_deref(), settings.display_2_rows);
    settings.show_column_labels = parse_bool(
        value("ShowColumnLabels").as_deref(),
        settings.show_column_labels,
    );
    settings.labels_color =
        crate::layout::parse_color(value("LabelsColor").as_deref(), settings.labels_color);

    settings.automatic_abbreviations = parse_bool(
        value("AutomaticAbbreviations")
            .or_else(|| value("AutomaticAbbreviation"))
            .as_deref(),
        settings.automatic_abbreviations,
    );

    settings.split_times_accuracy = parse_accuracy(
        value("SplitTimesAccuracy").as_deref(),
        settings.split_times_accuracy,
    );
    settings.deltas_accuracy = parse_accuracy(
        value("DeltasAccuracy").as_deref(),
        settings.deltas_accuracy,
    );
    settings.drop_decimals =
        parse_bool(value("DropDecimals").as_deref(), settings.drop_decimals);
    settings.override_deltas_color = parse_bool(
        value("OverrideDeltasColor").as_deref(),
        settings.override_deltas_color,
    );
    settings.deltas_color =
        crate::layout::parse_color(value("DeltasColor").as_deref(), settings.deltas_color);

    let split_gradient = value("CurrentSplitGradient").or_else(|| value("SplitGradient"));
    settings.current_split_gradient = GradientType::parse(
        split_gradient.as_deref().unwrap_or_default(),
        settings.current_split_gradient,
    );
    settings.background = parse_background_style(element, settings.background);

    settings.override_times_color = parse_bool(
        value("OverrideTimesColor").as_deref(),
        settings.override_times_color,
    );
    settings.before_times_color = crate::layout::parse_color(
        value("BeforeTimesColor").as_deref(),
        settings.before_times_color,
    );
    settings.current_times_color = crate::layout::parse_color(
        value("CurrentTimesColor").as_deref(),
        settings.current_times_color,
    );
    settings.after_times_color = crate::layout::parse_color(
        value("AfterTimesColor").as_deref(),
        settings.after_times_color,
    );

    // Version 1.3 introduced the per-state name colors.  Older layouts either
    // use a single SplitNamesColor (1.2) or no color at all (1.0/1.1).
    let modern_name_colors = version_at_least(version, (1, 3))
        || element_has_child(element, "BeforeNamesColor")
        || element_has_child(element, "CurrentNamesColor")
        || element_has_child(element, "AfterNamesColor");
    if modern_name_colors {
        settings.before_names_color = crate::layout::parse_color(
            value("BeforeNamesColor").as_deref(),
            settings.before_names_color,
        );
        settings.current_names_color = crate::layout::parse_color(
            value("CurrentNamesColor").as_deref(),
            settings.current_names_color,
        );
        settings.after_names_color = crate::layout::parse_color(
            value("AfterNamesColor").as_deref(),
            settings.after_names_color,
        );
        if let Some(override_color) = value("OverrideTextColor") {
            settings.override_text_color =
                parse_bool(Some(&override_color), settings.override_text_color);
        }
    } else {
        if version_at_least(version, (1, 2)) || element_has_child(element, "SplitNamesColor") {
            let color = crate::layout::parse_color(
                value("SplitNamesColor").as_deref(),
                Color32::WHITE,
            );
            settings.before_names_color = color;
            settings.current_names_color = color;
            settings.after_names_color = color;
        }
        settings.override_text_color = !parse_bool(value("UseTextColor").as_deref(), true);
    }

    // Columns replaced Comparison + ShowSplitTimes in Splits 1.5 and
    // Subsplits 1.7.  Our settings type intentionally only stores the two
    // standard columns, but custom column comparison/timing is still useful.
    let mut columns_found = false;
    if let Some(columns) = child(element, "Columns") {
        let mut saw_column = false;
        let mut split_time_column = false;
        let mut comparison = settings.comparison.clone();
        for column in children_named(columns, "Settings") {
            let column_type = child_text(column, "Type").unwrap_or_default();
            if column_type.trim().is_empty() {
                continue;
            }
            saw_column = true;
            match column_type.trim() {
                "SplitTime" | "SegmentTime" => split_time_column = true,
                "DeltaorSplitTime" | "SegmentDeltaorSegmentTime" => {
                    // A single combined column; the old ShowSplitTimes=false
                    // layout looked like this.
                }
                _ => {}
            }
            if let Some(column_comparison) = child_text(column, "Comparison") {
                if !column_comparison.trim().is_empty() {
                    comparison = column_comparison;
                }
            }
        }
        if saw_column {
            settings.show_split_times = split_time_column;
            settings.comparison = comparison;
            columns_found = true;
        }
    }
    if !columns_found {
        settings.show_split_times = parse_bool(
            value("ShowSplitTimes").as_deref(),
            settings.show_split_times,
        );
        if let Some(comparison) = value("Comparison") {
            settings.comparison = comparison;
        }
    }

    settings
}

fn element_has_child(element: &XmlElement, name: &str) -> bool {
    child(element, name).is_some()
}

pub fn splits_to_xml(settings: &SplitsSettings) -> XmlElement {
    splits_settings_node(settings, "1.6")
}

fn splits_settings_node(settings: &SplitsSettings, version: &str) -> XmlElement {
    let mut element = XmlElement::new("Settings");
    add_text(&mut element, "Version", version);

    add_color(
        &mut element,
        "CurrentSplitTopColor",
        settings.current_split_top_color,
    );
    add_color(
        &mut element,
        "CurrentSplitBottomColor",
        settings.current_split_bottom_color,
    );
    add_text(
        &mut element,
        "VisualSplitCount",
        settings.visual_split_count.to_string(),
    );
    add_text(
        &mut element,
        "SplitPreviewCount",
        settings.split_preview_count.to_string(),
    );
    add_bool(&mut element, "DisplayIcons", settings.display_icons);
    add_bool(
        &mut element,
        "ShowThinSeparators",
        settings.show_thin_separators,
    );
    add_bool(
        &mut element,
        "AlwaysShowLastSplit",
        settings.always_show_last_split,
    );
    add_text(&mut element, "SplitWidth", settings.split_width.to_string());
    add_bool(
        &mut element,
        "ShowSplitTimes",
        settings.show_split_times,
    );
    add_text(&mut element, "Comparison", settings.comparison.clone());
    add_text(
        &mut element,
        "SplitTimesAccuracy",
        settings.split_times_accuracy.label(),
    );
    add_text(
        &mut element,
        "DeltasAccuracy",
        settings.deltas_accuracy.label(),
    );
    add_bool(&mut element, "DropDecimals", settings.drop_decimals);
    add_bool(
        &mut element,
        "OverrideDeltasColor",
        settings.override_deltas_color,
    );
    add_color(&mut element, "DeltasColor", settings.deltas_color);

    add_bool(
        &mut element,
        "AutomaticAbbreviations",
        settings.automatic_abbreviations,
    );
    add_color(
        &mut element,
        "BeforeNamesColor",
        settings.before_names_color,
    );
    add_color(
        &mut element,
        "CurrentNamesColor",
        settings.current_names_color,
    );
    add_color(&mut element, "AfterNamesColor", settings.after_names_color);
    add_bool(
        &mut element,
        "OverrideTextColor",
        settings.override_text_color,
    );
    add_color(
        &mut element,
        "BeforeTimesColor",
        settings.before_times_color,
    );
    add_color(
        &mut element,
        "CurrentTimesColor",
        settings.current_times_color,
    );
    add_color(&mut element, "AfterTimesColor", settings.after_times_color);
    add_bool(
        &mut element,
        "OverrideTimesColor",
        settings.override_times_color,
    );

    add_bool(&mut element, "ShowBlankSplits", settings.show_blank_splits);
    add_bool(&mut element, "LockLastSplit", settings.lock_last_split);
    add_text(&mut element, "IconSize", settings.icon_size.to_string());
    add_bool(&mut element, "IconShadows", settings.icon_shadows);
    add_text(&mut element, "SplitHeight", settings.split_height.to_string());
    add_text(
        &mut element,
        "CurrentSplitGradient",
        gradient_xml_label(settings.current_split_gradient),
    );
    write_background_style(&mut element, &settings.background);
    add_bool(
        &mut element,
        "SeparatorLastSplit",
        settings.separator_last_split,
    );
    add_bool(&mut element, "Display2Rows", settings.display_2_rows);
    add_bool(
        &mut element,
        "ShowColumnLabels",
        settings.show_column_labels,
    );
    add_color(&mut element, "LabelsColor", settings.labels_color);

    let comparison = if settings.comparison.trim().is_empty() {
        CURRENT_COMPARISON
    } else {
        settings.comparison.trim()
    };
    let mut columns = XmlElement::new("Columns");
    if settings.show_split_times {
        columns.children.push(XMLNode::Element(column_node(
            "+/-",
            "Delta",
            comparison,
        )));
        columns.children.push(XMLNode::Element(column_node(
            "Time",
            "SplitTime",
            comparison,
        )));
    } else {
        columns.children.push(XMLNode::Element(column_node(
            "+/-",
            "DeltaorSplitTime",
            comparison,
        )));
    }
    element.children.push(XMLNode::Element(columns));

    element
}

fn column_node(name: &str, column_type: &str, comparison: &str) -> XmlElement {
    let mut element = XmlElement::new("Settings");
    add_text(&mut element, "Version", "1.5");
    add_text(&mut element, "Name", name);
    add_text(&mut element, "Type", column_type);
    add_text(&mut element, "Comparison", comparison);
    add_text(&mut element, "TimingMethod", "Current Timing Method");
    element
}

// ---------------------------------------------------------------------------
// Graph settings XML
// ---------------------------------------------------------------------------

pub fn graph_from_xml(settings: &XmlElement) -> GraphSettings {
    let mut result = GraphSettings::default();
    let _version = parse_version(settings);
    let value = |name: &str| child_text(settings, name);

    result.height = parse_f32(value("Height").as_deref(), result.height);
    result.width = parse_f32(value("Width").as_deref(), result.width);
    result.comparison = value("Comparison").unwrap_or(result.comparison);
    result.graph_color =
        crate::layout::parse_color(value("GraphColor").as_deref(), result.graph_color);
    result.graph_gold_color = crate::layout::parse_color(
        value("GraphGoldColor").as_deref(),
        result.graph_gold_color,
    );
    result.ahead_graph_color = crate::layout::parse_color(
        value("AheadGraphColor").as_deref(),
        result.ahead_graph_color,
    );
    result.behind_graph_color = crate::layout::parse_color(
        value("BehindGraphColor").as_deref(),
        result.behind_graph_color,
    );
    result.graph_lines_color = crate::layout::parse_color(
        value("GraphLinesColor").as_deref(),
        result.graph_lines_color,
    );
    result.gridlines_color = crate::layout::parse_color(
        value("GridlinesColor").as_deref(),
        result.gridlines_color,
    );

    // Version 1.2 split the fill colors into ahead/behind variants.  Older
    // layouts use the shared PartialFillColor/CompleteFillColor tags.
    let partial_behind = value("PartialFillColorBehind")
        .or_else(|| value("PartialFillColor"));
    let complete_behind = value("CompleteFillColorBehind")
        .or_else(|| value("CompleteFillColor"));
    let partial_ahead = value("PartialFillColorAhead")
        .or_else(|| value("PartialFillColor"));
    let complete_ahead = value("CompleteFillColorAhead")
        .or_else(|| value("CompleteFillColor"));
    result.partial_fill_color_behind = crate::layout::parse_color(
        partial_behind.as_deref(),
        result.partial_fill_color_behind,
    );
    result.complete_fill_color_behind = crate::layout::parse_color(
        complete_behind.as_deref(),
        result.complete_fill_color_behind,
    );
    result.partial_fill_color_ahead = crate::layout::parse_color(
        partial_ahead.as_deref(),
        result.partial_fill_color_ahead,
    );
    result.complete_fill_color_ahead = crate::layout::parse_color(
        complete_ahead.as_deref(),
        result.complete_fill_color_ahead,
    );

    result.flip_graph = parse_bool(value("FlipGraph").as_deref(), result.flip_graph);
    result.show_best_segments = parse_bool(
        value("ShowBestSegments").as_deref(),
        result.show_best_segments,
    );
    result.live_graph = parse_bool(value("LiveGraph").as_deref(), result.live_graph);
    result.background = parse_background_style(settings, result.background);

    result
}

pub fn graph_to_xml(settings: &GraphSettings) -> XmlElement {
    let mut element = XmlElement::new("Settings");
    add_text(&mut element, "Version", "1.5");
    add_text(&mut element, "Height", settings.height.to_string());
    add_text(&mut element, "Width", settings.width.to_string());
    add_color(
        &mut element,
        "BehindGraphColor",
        settings.behind_graph_color,
    );
    add_color(&mut element, "AheadGraphColor", settings.ahead_graph_color);
    add_color(&mut element, "GridlinesColor", settings.gridlines_color);
    add_color(
        &mut element,
        "PartialFillColorBehind",
        settings.partial_fill_color_behind,
    );
    add_color(
        &mut element,
        "CompleteFillColorBehind",
        settings.complete_fill_color_behind,
    );
    add_color(
        &mut element,
        "PartialFillColorAhead",
        settings.partial_fill_color_ahead,
    );
    add_color(
        &mut element,
        "CompleteFillColorAhead",
        settings.complete_fill_color_ahead,
    );
    add_color(&mut element, "GraphColor", settings.graph_color);
    add_color(&mut element, "GraphGoldColor", settings.graph_gold_color);
    add_color(&mut element, "ShadowsColor", Color32::from_rgba_unmultiplied(0, 0, 0, 0x38));
    add_color(
        &mut element,
        "GraphLinesColor",
        settings.graph_lines_color,
    );
    add_bool(&mut element, "LiveGraph", settings.live_graph);
    add_bool(&mut element, "FlipGraph", settings.flip_graph);
    add_text(&mut element, "Comparison", settings.comparison.clone());
    add_bool(
        &mut element,
        "ShowBestSegments",
        settings.show_best_segments,
    );
    write_background_style(&mut element, &settings.background);
    element
}

// ---------------------------------------------------------------------------
// Subsplits settings XML
// ---------------------------------------------------------------------------

fn subsplits_splits_default() -> SplitsSettings {
    let mut splits = SplitsSettings::default();
    // LiveSplit's Subsplits plugin defaults.
    splits.visual_split_count = 8;
    splits.show_thin_separators = false;
    splits.show_blank_splits = true;
    splits.show_column_labels = false;
    splits
}

pub fn subsplits_from_xml(settings: &XmlElement) -> SubsplitsSettings {
    let version = parse_version(settings);
    let splits = parse_splits_settings(settings, subsplits_splits_default());
    let mut result = SubsplitsSettings {
        splits,
        ..SubsplitsSettings::default()
    };
    let value = |name: &str| child_text(settings, name);

    result.show_subsplits = parse_bool(value("ShowSubsplits").as_deref(), false);
    result.show_header = parse_bool(value("ShowHeader").as_deref(), true);
    result.header_text = parse_header_text(settings);
    result.header_top_color = crate::layout::parse_color(
        value("HeaderTopColor").as_deref(),
        result.header_top_color,
    );
    result.header_bottom_color = crate::layout::parse_color(
        value("HeaderBottomColor").as_deref(),
        result.header_bottom_color,
    );
    result.header_gradient = GradientType::parse(
        value("HeaderGradient").as_deref().unwrap_or_default(),
        result.header_gradient,
    );
    result.header_text_color = crate::layout::parse_color(
        value("HeaderTextColor").as_deref(),
        result.header_text_color,
    );
    result.header_times_color = crate::layout::parse_color(
        value("HeaderTimesColor").as_deref(),
        result.header_times_color,
    );
    result.header_accuracy = parse_accuracy(
        value("HeaderAccuracy").as_deref(),
        result.header_accuracy,
    );
    result.header_timing_method = TimingMethodSetting::parse(
        value("HeaderTimingMethod")
            .as_deref()
            .unwrap_or("Current Timing Method"),
        result.header_timing_method,
    );

    let header_comparison = value("HeaderComparison")
        .or_else(|| {
            if !version_at_least(version, (1, 7)) {
                value("Comparison")
            } else {
                None
            }
        })
        .or_else(|| Some(result.splits.comparison.clone()));
    result.header_comparison = header_comparison
        .filter(|comparison| !comparison.trim().is_empty())
        .unwrap_or_else(|| CURRENT_COMPARISON.to_string());

    result.current_section_only = parse_bool(
        value("CurrentSectionOnly").as_deref(),
        result.current_section_only,
    );
    result.minimum_major_splits = parse_i32(
        value("MinimumMajorSplits").as_deref(),
        result.minimum_major_splits,
    );

    result
}

fn parse_header_text(element: &XmlElement) -> String {
    if let Some(custom) = child_text(element, "HeaderTextValue") {
        if !custom.trim().is_empty() {
            return custom;
        }
    }
    match child_text(element, "HeaderText") {
        Some(text) if text.eq_ignore_ascii_case("true") => String::new(),
        Some(text) if text.eq_ignore_ascii_case("false") => HEADER_TEXT_HIDDEN.to_string(),
        Some(text) if !text.trim().is_empty() => text,
        _ => String::new(),
    }
}

pub fn subsplits_to_xml(settings: &SubsplitsSettings) -> XmlElement {
    let mut element = splits_settings_node(&settings.splits, "1.7");

    // The Subsplits plugin uses the singular spelling.
    add_bool(
        &mut element,
        "AutomaticAbbreviation",
        settings.splits.automatic_abbreviations,
    );
    add_text(
        &mut element,
        "MinimumMajorSplits",
        settings.minimum_major_splits.to_string(),
    );
    add_bool(&mut element, "ShowSubsplits", settings.show_subsplits);
    add_bool(
        &mut element,
        "CurrentSectionOnly",
        settings.current_section_only,
    );
    add_bool(&mut element, "ShowHeader", settings.show_header);
    add_text(
        &mut element,
        "HeaderComparison",
        settings.header_comparison.clone(),
    );
    add_text(
        &mut element,
        "HeaderTimingMethod",
        settings.header_timing_method.label(),
    );
    add_color(&mut element, "HeaderTopColor", settings.header_top_color);
    add_color(
        &mut element,
        "HeaderBottomColor",
        settings.header_bottom_color,
    );
    add_text(
        &mut element,
        "HeaderGradient",
        gradient_xml_label(settings.header_gradient),
    );
    add_color(
        &mut element,
        "HeaderTextColor",
        settings.header_text_color,
    );
    add_color(
        &mut element,
        "HeaderTimesColor",
        settings.header_times_color,
    );
    add_text(
        &mut element,
        "HeaderAccuracy",
        settings.header_accuracy.label(),
    );
    add_bool(
        &mut element,
        "HeaderText",
        settings.header_text != HEADER_TEXT_HIDDEN,
    );
    if settings.header_text != HEADER_TEXT_HIDDEN && !settings.header_text.is_empty() {
        add_text(
            &mut element,
            "HeaderTextValue",
            settings.header_text.clone(),
        );
    }
    add_bool(&mut element, "HeaderTimes", true);

    // Fields that exist in the original plugin but not in the Rust settings
    // model.  Persisting the defaults keeps layouts usable in LiveSplit.
    add_bool(&mut element, "IndentBlankIcons", true);
    add_bool(&mut element, "IndentSubsplits", true);
    add_bool(&mut element, "HideSubsplits", false);
    add_bool(&mut element, "OverrideSubsplitColor", false);
    add_color(
        &mut element,
        "SubsplitTopColor",
        Color32::from_rgba_unmultiplied(0, 0, 0, 0x8D),
    );
    add_color(
        &mut element,
        "SubsplitBottomColor",
        Color32::TRANSPARENT,
    );
    add_text(&mut element, "SubsplitGradient", "Plain");
    add_bool(&mut element, "IndentSectionSplit", true);
    add_bool(&mut element, "ShowIconSectionSplit", true);
    add_bool(&mut element, "ShowSectionIcon", true);
    add_bool(&mut element, "SectionTimer", true);
    add_color(
        &mut element,
        "SectionTimerColor",
        Color32::from_rgb(0x77, 0x77, 0x77),
    );
    add_bool(&mut element, "SectionTimerGradient", true);
    add_text(&mut element, "SectionTimerAccuracy", "Tenths");

    element
}

// ---------------------------------------------------------------------------
// Layout helpers
// ---------------------------------------------------------------------------

/// Height of one split row in points.  LiveSplit stores the split height in
/// units of ten pixels.
pub fn row_height(settings: &SplitsSettings) -> f32 {
    let pixels = settings.split_height * 10.0;
    if !pixels.is_finite() {
        36.0
    } else {
        pixels.clamp(12.0, 250.0)
    }
}

fn visible_slots(settings: &SplitsSettings, segment_count: usize) -> usize {
    let capacity = settings.visual_split_count.max(1) as usize;
    if settings.show_blank_splits {
        capacity
    } else {
        segment_count.min(capacity).max(1)
    }
}

pub fn splits_height(timer: &livesplit_core::Timer, settings: &SplitsSettings, scale: f32) -> f32 {
    let scale = scale.max(0.1);
    let slots = visible_slots(settings, timer.run.segments.len());
    let label_rows = usize::from(settings.show_column_labels);
    (slots + label_rows) as f32 * row_height(settings) * scale
}

struct ColumnGeometry {
    right: f32,
    time_right: f32,
    delta_right: f32,
    delta_width: f32,
    single_width: f32,
    gap: f32,
    show_time_column: bool,
}

impl ColumnGeometry {
    fn name_right(&self) -> f32 {
        if self.show_time_column {
            self.delta_right - self.delta_width - self.gap
        } else {
            self.right - self.single_width - self.gap
        }
    }
}

fn build_formatters(settings: &SplitsSettings) -> (GeneralTimeFormatter, DeltaTimeFormatter) {
    let time_formatter = GeneralTimeFormatter {
        accuracy: settings.split_times_accuracy,
        digits_format: DigitsFormat::SingleDigitSeconds,
        null_format: NullFormat::Dash,
        ..GeneralTimeFormatter::default()
    };
    let mut delta_formatter = DeltaTimeFormatter::default();
    delta_formatter.set_accuracy(settings.deltas_accuracy);
    delta_formatter.set_drop_decimals(settings.drop_decimals);
    (time_formatter, delta_formatter)
}

fn column_geometry(
    painter: &egui::Painter,
    rect: Rect,
    settings: &SplitsSettings,
    font: &egui::FontId,
    time_formatter: &GeneralTimeFormatter,
    delta_formatter: &DeltaTimeFormatter,
    scale: f32,
) -> ColumnGeometry {
    let time_sample = time_formatter.format(Some(TimeSpan::from_seconds(9 * 3600)));
    let delta_sample = delta_formatter.format(Some(TimeSpan::from_seconds(9 * 3600)));
    let time_width = text_width(painter, &time_sample, font);
    let delta_width = text_width(painter, &delta_sample, font);
    let gap = 5.0 * scale;
    let right = rect.right() - 7.0 * scale;
    let show_time_column = settings.show_split_times;
    let time_right = right;
    let delta_right = if show_time_column {
        right - time_width - gap
    } else {
        right
    };
    ColumnGeometry {
        right,
        time_right,
        delta_right,
        delta_width,
        single_width: time_width.max(delta_width),
        gap,
        show_time_column,
    }
}

fn paint_splits_background(painter: &egui::Painter, rect: Rect, settings: &SplitsSettings) {
    if settings.background.gradient != GradientType::Alternating {
        paint_background(painter, rect, &settings.background);
    }
}

fn paint_row_background(
    painter: &egui::Painter,
    rect: Rect,
    settings: &SplitsSettings,
    parity: usize,
) {
    if settings.background.gradient == GradientType::Alternating {
        let color = if parity % 2 == 0 {
            settings.background.color
        } else {
            settings.background.color2
        };
        if color.a() > 0 {
            painter.rect_filled(rect, 0.0, color);
        }
    }
}

fn paint_current_split(painter: &egui::Painter, rect: Rect, settings: &SplitsSettings) {
    if gradient_is_plain(settings.current_split_gradient) {
        if settings.current_split_top_color.a() > 0 {
            painter.rect_filled(rect, 0.0, settings.current_split_top_color);
        }
    } else {
        paint_gradient(
            painter,
            rect,
            settings.current_split_top_color,
            settings.current_split_bottom_color,
            settings.current_split_gradient.is_horizontal(),
        );
    }
}

fn paint_header(painter: &egui::Painter, rect: Rect, settings: &SubsplitsSettings) {
    if gradient_is_plain(settings.header_gradient) {
        if settings.header_top_color.a() > 0 {
            painter.rect_filled(rect, 0.0, settings.header_top_color);
        }
    } else {
        paint_gradient(
            painter,
            rect,
            settings.header_top_color,
            settings.header_bottom_color,
            settings.header_gradient.is_horizontal(),
        );
    }
}

fn draw_column_labels(
    ui: &mut Ui,
    rect: Rect,
    ctx: &ComponentContext,
    settings: &SplitsSettings,
    geometry: &ColumnGeometry,
) {
    let painter = ui.painter();
    if settings.background.gradient == GradientType::Alternating {
        let color = settings.background.color;
        if color.a() > 0 {
            painter.rect_filled(rect, 0.0, color);
        }
    }

    let font = ctx.text_font();
    let color = settings.labels_color;
    let shadow = if ctx.theme.drop_shadows {
        Some(ctx.theme.shadows_color)
    } else {
        None
    };
    let outline = Some(ctx.theme.text_outline_color);
    let center_y = rect.center().y;
    if geometry.show_time_column {
        paint_text(
            painter,
            Pos2::new(geometry.delta_right, center_y),
            Align2::RIGHT_CENTER,
            "+/-",
            font.clone(),
            color,
            shadow,
            outline,
        );
        paint_text(
            painter,
            Pos2::new(geometry.time_right, center_y),
            Align2::RIGHT_CENTER,
            "Time",
            font,
            color,
            shadow,
            outline,
        );
    } else {
        paint_text(
            painter,
            Pos2::new(geometry.right, center_y),
            Align2::RIGHT_CENTER,
            "+/-",
            font,
            color,
            shadow,
            outline,
        );
    }
}

fn name_abbreviations(name: &str) -> Vec<String> {
    let mut result = Vec::new();
    result.push(name.to_string());
    let words: Vec<&str> = name.split_whitespace().collect();
    if words.len() > 1 {
        let initials: String = words
            .iter()
            .filter_map(|word| word.chars().next())
            .collect();
        result.push(initials);
        let mut mixed = String::new();
        for word in &words[..words.len() - 1] {
            if let Some(character) = word.chars().next() {
                mixed.push(character);
            }
        }
        mixed.push_str(words[words.len() - 1]);
        result.push(mixed);
    }
    result.push(name.chars().filter(|character| !character.is_whitespace()).collect());
    result.push(
        name.chars()
            .filter(|character| {
                !matches!(
                    character.to_ascii_lowercase(),
                    'a' | 'e' | 'i' | 'o' | 'u'
                )
            })
            .collect(),
    );
    result
}

fn display_name(
    painter: &egui::Painter,
    name: &str,
    font: &egui::FontId,
    max_width: f32,
    automatic: bool,
) -> String {
    if automatic {
        for candidate in name_abbreviations(name) {
            if candidate.is_empty() {
                continue;
            }
            if text_width(painter, &candidate, font) <= max_width {
                return candidate;
            }
        }
    }
    fit_text(painter, name, font, max_width)
}

#[allow(clippy::too_many_arguments)]
fn draw_split_row(
    ui: &mut Ui,
    row: Rect,
    ctx: &mut ComponentContext,
    settings: &SplitsSettings,
    geometry: &ColumnGeometry,
    segment_index: usize,
    name: &str,
    parity: usize,
    indent: f32,
    active: bool,
    reserve_icon: bool,
    time_formatter: &GeneralTimeFormatter,
    delta_formatter: &DeltaTimeFormatter,
) {
    let scale = ctx.scale.max(0.1);
    let timer = ctx.timer;
    let segment = &timer.run.segments[segment_index];

    {
        let painter = ui.painter();
        paint_row_background(painter, row, settings, parity);
        if active {
            paint_current_split(painter, row, settings);
        }
    }

    let icon_size = if reserve_icon {
        (settings.icon_size * scale)
            .clamp(0.0, (row.height() - 2.0 * scale).max(0.0))
    } else {
        0.0
    };
    let icon_column = if reserve_icon {
        icon_size + 7.0 * scale
    } else {
        0.0
    };

    if reserve_icon {
        if let Some(bytes) = segment.icon.as_deref() {
            if !bytes.is_empty() {
                let texture = ctx.icons.texture(ui.ctx(), bytes);
                if let Some(texture) = texture {
                    let painter = ui.painter();
                    let icon_rect = Rect::from_center_size(
                        Pos2::new(
                            row.left() + 7.0 * scale + indent + icon_size * 0.5,
                            row.center().y,
                        ),
                        Vec2::splat(icon_size),
                    );
                    let uv = Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1.0, 1.0));
                    if settings.icon_shadows {
                        painter.image(
                            texture,
                            icon_rect.translate(Vec2::new(2.0 * scale, 2.0 * scale)),
                            uv,
                            ctx.theme.shadows_color,
                        );
                    }
                    painter.image(texture, icon_rect, uv, Color32::WHITE);
                }
            }
        }
    }

    let font = ctx.text_font();
    let times_font = ctx.times_font();
    let text_left = row.left() + 7.0 * scale + icon_column + indent;
    let name_width = if settings.display_2_rows {
        (geometry.right - text_left).max(6.0)
    } else {
        (geometry.name_right() - text_left).max(6.0)
    };
    let shadow = if ctx.theme.drop_shadows {
        Some(ctx.theme.shadows_color)
    } else {
        None
    };
    let outline = Some(ctx.theme.text_outline_color);

    let raw_current = timer.current_split_index;
    let completed_limit = raw_current.max(0);
    let display_current = if raw_current < 0 {
        0usize
    } else {
        (raw_current as usize).min(timer.run.segments.len().saturating_sub(1))
    };
    let completed = (segment_index as i32) < completed_limit;
    let color_state = if completed {
        0u8
    } else if segment_index == display_current {
        1u8
    } else {
        2u8
    };

    let name_color = if settings.override_text_color {
        match color_state {
            0 => settings.before_names_color,
            1 => settings.current_names_color,
            _ => settings.after_names_color,
        }
    } else {
        ctx.theme.text_color
    };
    let time_color = if settings.override_times_color {
        match color_state {
            0 => settings.before_times_color,
            1 => settings.current_times_color,
            _ => settings.after_times_color,
        }
    } else {
        ctx.theme.text_color
    };

    let method = timer.current_timing_method;
    let comparison = ctx.resolve_comparison(&settings.comparison).to_string();
    let split_value = segment.split_time.get(method);
    let comparison_value = segment.comparison(&comparison).get(method);
    let delta_value = split_value
        .zip(comparison_value)
        .map(|(split, comparison)| split - comparison);

    let mut delta_text = String::new();
    let time_text;
    let mut combined_displayed_delta = false;
    let mut delta_color = ctx.theme.text_color;

    if completed {
        if geometry.show_time_column {
            time_text = time_formatter.format(split_value);
            if let Some(delta) = delta_value {
                delta_text = delta_formatter.format(Some(delta));
                delta_color = analysis::split_color(
                    timer,
                    Some(delta),
                    segment_index as i32,
                    true,
                    ctx.theme.show_best_segments,
                    &comparison,
                    method,
                )
                .map(|color| ctx.theme.color_for_split(color))
                .unwrap_or(ctx.theme.text_color);
            }
        } else if let Some(delta) = delta_value {
            time_text = delta_formatter.format(Some(delta));
            combined_displayed_delta = true;
            delta_color = analysis::split_color(
                timer,
                Some(delta),
                segment_index as i32,
                true,
                ctx.theme.show_best_segments,
                &comparison,
                method,
            )
            .map(|color| ctx.theme.color_for_split(color))
            .unwrap_or(ctx.theme.text_color);
        } else {
            time_text = time_formatter.format(split_value);
        }
    } else {
        let live_delta = if active {
            analysis::check_live_delta(
                timer,
                true,
                &comparison,
                method,
                ctx.theme.show_best_segments,
            )
        } else {
            None
        };
        if geometry.show_time_column {
            time_text = time_formatter.format(comparison_value);
            if let Some(delta) = live_delta {
                delta_text = delta_formatter.format(Some(delta));
                delta_color = if settings.override_deltas_color {
                    settings.deltas_color
                } else {
                    ctx.theme.text_color
                };
            }
        } else if let Some(delta) = live_delta {
            time_text = delta_formatter.format(Some(delta));
            combined_displayed_delta = true;
            delta_color = if settings.override_deltas_color {
                settings.deltas_color
            } else {
                ctx.theme.text_color
            };
        } else {
            time_text = time_formatter.format(comparison_value);
        }
    }

    let painter = ui.painter();
    let (name_y, column_y) = if settings.display_2_rows {
        (
            row.top() + row.height() * 0.28,
            row.top() + row.height() * 0.76,
        )
    } else {
        (row.center().y, row.center().y)
    };
    let name_y = name_y.clamp(row.top(), row.bottom());
    let column_y = column_y.clamp(row.top(), row.bottom());

    let displayed_name = display_name(
        painter,
        name,
        &font,
        name_width,
        settings.automatic_abbreviations,
    );
    paint_text(
        painter,
        Pos2::new(text_left, name_y),
        Align2::LEFT_CENTER,
        &displayed_name,
        font,
        name_color,
        shadow,
        outline,
    );

    if geometry.show_time_column {
        paint_text(
            painter,
            Pos2::new(geometry.time_right, column_y),
            Align2::RIGHT_CENTER,
            &time_text,
            times_font.clone(),
            time_color,
            shadow,
            outline,
        );
        if !delta_text.is_empty() {
            paint_text(
                painter,
                Pos2::new(geometry.delta_right, column_y),
                Align2::RIGHT_CENTER,
                &delta_text,
                times_font,
                delta_color,
                shadow,
                outline,
            );
        }
    } else {
        let color = if combined_displayed_delta {
            delta_color
        } else {
            time_color
        };
        paint_text(
            painter,
            Pos2::new(geometry.right, column_y),
            Align2::RIGHT_CENTER,
            &time_text,
            times_font,
            color,
            shadow,
            outline,
        );
    }
}

fn build_split_rows(
    settings: &SplitsSettings,
    segment_count: usize,
    current: usize,
) -> (Vec<Option<usize>>, Option<usize>) {
    if segment_count == 0 {
        return (vec![None], None);
    }

    let slots = visible_slots(settings, segment_count);
    let preview = settings.split_preview_count.max(0) as usize;
    let pin_last = settings.always_show_last_split;
    let mut rows = Vec::with_capacity(slots);

    if pin_last {
        if slots <= 1 {
            rows.push(Some(segment_count - 1));
            return (rows, Some(0));
        }
        let scroll_slots = slots - 1;
        let main_count = segment_count - 1;
        if main_count <= scroll_slots {
            for index in 0..main_count {
                rows.push(Some(index));
            }
            if settings.lock_last_split {
                while rows.len() < scroll_slots {
                    rows.push(None);
                }
                let last_row = rows.len();
                rows.push(Some(segment_count - 1));
                (rows, Some(last_row))
            } else {
                let last_row = rows.len();
                rows.push(Some(segment_count - 1));
                while rows.len() < slots {
                    rows.push(None);
                }
                (rows, Some(last_row))
            }
        } else {
            let target = slots.saturating_sub(2 + preview);
            let current_main = current.min(main_count - 1);
            let max_start = main_count - scroll_slots;
            let start = current_main.saturating_sub(target).min(max_start);
            for index in start..start + scroll_slots {
                rows.push(Some(index));
            }
            let last_row = rows.len();
            rows.push(Some(segment_count - 1));
            (rows, Some(last_row))
        }
    } else {
        let target = slots.saturating_sub(3 + preview);
        let max_start = segment_count.saturating_sub(slots);
        let start = current.saturating_sub(target).min(max_start);
        for index in start..(start + slots).min(segment_count) {
            rows.push(Some(index));
        }
        while rows.len() < slots {
            rows.push(None);
        }
        (rows, None)
    }
}

pub fn draw_splits(ui: &mut Ui, rect: Rect, ctx: &mut ComponentContext, settings: &SplitsSettings) {
    if rect.width() <= 0.0 || rect.height() <= 0.0 {
        return;
    }
    let scale = ctx.scale.max(0.1);
    let row_height = row_height(settings) * scale;
    if !row_height.is_finite() || row_height <= 0.0 {
        return;
    }

    let segment_count = ctx.timer.run.segments.len();
    let painter = ui.painter();
    paint_splits_background(painter, rect, settings);
    if segment_count == 0 {
        return;
    }

    let label_height = if settings.show_column_labels {
        row_height
    } else {
        0.0
    };
    let table_top = rect.top() + label_height;
    let available_rows = ((rect.bottom() - table_top) / row_height)
        .floor()
        .max(0.0) as usize;
    if available_rows == 0 {
        return;
    }

    let (time_formatter, delta_formatter) = build_formatters(settings);
    let times_font = ctx.times_font();
    let geometry = column_geometry(
        ui.painter(),
        rect,
        settings,
        &times_font,
        &time_formatter,
        &delta_formatter,
        scale,
    );

    if settings.show_column_labels {
        let label_rect = Rect::from_min_size(
            rect.min,
            Vec2::new(rect.width(), row_height),
        );
        draw_column_labels(ui, label_rect, ctx, settings, &geometry);
    }

    let raw_current = ctx.timer.current_split_index;
    let current = if raw_current < 0 {
        0
    } else {
        (raw_current as usize).min(segment_count - 1)
    };
    let (rows, last_row) = build_split_rows(settings, segment_count, current);
    let reserve_icon = settings.display_icons
        && ctx
            .timer
            .run
            .segments
            .iter()
            .any(|segment| segment.icon.as_deref().map_or(false, |icon| !icon.is_empty()));
    let active_phase = matches!(ctx.timer.phase, TimerPhase::Running | TimerPhase::Paused);

    for (visible_index, row_segment) in rows.iter().take(available_rows).enumerate() {
        let row = Rect::from_min_size(
            Pos2::new(
                rect.left(),
                table_top + visible_index as f32 * row_height,
            ),
            Vec2::new(rect.width(), row_height),
        );
        if row.top() >= rect.bottom() {
            break;
        }

        let is_last_separator = last_row == Some(visible_index)
            && settings.always_show_last_split
            && visible_index > 0;

        if let Some(segment_index) = *row_segment {
            let active = active_phase
                && raw_current >= 0
                && segment_index == raw_current as usize;
            let parity = segment_index + usize::from(settings.show_column_labels);
            let name = ctx.timer.run.segments[segment_index].name.clone();
            draw_split_row(
                ui,
                row,
                ctx,
                settings,
                &geometry,
                segment_index,
                &name,
                parity,
                0.0,
                active,
                reserve_icon,
                &time_formatter,
                &delta_formatter,
            );
        } else {
            // Blank split rows use the first alternating color.
            paint_row_background(ui.painter(), row, settings, 0);
        }

        // Separators are painted last so the row/gradient fills do not cover
        // them.  This mirrors LiveSplit's dedicated separator components.
        if is_last_separator && settings.separator_last_split {
            ui.painter().line_segment(
                [row.left_top(), row.right_top()],
                Stroke::new(2.0 * scale, ctx.theme.separators_color),
            );
        } else if settings.show_thin_separators && visible_index > 0 {
            ui.painter().line_segment(
                [row.left_top(), row.right_top()],
                Stroke::new(1.0 * scale, ctx.theme.thin_separators_color),
            );
        }
    }

    if settings.show_column_labels {
        ui.painter().line_segment(
            [
                Pos2::new(rect.left(), table_top),
                Pos2::new(rect.right(), table_top),
            ],
            Stroke::new(2.0 * scale, ctx.theme.separators_color),
        );
    }
}

// ---------------------------------------------------------------------------
// Graph rendering
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
struct GraphPoint {
    position: Pos2,
    delta: f64,
    segment: Option<usize>,
    live: bool,
}

pub fn draw_graph(ui: &mut Ui, rect: Rect, ctx: &mut ComponentContext, settings: &GraphSettings) {
    if rect.width() <= 1.0 || rect.height() <= 1.0 {
        return;
    }
    let painter = ui.painter();
    paint_background(painter, rect, &settings.background);

    let timer = ctx.timer;
    let method = timer.current_timing_method;
    let comparison = ctx.resolve_comparison(&settings.comparison).to_string();
    let scale = ctx.scale.max(0.1);
    let segment_count = timer.run.segments.len();
    if segment_count == 0 {
        return;
    }

    let mut completed: Vec<(usize, f64, TimeSpan)> = Vec::new();
    let mut final_time = TimeSpan::ZERO;
    for (index, segment) in timer.run.segments.iter().enumerate() {
        if let (Some(split), Some(target)) = (
            segment.split_time.get(method),
            segment.comparison(&comparison).get(method),
        ) {
            completed.push((index, (split - target).as_secs_f64(), split));
            if split > final_time {
                final_time = split;
            }
        }
    }

    let running = matches!(timer.phase, TimerPhase::Running | TimerPhase::Paused);
    let mut live_delta = None;
    if settings.live_graph && running {
        if let Some(current_time) = timer.current_time().get(method) {
            final_time = current_time;
            let index = timer.current_split_index;
            if index >= 0 {
                if let Some(segment) = timer.run.segments.get(index as usize) {
                    if let Some(target) = segment.comparison(&comparison).get(method) {
                        live_delta = Some((current_time - target).as_secs_f64());
                    }
                }
            }
        }
    }

    let final_seconds = final_time.as_secs_f64().max(0.000_001);
    let mut max_abs = 0.5_f64;
    for (_, delta, _) in &completed {
        max_abs = max_abs.max(delta.abs());
    }
    if let Some(delta) = live_delta {
        max_abs = max_abs.max(delta.abs());
    }

    let center = rect.center().y;
    let plot_half = (rect.height() * 0.5 - 4.0 * scale).max(2.0);
    let y_for = |delta: f64| {
        let fraction = (delta / max_abs) as f32;
        let y = if settings.flip_graph {
            center + fraction * plot_half
        } else {
            center - fraction * plot_half
        };
        y.clamp(rect.top() + 1.0, rect.bottom() - 1.0)
    };

    let (top_color, bottom_color) = if settings.flip_graph {
        (settings.ahead_graph_color, settings.behind_graph_color)
    } else {
        (settings.behind_graph_color, settings.ahead_graph_color)
    };
    if top_color.a() > 0 {
        painter.rect_filled(
            Rect::from_min_max(rect.min, Pos2::new(rect.right(), center)),
            0.0,
            top_color,
        );
    }
    if bottom_color.a() > 0 {
        painter.rect_filled(
            Rect::from_min_max(Pos2::new(rect.left(), center), rect.max),
            0.0,
            bottom_color,
        );
    }

    // Grid lines.  Pick intervals that keep the grid readable at any size.
    let width_steps = (rect.width() / (30.0 * scale)).max(1.0) as f64;
    let mut x_interval = 1.0_f64;
    while final_seconds / x_interval > width_steps {
        x_interval *= 6.0;
    }
    let mut x = x_interval;
    while x < final_seconds {
        let px = rect.left() + (x / final_seconds) as f32 * rect.width();
        painter.line_segment(
            [
                Pos2::new(px, rect.top()),
                Pos2::new(px, rect.bottom()),
            ],
            Stroke::new(1.0 * scale, settings.gridlines_color),
        );
        x += x_interval;
    }

    let height_steps = (plot_half / (20.0 * scale)).max(1.0) as f64;
    let mut y_interval = 1.0_f64;
    while max_abs / y_interval > height_steps {
        y_interval *= 6.0;
    }
    let mut step = 1.0_f64;
    while step * y_interval <= max_abs + 0.000_001 {
        let offset = (step * y_interval / max_abs) as f32 * plot_half;
        for y in [center - offset, center + offset] {
            if y > rect.top() && y < rect.bottom() {
                painter.line_segment(
                    [Pos2::new(rect.left(), y), Pos2::new(rect.right(), y)],
                    Stroke::new(1.0 * scale, settings.gridlines_color),
                );
            }
        }
        step += 1.0;
    }

    painter.line_segment(
        [
            Pos2::new(rect.left(), center),
            Pos2::new(rect.right(), center),
        ],
        Stroke::new(1.0 * scale, settings.graph_lines_color),
    );

    // Build the polyline.  A virtual origin at zero keeps the fill anchored to
    // the center line even when the first split is already ahead/behind.
    let mut points: Vec<GraphPoint> = Vec::new();
    points.push(GraphPoint {
        position: Pos2::new(rect.left(), center),
        delta: 0.0,
        segment: None,
        live: false,
    });
    for (index, delta, split) in &completed {
        let fraction = (split.as_secs_f64() / final_seconds) as f32;
        let x = rect.left() + fraction.clamp(0.0, 1.0) * rect.width();
        points.push(GraphPoint {
            position: Pos2::new(x, y_for(*delta)),
            delta: *delta,
            segment: Some(*index),
            live: false,
        });
    }
    if let Some(delta) = live_delta {
        let segment = if timer.current_split_index >= 0 {
            Some(timer.current_split_index as usize)
        } else {
            None
        };
        points.push(GraphPoint {
            position: Pos2::new(rect.right(), y_for(delta)),
            delta,
            segment,
            live: true,
        });
    }

    let center_x = |point: &GraphPoint| Pos2::new(point.position.x, center);
    for pair in points.windows(2) {
        let first = pair[0];
        let second = pair[1];
        let partial = first.live || second.live;
        let fill_for = |delta: f64| {
            if delta <= 0.0 {
                if partial {
                    settings.partial_fill_color_ahead
                } else {
                    settings.complete_fill_color_ahead
                }
            } else if partial {
                settings.partial_fill_color_behind
            } else {
                settings.complete_fill_color_behind
            }
        };

        if (first.delta <= 0.0 && second.delta <= 0.0)
            || (first.delta > 0.0 && second.delta > 0.0)
        {
            let color = fill_for(second.delta);
            if color.a() > 0 {
                painter.add(egui::Shape::convex_polygon(
                    vec![
                        first.position,
                        second.position,
                        center_x(&second),
                        center_x(&first),
                    ],
                    color,
                    Stroke::NONE,
                ));
            }
        } else if (second.delta - first.delta).abs() > f64::EPSILON {
            let ratio = (-first.delta) / (second.delta - first.delta);
            let cross_x = first.position.x
                + (second.position.x - first.position.x) * ratio as f32;
            let crossing = Pos2::new(cross_x, center);
            let first_color = fill_for(first.delta);
            let second_color = fill_for(second.delta);
            if first_color.a() > 0 {
                painter.add(egui::Shape::convex_polygon(
                    vec![first.position, crossing, center_x(&first)],
                    first_color,
                    Stroke::NONE,
                ));
            }
            if second_color.a() > 0 {
                painter.add(egui::Shape::convex_polygon(
                    vec![crossing, second.position, center_x(&second)],
                    second_color,
                    Stroke::NONE,
                ));
            }
        }
    }

    let shadow = if ctx.theme.drop_shadows {
        Some(ctx.theme.shadows_color)
    } else {
        None
    };
    let best_segment = |index: usize| {
        settings.show_best_segments
            && analysis::check_best_segment(timer, index as i32, method)
    };

    for pair in points.windows(2) {
        let second = pair[1];
        let color = if second.live {
            if second.delta <= 0.0 {
                settings.ahead_graph_color
            } else {
                settings.behind_graph_color
            }
        } else if second
            .segment
            .map(best_segment)
            .unwrap_or(false)
        {
            settings.graph_gold_color
        } else {
            settings.graph_color
        };
        let width = 2.0 * scale;
        if let Some(shadow) = shadow.filter(|color| color.a() > 0) {
            painter.line_segment(
                [
                    pair[0].position + Vec2::new(1.5 * scale, 2.0 * scale),
                    second.position + Vec2::new(1.5 * scale, 2.0 * scale),
                ],
                Stroke::new(width, shadow),
            );
        }
        painter.line_segment(
            [pair[0].position, second.position],
            Stroke::new(width, color),
        );
    }

    for point in points.iter().skip(1) {
        let color = if point.live {
            if point.delta <= 0.0 {
                settings.ahead_graph_color
            } else {
                settings.behind_graph_color
            }
        } else if point.segment.map(best_segment).unwrap_or(false) {
            settings.graph_gold_color
        } else {
            settings.graph_color
        };
        if let Some(shadow) = shadow.filter(|color| color.a() > 0) {
            painter.circle_filled(
                point.position + Vec2::new(1.5 * scale, 2.0 * scale),
                2.6 * scale,
                shadow,
            );
        }
        painter.circle_filled(point.position, 2.4 * scale, color);
    }
}

// ---------------------------------------------------------------------------
// Subsplits rendering
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
struct Section {
    start: usize,
    end: usize,
}

#[derive(Clone, Copy)]
enum SubRow {
    Header(usize),
    Segment(usize, usize),
    Blank,
}

fn sections(segments: &[livesplit_core::Segment]) -> Vec<Section> {
    let mut result = Vec::new();
    if segments.is_empty() {
        return result;
    }
    let mut end = segments.len() - 1;
    loop {
        let mut start = end;
        while start > 0 && segments[start - 1].name.trim_start().starts_with('-') {
            start -= 1;
        }
        result.push(Section { start, end });
        if start == 0 {
            break;
        }
        end = start - 1;
    }
    result.reverse();
    result
}

fn braced_name(name: &str) -> Option<(&str, &str)> {
    let trimmed = name.trim();
    if !trimmed.starts_with('{') {
        return None;
    }
    let closing = trimmed.find('}')?;
    let section = trimmed[1..closing].trim();
    if section.is_empty() {
        return None;
    }
    Some((section, trimmed[closing + 1..].trim()))
}

fn section_display_name(segment_name: &str) -> String {
    let trimmed = segment_name.trim();
    if let Some((section, _)) = braced_name(trimmed) {
        section.to_string()
    } else {
        trimmed.trim_start_matches('-').to_string()
    }
}

fn segment_display_name(segment_name: &str) -> String {
    let without_dash = segment_name.trim().strip_prefix('-').unwrap_or(segment_name.trim());
    if let Some((_, rest)) = braced_name(without_dash) {
        if rest.is_empty() {
            section_display_name(without_dash)
        } else {
            rest.to_string()
        }
    } else {
        without_dash.to_string()
    }
}

fn header_label(settings: &SubsplitsSettings, derived: &str) -> String {
    if settings.header_text == HEADER_TEXT_HIDDEN {
        return String::new();
    }
    if settings.header_text.trim().is_empty() {
        return derived.to_string();
    }
    settings
        .header_text
        .replace("{section}", derived)
        .replace("{Section}", derived)
        .replace("{name}", derived)
}

#[allow(clippy::too_many_arguments)]
fn draw_header_row(
    ui: &mut Ui,
    row: Rect,
    ctx: &mut ComponentContext,
    settings: &SubsplitsSettings,
    section_index: usize,
    section: Section,
    parity: usize,
    reserve_icon: bool,
) {
    let scale = ctx.scale.max(0.1);
    let timer = ctx.timer;
    let segment = &timer.run.segments[section.end];
    let derived = section_display_name(&segment.name);
    let label = header_label(settings, &derived);

    {
        let painter = ui.painter();
        paint_row_background(painter, row, &settings.splits, parity);
        paint_header(painter, row, settings);
    }

    let icon_size = if reserve_icon {
        (settings.splits.icon_size * scale)
            .clamp(0.0, (row.height() - 2.0 * scale).max(0.0))
    } else {
        0.0
    };
    let icon_column = if reserve_icon {
        icon_size + 7.0 * scale
    } else {
        0.0
    };

    if reserve_icon {
        if let Some(bytes) = segment.icon.as_deref() {
            if !bytes.is_empty() {
                let texture = ctx.icons.texture(ui.ctx(), bytes);
                if let Some(texture) = texture {
                    let painter = ui.painter();
                    let icon_rect = Rect::from_center_size(
                        Pos2::new(
                            row.left() + 7.0 * scale + icon_size * 0.5,
                            row.center().y,
                        ),
                        Vec2::splat(icon_size),
                    );
                    let uv = Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1.0, 1.0));
                    if settings.splits.icon_shadows {
                        painter.image(
                            texture,
                            icon_rect.translate(Vec2::new(2.0 * scale, 2.0 * scale)),
                            uv,
                            ctx.theme.shadows_color,
                        );
                    }
                    painter.image(texture, icon_rect, uv, Color32::WHITE);
                }
            }
        }
    }

    let method = ctx.resolved_timing_method(settings.header_timing_method);
    let comparison = ctx
        .resolve_comparison(&settings.header_comparison)
        .to_string();
    let target_end = segment.comparison(&comparison).get(method);
    let target_start = if section.start > 0 {
        timer.run.segments[section.start - 1]
            .comparison(&comparison)
            .get(method)
    } else {
        Some(TimeSpan::ZERO)
    };
    let target_duration = match (target_end, target_start) {
        (Some(end), Some(start)) => Some(end - start),
        _ => None,
    };

    let start_split = if section.start > 0 {
        timer.run.segments[section.start - 1].split_time.get(method)
    } else {
        Some(TimeSpan::ZERO)
    };
    let end_split = segment.split_time.get(method);
    let raw_current = timer.current_split_index;
    let current = if raw_current < 0 {
        0
    } else {
        (raw_current as usize).min(timer.run.segments.len() - 1)
    };
    let running = matches!(timer.phase, TimerPhase::Running | TimerPhase::Paused);
    let actual_duration = if let (Some(start), Some(end)) = (start_split, end_split) {
        Some(end - start)
    } else if running && current >= section.start && current <= section.end {
        timer
            .current_time()
            .get(method)
            .zip(start_split)
            .map(|(now, start)| now - start)
    } else {
        None
    };

    let header_time_formatter = GeneralTimeFormatter {
        accuracy: settings.header_accuracy,
        digits_format: DigitsFormat::SingleDigitSeconds,
        null_format: NullFormat::Dash,
        ..GeneralTimeFormatter::default()
    };
    let mut delta_formatter = DeltaTimeFormatter::default();
    delta_formatter.set_accuracy(settings.splits.deltas_accuracy);
    delta_formatter.set_drop_decimals(settings.splits.drop_decimals);

    let time_text = if let (Some(actual), Some(target)) = (actual_duration, target_duration) {
        delta_formatter.format(Some(actual - target))
    } else if let Some(actual) = actual_duration {
        header_time_formatter.format(Some(actual))
    } else {
        header_time_formatter.format(target_duration)
    };

    let font = ctx.text_font();
    let times_font = ctx.times_font();
    let shadow = if ctx.theme.drop_shadows {
        Some(ctx.theme.shadows_color)
    } else {
        None
    };
    let outline = Some(ctx.theme.text_outline_color);
    let time_width = text_width(ui.painter(), &time_text, &times_font).max(0.0);
    let text_left = row.left() + 7.0 * scale + icon_column;
    let name_width = (row.right() - 7.0 * scale - time_width - 8.0 * scale - text_left).max(6.0);
    let displayed = display_name(
        ui.painter(),
        &label,
        &font,
        name_width,
        settings.splits.automatic_abbreviations,
    );
    paint_text(
        ui.painter(),
        Pos2::new(text_left, row.center().y),
        Align2::LEFT_CENTER,
        &displayed,
        font,
        settings.header_text_color,
        shadow,
        outline,
    );
    paint_text(
        ui.painter(),
        Pos2::new(row.right() - 7.0 * scale, row.center().y),
        Align2::RIGHT_CENTER,
        &time_text,
        times_font,
        settings.header_times_color,
        shadow,
        outline,
    );
    let _ = section_index;
}

pub fn draw_subsplits(
    ui: &mut Ui,
    rect: Rect,
    ctx: &mut ComponentContext,
    settings: &SubsplitsSettings,
) {
    if !settings.show_subsplits {
        draw_splits(ui, rect, ctx, &settings.splits);
        return;
    }
    if rect.width() <= 0.0 || rect.height() <= 0.0 {
        return;
    }
    let scale = ctx.scale.max(0.1);
    let row_height = row_height(&settings.splits) * scale;
    if !row_height.is_finite() || row_height <= 0.0 {
        return;
    }
    let segment_count = ctx.timer.run.segments.len();
    let painter = ui.painter();
    paint_splits_background(painter, rect, &settings.splits);
    if segment_count == 0 {
        return;
    }

    let groups = sections(&ctx.timer.run.segments);
    if groups.is_empty() {
        return;
    }
    let raw_current = ctx.timer.current_split_index;
    let current = if raw_current < 0 {
        0
    } else {
        (raw_current as usize).min(segment_count - 1)
    };
    let current_section = groups
        .iter()
        .position(|section| current >= section.start && current <= section.end)
        .unwrap_or(0);

    let mut rows: Vec<SubRow> = Vec::new();
    for (section_index, section) in groups.iter().enumerate() {
        if settings.current_section_only && section_index != current_section {
            continue;
        }
        if settings.show_header && section.end > section.start {
            rows.push(SubRow::Header(section_index));
        }
        for index in section.start..=section.end {
            rows.push(SubRow::Segment(index, section_index));
        }
    }
    if rows.is_empty() {
        return;
    }

    let label_height = if settings.splits.show_column_labels {
        row_height
    } else {
        0.0
    };
    let table_top = rect.top() + label_height;
    let available_rows = ((rect.bottom() - table_top) / row_height)
        .floor()
        .max(0.0) as usize;
    if available_rows == 0 {
        return;
    }

    let (time_formatter, delta_formatter) = build_formatters(&settings.splits);
    let times_font = ctx.times_font();
    let geometry = column_geometry(
        ui.painter(),
        rect,
        &settings.splits,
        &times_font,
        &time_formatter,
        &delta_formatter,
        scale,
    );
    if settings.splits.show_column_labels {
        let label_rect = Rect::from_min_size(rect.min, Vec2::new(rect.width(), row_height));
        draw_column_labels(ui, label_rect, ctx, &settings.splits, &geometry);
    }

    let current_row = rows
        .iter()
        .position(|row| matches!(row, SubRow::Segment(index, _) if *index == current))
        .or_else(|| {
            rows.iter().position(
                |row| matches!(row, SubRow::Header(section) if *section == current_section),
            )
        })
        .unwrap_or(0);

    if rows.len() > available_rows {
        let preview = settings.splits.split_preview_count.max(0) as usize;
        let target = available_rows.saturating_sub(3 + preview);
        let start = current_row
            .saturating_sub(target)
            .min(rows.len() - available_rows);
        rows = rows[start..start + available_rows].to_vec();
    } else if settings.splits.show_blank_splits {
        while rows.len() < available_rows {
            rows.push(SubRow::Blank);
        }
    }

    let reserve_icon = settings.splits.display_icons
        && ctx
            .timer
            .run
            .segments
            .iter()
            .any(|segment| segment.icon.as_deref().map_or(false, |icon| !icon.is_empty()));
    let active_phase = matches!(ctx.timer.phase, TimerPhase::Running | TimerPhase::Paused);

    for (visible_index, sub_row) in rows.iter().take(available_rows).enumerate() {
        let row = Rect::from_min_size(
            Pos2::new(
                rect.left(),
                table_top + visible_index as f32 * row_height,
            ),
            Vec2::new(rect.width(), row_height),
        );
        if row.top() >= rect.bottom() {
            break;
        }

        let is_last = match sub_row {
            SubRow::Segment(index, _) => *index == segment_count - 1,
            _ => false,
        };

        match *sub_row {
            SubRow::Header(section_index) => {
                let section = groups[section_index];
                let parity =
                    section_index + usize::from(settings.splits.show_column_labels);
                draw_header_row(
                    ui,
                    row,
                    ctx,
                    settings,
                    section_index,
                    section,
                    parity,
                    reserve_icon,
                );
            }
            SubRow::Segment(segment_index, section_index) => {
                let section = groups[section_index];
                let active = active_phase
                    && raw_current >= 0
                    && segment_index == raw_current as usize;
                let indent = if section.end > section.start && segment_index < section.end {
                    12.0 * scale
                } else {
                    0.0
                };
                let parity =
                    section_index + usize::from(settings.splits.show_column_labels);
                let name = segment_display_name(&ctx.timer.run.segments[segment_index].name);
                draw_split_row(
                    ui,
                    row,
                    ctx,
                    &settings.splits,
                    &geometry,
                    segment_index,
                    &name,
                    parity,
                    indent,
                    active,
                    reserve_icon,
                    &time_formatter,
                    &delta_formatter,
                );
            }
            SubRow::Blank => {
                paint_row_background(ui.painter(), row, &settings.splits, 0);
            }
        }

        // Draw after the row fill, otherwise alternating backgrounds and
        // header gradients would hide the separator lines.
        if is_last && settings.splits.always_show_last_split && visible_index > 0 {
            let stroke = if settings.splits.separator_last_split {
                2.0 * scale
            } else {
                1.0 * scale
            };
            let color = if settings.splits.separator_last_split {
                ctx.theme.separators_color
            } else {
                ctx.theme.thin_separators_color
            };
            ui.painter().line_segment(
                [row.left_top(), row.right_top()],
                Stroke::new(stroke, color),
            );
        } else if settings.splits.show_thin_separators && visible_index > 0 {
            ui.painter().line_segment(
                [row.left_top(), row.right_top()],
                Stroke::new(1.0 * scale, ctx.theme.thin_separators_color),
            );
        }
    }

    if settings.splits.show_column_labels {
        ui.painter().line_segment(
            [
                Pos2::new(rect.left(), table_top),
                Pos2::new(rect.right(), table_top),
            ],
            Stroke::new(2.0 * scale, ctx.theme.separators_color),
        );
    }
}

// ---------------------------------------------------------------------------
// Settings UI
// ---------------------------------------------------------------------------

fn color_setting(ui: &mut Ui, label: &str, color: &mut Color32) {
    ui.horizontal(|ui| {
        ui.color_edit_button_srgba(color);
        ui.label(label);
    });
}

fn accuracy_combo(ui: &mut Ui, label: &str, accuracy: &mut TimeAccuracy) {
    egui::ComboBox::from_id_salt(label)
        .selected_text(accuracy.label())
        .show_ui(ui, |ui| {
            for value in [
                TimeAccuracy::Seconds,
                TimeAccuracy::Tenths,
                TimeAccuracy::Hundredths,
                TimeAccuracy::Milliseconds,
            ] {
                ui.selectable_value(accuracy, value, value.label());
            }
        });
}

fn gradient_combo(
    ui: &mut Ui,
    label: &str,
    gradient: &mut GradientType,
    include_alternating: bool,
) {
    ui.horizontal(|ui| {
        ui.label(label);
        egui::ComboBox::from_id_salt(label)
            .selected_text(gradient.label())
            .show_ui(ui, |ui| {
                for value in [
                    GradientType::Plain,
                    GradientType::Vertical,
                    GradientType::Horizontal,
                ] {
                    ui.selectable_value(gradient, value, value.label());
                }
                if include_alternating {
                    ui.selectable_value(
                        gradient,
                        GradientType::Alternating,
                        GradientType::Alternating.label(),
                    );
                }
            });
    });
}

fn comparison_combo(ui: &mut Ui, label: &str, comparison: &mut String) {
    const PRESETS: &[&str] = &[
        CURRENT_COMPARISON,
        livesplit_core::PERSONAL_BEST,
        livesplit_core::BEST_SEGMENTS,
        livesplit_core::BEST_SPLIT_TIMES,
        livesplit_core::LATEST_RUN,
        livesplit_core::AVERAGE_SEGMENTS,
        livesplit_core::MEDIAN_SEGMENTS,
        livesplit_core::WORST_SEGMENTS,
    ];
    ui.horizontal(|ui| {
        ui.label(label);
        egui::ComboBox::from_id_salt(label)
            .selected_text(comparison.clone())
            .show_ui(ui, |ui| {
                for preset in PRESETS {
                    if ui
                        .selectable_label(comparison == preset, *preset)
                        .clicked()
                    {
                        *comparison = (*preset).to_string();
                    }
                }
                ui.separator();
                ui.label("Custom comparison:");
                ui.text_edit_singleline(comparison);
            });
    });
}

fn timing_method_combo(ui: &mut Ui, label: &str, method: &mut TimingMethodSetting) {
    ui.horizontal(|ui| {
        ui.label(label);
        egui::ComboBox::from_id_salt(label)
            .selected_text(method.label())
            .show_ui(ui, |ui| {
                for value in [
                    TimingMethodSetting::Current,
                    TimingMethodSetting::RealTime,
                    TimingMethodSetting::GameTime,
                ] {
                    ui.selectable_value(method, value, value.label());
                }
            });
    });
}

pub fn splits_settings_ui(ui: &mut Ui, settings: &mut SplitsSettings) {
    egui::CollapsingHeader::new("Current split")
        .default_open(true)
        .show(ui, |ui| {
            color_setting(
                ui,
                "Top color",
                &mut settings.current_split_top_color,
            );
            color_setting(
                ui,
                "Bottom color",
                &mut settings.current_split_bottom_color,
            );
            gradient_combo(
                ui,
                "Current split gradient",
                &mut settings.current_split_gradient,
                false,
            );
        });

    egui::CollapsingHeader::new("Layout")
        .default_open(true)
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label("Visual split count:");
                ui.add(
                    egui::DragValue::new(&mut settings.visual_split_count)
                        .speed(1.0)
                        .range(1..=200),
                );
            });
            ui.horizontal(|ui| {
                ui.label("Upcoming split preview count:");
                ui.add(
                    egui::DragValue::new(&mut settings.split_preview_count)
                        .speed(1.0)
                        .range(0..=100),
                );
            });
            ui.horizontal(|ui| {
                ui.label("Split height (1/10 px):");
                ui.add(
                    egui::DragValue::new(&mut settings.split_height)
                        .speed(0.1)
                        .range(0.1..=25.0),
                );
            });
            ui.horizontal(|ui| {
                ui.label("Split width (horizontal layouts):");
                ui.add(
                    egui::DragValue::new(&mut settings.split_width)
                        .speed(1.0)
                        .range(5.0..=500.0),
                );
            });
            ui.checkbox(&mut settings.display_2_rows, "Display 2 rows");
            ui.checkbox(&mut settings.show_blank_splits, "Show blank splits");
            ui.checkbox(&mut settings.lock_last_split, "Lock last split");
            ui.checkbox(
                &mut settings.always_show_last_split,
                "Always show last split",
            );
            ui.checkbox(
                &mut settings.separator_last_split,
                "Separator before last split",
            );
            ui.checkbox(
                &mut settings.show_thin_separators,
                "Show thin separators",
            );
            ui.checkbox(
                &mut settings.show_column_labels,
                "Show column labels",
            );
            ui.add_enabled_ui(settings.show_column_labels, |ui| {
                color_setting(ui, "Label color", &mut settings.labels_color);
            });
            ui.checkbox(
                &mut settings.automatic_abbreviations,
                "Automatic abbreviations",
            );
        });

    egui::CollapsingHeader::new("Columns")
        .default_open(true)
        .show(ui, |ui| {
            ui.checkbox(&mut settings.show_split_times, "Show split times");
            comparison_combo(ui, "Comparison", &mut settings.comparison);
            ui.horizontal(|ui| {
                ui.label("Split time accuracy:");
                accuracy_combo(ui, "split-time-accuracy", &mut settings.split_times_accuracy);
            });
            ui.horizontal(|ui| {
                ui.label("Delta accuracy:");
                accuracy_combo(ui, "delta-accuracy", &mut settings.deltas_accuracy);
            });
            ui.checkbox(&mut settings.drop_decimals, "Drop decimals");
        });

    egui::CollapsingHeader::new("Icons")
        .default_open(false)
        .show(ui, |ui| {
            ui.checkbox(&mut settings.display_icons, "Display icons");
            ui.add_enabled_ui(settings.display_icons, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Icon size:");
                    ui.add(
                        egui::DragValue::new(&mut settings.icon_size)
                            .speed(1.0)
                            .range(0.0..=128.0),
                    );
                });
                ui.checkbox(&mut settings.icon_shadows, "Icon shadows");
            });
        });

    egui::CollapsingHeader::new("Name colors")
        .default_open(false)
        .show(ui, |ui| {
            ui.checkbox(
                &mut settings.override_text_color,
                "Override split name colors",
            );
            ui.add_enabled_ui(settings.override_text_color, |ui| {
                color_setting(ui, "Before current split", &mut settings.before_names_color);
                color_setting(
                    ui,
                    "Current split",
                    &mut settings.current_names_color,
                );
                color_setting(ui, "After current split", &mut settings.after_names_color);
            });
        });

    egui::CollapsingHeader::new("Time colors")
        .default_open(false)
        .show(ui, |ui| {
            ui.checkbox(
                &mut settings.override_times_color,
                "Override split time colors",
            );
            ui.add_enabled_ui(settings.override_times_color, |ui| {
                color_setting(ui, "Before current split", &mut settings.before_times_color);
                color_setting(
                    ui,
                    "Current split",
                    &mut settings.current_times_color,
                );
                color_setting(ui, "After current split", &mut settings.after_times_color);
            });
            ui.checkbox(
                &mut settings.override_deltas_color,
                "Override live delta color",
            );
            ui.add_enabled_ui(settings.override_deltas_color, |ui| {
                color_setting(ui, "Live delta color", &mut settings.deltas_color);
            });
        });

    egui::CollapsingHeader::new("Background")
        .default_open(false)
        .show(ui, |ui| {
            color_setting(ui, "Background color", &mut settings.background.color);
            color_setting(
                ui,
                "Alternate / gradient color",
                &mut settings.background.color2,
            );
            gradient_combo(
                ui,
                "Background gradient",
                &mut settings.background.gradient,
                true,
            );
        });
}

pub fn graph_settings_ui(ui: &mut Ui, settings: &mut GraphSettings) {
    egui::CollapsingHeader::new("Graph")
        .default_open(true)
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label("Height:");
                ui.add(
                    egui::DragValue::new(&mut settings.height)
                        .speed(1.0)
                        .range(20.0..=2000.0),
                );
            });
            ui.horizontal(|ui| {
                ui.label("Width:");
                ui.add(
                    egui::DragValue::new(&mut settings.width)
                        .speed(1.0)
                        .range(20.0..=2000.0),
                );
            });
            comparison_combo(ui, "Comparison", &mut settings.comparison);
            ui.checkbox(&mut settings.live_graph, "Live graph");
            ui.checkbox(&mut settings.flip_graph, "Flip graph");
            ui.checkbox(
                &mut settings.show_best_segments,
                "Show best segments in gold",
            );
        });

    egui::CollapsingHeader::new("Colors")
        .default_open(true)
        .show(ui, |ui| {
            color_setting(ui, "Graph line", &mut settings.graph_color);
            color_setting(ui, "Best segment", &mut settings.graph_gold_color);
            color_setting(ui, "Ahead region", &mut settings.ahead_graph_color);
            color_setting(ui, "Behind region", &mut settings.behind_graph_color);
            color_setting(ui, "Center line", &mut settings.graph_lines_color);
            color_setting(ui, "Grid lines", &mut settings.gridlines_color);
            color_setting(
                ui,
                "Partial fill ahead",
                &mut settings.partial_fill_color_ahead,
            );
            color_setting(
                ui,
                "Complete fill ahead",
                &mut settings.complete_fill_color_ahead,
            );
            color_setting(
                ui,
                "Partial fill behind",
                &mut settings.partial_fill_color_behind,
            );
            color_setting(
                ui,
                "Complete fill behind",
                &mut settings.complete_fill_color_behind,
            );
        });

    egui::CollapsingHeader::new("Background")
        .default_open(false)
        .show(ui, |ui| {
            color_setting(ui, "Background color", &mut settings.background.color);
            color_setting(
                ui,
                "Background color 2",
                &mut settings.background.color2,
            );
            gradient_combo(
                ui,
                "Background gradient",
                &mut settings.background.gradient,
                true,
            );
        });
}

pub fn subsplits_settings_ui(ui: &mut Ui, settings: &mut SubsplitsSettings) {
    ui.checkbox(&mut settings.show_subsplits, "Show subsplits");
    if !settings.show_subsplits {
        ui.label("Subsplits are disabled; the component renders the normal splits table.");
    }

    egui::CollapsingHeader::new("Sections")
        .default_open(true)
        .show(ui, |ui| {
            ui.checkbox(&mut settings.show_header, "Show section headers");
            let mut show_names = settings.header_text != HEADER_TEXT_HIDDEN;
            if ui
                .checkbox(&mut show_names, "Show section name in header")
                .changed()
            {
                if show_names {
                    if settings.header_text == HEADER_TEXT_HIDDEN {
                        settings.header_text.clear();
                    }
                } else {
                    settings.header_text = HEADER_TEXT_HIDDEN.to_string();
                }
            }
            if show_names {
                ui.horizontal(|ui| {
                    ui.label("Custom header text:");
                    ui.text_edit_singleline(&mut settings.header_text);
                });
                ui.label("Leave empty to use the section name.  {section} is replaced by it.");
            }
            ui.checkbox(
                &mut settings.current_section_only,
                "Show only the current section",
            );
            ui.horizontal(|ui| {
                ui.label("Minimum major splits:");
                ui.add(
                    egui::DragValue::new(&mut settings.minimum_major_splits)
                        .speed(1.0)
                        .range(0..=100),
                );
            });
        });

    egui::CollapsingHeader::new("Section header")
        .default_open(true)
        .show(ui, |ui| {
            color_setting(ui, "Top color", &mut settings.header_top_color);
            color_setting(ui, "Bottom color", &mut settings.header_bottom_color);
            gradient_combo(
                ui,
                "Header gradient",
                &mut settings.header_gradient,
                false,
            );
            color_setting(ui, "Header text color", &mut settings.header_text_color);
            color_setting(ui, "Header times color", &mut settings.header_times_color);
            ui.horizontal(|ui| {
                ui.label("Header accuracy:");
                accuracy_combo(ui, "header-accuracy", &mut settings.header_accuracy);
            });
            timing_method_combo(
                ui,
                "Header timing method",
                &mut settings.header_timing_method,
            );
            comparison_combo(
                ui,
                "Header comparison",
                &mut settings.header_comparison,
            );
        });

    egui::CollapsingHeader::new("Common splits settings")
        .default_open(false)
        .show(ui, |ui| {
            splits_settings_ui(ui, &mut settings.splits);
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(xml: &str) -> XmlElement {
        crate::xml::parse_xml(xml).expect("valid test XML")
    }

    #[test]
    fn splits_settings_roundtrip() {
        let xml = r#"<Settings>
            <Version>1.6</Version>
            <CurrentSplitTopColor>FF3373F4</CurrentSplitTopColor>
            <CurrentSplitBottomColor>80153574</CurrentSplitBottomColor>
            <VisualSplitCount>9</VisualSplitCount>
            <SplitPreviewCount>3</SplitPreviewCount>
            <DisplayIcons>False</DisplayIcons>
            <ShowThinSeparators>True</ShowThinSeparators>
            <AlwaysShowLastSplit>True</AlwaysShowLastSplit>
            <SplitWidth>25.5</SplitWidth>
            <SplitTimesAccuracy>Hundredths</SplitTimesAccuracy>
            <AutomaticAbbreviations>True</AutomaticAbbreviations>
            <BeforeNamesColor>FF010203</BeforeNamesColor>
            <CurrentNamesColor>FF040506</CurrentNamesColor>
            <AfterNamesColor>FF070809</AfterNamesColor>
            <OverrideTextColor>True</OverrideTextColor>
            <BeforeTimesColor>FF111213</BeforeTimesColor>
            <CurrentTimesColor>FF141516</CurrentTimesColor>
            <AfterTimesColor>FF171819</AfterTimesColor>
            <OverrideTimesColor>True</OverrideTimesColor>
            <ShowBlankSplits>False</ShowBlankSplits>
            <LockLastSplit>False</LockLastSplit>
            <IconSize>18</IconSize>
            <IconShadows>False</IconShadows>
            <SplitHeight>4.5</SplitHeight>
            <CurrentSplitGradient>Horizontal</CurrentSplitGradient>
            <BackgroundColor>FF202122</BackgroundColor>
            <BackgroundColor2>FF232425</BackgroundColor2>
            <BackgroundGradient>Vertical</BackgroundGradient>
            <SeparatorLastSplit>False</SeparatorLastSplit>
            <DeltasAccuracy>Milliseconds</DeltasAccuracy>
            <DropDecimals>False</DropDecimals>
            <OverrideDeltasColor>True</OverrideDeltasColor>
            <DeltasColor>FF262728</DeltasColor>
            <Display2Rows>True</Display2Rows>
            <ShowColumnLabels>True</ShowColumnLabels>
            <LabelsColor>FF292A2B</LabelsColor>
            <Columns>
                <Settings><Version>1.5</Version><Name>+/-</Name><Type>Delta</Type><Comparison>Personal Best</Comparison><TimingMethod>Current Timing Method</TimingMethod></Settings>
                <Settings><Version>1.5</Version><Name>Time</Name><Type>SplitTime</Type><Comparison>Personal Best</Comparison><TimingMethod>Current Timing Method</TimingMethod></Settings>
            </Columns>
        </Settings>"#;
        let settings = splits_from_xml(&parse(xml));
        assert_eq!(settings.visual_split_count, 9);
        assert_eq!(settings.split_preview_count, 3);
        assert!(!settings.display_icons);
        assert_eq!(settings.split_times_accuracy, TimeAccuracy::Hundredths);
        assert_eq!(settings.deltas_accuracy, TimeAccuracy::Milliseconds);
        assert_eq!(settings.split_height, 4.5);
        assert_eq!(settings.comparison, "Personal Best");
        assert!(settings.show_split_times);
        assert!(settings.automatic_abbreviations);
        assert_eq!(
            settings.current_split_gradient,
            GradientType::Horizontal
        );
        assert_eq!(settings.background.gradient, GradientType::Vertical);
        assert_eq!(settings.background.color, Color32::from_rgb(0x20, 0x21, 0x22));

        let saved = splits_to_xml(&settings);
        let reparsed = splits_from_xml(&saved);
        assert_eq!(reparsed.visual_split_count, 9);
        assert_eq!(reparsed.split_preview_count, 3);
        assert_eq!(reparsed.split_times_accuracy, TimeAccuracy::Hundredths);
        assert_eq!(reparsed.deltas_accuracy, TimeAccuracy::Milliseconds);
        assert_eq!(reparsed.split_height, 4.5);
        assert_eq!(reparsed.comparison, "Personal Best");
        assert_eq!(reparsed.current_names_color, settings.current_names_color);
    }

    #[test]
    fn old_splits_layout_is_understood() {
        let xml = r#"<Settings>
            <SplitNamesColor>FF102030</SplitNamesColor>
            <UseTextColor>False</UseTextColor>
            <ShowSplitTimes>False</ShowSplitTimes>
            <Comparison>Best Segments</Comparison>
            <SplitHeight>3.5</SplitHeight>
        </Settings>"#;
        let settings = splits_from_xml(&parse(xml));
        assert_eq!(settings.before_names_color, Color32::from_rgb(0x10, 0x20, 0x30));
        assert_eq!(settings.current_names_color, settings.before_names_color);
        assert!(settings.override_text_color);
        assert!(!settings.show_split_times);
        assert_eq!(settings.comparison, "Best Segments");
    }

    #[test]
    fn graph_legacy_fill_colors_fall_back() {
        let xml = r#"<Settings>
            <Version>1.0</Version>
            <Height>100</Height>
            <Width>200</Width>
            <PartialFillColor>40223344</PartialFillColor>
            <CompleteFillColor>80223344</CompleteFillColor>
        </Settings>"#;
        let settings = graph_from_xml(&parse(xml));
        assert_eq!(settings.height, 100.0);
        assert_eq!(settings.width, 200.0);
        assert_eq!(
            settings.partial_fill_color_ahead,
            settings.partial_fill_color_behind
        );
        assert_eq!(
            settings.complete_fill_color_ahead,
            settings.complete_fill_color_behind
        );
    }

    #[test]
    fn subsplits_delegate_and_hidden_header_roundtrip() {
        let xml = r#"<Settings>
            <Version>1.7</Version>
            <VisualSplitCount>7</VisualSplitCount>
            <AutomaticAbbreviation>True</AutomaticAbbreviation>
            <ShowSubsplits>True</ShowSubsplits>
            <ShowHeader>False</ShowHeader>
            <HeaderText>False</HeaderText>
            <CurrentSectionOnly>True</CurrentSectionOnly>
            <MinimumMajorSplits>2</MinimumMajorSplits>
            <HeaderComparison>Personal Best</HeaderComparison>
            <HeaderTimingMethod>Game Time</HeaderTimingMethod>
            <HeaderAccuracy>Milliseconds</HeaderAccuracy>
        </Settings>"#;
        let settings = subsplits_from_xml(&parse(xml));
        assert_eq!(settings.splits.visual_split_count, 7);
        assert!(settings.splits.automatic_abbreviations);
        assert!(settings.show_subsplits);
        assert!(!settings.show_header);
        assert_eq!(settings.header_text, HEADER_TEXT_HIDDEN);
        assert!(settings.current_section_only);
        assert_eq!(settings.minimum_major_splits, 2);
        assert_eq!(settings.header_comparison, "Personal Best");
        assert_eq!(settings.header_timing_method, TimingMethodSetting::GameTime);
        assert_eq!(settings.header_accuracy, TimeAccuracy::Milliseconds);

        let reparsed = subsplits_from_xml(&subsplits_to_xml(&settings));
        assert_eq!(reparsed.header_text, HEADER_TEXT_HIDDEN);
        assert_eq!(reparsed.splits.visual_split_count, 7);
        assert_eq!(reparsed.minimum_major_splits, 2);
    }

    #[test]
    fn section_grouping_matches_livesplit_legacy_syntax() {
        let segments = vec![
            livesplit_core::Segment::new("Intro"),
            livesplit_core::Segment::new("-Save the dog"),
            livesplit_core::Segment::new("-Kill the boss"),
            livesplit_core::Segment::new("{Deku Tree} First room"),
            livesplit_core::Segment::new("{Deku Tree} Boss"),
            livesplit_core::Segment::new("Outside"),
        ];
        let groups = sections(&segments);
        assert_eq!(groups.len(), 4);
        assert_eq!((groups[0].start, groups[0].end), (0, 0));
        assert_eq!((groups[1].start, groups[1].end), (1, 3));
        assert_eq!((groups[2].start, groups[2].end), (4, 4));
        assert_eq!((groups[3].start, groups[3].end), (5, 5));
        assert_eq!(section_display_name(&segments[3].name), "Deku Tree");
        assert_eq!(segment_display_name(&segments[3].name), "First room");
        assert_eq!(segment_display_name(&segments[4].name), "Boss");
        assert_eq!(segment_display_name("-Save the dog"), "Save the dog");
    }

    #[test]
    fn split_window_never_panics_and_pins_last_split() {
        let settings = SplitsSettings {
            visual_split_count: 6,
            split_preview_count: 2,
            show_blank_splits: true,
            always_show_last_split: true,
            lock_last_split: true,
            ..SplitsSettings::default()
        };
        for current in 0..30 {
            let (rows, last) = build_split_rows(&settings, 12, current);
            assert_eq!(rows.len(), 6);
            assert_eq!(last, Some(5));
            assert_eq!(rows[5], Some(11));
        }
        let (rows, last) = build_split_rows(&settings, 3, 0);
        assert_eq!(rows.len(), 6);
        assert_eq!(last, Some(5));
        assert_eq!(rows[5], Some(2));
    }
}
