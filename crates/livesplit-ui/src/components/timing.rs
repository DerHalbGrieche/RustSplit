//! Timing-related components.
//!
//! These are ports of LiveSplit's Timer, Detailed Timer, Previous Segment,
//! Delta, Possible Time Save, Current Comparison, Current Pace/Run Prediction
//! and Manual Game Time components.

use egui::{Align2, Color32, FontId, Rect, TextEdit, Ui};

use livesplit_core::{
    analysis, DigitsFormat, GeneralTimeFormatter, NullFormat, TimeAccuracy, TimeSpan, Timer,
    TimerPhase, TimingMethod, AVERAGE_SEGMENTS, BEST_SEGMENTS, CURRENT_COMPARISON, NONE,
    PERSONAL_BEST, WORST_SEGMENTS,
};
use xmltree::XMLNode;

use crate::canvas::{fit_text, paint_background, paint_gradient, paint_text, text_width};
use crate::components::{
    ComponentContext, CurrentComparisonSettings, CurrentPaceSettings, DeltaSettings,
    DetailedTimerSettings, ManualGameTimeSettings, PossibleTimeSaveSettings,
    PreviousSegmentSettings, RunPredictionSettings, TimerSettings, TimingMethodSetting,
};
use crate::layout::{
    parse_color, BackgroundStyle, FontSpec, GradientType, LayoutMode, LayoutSettings,
};
use crate::layout_xml::{font_to_xml, parse_font};
use crate::xml::{child, child_text, element_with_text, parse_bool, parse_f32, XmlElement};

const COMPARISONS: &[&str] = &[
    CURRENT_COMPARISON,
    PERSONAL_BEST,
    BEST_SEGMENTS,
    "Best Split Times",
    "Latest Run",
    AVERAGE_SEGMENTS,
    "Median Segments",
    WORST_SEGMENTS,
    "Balanced PB",
    NONE,
];

// ---------------------------------------------------------------------------
// Small XML helpers
// ---------------------------------------------------------------------------

fn push_text(parent: &mut XmlElement, name: &str, value: impl ToString) {
    parent
        .children
        .push(XMLNode::Element(element_with_text(name, value.to_string())));
}

fn push_bool(parent: &mut XmlElement, name: &str, value: bool) {
    push_text(parent, name, bool_text(value));
}

fn push_float(parent: &mut XmlElement, name: &str, value: f32) {
    push_text(parent, name, format!("{value}"));
}

fn push_color(parent: &mut XmlElement, name: &str, color: Color32) {
    let [red, green, blue, alpha] = color.to_srgba_unmultiplied();
    push_text(parent, name, format!("{alpha:02X}{red:02X}{green:02X}{blue:02X}"));
}

fn push_element(parent: &mut XmlElement, element: XmlElement) {
    parent.children.push(XMLNode::Element(element));
}

fn bool_text(value: bool) -> &'static str {
    if value {
        "True"
    } else {
        "False"
    }
}

fn value_bool(element: &XmlElement, name: &str, default: bool) -> bool {
    parse_bool(child_text(element, name).as_deref(), default)
}

fn value_float(element: &XmlElement, name: &str, default: f32) -> f32 {
    parse_f32(child_text(element, name).as_deref(), default)
}

fn value_string(element: &XmlElement, name: &str, default: &str) -> String {
    child_text(element, name).unwrap_or_else(|| default.to_string())
}

fn value_color(element: &XmlElement, name: &str, default: Color32) -> Color32 {
    parse_color(child_text(element, name).as_deref(), default)
}

fn value_accuracy(element: &XmlElement, name: &str, default: TimeAccuracy) -> TimeAccuracy {
    parse_accuracy(child_text(element, name).as_deref(), default)
}

fn value_timing_method(
    element: &XmlElement,
    name: &str,
    default: TimingMethodSetting,
) -> TimingMethodSetting {
    child_text(element, name)
        .map(|value| TimingMethodSetting::parse(&value, default))
        .unwrap_or(default)
}

fn value_gradient(element: &XmlElement, name: &str, default: GradientType) -> GradientType {
    child_text(element, name)
        .map(|value| GradientType::parse(&value, default))
        .unwrap_or(default)
}

fn value_background(element: &XmlElement, default: BackgroundStyle) -> BackgroundStyle {
    BackgroundStyle {
        color: value_color(element, "BackgroundColor", default.color),
        color2: value_color(element, "BackgroundColor2", default.color2),
        gradient: value_gradient(element, "BackgroundGradient", default.gradient),
    }
}

fn push_background(parent: &mut XmlElement, background: &BackgroundStyle) {
    push_color(parent, "BackgroundColor", background.color);
    push_color(parent, "BackgroundColor2", background.color2);
    push_text(
        parent,
        "BackgroundGradient",
        gradient_name(background.gradient),
    );
}

fn gradient_name(gradient: GradientType) -> &'static str {
    match gradient {
        GradientType::Plain => "Plain",
        GradientType::Vertical => "Vertical",
        GradientType::Horizontal => "Horizontal",
        GradientType::Alternating => "Alternating",
        GradientType::PlainWithDeltaColor => "PlainWithDeltaColor",
        GradientType::VerticalWithDeltaColor => "VerticalWithDeltaColor",
        GradientType::HorizontalWithDeltaColor => "HorizontalWithDeltaColor",
    }
}

fn parse_accuracy(value: Option<&str>, default: TimeAccuracy) -> TimeAccuracy {
    value
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(default)
}

fn parse_digits_format(value: Option<&str>, default: DigitsFormat) -> DigitsFormat {
    value
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(default)
}

/// Parse LiveSplit's combined timer format (`1.23`, `01.2`, `0:00:01.234`).
fn parse_timer_format(value: &str) -> Option<(DigitsFormat, TimeAccuracy)> {
    let (digits, decimals) = value
        .split_once('.')
        .map(|(digits, decimals)| (digits, decimals))
        .unwrap_or((value, ""));
    let digits_format = parse_digits_format(Some(digits), DigitsFormat::SingleDigitSeconds);
    let accuracy = match decimals {
        "" => TimeAccuracy::Seconds,
        "2" => TimeAccuracy::Tenths,
        "23" => TimeAccuracy::Hundredths,
        "234" => TimeAccuracy::Milliseconds,
        _ => return None,
    };
    Some((digits_format, accuracy))
}

fn timer_format_string(digits: DigitsFormat, accuracy: TimeAccuracy) -> String {
    format!("{}{}", digits.label(), accuracy_suffix(accuracy))
}

fn accuracy_suffix(accuracy: TimeAccuracy) -> &'static str {
    match accuracy {
        TimeAccuracy::Seconds => "",
        TimeAccuracy::Tenths => ".2",
        TimeAccuracy::Hundredths => ".23",
        TimeAccuracy::Milliseconds => ".234",
    }
}

// ---------------------------------------------------------------------------
// Formatting helpers
// ---------------------------------------------------------------------------

fn general_formatter(digits: DigitsFormat, accuracy: TimeAccuracy) -> GeneralTimeFormatter {
    GeneralTimeFormatter {
        accuracy,
        digits_format: digits,
        null_format: NullFormat::Dash,
        ..Default::default()
    }
}

fn format_with_digits(
    time: Option<TimeSpan>,
    digits: DigitsFormat,
    accuracy: TimeAccuracy,
) -> String {
    general_formatter(digits, accuracy).format(time)
}

fn format_time_accuracy(time: Option<TimeSpan>, accuracy: TimeAccuracy) -> String {
    general_formatter(DigitsFormat::SingleDigitSeconds, accuracy).format(time)
}

fn format_delta(time: Option<TimeSpan>, accuracy: TimeAccuracy, drop_decimals: bool) -> String {
    let mut formatter = livesplit_core::DeltaTimeFormatter::default();
    formatter.set_accuracy(accuracy);
    formatter.set_drop_decimals(drop_decimals);
    formatter.format(time)
}

fn split_time_text(text: &str) -> (String, String) {
    if text == "-" {
        return (String::new(), "-".to_string());
    }
    match text.find('.') {
        Some(index) => (text[..index].to_string(), text[index..].to_string()),
        None => (text.to_string(), String::new()),
    }
}

// ---------------------------------------------------------------------------
// Shared painting helpers
// ---------------------------------------------------------------------------

fn themed_text(
    painter: &egui::Painter,
    pos: egui::Pos2,
    align: Align2,
    text: &str,
    font: FontId,
    color: Color32,
    theme: &LayoutSettings,
) {
    paint_text(
        painter,
        pos,
        align,
        text,
        font,
        color,
        if theme.drop_shadows {
            Some(theme.shadows_color)
        } else {
            None
        },
        Some(theme.text_outline_color),
    );
}

fn sized_font(ctx: &ComponentContext, spec: &FontSpec, max_unscaled_size: f32) -> FontId {
    let mut spec = spec.clone();
    spec.size = spec.size.min(max_unscaled_size).max(4.0);
    ctx.font(&spec)
}

/// Approximate LiveSplit's gradient text with a single blended color.  The
/// gradient itself is vertical in LiveSplit, which egui's text painter cannot
/// express directly.
fn timer_gradient_color(color: Color32) -> Color32 {
    let hsva = egui::ecolor::Hsva::from(color);
    let top = egui::ecolor::Hsva::new(
        hsva.h,
        0.5 * hsva.s,
        (1.5 * hsva.v + 0.1).min(1.0),
        hsva.a,
    );
    let bottom = egui::ecolor::Hsva::new(hsva.h, hsva.s, 0.8 * hsva.v, hsva.a);
    Color32::from(egui::ecolor::Hsva::new(
        hsva.h,
        0.5 * (top.s + bottom.s),
        0.5 * (top.v + bottom.v),
        hsva.a,
    ))
}

/// Timer backgrounds can derive their colors from the current timer color.
fn paint_timer_background(
    painter: &egui::Painter,
    rect: Rect,
    style: &BackgroundStyle,
    timer_color: Color32,
) {
    let delta_gradient = matches!(
        style.gradient,
        GradientType::PlainWithDeltaColor
            | GradientType::VerticalWithDeltaColor
            | GradientType::HorizontalWithDeltaColor
    );
    if !delta_gradient {
        paint_background(painter, rect, style);
        return;
    }

    let hsva = egui::ecolor::Hsva::from(timer_color);
    let base = Color32::from(egui::ecolor::Hsva::new(
        hsva.h,
        0.5 * hsva.s,
        0.25 * hsva.v,
        1.0,
    ));
    let with_alpha = |alpha: f32| {
        Color32::from_rgba_unmultiplied(
            base.r(),
            base.g(),
            base.b(),
            (alpha.clamp(0.0, 1.0) * 255.0).round() as u8,
        )
    };

    match style.gradient {
        GradientType::PlainWithDeltaColor => {
            let color = with_alpha(timer_color.a() as f32 / 255.0 * 7.0 / 12.0);
            if color.a() > 0 {
                painter.rect_filled(rect, 0.0, color);
            }
        }
        GradientType::VerticalWithDeltaColor => {
            paint_gradient(
                painter,
                rect,
                with_alpha(timer_color.a() as f32 / 255.0 / 6.0),
                with_alpha(timer_color.a() as f32 / 255.0),
                false,
            );
        }
        GradientType::HorizontalWithDeltaColor => {
            paint_gradient(
                painter,
                rect,
                with_alpha(timer_color.a() as f32 / 255.0 / 6.0),
                with_alpha(timer_color.a() as f32 / 255.0),
                true,
            );
        }
        _ => paint_background(painter, rect, style),
    }
}

/// Draw a LiveSplit-style timer text: the integer part in the timer font and
/// the decimal places in a smaller font.  Returns the total drawn width.
#[allow(clippy::too_many_arguments)]
fn draw_timer_text(
    painter: &egui::Painter,
    rect: Rect,
    ctx: &ComponentContext,
    text: &str,
    color: Color32,
    center: bool,
    decimals_size: f32,
    gradient: bool,
) -> f32 {
    let (big_text, small_text) = split_time_text(text);
    let spec = &ctx.theme.timer_font;
    let base_big = spec.size.max(4.0);
    let base_small = (spec.size / 50.0 * decimals_size).max(1.0);
    let pad = 7.0 * ctx.scale;
    let available = (rect.width() - 2.0 * pad).max(1.0);

    let big_font = ctx.timer_font(base_big, spec.bold);
    let small_font = ctx.timer_font(base_small, spec.bold);
    let reference_width =
        text_width(painter, "88:88:88", &big_font) + text_width(painter, &small_text, &small_font)
            + 11.0 * ctx.scale;
    let width_factor =
        (available / (reference_width - 14.0 * ctx.scale).max(1.0)).max(0.05);
    let height_factor = (rect.height() / (45.0 * ctx.scale)).max(0.05);
    let factor = width_factor.min(height_factor).min(8.0);

    let mut big_font = ctx.timer_font(base_big * factor, spec.bold);
    let mut small_font = ctx.timer_font(base_small * factor, spec.bold);
    let mut big_width = text_width(painter, &big_text, &big_font);
    let mut small_width = text_width(painter, &small_text, &small_font);
    let mut total = big_width + small_width;
    if total > available && total > 0.0 {
        let shrink = available / total;
        big_font = ctx.timer_font(base_big * factor * shrink, spec.bold);
        small_font = ctx.timer_font(base_small * factor * shrink, spec.bold);
        big_width = text_width(painter, &big_text, &big_font);
        small_width = text_width(painter, &small_text, &small_font);
        total = big_width + small_width;
    }

    let color = if gradient {
        timer_gradient_color(color)
    } else {
        color
    };
    let center_y = rect.center().y;
    let left = if center {
        rect.center().x - total / 2.0
    } else {
        rect.right() - pad - total
    };
    let small_y = center_y + (big_font.size - small_font.size) * 0.34;

    themed_text(
        painter,
        egui::pos2(left, center_y),
        Align2::LEFT_CENTER,
        &big_text,
        big_font,
        color,
        ctx.theme,
    );
    themed_text(
        painter,
        egui::pos2(left + big_width, small_y),
        Align2::LEFT_CENTER,
        &small_text,
        small_font,
        color,
        ctx.theme,
    );
    total
}

/// Draw a key on the left and a value on the right, optionally in two rows.
#[allow(clippy::too_many_arguments)]
fn draw_key_value(
    painter: &egui::Painter,
    rect: Rect,
    ctx: &ComponentContext,
    key: &str,
    value: &str,
    key_color: Color32,
    value_color: Color32,
    two_rows: bool,
) {
    if rect.width() <= 0.0 || rect.height() <= 0.0 {
        return;
    }
    let scale = ctx.scale;
    let pad = (8.0 * scale).min(rect.width() * 0.2).max(1.0);
    let available_width = (rect.width() - 2.0 * pad).max(1.0);
    let unscaled_height = rect.height() / scale;

    if two_rows {
        let max_size = (unscaled_height * 0.45).max(4.0);
        let key_font = sized_font(ctx, &ctx.theme.text_font, max_size);
        let value_font = sized_font(ctx, &ctx.theme.times_font, max_size);
        let key = fit_text(painter, key, &key_font, available_width);
        let value = fit_text(painter, value, &value_font, available_width);
        let inset = (2.0 * scale).min(rect.height() * 0.1);
        themed_text(
            painter,
            egui::pos2(rect.left() + pad, rect.top() + inset),
            Align2::LEFT_TOP,
            &key,
            key_font,
            key_color,
            ctx.theme,
        );
        themed_text(
            painter,
            egui::pos2(rect.right() - pad, rect.bottom() - inset),
            Align2::RIGHT_BOTTOM,
            &value,
            value_font,
            value_color,
            ctx.theme,
        );
    } else {
        let max_size = (unscaled_height * 0.72).max(4.0);
        let key_font = sized_font(ctx, &ctx.theme.text_font, max_size);
        let value_font = sized_font(ctx, &ctx.theme.times_font, max_size);
        let value = fit_text(painter, value, &value_font, available_width);
        let value_width = text_width(painter, &value, &value_font);
        let key_width = (available_width - value_width - pad).max(1.0);
        let key = fit_text(painter, key, &key_font, key_width);
        themed_text(
            painter,
            egui::pos2(rect.left() + pad, rect.center().y),
            Align2::LEFT_CENTER,
            &key,
            key_font,
            key_color,
            ctx.theme,
        );
        themed_text(
            painter,
            egui::pos2(rect.right() - pad, rect.center().y),
            Align2::RIGHT_CENTER,
            &value,
            value_font,
            value_color,
            ctx.theme,
        );
    }
}

// ---------------------------------------------------------------------------
// Comparison / timer analysis helpers
// ---------------------------------------------------------------------------

fn effective_comparison(timer: &Timer, setting: &str) -> String {
    let resolved = analysis::resolve_comparison(timer, setting).to_string();
    if resolved == NONE || timer.run.contains_comparison(&resolved) {
        resolved
    } else {
        timer.current_comparison.clone()
    }
}

fn short_comparison(comparison: &str) -> &str {
    comparison.strip_prefix("[Race] ").unwrap_or(comparison)
}

fn timer_time(timer: &Timer, method: TimingMethod) -> Option<TimeSpan> {
    if timer.phase == TimerPhase::NotRunning {
        Some(timer.run.offset)
    } else {
        let time = timer.current_time().get(method);
        if time.is_none() && method == TimingMethod::GameTime {
            timer.current_time().get(TimingMethod::RealTime)
        } else {
            time
        }
    }
}

fn timer_phase_color(
    timer: &Timer,
    theme: &LayoutSettings,
    method: TimingMethod,
    comparison: &str,
) -> Color32 {
    let current = timer.current_time().get(method);
    if timer.phase == TimerPhase::NotRunning || current.is_some_and(|time| time < TimeSpan::ZERO) {
        return theme.not_running_color;
    }
    match timer.phase {
        TimerPhase::Paused => theme.paused_color,
        TimerPhase::Ended => {
            let last_comparison = timer
                .run
                .segments
                .last()
                .and_then(|segment| segment.comparison(comparison).get(method));
            match (last_comparison, current) {
                (Some(target), Some(time)) if time >= target => theme.behind_losing_time_color,
                _ => theme.personal_best_color,
            }
        }
        TimerPhase::Running => {
            let index = timer.current_split_index;
            let Some(segment) = timer.run.segments.get(index.max(0) as usize) else {
                return theme.ahead_gaining_time_color;
            };
            let Some(comparison_time) = segment.comparison(comparison).get(method) else {
                return theme.ahead_gaining_time_color;
            };
            let Some(current) = current else {
                return theme.ahead_gaining_time_color;
            };
            analysis::split_color(
                timer,
                Some(current - comparison_time),
                index,
                true,
                false,
                comparison,
                method,
            )
            .map(|color| theme.color_for_split(color))
            .unwrap_or(theme.ahead_gaining_time_color)
        }
        TimerPhase::NotRunning => theme.not_running_color,
    }
}

/// LiveSplit's per-segment possible time save calculation.
fn possible_time_save_for_segment(
    timer: &Timer,
    split_index: i32,
    comparison: &str,
    method: TimingMethod,
    live: bool,
) -> Option<TimeSpan> {
    if split_index < 0 || split_index as usize >= timer.run.segments.len() {
        return None;
    }
    let mut index = split_index;
    let mut previous_time = TimeSpan::ZERO;
    let mut best_segments = timer
        .run
        .segments
        .get(index as usize)?
        .best_segment_time
        .get(method)?;

    while index > 0 {
        let previous_comparison = timer
            .run
            .segments
            .get((index - 1) as usize)?
            .comparison(comparison)
            .get(method);
        if let Some(time) = previous_comparison {
            previous_time = time;
            break;
        }
        index -= 1;
        best_segments = best_segments
            + timer
                .run
                .segments
                .get(index as usize)?
                .best_segment_time
                .get(method)?;
    }

    let comparison_time = timer
        .run
        .segments
        .get(index as usize)?
        .comparison(comparison)
        .get(method)?;
    let mut time = comparison_time - previous_time - best_segments;

    if live && index == timer.current_split_index {
        if let Some(segment_delta) =
            analysis::get_live_segment_delta(timer, index, comparison, method)
        {
            let segment_delta = TimeSpan::ZERO - segment_delta;
            if segment_delta < time {
                time = segment_delta;
            }
        }
    }

    if time < TimeSpan::ZERO {
        time = TimeSpan::ZERO;
    }
    Some(time)
}

fn current_pace_value(
    timer: &Timer,
    comparison: &str,
    method: TimingMethod,
) -> Option<TimeSpan> {
    let last = timer.run.segments.last()?;
    if timer.phase == TimerPhase::Ended {
        return last.split_time.get(method);
    }
    let last_comparison = last.comparison(comparison).get(method)?;
    match timer.phase {
        TimerPhase::NotRunning => Some(last_comparison),
        TimerPhase::Running | TimerPhase::Paused => {
            let index = timer.current_split_index;
            let mut delta = analysis::get_last_delta(timer, index, comparison, method)
                .unwrap_or(TimeSpan::ZERO);
            if let Some(segment) = timer.run.segments.get(index.max(0) as usize) {
                if let (Some(current), Some(comparison_time)) = (
                    timer.current_time().get(method),
                    segment.comparison(comparison).get(method),
                ) {
                    let live = current - comparison_time;
                    if live > delta {
                        delta = live;
                    }
                }
            }
            Some(last_comparison + delta)
        }
        TimerPhase::Ended => last.split_time.get(method),
    }
}

fn prediction_label(comparison: &str) -> String {
    match comparison {
        PERSONAL_BEST => "Current Pace".to_string(),
        BEST_SEGMENTS => "Best Possible Time".to_string(),
        WORST_SEGMENTS => "Worst Possible Time".to_string(),
        AVERAGE_SEGMENTS => "Predicted Time".to_string(),
        CURRENT_COMPARISON => "Current Pace".to_string(),
        other => format!("Current Pace ({})", short_comparison(other)),
    }
}

fn segment_elapsed_time(timer: &Timer, method: TimingMethod) -> Option<TimeSpan> {
    if timer.phase == TimerPhase::NotRunning {
        return Some(timer.run.offset);
    }
    let current = timer.current_time().get(method).or_else(|| {
        if method == TimingMethod::GameTime {
            timer.current_time().get(TimingMethod::RealTime)
        } else {
            None
        }
    })?;
    let ended_delay = if timer.phase == TimerPhase::Ended { 1 } else { 0 };
    let previous_index = timer.current_split_index - 1 - ended_delay;
    let previous = if previous_index >= 0 {
        timer
            .run
            .segments
            .get(previous_index as usize)
            .and_then(|segment| segment.split_time.get(method))
            .unwrap_or(TimeSpan::ZERO)
    } else {
        TimeSpan::ZERO
    };
    Some(current - previous)
}

fn segment_comparison_time(
    timer: &Timer,
    segment_index: i32,
    comparison: &str,
    method: TimingMethod,
) -> Option<TimeSpan> {
    let index = segment_index.max(0) as usize;
    let current = timer.run.segments.get(index)?;
    if comparison == BEST_SEGMENTS {
        return current.best_segment_time.get(method);
    }
    if index == 0 {
        return current.comparison(comparison).get(method);
    }
    let previous = timer.run.segments.get(index - 1)?;
    Some(
        current.comparison(comparison).get(method)?
            - previous.comparison(comparison).get(method)?,
    )
}

fn segment_display_index(timer: &Timer) -> Option<usize> {
    if timer.current_split_index < 0 || timer.run.segments.is_empty() {
        return None;
    }
    Some((timer.current_split_index as usize).min(timer.run.segments.len() - 1))
}

fn segment_display_name(name: &str, is_last: bool) -> String {
    if let Some(rest) = name.strip_prefix('-') {
        if !is_last {
            return rest.to_string();
        }
    }
    if name.starts_with('{') {
        if let Some(close) = name.find('}') {
            return name[close + 1..].trim().to_string();
        }
    }
    name.to_string()
}

// ---------------------------------------------------------------------------
// Timer
// ---------------------------------------------------------------------------

pub fn timer_from_xml(settings: &XmlElement) -> TimerSettings {
    let defaults = TimerSettings::default();
    let mut result = defaults.clone();

    result.timer_height = value_float(settings, "TimerHeight", defaults.timer_height);
    result.timer_width = value_float(settings, "TimerWidth", defaults.timer_width);
    result.decimals_size = value_float(settings, "DecimalsSize", defaults.decimals_size);
    result.timer_color = value_color(settings, "TimerColor", defaults.timer_color);
    result.background = value_background(settings, defaults.background);
    result.center_timer = value_bool(settings, "CenterTimer", defaults.center_timer);
    result.show_gradient = value_bool(settings, "ShowGradient", defaults.show_gradient);
    result.timing_method = value_timing_method(settings, "TimingMethod", defaults.timing_method);

    result.override_split_colors = match child_text(settings, "OverrideSplitColors") {
        Some(value) => parse_bool(Some(&value), defaults.override_split_colors),
        None => match child_text(settings, "UseSplitColors") {
            Some(value) => !parse_bool(Some(&value), true),
            None => defaults.override_split_colors,
        },
    };

    if let Some(format) = child_text(settings, "TimerFormat") {
        if let Some((digits, accuracy)) = parse_timer_format(&format) {
            result.digits_format = digits;
            result.accuracy = accuracy;
        }
    } else if let Some(accuracy) = child_text(settings, "TimerAccuracy") {
        result.digits_format = DigitsFormat::SingleDigitSeconds;
        result.accuracy = parse_accuracy(Some(&accuracy), defaults.accuracy);
    }

    result
}

pub fn timer_to_xml(settings: &TimerSettings) -> XmlElement {
    let mut root = XmlElement::new("Settings");
    push_text(&mut root, "Version", "1.5");
    push_float(&mut root, "TimerHeight", settings.timer_height);
    push_float(&mut root, "TimerWidth", settings.timer_width);
    push_text(
        &mut root,
        "TimerFormat",
        timer_format_string(settings.digits_format, settings.accuracy),
    );
    push_bool(
        &mut root,
        "OverrideSplitColors",
        settings.override_split_colors,
    );
    push_bool(&mut root, "ShowGradient", settings.show_gradient);
    push_color(&mut root, "TimerColor", settings.timer_color);
    push_background(&mut root, &settings.background);
    push_bool(&mut root, "CenterTimer", settings.center_timer);
    push_text(&mut root, "TimingMethod", settings.timing_method.label());
    push_float(&mut root, "DecimalsSize", settings.decimals_size);
    root
}

pub fn draw_timer(ui: &mut Ui, rect: Rect, ctx: &mut ComponentContext, settings: &TimerSettings) {
    let method = ctx.resolved_timing_method(settings.timing_method);
    let time = timer_time(ctx.timer, method);
    let comparison = ctx.timer.current_comparison.clone();
    let color = if settings.override_split_colors {
        settings.timer_color
    } else {
        timer_phase_color(ctx.timer, ctx.theme, method, &comparison)
    };

    let painter = ui.painter();
    paint_timer_background(painter, rect, &settings.background, color);

    let text = if time.is_some() {
        format_with_digits(time, settings.digits_format, settings.accuracy)
    } else {
        String::new()
    };
    if text.is_empty() {
        draw_timer_text(
            painter,
            rect,
            ctx,
            "-",
            color,
            settings.center_timer,
            settings.decimals_size,
            settings.show_gradient,
        );
    } else {
        draw_timer_text(
            painter,
            rect,
            ctx,
            &text,
            color,
            settings.center_timer,
            settings.decimals_size,
            settings.show_gradient,
        );
    }
}

pub fn timer_settings_ui(ui: &mut Ui, settings: &mut TimerSettings) {
    egui::Grid::new("timer-settings-grid")
        .num_columns(2)
        .striped(true)
        .show(ui, |ui| {
            ui.label("Layout mode");
            egui::ComboBox::from_id_salt("timer-mode")
                .selected_text(settings.mode.label())
                .show_ui(ui, |ui| {
                    for mode in [LayoutMode::Vertical, LayoutMode::Horizontal] {
                        ui.selectable_value(&mut settings.mode, mode, mode.label());
                    }
                });
            ui.end_row();

            ui.label("Height / Width");
            ui.horizontal(|ui| {
                ui.add(
                    egui::DragValue::new(&mut settings.timer_height)
                        .speed(0.5)
                        .range(1.0..=2000.0)
                        .prefix("H "),
                );
                ui.add(
                    egui::DragValue::new(&mut settings.timer_width)
                        .speed(0.5)
                        .range(1.0..=4000.0)
                        .prefix("W "),
                );
            });
            ui.end_row();

            ui.label("Timer format");
            ui.horizontal(|ui| {
                digits_format_combo(ui, "timer-digits", &mut settings.digits_format);
                accuracy_combo(ui, "timer-accuracy", &mut settings.accuracy);
            });
            ui.end_row();

            ui.label("Decimals size");
            ui.add(
                egui::DragValue::new(&mut settings.decimals_size)
                    .speed(0.5)
                    .range(0.0..=100.0),
            );
            ui.end_row();

            ui.label("Timer color");
            ui.horizontal(|ui| {
                ui.checkbox(
                    &mut settings.override_split_colors,
                    "Override split colors",
                );
                ui.add_enabled_ui(settings.override_split_colors, |ui| {
                    ui.color_edit_button_srgba(&mut settings.timer_color);
                });
            });
            ui.end_row();

            ui.label("Center timer");
            ui.checkbox(&mut settings.center_timer, "");
            ui.end_row();

            ui.label("Timer gradient");
            ui.checkbox(&mut settings.show_gradient, "");
            ui.end_row();

            ui.label("Timing method");
            timing_method_combo(ui, "timer-timing", &mut settings.timing_method);
            ui.end_row();

            background_rgb_row(ui, "timer-background", &mut settings.background);
        });
}

// ---------------------------------------------------------------------------
// Previous Segment
// ---------------------------------------------------------------------------

pub fn previous_segment_from_xml(settings: &XmlElement) -> PreviousSegmentSettings {
    let defaults = PreviousSegmentSettings::default();
    let mut result = defaults.clone();
    result.text_color = value_color(settings, "TextColor", defaults.text_color);
    result.override_text_color =
        value_bool(settings, "OverrideTextColor", defaults.override_text_color);
    result.background = value_background(settings, defaults.background);
    result.delta_accuracy = value_accuracy(settings, "DeltaAccuracy", defaults.delta_accuracy);
    result.time_save_accuracy =
        value_accuracy(settings, "TimeSaveAccuracy", defaults.time_save_accuracy);
    result.drop_decimals = value_bool(settings, "DropDecimals", defaults.drop_decimals);
    result.comparison = value_string(settings, "Comparison", &defaults.comparison);
    result.display_2_rows = value_bool(settings, "Display2Rows", defaults.display_2_rows);
    result.show_possible_time_save = value_bool(
        settings,
        "ShowPossibleTimeSave",
        defaults.show_possible_time_save,
    );
    result
}

pub fn previous_segment_to_xml(settings: &PreviousSegmentSettings) -> XmlElement {
    let mut root = XmlElement::new("Settings");
    push_text(&mut root, "Version", "1.6");
    push_color(&mut root, "TextColor", settings.text_color);
    push_bool(
        &mut root,
        "OverrideTextColor",
        settings.override_text_color,
    );
    push_background(&mut root, &settings.background);
    push_text(
        &mut root,
        "DeltaAccuracy",
        accuracy_name(settings.delta_accuracy),
    );
    push_bool(&mut root, "DropDecimals", settings.drop_decimals);
    push_text(&mut root, "Comparison", &settings.comparison);
    push_bool(&mut root, "Display2Rows", settings.display_2_rows);
    push_bool(
        &mut root,
        "ShowPossibleTimeSave",
        settings.show_possible_time_save,
    );
    push_text(
        &mut root,
        "TimeSaveAccuracy",
        accuracy_name(settings.time_save_accuracy),
    );
    root
}

fn accuracy_name(accuracy: TimeAccuracy) -> &'static str {
    match accuracy {
        TimeAccuracy::Seconds => "Seconds",
        TimeAccuracy::Tenths => "Tenths",
        TimeAccuracy::Hundredths => "Hundredths",
        TimeAccuracy::Milliseconds => "Milliseconds",
    }
}

pub fn draw_previous_segment(
    ui: &mut Ui,
    rect: Rect,
    ctx: &mut ComponentContext,
    settings: &PreviousSegmentSettings,
) {
    let method = ctx.timer.current_timing_method;
    let comparison = effective_comparison(ctx.timer, &settings.comparison);
    let phase = ctx.timer.phase;
    let current_index = ctx.timer.current_split_index;

    let mut label = "Previous Segment".to_string();
    let mut value = None;
    let mut possible_save_index = None;
    let mut value_color = if settings.override_text_color {
        settings.text_color
    } else {
        ctx.theme.text_color
    };

    if phase != TimerPhase::NotRunning {
        let live = analysis::check_live_delta(
            ctx.timer,
            false,
            &comparison,
            method,
            ctx.theme.show_best_segments,
        );
        if let Some(delta) = live {
            label = "Live Segment".to_string();
            value = Some(delta);
            possible_save_index = Some(current_index);
            value_color = analysis::split_color(
                ctx.timer,
                Some(delta),
                current_index,
                false,
                false,
                &comparison,
                method,
            )
            .map(|color| ctx.theme.color_for_split(color))
            .unwrap_or(value_color);
        } else if current_index > 0 {
            value =
                analysis::get_previous_segment_delta(ctx.timer, current_index - 1, &comparison, method);
            possible_save_index = Some(current_index - 1);
            value_color = analysis::split_color(
                ctx.timer,
                value,
                current_index - 1,
                false,
                true,
                &comparison,
                method,
            )
            .map(|color| ctx.theme.color_for_split(color))
            .unwrap_or(value_color);
        } else {
            value_color = analysis::split_color(
                ctx.timer,
                None,
                current_index - 1,
                true,
                true,
                &comparison,
                method,
            )
            .map(|color| ctx.theme.color_for_split(color))
            .unwrap_or(value_color);
        }
    }

    if settings.comparison != CURRENT_COMPARISON {
        label = format!("{label} ({})", short_comparison(&comparison));
    }

    let mut text = format_delta(value, settings.delta_accuracy, settings.drop_decimals);
    if settings.show_possible_time_save {
        let time_save = possible_save_index.and_then(|index| {
            possible_time_save_for_segment(ctx.timer, index, &comparison, method, false)
        });
        text = format!(
            "{text} / {}",
            format_time_accuracy(time_save, settings.time_save_accuracy)
        );
    }

    let key_color = if settings.override_text_color {
        settings.text_color
    } else {
        ctx.theme.text_color
    };
    let painter = ui.painter();
    paint_background(painter, rect, &settings.background);
    draw_key_value(
        painter,
        rect,
        ctx,
        &label,
        &text,
        key_color,
        value_color,
        settings.display_2_rows,
    );
}

pub fn previous_segment_settings_ui(ui: &mut Ui, settings: &mut PreviousSegmentSettings) {
    egui::Grid::new("previous-segment-settings-grid")
        .num_columns(2)
        .striped(true)
        .show(ui, |ui| {
            text_color_row(
                ui,
                "Text color",
                &mut settings.override_text_color,
                &mut settings.text_color,
            );

            ui.label("Comparison");
            comparison_combo(ui, "previous-comparison", &mut settings.comparison);
            ui.end_row();

            ui.label("Delta accuracy");
            accuracy_combo(ui, "previous-delta-accuracy", &mut settings.delta_accuracy);
            ui.end_row();

            ui.label("Time save accuracy");
            accuracy_combo(ui, "previous-save-accuracy", &mut settings.time_save_accuracy);
            ui.end_row();

            ui.label("Drop decimals");
            ui.checkbox(&mut settings.drop_decimals, "");
            ui.end_row();

            ui.label("Show possible time save");
            ui.checkbox(&mut settings.show_possible_time_save, "");
            ui.end_row();

            ui.label("Two rows");
            ui.checkbox(&mut settings.display_2_rows, "");
            ui.end_row();

            background_rgb_row(ui, "previous-background", &mut settings.background);
        });
}

// ---------------------------------------------------------------------------
// Delta
// ---------------------------------------------------------------------------

pub fn delta_from_xml(settings: &XmlElement) -> DeltaSettings {
    let defaults = DeltaSettings::default();
    let mut result = defaults.clone();
    result.text_color = value_color(settings, "TextColor", defaults.text_color);
    result.override_text_color =
        value_bool(settings, "OverrideTextColor", defaults.override_text_color);
    result.accuracy = value_accuracy(settings, "Accuracy", defaults.accuracy);
    result.background = value_background(settings, defaults.background);
    result.comparison = value_string(settings, "Comparison", &defaults.comparison);
    result.display_2_rows = value_bool(settings, "Display2Rows", defaults.display_2_rows);
    result.drop_decimals = value_bool(settings, "DropDecimals", defaults.drop_decimals);
    result.override_text = value_bool(settings, "OverrideText", defaults.override_text);
    result.differential_text =
        value_bool(settings, "DifferentialText", defaults.differential_text);
    result.custom_text = value_string(settings, "CustomText", &defaults.custom_text);
    result
}

pub fn delta_to_xml(settings: &DeltaSettings) -> XmlElement {
    let mut root = XmlElement::new("Settings");
    push_text(&mut root, "Version", "1.4");
    push_color(&mut root, "TextColor", settings.text_color);
    push_bool(&mut root, "OverrideTextColor", settings.override_text_color);
    push_text(&mut root, "Accuracy", accuracy_name(settings.accuracy));
    push_background(&mut root, &settings.background);
    push_text(&mut root, "Comparison", &settings.comparison);
    push_bool(&mut root, "Display2Rows", settings.display_2_rows);
    push_bool(&mut root, "DropDecimals", settings.drop_decimals);
    push_bool(&mut root, "OverrideText", settings.override_text);
    push_bool(&mut root, "DifferentialText", settings.differential_text);
    push_text(&mut root, "CustomText", &settings.custom_text);
    // The port has a single custom label; use it for the ahead label too.
    push_text(&mut root, "CustomTextAhead", &settings.custom_text);
    root
}

pub fn draw_delta(ui: &mut Ui, rect: Rect, ctx: &mut ComponentContext, settings: &DeltaSettings) {
    let method = ctx.timer.current_timing_method;
    let comparison = effective_comparison(ctx.timer, &settings.comparison);
    let index = ctx.timer.current_split_index;
    let mut use_live_delta = false;
    let value = match ctx.timer.phase {
        TimerPhase::Running | TimerPhase::Paused => {
            let mut delta = analysis::get_last_delta(ctx.timer, index, &comparison, method);
            let live = ctx.timer.run.segments.get(index.max(0) as usize).and_then(|segment| {
                let current = ctx.timer.current_time().get(method)?;
                let comparison_time = segment.comparison(&comparison).get(method)?;
                Some(current - comparison_time)
            });
            if let Some(live) = live {
                if delta.is_some_and(|delta| live > delta)
                    || (delta.is_none() && live > TimeSpan::ZERO)
                {
                    delta = Some(live);
                    use_live_delta = true;
                }
            }
            delta
        }
        TimerPhase::Ended => ctx
            .timer
            .run
            .segments
            .last()
            .and_then(|segment| {
                Some(
                    segment.split_time.get(method)?
                        - segment.comparison(&comparison).get(method)?,
                )
            }),
        TimerPhase::NotRunning => None,
    };

    let comparison_name = short_comparison(&comparison).to_string();
    let label = if settings.override_text {
        // The shared settings type has no separate "ahead" label, so a
        // differential label is only shown while behind; the comparison name
        // is kept while ahead.
        if settings.differential_text
            && !value.is_some_and(|value| value < TimeSpan::ZERO)
        {
            comparison_name
        } else {
            settings.custom_text.clone()
        }
    } else {
        comparison_name
    };
    let key_color = if settings.override_text_color {
        settings.text_color
    } else {
        ctx.theme.text_color
    };
    let color_index = index - if use_live_delta { 0 } else { 1 };
    let value_color = analysis::split_color(
        ctx.timer,
        value,
        color_index,
        true,
        false,
        &comparison,
        method,
    )
    .map(|color| ctx.theme.color_for_split(color))
    .unwrap_or(key_color);

    let text = format_delta(value, settings.accuracy, settings.drop_decimals);
    let painter = ui.painter();
    paint_background(painter, rect, &settings.background);
    draw_key_value(
        painter,
        rect,
        ctx,
        &label,
        &text,
        key_color,
        value_color,
        settings.display_2_rows,
    );
}

pub fn delta_settings_ui(ui: &mut Ui, settings: &mut DeltaSettings) {
    egui::Grid::new("delta-settings-grid")
        .num_columns(2)
        .striped(true)
        .show(ui, |ui| {
            text_color_row(
                ui,
                "Text color",
                &mut settings.override_text_color,
                &mut settings.text_color,
            );

            ui.label("Comparison");
            comparison_combo(ui, "delta-comparison", &mut settings.comparison);
            ui.end_row();

            ui.label("Accuracy");
            accuracy_combo(ui, "delta-accuracy", &mut settings.accuracy);
            ui.end_row();

            ui.label("Drop decimals");
            ui.checkbox(&mut settings.drop_decimals, "");
            ui.end_row();

            ui.label("Override text");
            ui.checkbox(&mut settings.override_text, "");
            ui.end_row();

            ui.label("Custom text");
            ui.add(TextEdit::singleline(&mut settings.custom_text).desired_width(180.0));
            ui.end_row();

            ui.label("Differential text");
            ui.checkbox(&mut settings.differential_text, "");
            ui.end_row();

            ui.label("Two rows");
            ui.checkbox(&mut settings.display_2_rows, "");
            ui.end_row();

            background_rgb_row(ui, "delta-background", &mut settings.background);
        });
}

// ---------------------------------------------------------------------------
// Detailed Timer
// ---------------------------------------------------------------------------

pub fn detailed_timer_from_xml(settings: &XmlElement) -> DetailedTimerSettings {
    let defaults = DetailedTimerSettings::default();
    let mut result = defaults.clone();

    result.height = value_float(settings, "Height", defaults.height);
    result.width = value_float(settings, "Width", defaults.width);
    result.timer_color = value_color(settings, "TimerColor", defaults.timer_color);
    result.segment_timer_color =
        value_color(settings, "SegmentTimerColor", defaults.segment_timer_color);
    result.background = value_background(settings, defaults.background);
    result.segment_times_accuracy = value_accuracy(
        settings,
        "SegmentTimesAccuracy",
        defaults.segment_times_accuracy,
    );
    result.segment_times_color =
        value_color(settings, "SegmentTimesColor", defaults.segment_times_color);
    result.show_split_name = value_bool(settings, "ShowSplitName", defaults.show_split_name);
    result.split_name_color =
        value_color(settings, "SplitNameColor", defaults.split_name_color);
    if let Some(font) = child(settings, "SplitNameFont") {
        result.split_name_font = parse_font(font, &defaults.split_name_font);
    }
    result.comparison = value_string(settings, "Comparison", &defaults.comparison);
    result.comparison2 = value_string(settings, "Comparison2", &defaults.comparison2);
    result.hide_comparison = value_bool(settings, "HideComparison", defaults.hide_comparison);
    result.timing_method = value_timing_method(settings, "TimingMethod", defaults.timing_method);
    result.display_icon = value_bool(settings, "DisplayIcon", defaults.display_icon);
    result.icon_size = value_float(settings, "IconSize", defaults.icon_size);

    if let Some(format) = child_text(settings, "TimerFormat") {
        if let Some((digits, accuracy)) = parse_timer_format(&format) {
            result.timer_format = (digits, accuracy);
        }
    } else if let Some(accuracy) = child_text(settings, "TimerAccuracy") {
        result.timer_format = (
            DigitsFormat::SingleDigitSeconds,
            parse_accuracy(Some(&accuracy), defaults.timer_format.1),
        );
    }
    if let Some(format) = child_text(settings, "SegmentTimerFormat") {
        if let Some((digits, accuracy)) = parse_timer_format(&format) {
            result.segment_timer_format = (digits, accuracy);
        }
    } else if let Some(accuracy) = child_text(settings, "SegmentTimerAccuracy") {
        result.segment_timer_format = (
            DigitsFormat::SingleDigitSeconds,
            parse_accuracy(Some(&accuracy), defaults.segment_timer_format.1),
        );
    }

    result
}

pub fn detailed_timer_to_xml(settings: &DetailedTimerSettings) -> XmlElement {
    let mut root = XmlElement::new("Settings");
    push_text(&mut root, "Version", "1.5");
    push_float(&mut root, "Height", settings.height);
    push_float(&mut root, "Width", settings.width);
    // The shared settings struct does not retain this ratio; LiveSplit's
    // default of 40% is written back.
    push_float(&mut root, "SegmentTimerSizeRatio", 40.0);
    push_bool(&mut root, "TimerShowGradient", true);
    push_bool(&mut root, "OverrideTimerColors", true);
    push_bool(&mut root, "SegmentTimerShowGradient", true);
    push_text(
        &mut root,
        "TimerFormat",
        timer_format_string(settings.timer_format.0, settings.timer_format.1),
    );
    push_text(
        &mut root,
        "SegmentTimerFormat",
        timer_format_string(
            settings.segment_timer_format.0,
            settings.segment_timer_format.1,
        ),
    );
    push_text(
        &mut root,
        "SegmentTimesAccuracy",
        accuracy_name(settings.segment_times_accuracy),
    );
    push_color(&mut root, "TimerColor", settings.timer_color);
    push_color(&mut root, "SegmentTimerColor", settings.segment_timer_color);
    push_color(&mut root, "SegmentLabelsColor", Color32::WHITE);
    push_color(&mut root, "SegmentTimesColor", settings.segment_times_color);
    push_element(&mut root, font_to_xml("SegmentLabelsFont", &FontSpec::text()));
    push_element(&mut root, font_to_xml("SegmentTimesFont", &FontSpec::times()));
    push_element(
        &mut root,
        font_to_xml("SplitNameFont", &settings.split_name_font),
    );
    push_bool(&mut root, "DisplayIcon", settings.display_icon);
    push_float(&mut root, "IconSize", settings.icon_size);
    push_bool(&mut root, "ShowSplitName", settings.show_split_name);
    push_color(&mut root, "SplitNameColor", settings.split_name_color);
    push_background(&mut root, &settings.background);
    push_text(&mut root, "Comparison", &settings.comparison);
    push_text(&mut root, "Comparison2", &settings.comparison2);
    push_bool(&mut root, "HideComparison", settings.hide_comparison);
    push_text(&mut root, "TimingMethod", settings.timing_method.label());
    push_float(&mut root, "DecimalsSize", 35.0);
    push_float(&mut root, "SegmentTimerDecimalsSize", 35.0);
    root
}

pub fn draw_detailed_timer(
    ui: &mut Ui,
    rect: Rect,
    ctx: &mut ComponentContext,
    settings: &DetailedTimerSettings,
) {
    let method = ctx.resolved_timing_method(settings.timing_method);
    let timer_color = match ctx.timer.phase {
        TimerPhase::NotRunning => ctx.theme.not_running_color,
        TimerPhase::Paused => ctx.theme.paused_color,
        _ => settings.timer_color,
    };

    let painter = ui.painter();
    paint_timer_background(painter, rect, &settings.background, timer_color);

    let top_height = rect.height() * 0.60;
    let bottom_height = (rect.height() - top_height).max(0.0);
    let pad = 7.0 * ctx.scale;
    let index = segment_display_index(ctx.timer);

    // Icon (if enabled and the current segment has one).
    let mut icon_width = 0.0;
    if settings.display_icon {
        if let Some(segment) = index.and_then(|index| ctx.timer.run.segments.get(index)) {
            if let Some(bytes) = segment.icon.as_deref() {
                if let Some(texture) = ctx.icons.texture(ui.ctx(), bytes) {
                    let max_size = (rect.width() - 14.0 * ctx.scale).max(1.0);
                    let size = (settings.icon_size * ctx.scale).min(max_size).max(1.0);
                    let texture_size = ui
                        .ctx()
                        .tex_manager()
                        .read()
                        .meta(texture)
                        .map(|meta| meta.size)
                        .unwrap_or([1, 1]);
                    let texture_width = texture_size[0].max(1) as f32;
                    let texture_height = texture_size[1].max(1) as f32;
                    let ratio = if texture_width > texture_height {
                        texture_height / texture_width
                    } else {
                        texture_width / texture_height
                    };
                    let draw_size = if texture_width > texture_height {
                        egui::vec2(size, size * ratio)
                    } else {
                        egui::vec2(size * ratio, size)
                    };
                    let icon_rect = Rect::from_center_size(
                        egui::pos2(
                            rect.left() + pad + size / 2.0,
                            rect.center().y,
                        ),
                        draw_size,
                    );
                    painter.image(
                        texture,
                        icon_rect,
                        Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                        Color32::WHITE,
                    );
                    icon_width = size + 7.5 * ctx.scale;
                }
            }
        }
    }

    // Big timer in the top row.
    let timer_time = timer_time(ctx.timer, method);
    let timer_text = format_with_digits(
        timer_time,
        settings.timer_format.0,
        settings.timer_format.1,
    );
    let timer_width = draw_timer_text(
        painter,
        Rect::from_min_max(
            egui::pos2(rect.left(), rect.top()),
            egui::pos2(rect.right(), rect.top() + top_height),
        ),
        ctx,
        &timer_text,
        timer_color,
        false,
        35.0,
        false,
    );

    // Segment name on the left of the timer.
    if settings.show_split_name {
        if let Some(segment) = index.and_then(|index| ctx.timer.run.segments.get(index)) {
            let name_left = rect.left() + icon_width + 5.0 * ctx.scale;
            let name_right = rect.right() - pad - timer_width - 5.0 * ctx.scale;
            let max_width = (name_right - name_left).max(0.0);
            if max_width > 4.0 {
                let name = segment_display_name(
                    &segment.name,
                    index.is_some_and(|index| index + 1 == ctx.timer.run.segments.len()),
                );
                let font = ctx.font(&settings.split_name_font);
                let name = fit_text(painter, &name, &font, max_width);
                themed_text(
                    painter,
                    egui::pos2(name_left, rect.top() + top_height * 0.5),
                    Align2::LEFT_CENTER,
                    &name,
                    font,
                    settings.split_name_color,
                    ctx.theme,
                );
            }
        }
    }

    // Segment timer in the bottom right.
    let segment_time = segment_elapsed_time(ctx.timer, method);
    let segment_text = format_with_digits(
        segment_time,
        settings.segment_timer_format.0,
        settings.segment_timer_format.1,
    );
    let bottom_center = rect.top() + top_height + bottom_height * 0.5;
    let base_size = ((bottom_height * 0.88) / ctx.scale)
        .min(ctx.theme.timer_font.size * 1.8)
        .max(4.0);
    let mut segment_font = ctx.timer_font(base_size, ctx.theme.timer_font.bold);
    let max_segment_width = (rect.width() * 0.55 - 2.0 * pad).max(1.0);
    let mut segment_width = text_width(painter, &segment_text, &segment_font);
    if segment_width > max_segment_width && segment_width > 0.0 {
        segment_font = ctx.timer_font(
            base_size * max_segment_width / segment_width,
            ctx.theme.timer_font.bold,
        );
        segment_width = text_width(painter, &segment_text, &segment_font);
    }
    themed_text(
        painter,
        egui::pos2(rect.right() - pad, bottom_center),
        Align2::RIGHT_CENTER,
        &segment_text,
        segment_font,
        settings.segment_timer_color,
        ctx.theme,
    );

    // Comparison labels and segment times on the bottom left.
    if index.is_some() {
        let (comparison, comparison2, hide_comparison) =
            detailed_timer_comparisons(ctx.timer, settings);
        let right = rect.right() - pad - segment_width - 5.0 * ctx.scale;
        let left = rect.left() + icon_width + 5.0 * ctx.scale;
        let width = (right - left).max(1.0);
        let rows = if hide_comparison { 1 } else { 2 };
        let row_height = (bottom_height / rows as f32).max(1.0);
        let row_size = ((row_height / ctx.scale) * 0.78).max(4.0);
        let label_font = sized_font(ctx, &ctx.theme.text_font, row_size);
        let value_font = sized_font(ctx, &ctx.theme.times_font, row_size);

        let index = index.unwrap_or(0) as i32;
        if let Some(time) =
            segment_comparison_time(ctx.timer, index, &comparison, method)
        {
            let row_center = if hide_comparison {
                rect.top() + top_height + bottom_height * 0.5
            } else {
                rect.top() + top_height + row_height * 0.5
            };
            draw_segment_comparison_row(
                painter,
                ctx,
                left,
                row_center,
                width,
                &comparison,
                time,
                settings.segment_times_accuracy,
                label_font.clone(),
                value_font.clone(),
                settings.segment_times_color,
            );
        }
        if !hide_comparison {
            if let Some(time) =
                segment_comparison_time(ctx.timer, index, &comparison2, method)
            {
                let row_center = rect.top() + top_height + row_height * 1.5;
                draw_segment_comparison_row(
                    painter,
                    ctx,
                    left,
                    row_center,
                    width,
                    &comparison2,
                    time,
                    settings.segment_times_accuracy,
                    label_font,
                    value_font,
                    settings.segment_times_color,
                );
            }
        }
    }
}

fn detailed_timer_comparisons(
    timer: &Timer,
    settings: &DetailedTimerSettings,
) -> (String, String, bool) {
    let mut comparison = analysis::resolve_comparison(timer, &settings.comparison).to_string();
    let comparison2 = analysis::resolve_comparison(timer, &settings.comparison2).to_string();
    let valid1 = comparison != NONE && timer.run.contains_comparison(&comparison);
    let valid2 = comparison2 != NONE && timer.run.contains_comparison(&comparison2);
    let mut hide = settings.hide_comparison || !valid2;

    if hide {
        hide = true;
        if !valid1 {
            comparison = timer.current_comparison.clone();
        }
    } else if !valid1 {
        hide = true;
        comparison = comparison2.clone();
    } else if comparison == comparison2 {
        hide = true;
    }
    if comparison2 == NONE {
        hide = true;
    }
    (comparison, comparison2, hide)
}

#[allow(clippy::too_many_arguments)]
fn draw_segment_comparison_row(
    painter: &egui::Painter,
    ctx: &ComponentContext,
    left: f32,
    center_y: f32,
    width: f32,
    comparison: &str,
    time: TimeSpan,
    accuracy: TimeAccuracy,
    label_font: FontId,
    value_font: FontId,
    value_color: Color32,
) {
    let pad = 4.0 * ctx.scale;
    let label = format!("{}:", short_comparison(comparison));
    let label = fit_text(painter, &label, &label_font, (width * 0.55).max(1.0));
    let label_width = text_width(painter, &label, &label_font);
    let value_width = (width - label_width - pad).max(1.0);
    let value = fit_text(
        painter,
        &format_time_accuracy(Some(time), accuracy),
        &value_font,
        value_width,
    );
    themed_text(
        painter,
        egui::pos2(left, center_y),
        Align2::LEFT_CENTER,
        &label,
        label_font,
        ctx.theme.text_color,
        ctx.theme,
    );
    themed_text(
        painter,
        egui::pos2(left + label_width + pad, center_y),
        Align2::LEFT_CENTER,
        &value,
        value_font,
        value_color,
        ctx.theme,
    );
}

pub fn detailed_timer_settings_ui(ui: &mut Ui, settings: &mut DetailedTimerSettings) {
    egui::Grid::new("detailed-timer-settings-grid")
        .num_columns(2)
        .striped(true)
        .show(ui, |ui| {
            ui.label("Height / Width");
            ui.horizontal(|ui| {
                ui.add(
                    egui::DragValue::new(&mut settings.height)
                        .speed(0.5)
                        .range(1.0..=2000.0)
                        .prefix("H "),
                );
                ui.add(
                    egui::DragValue::new(&mut settings.width)
                        .speed(0.5)
                        .range(1.0..=4000.0)
                        .prefix("W "),
                );
            });
            ui.end_row();

            ui.label("Timer format");
            ui.horizontal(|ui| {
                digits_format_combo(ui, "detailed-timer-digits", &mut settings.timer_format.0);
                accuracy_combo(ui, "detailed-timer-accuracy", &mut settings.timer_format.1);
            });
            ui.end_row();

            ui.label("Segment timer format");
            ui.horizontal(|ui| {
                digits_format_combo(
                    ui,
                    "detailed-segment-digits",
                    &mut settings.segment_timer_format.0,
                );
                accuracy_combo(
                    ui,
                    "detailed-segment-accuracy",
                    &mut settings.segment_timer_format.1,
                );
            });
            ui.end_row();

            ui.label("Segment times accuracy");
            accuracy_combo(
                ui,
                "detailed-segment-times-accuracy",
                &mut settings.segment_times_accuracy,
            );
            ui.end_row();

            ui.label("Timer color");
            ui.color_edit_button_srgba(&mut settings.timer_color);
            ui.end_row();

            ui.label("Segment timer color");
            ui.color_edit_button_srgba(&mut settings.segment_timer_color);
            ui.end_row();

            ui.label("Segment times color");
            ui.color_edit_button_srgba(&mut settings.segment_times_color);
            ui.end_row();

            ui.label("Show split name");
            ui.checkbox(&mut settings.show_split_name, "");
            ui.end_row();

            ui.label("Split name color");
            ui.color_edit_button_srgba(&mut settings.split_name_color);
            ui.end_row();

            ui.label("Split name font");
            font_ui(ui, &mut settings.split_name_font);
            ui.end_row();

            ui.label("Comparison");
            comparison_combo(ui, "detailed-comparison", &mut settings.comparison);
            ui.end_row();

            ui.label("Second comparison");
            comparison_combo(ui, "detailed-comparison2", &mut settings.comparison2);
            ui.end_row();

            ui.label("Hide comparison");
            ui.checkbox(&mut settings.hide_comparison, "");
            ui.end_row();

            ui.label("Timing method");
            timing_method_combo(ui, "detailed-timing", &mut settings.timing_method);
            ui.end_row();

            ui.label("Display icon");
            ui.checkbox(&mut settings.display_icon, "");
            ui.end_row();

            ui.label("Icon size");
            ui.add(
                egui::DragValue::new(&mut settings.icon_size)
                    .speed(0.5)
                    .range(0.0..=200.0),
            );
            ui.end_row();

            background_rgb_row(ui, "detailed-background", &mut settings.background);
        });
}

// ---------------------------------------------------------------------------
// Possible Time Save
// ---------------------------------------------------------------------------

pub fn possible_time_save_from_xml(settings: &XmlElement) -> PossibleTimeSaveSettings {
    let defaults = PossibleTimeSaveSettings::default();
    let mut result = defaults.clone();
    result.text_color = value_color(settings, "TextColor", defaults.text_color);
    result.override_text_color =
        value_bool(settings, "OverrideTextColor", defaults.override_text_color);
    result.time_color = value_color(settings, "TimeColor", defaults.time_color);
    result.override_time_color =
        value_bool(settings, "OverrideTimeColor", defaults.override_time_color);
    result.accuracy = value_accuracy(settings, "Accuracy", defaults.accuracy);
    result.drop_decimals = value_bool(settings, "DropDecimals", defaults.drop_decimals);
    result.background = value_background(settings, defaults.background);
    result.comparison = value_string(settings, "Comparison", &defaults.comparison);
    result.display_2_rows = value_bool(settings, "Display2Rows", defaults.display_2_rows);
    result.total_time_save = value_bool(settings, "TotalTimeSave", defaults.total_time_save);
    result
}

pub fn possible_time_save_to_xml(settings: &PossibleTimeSaveSettings) -> XmlElement {
    let mut root = XmlElement::new("Settings");
    push_text(&mut root, "Version", "1.5");
    push_color(&mut root, "TextColor", settings.text_color);
    push_bool(&mut root, "OverrideTextColor", settings.override_text_color);
    push_color(&mut root, "TimeColor", settings.time_color);
    push_bool(&mut root, "OverrideTimeColor", settings.override_time_color);
    push_text(&mut root, "Accuracy", accuracy_name(settings.accuracy));
    push_bool(&mut root, "DropDecimals", settings.drop_decimals);
    push_background(&mut root, &settings.background);
    push_text(&mut root, "Comparison", &settings.comparison);
    push_bool(&mut root, "Display2Rows", settings.display_2_rows);
    push_bool(&mut root, "TotalTimeSave", settings.total_time_save);
    root
}

pub fn draw_possible_time_save(
    ui: &mut Ui,
    rect: Rect,
    ctx: &mut ComponentContext,
    settings: &PossibleTimeSaveSettings,
) {
    let method = ctx.timer.current_timing_method;
    let comparison = effective_comparison(ctx.timer, &settings.comparison);
    let value = if settings.total_time_save {
        if ctx.timer.phase == TimerPhase::Ended {
            Some(TimeSpan::ZERO)
        } else {
            let start = ctx.timer.current_split_index.max(0) as usize;
            let mut total = TimeSpan::ZERO;
            for index in start..ctx.timer.run.segments.len() {
                if let Some(save) = possible_time_save_for_segment(
                    ctx.timer,
                    index as i32,
                    &comparison,
                    method,
                    true,
                ) {
                    total = total + save;
                }
            }
            Some(total)
        }
    } else if matches!(
        ctx.timer.phase,
        TimerPhase::Running | TimerPhase::Paused
    ) {
        possible_time_save_for_segment(
            ctx.timer,
            ctx.timer.current_split_index,
            &comparison,
            method,
            false,
        )
    } else {
        None
    };

    let mut label = if settings.total_time_save {
        "Total Possible Time Save".to_string()
    } else {
        "Possible Time Save".to_string()
    };
    if settings.comparison != CURRENT_COMPARISON {
        label = format!("{label} ({})", short_comparison(&comparison));
    }
    let key_color = if settings.override_text_color {
        settings.text_color
    } else {
        ctx.theme.text_color
    };
    let value_color = if settings.override_time_color {
        settings.time_color
    } else {
        ctx.theme.text_color
    };
    let text = format_time_with_drop(value, settings.accuracy, settings.drop_decimals);

    let painter = ui.painter();
    paint_background(painter, rect, &settings.background);
    draw_key_value(
        painter,
        rect,
        ctx,
        &label,
        &text,
        key_color,
        value_color,
        settings.display_2_rows,
    );
}

fn format_time_with_drop(
    time: Option<TimeSpan>,
    accuracy: TimeAccuracy,
    drop_decimals: bool,
) -> String {
    let mut formatter = general_formatter(DigitsFormat::SingleDigitSeconds, accuracy);
    formatter.drop_decimals = drop_decimals;
    formatter.format(time)
}

pub fn possible_time_save_settings_ui(ui: &mut Ui, settings: &mut PossibleTimeSaveSettings) {
    egui::Grid::new("possible-time-save-settings-grid")
        .num_columns(2)
        .striped(true)
        .show(ui, |ui| {
            text_color_row(
                ui,
                "Text color",
                &mut settings.override_text_color,
                &mut settings.text_color,
            );
            text_color_row(
                ui,
                "Time color",
                &mut settings.override_time_color,
                &mut settings.time_color,
            );

            ui.label("Comparison");
            comparison_combo(ui, "possible-comparison", &mut settings.comparison);
            ui.end_row();

            ui.label("Accuracy");
            accuracy_combo(ui, "possible-accuracy", &mut settings.accuracy);
            ui.end_row();

            ui.label("Drop decimals");
            ui.checkbox(&mut settings.drop_decimals, "");
            ui.end_row();

            ui.label("Total time save");
            ui.checkbox(&mut settings.total_time_save, "");
            ui.end_row();

            ui.label("Two rows");
            ui.checkbox(&mut settings.display_2_rows, "");
            ui.end_row();

            background_rgb_row(ui, "possible-background", &mut settings.background);
        });
}

// ---------------------------------------------------------------------------
// Current Comparison
// ---------------------------------------------------------------------------

pub fn current_comparison_from_xml(settings: &XmlElement) -> CurrentComparisonSettings {
    let defaults = CurrentComparisonSettings::default();
    let mut result = defaults.clone();
    result.text_color = value_color(settings, "TextColor", defaults.text_color);
    result.override_text_color =
        value_bool(settings, "OverrideTextColor", defaults.override_text_color);
    result.time_color = value_color(settings, "TimeColor", defaults.time_color);
    result.override_time_color =
        value_bool(settings, "OverrideTimeColor", defaults.override_time_color);
    result.background = value_background(settings, defaults.background);
    result.display_2_rows = value_bool(settings, "Display2Rows", defaults.display_2_rows);
    result
}

pub fn current_comparison_to_xml(settings: &CurrentComparisonSettings) -> XmlElement {
    let mut root = XmlElement::new("Settings");
    push_text(&mut root, "Version", "1.4");
    push_color(&mut root, "TextColor", settings.text_color);
    push_bool(&mut root, "OverrideTextColor", settings.override_text_color);
    push_color(&mut root, "TimeColor", settings.time_color);
    push_bool(&mut root, "OverrideTimeColor", settings.override_time_color);
    push_background(&mut root, &settings.background);
    push_bool(&mut root, "Display2Rows", settings.display_2_rows);
    root
}

pub fn draw_current_comparison(
    ui: &mut Ui,
    rect: Rect,
    ctx: &mut ComponentContext,
    settings: &CurrentComparisonSettings,
) {
    let key_color = if settings.override_text_color {
        settings.text_color
    } else {
        ctx.theme.text_color
    };
    let value_color = if settings.override_time_color {
        settings.time_color
    } else {
        ctx.theme.text_color
    };
    let painter = ui.painter();
    paint_background(painter, rect, &settings.background);
    draw_key_value(
        painter,
        rect,
        ctx,
        "Comparing Against",
        &ctx.timer.current_comparison,
        key_color,
        value_color,
        settings.display_2_rows,
    );
}

pub fn current_comparison_settings_ui(ui: &mut Ui, settings: &mut CurrentComparisonSettings) {
    egui::Grid::new("current-comparison-settings-grid")
        .num_columns(2)
        .striped(true)
        .show(ui, |ui| {
            text_color_row(
                ui,
                "Text color",
                &mut settings.override_text_color,
                &mut settings.text_color,
            );
            text_color_row(
                ui,
                "Time color",
                &mut settings.override_time_color,
                &mut settings.time_color,
            );

            ui.label("Two rows");
            ui.checkbox(&mut settings.display_2_rows, "");
            ui.end_row();

            background_rgb_row(ui, "current-comparison-background", &mut settings.background);
        });
}

// ---------------------------------------------------------------------------
// Current Pace / Run Prediction
// ---------------------------------------------------------------------------

pub fn current_pace_from_xml(settings: &XmlElement) -> CurrentPaceSettings {
    let defaults = CurrentPaceSettings::default();
    let mut result = defaults.clone();
    result.text_color = value_color(settings, "TextColor", defaults.text_color);
    result.override_text_color =
        value_bool(settings, "OverrideTextColor", defaults.override_text_color);
    result.time_color = value_color(settings, "TimeColor", defaults.time_color);
    result.override_time_color =
        value_bool(settings, "OverrideTimeColor", defaults.override_time_color);
    result.background = value_background(settings, defaults.background);
    result.comparison = value_string(settings, "Comparison", &defaults.comparison);
    result.accuracy = value_accuracy(settings, "Accuracy", defaults.accuracy);
    result.display_2_rows = value_bool(settings, "Display2Rows", defaults.display_2_rows);
    result
}

pub fn current_pace_to_xml(settings: &CurrentPaceSettings) -> XmlElement {
    let mut root = XmlElement::new("Settings");
    push_text(&mut root, "Version", "1.4");
    push_color(&mut root, "TextColor", settings.text_color);
    push_bool(&mut root, "OverrideTextColor", settings.override_text_color);
    push_color(&mut root, "TimeColor", settings.time_color);
    push_bool(&mut root, "OverrideTimeColor", settings.override_time_color);
    push_text(&mut root, "Accuracy", accuracy_name(settings.accuracy));
    push_background(&mut root, &settings.background);
    push_text(&mut root, "Comparison", &settings.comparison);
    push_bool(&mut root, "Display2Rows", settings.display_2_rows);
    root
}

pub fn draw_current_pace(
    ui: &mut Ui,
    rect: Rect,
    ctx: &mut ComponentContext,
    settings: &CurrentPaceSettings,
) {
    draw_prediction(
        ui,
        rect,
        ctx,
        &settings.comparison,
        settings.accuracy,
        settings.display_2_rows,
        settings.override_text_color,
        settings.text_color,
        settings.override_time_color,
        settings.time_color,
        &settings.background,
    );
}

pub fn current_pace_settings_ui(ui: &mut Ui, settings: &mut CurrentPaceSettings) {
    egui::Grid::new("current-pace-settings-grid")
        .num_columns(2)
        .striped(true)
        .show(ui, |ui| {
            text_color_row(
                ui,
                "Text color",
                &mut settings.override_text_color,
                &mut settings.text_color,
            );
            text_color_row(
                ui,
                "Time color",
                &mut settings.override_time_color,
                &mut settings.time_color,
            );

            ui.label("Comparison");
            comparison_combo(ui, "current-pace-comparison", &mut settings.comparison);
            ui.end_row();

            ui.label("Accuracy");
            accuracy_combo(ui, "current-pace-accuracy", &mut settings.accuracy);
            ui.end_row();

            ui.label("Two rows");
            ui.checkbox(&mut settings.display_2_rows, "");
            ui.end_row();

            background_rgb_row(ui, "current-pace-background", &mut settings.background);
        });
}

pub fn run_prediction_from_xml(settings: &XmlElement) -> RunPredictionSettings {
    let defaults = RunPredictionSettings::default();
    let mut result = defaults.clone();
    result.text_color = value_color(settings, "TextColor", defaults.text_color);
    result.override_text_color =
        value_bool(settings, "OverrideTextColor", defaults.override_text_color);
    result.time_color = value_color(settings, "TimeColor", defaults.time_color);
    result.override_time_color =
        value_bool(settings, "OverrideTimeColor", defaults.override_time_color);
    result.accuracy = value_accuracy(settings, "Accuracy", defaults.accuracy);
    result.background = value_background(settings, defaults.background);
    result.comparison = value_string(settings, "Comparison", &defaults.comparison);
    result.display_2_rows = value_bool(settings, "Display2Rows", defaults.display_2_rows);
    result
}

pub fn run_prediction_to_xml(settings: &RunPredictionSettings) -> XmlElement {
    let mut root = XmlElement::new("Settings");
    push_text(&mut root, "Version", "1.4");
    push_color(&mut root, "TextColor", settings.text_color);
    push_bool(&mut root, "OverrideTextColor", settings.override_text_color);
    push_color(&mut root, "TimeColor", settings.time_color);
    push_bool(&mut root, "OverrideTimeColor", settings.override_time_color);
    push_text(&mut root, "Accuracy", accuracy_name(settings.accuracy));
    push_background(&mut root, &settings.background);
    push_text(&mut root, "Comparison", &settings.comparison);
    push_bool(&mut root, "Display2Rows", settings.display_2_rows);
    root
}

pub fn draw_run_prediction(
    ui: &mut Ui,
    rect: Rect,
    ctx: &mut ComponentContext,
    settings: &RunPredictionSettings,
) {
    draw_prediction(
        ui,
        rect,
        ctx,
        &settings.comparison,
        settings.accuracy,
        settings.display_2_rows,
        settings.override_text_color,
        settings.text_color,
        settings.override_time_color,
        settings.time_color,
        &settings.background,
    );
}

pub fn run_prediction_settings_ui(ui: &mut Ui, settings: &mut RunPredictionSettings) {
    egui::Grid::new("run-prediction-settings-grid")
        .num_columns(2)
        .striped(true)
        .show(ui, |ui| {
            text_color_row(
                ui,
                "Text color",
                &mut settings.override_text_color,
                &mut settings.text_color,
            );
            text_color_row(
                ui,
                "Time color",
                &mut settings.override_time_color,
                &mut settings.time_color,
            );

            ui.label("Comparison");
            comparison_combo(ui, "run-prediction-comparison", &mut settings.comparison);
            ui.end_row();

            ui.label("Accuracy");
            accuracy_combo(ui, "run-prediction-accuracy", &mut settings.accuracy);
            ui.end_row();

            ui.label("Two rows");
            ui.checkbox(&mut settings.display_2_rows, "");
            ui.end_row();

            background_rgb_row(ui, "run-prediction-background", &mut settings.background);
        });
}

#[allow(clippy::too_many_arguments)]
fn draw_prediction(
    ui: &mut Ui,
    rect: Rect,
    ctx: &mut ComponentContext,
    comparison: &str,
    accuracy: TimeAccuracy,
    display_2_rows: bool,
    override_text_color: bool,
    text_color: Color32,
    override_time_color: bool,
    time_color: Color32,
    background: &BackgroundStyle,
) {
    let method = ctx.timer.current_timing_method;
    let comparison = effective_comparison(ctx.timer, comparison);
    let label = prediction_label(&comparison);
    let value = if ctx.timer.phase == TimerPhase::NotRunning
        && label.starts_with("Current Pace")
    {
        None
    } else {
        current_pace_value(ctx.timer, &comparison, method)
    };
    let key_color = if override_text_color {
        text_color
    } else {
        ctx.theme.text_color
    };
    let value_color = if override_time_color {
        time_color
    } else {
        ctx.theme.text_color
    };
    let text = format_time_accuracy(value, accuracy);
    let painter = ui.painter();
    paint_background(painter, rect, background);
    draw_key_value(
        painter,
        rect,
        ctx,
        &label,
        &text,
        key_color,
        value_color,
        display_2_rows,
    );
}

// ---------------------------------------------------------------------------
// Manual Game Time
// ---------------------------------------------------------------------------

pub fn manual_game_time_from_xml(settings: &XmlElement) -> ManualGameTimeSettings {
    let defaults = ManualGameTimeSettings::default();
    ManualGameTimeSettings {
        use_segment_times: value_bool(settings, "UseSegmentTimes", defaults.use_segment_times),
    }
}

pub fn manual_game_time_to_xml(settings: &ManualGameTimeSettings) -> XmlElement {
    let mut root = XmlElement::new("Settings");
    push_text(&mut root, "Version", "1.4");
    push_bool(&mut root, "UseSegmentTimes", settings.use_segment_times);
    root
}

pub fn draw_manual_game_time(
    ui: &mut Ui,
    rect: Rect,
    ctx: &mut ComponentContext,
    settings: &ManualGameTimeSettings,
) {
    let method = TimingMethod::GameTime;
    let value = if settings.use_segment_times {
        segment_elapsed_time(ctx.timer, method)
            .or_else(|| segment_elapsed_time(ctx.timer, TimingMethod::RealTime))
    } else {
        ctx.timer
            .current_time()
            .game_time
            .or_else(|| ctx.timer.current_time().real_time)
    };
    let text = format_time_accuracy(value, TimeAccuracy::Hundredths);
    let painter = ui.painter();
    draw_key_value(
        painter,
        rect,
        ctx,
        "Game Time",
        &text,
        ctx.theme.text_color,
        ctx.theme.text_color,
        false,
    );
}

pub fn manual_game_time_settings_ui(ui: &mut Ui, settings: &mut ManualGameTimeSettings) {
    ui.checkbox(&mut settings.use_segment_times, "Use segment times");
}

// ---------------------------------------------------------------------------
// Sum of Segments
// ---------------------------------------------------------------------------

pub fn draw_sum_of_segments(ui: &mut Ui, rect: Rect, ctx: &mut ComponentContext) {
    let total: Option<TimeSpan> = ctx
        .timer
        .run
        .segments
        .iter()
        .map(|segment| {
            segment
                .best_segment_time
                .get(ctx.timer.current_timing_method)
        })
        .try_fold(TimeSpan::ZERO, |total, time| {
            time.map(|time| total + time)
        });
    let text = format_time_accuracy(total, TimeAccuracy::Seconds);
    let painter = ui.painter();
    draw_key_value(
        painter,
        rect,
        ctx,
        "Sum of Segments",
        &text,
        ctx.theme.text_color,
        ctx.theme.text_color,
        false,
    );
}

// ---------------------------------------------------------------------------
// Settings widget helpers
// ---------------------------------------------------------------------------

fn accuracy_combo(ui: &mut Ui, id: &str, accuracy: &mut TimeAccuracy) {
    egui::ComboBox::from_id_salt(id)
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

fn digits_format_combo(ui: &mut Ui, id: &str, digits: &mut DigitsFormat) {
    egui::ComboBox::from_id_salt(id)
        .selected_text(digits.label())
        .show_ui(ui, |ui| {
            for value in [
                DigitsFormat::SingleDigitSeconds,
                DigitsFormat::DoubleDigitSeconds,
                DigitsFormat::SingleDigitMinutes,
                DigitsFormat::DoubleDigitMinutes,
                DigitsFormat::SingleDigitHours,
                DigitsFormat::DoubleDigitHours,
            ] {
                ui.selectable_value(digits, value, value.label());
            }
        });
}

fn timing_method_combo(ui: &mut Ui, id: &str, method: &mut TimingMethodSetting) {
    egui::ComboBox::from_id_salt(id)
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
}

fn comparison_combo(ui: &mut Ui, id: &str, comparison: &mut String) {
    ui.horizontal(|ui| {
        let selected = comparison.clone();
        let custom = (!COMPARISONS.contains(&comparison.as_str())).then(|| comparison.clone());
        egui::ComboBox::from_id_salt(id)
            .selected_text(selected)
            .show_ui(ui, |ui| {
                for option in COMPARISONS {
                    ui.selectable_value(comparison, (*option).to_string(), *option);
                }
                if let Some(custom) = custom {
                    ui.selectable_value(comparison, custom.clone(), custom);
                }
            });
        ui.add(TextEdit::singleline(comparison).desired_width(90.0));
    });
}

fn gradient_combo(ui: &mut Ui, id: &str, gradient: &mut GradientType) {
    egui::ComboBox::from_id_salt(id)
        .selected_text(gradient.label())
        .show_ui(ui, |ui| {
            for value in [
                GradientType::Plain,
                GradientType::Vertical,
                GradientType::Horizontal,
                GradientType::Alternating,
                GradientType::PlainWithDeltaColor,
                GradientType::VerticalWithDeltaColor,
                GradientType::HorizontalWithDeltaColor,
            ] {
                ui.selectable_value(gradient, value, value.label());
            }
        });
}

fn text_color_row(
    ui: &mut Ui,
    label: &str,
    override_color: &mut bool,
    color: &mut Color32,
) {
    ui.label(label);
    ui.horizontal(|ui| {
        ui.checkbox(override_color, "Override");
        ui.add_enabled_ui(*override_color, |ui| {
            ui.color_edit_button_srgba(color);
        });
    });
    ui.end_row();
}

fn background_rgb_row(ui: &mut Ui, id: &str, background: &mut BackgroundStyle) {
    ui.label("Background");
    ui.vertical(|ui| {
        gradient_combo(ui, id, &mut background.gradient);
        ui.horizontal(|ui| {
            ui.label("1");
            ui.color_edit_button_srgba(&mut background.color);
            ui.label("2");
            ui.color_edit_button_srgba(&mut background.color2);
        });
    });
    ui.end_row();
}

fn font_ui(ui: &mut Ui, font: &mut FontSpec) {
    ui.horizontal(|ui| {
        if ui
            .add(TextEdit::singleline(&mut font.name).desired_width(110.0))
            .changed()
        {
            font.raw = None;
        }
        if ui
            .add(
                egui::DragValue::new(&mut font.size)
                    .speed(0.25)
                    .range(4.0..=200.0),
            )
            .changed()
        {
            font.raw = None;
        }
        if ui.checkbox(&mut font.bold, "Bold").changed() {
            font.raw = None;
        }
        if ui.checkbox(&mut font.italic, "Italic").changed() {
            font.raw = None;
        }
    });
}
