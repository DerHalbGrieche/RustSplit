//! Basic/informational components: title, text, separators, counters and
//! informational widgets.
//!
//! The XML readers/writers mirror LiveSplit's component settings.  Rendering
//! is intentionally self contained: it only uses the timer/run model and the
//! shared [`ComponentContext`].

use egui::{Align2, Color32, FontId, Rect, Ui};
use livesplit_core::{
    format_time, GeneralTimeFormatter, Run, TimeAccuracy, TimeSpan, Timer, TimerPhase,
    TimingMethod,
};
use xmltree::XMLNode;

use crate::canvas::{fit_text, paint_background, paint_text, text_width};
use crate::components::{
    BlankSpaceSettings, ComparisonTimeSettings, ComponentContext, CounterSettings,
    HotkeyIndicatorSettings, SeparatorSettings, SumOfBestSettings, TextSettings,
    ThinSeparatorSettings, TimingMethodSetting, TitleSettings, TotalPlaytimeSettings,
    WorldRecordSettings,
};
use crate::layout::{color_to_hex, parse_color, BackgroundStyle, FontSpec, GradientType};
use crate::layout_xml::{font_to_xml, parse_font};
use crate::xml::{
    child, child_text, element_with_text, parse_bool, parse_f32, parse_i32, XmlElement,
};

// ---------------------------------------------------------------------------
// XML helpers
// ---------------------------------------------------------------------------

fn push_element(parent: &mut XmlElement, element: XmlElement) {
    parent.children.push(XMLNode::Element(element));
}

fn push_text(parent: &mut XmlElement, name: &str, value: impl Into<String>) {
    parent
        .children
        .push(XMLNode::Element(element_with_text(name, value)));
}

fn push_bool(parent: &mut XmlElement, name: &str, value: bool) {
    push_text(parent, name, if value { "True" } else { "False" });
}

fn push_i32(parent: &mut XmlElement, name: &str, value: i32) {
    push_text(parent, name, value.to_string());
}

fn push_f32(parent: &mut XmlElement, name: &str, value: f32) {
    push_text(parent, name, value.to_string());
}

fn push_color(parent: &mut XmlElement, name: &str, color: Color32) {
    push_text(parent, name, color_to_hex(color));
}

fn push_font(parent: &mut XmlElement, name: &str, font: &FontSpec) {
    push_element(parent, font_to_xml(name, font));
}

fn bool_value(element: &XmlElement, name: &str, default: bool) -> bool {
    parse_bool(child_text(element, name).as_deref(), default)
}

fn f32_value(element: &XmlElement, name: &str, default: f32) -> f32 {
    parse_f32(child_text(element, name).as_deref(), default)
}

fn i32_value(element: &XmlElement, name: &str, default: i32) -> i32 {
    parse_i32(child_text(element, name).as_deref(), default)
}

fn background_from_xml(element: &XmlElement, default: &BackgroundStyle) -> BackgroundStyle {
    let mut background = *default;
    background.color = parse_color(
        child_text(element, "BackgroundColor").as_deref(),
        background.color,
    );
    background.color2 = parse_color(
        child_text(element, "BackgroundColor2").as_deref(),
        background.color2,
    );
    if let Some(gradient) = child_text(element, "BackgroundGradient") {
        background.gradient = GradientType::parse(&gradient, background.gradient);
    }
    background
}

fn push_background(parent: &mut XmlElement, background: &BackgroundStyle) {
    push_color(parent, "BackgroundColor", background.color);
    push_color(parent, "BackgroundColor2", background.color2);
    push_text(
        parent,
        "BackgroundGradient",
        gradient_xml_name(background.gradient),
    );
}

fn gradient_xml_name(gradient: GradientType) -> &'static str {
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

fn accuracy_from_xml(value: Option<&str>, default: TimeAccuracy) -> TimeAccuracy {
    match value
        .map(|value| value.trim().to_ascii_lowercase())
        .unwrap_or_default()
        .as_str()
    {
        "seconds" => TimeAccuracy::Seconds,
        "tenths" => TimeAccuracy::Tenths,
        "hundredths" => TimeAccuracy::Hundredths,
        "milliseconds" => TimeAccuracy::Milliseconds,
        _ => default,
    }
}

fn accuracy_xml_name(accuracy: TimeAccuracy) -> &'static str {
    match accuracy {
        TimeAccuracy::Seconds => "Seconds",
        TimeAccuracy::Tenths => "Tenths",
        TimeAccuracy::Hundredths => "Hundredths",
        TimeAccuracy::Milliseconds => "Milliseconds",
    }
}

fn timing_method_from_xml(value: Option<&str>, default: TimingMethodSetting) -> TimingMethodSetting {
    value
        .map(|value| TimingMethodSetting::parse(value, default))
        .unwrap_or(default)
}

fn timing_method_xml_name(method: TimingMethodSetting) -> &'static str {
    match method {
        TimingMethodSetting::Current => "Current Timing Method",
        TimingMethodSetting::RealTime => "Real Time",
        TimingMethodSetting::GameTime => "Game Time",
    }
}

fn comparison_kind_from_xml(value: Option<&str>) -> i32 {
    match value
        .map(|value| value.trim().to_ascii_lowercase())
        .unwrap_or_default()
        .as_str()
    {
        "1" | "split" | "splittime" | "split time" => 1,
        "2" | "segment" | "segmenttime" | "segment time" => 2,
        _ => 0,
    }
}

fn comparison_kind_xml_name(kind: i32) -> &'static str {
    match kind {
        1 => "SplitTime",
        2 => "SegmentTime",
        _ => "FinalTime",
    }
}

fn precision_from_xml(value: Option<&str>) -> i32 {
    match value
        .map(|value| value.trim().to_ascii_lowercase())
        .unwrap_or_default()
        .as_str()
    {
        "seconds" | "1" => 1,
        "milliseconds" | "2" => 2,
        _ => 0,
    }
}

fn precision_xml_name(precision: i32) -> &'static str {
    match precision {
        1 => "Seconds",
        2 => "Milliseconds",
        _ => "FromLeaderboard",
    }
}

/// LiveSplit serializes the platform/region filters as booleans.  The shared
/// Rust settings use strings, so we keep a normalized flag / optional filter
/// value in them.
fn filter_from_xml(value: Option<String>) -> String {
    let Some(value) = value else {
        return String::new();
    };
    let value = value.trim();
    if value.is_empty() || value.eq_ignore_ascii_case("false") || value == "0" {
        String::new()
    } else if value.eq_ignore_ascii_case("true") || value == "1" {
        "true".to_string()
    } else {
        value.to_string()
    }
}

fn filter_enabled(value: &str) -> bool {
    let value = value.trim();
    !value.is_empty() && !value.eq_ignore_ascii_case("false") && value != "0"
}

// ---------------------------------------------------------------------------
// Title
// ---------------------------------------------------------------------------

pub fn title_from_xml(settings: &XmlElement) -> TitleSettings {
    let mut result = TitleSettings::default();
    result.show_game_name = bool_value(settings, "ShowGameName", result.show_game_name);
    result.show_category_name =
        bool_value(settings, "ShowCategoryName", result.show_category_name);
    result.show_attempt_count =
        bool_value(settings, "ShowAttemptCount", result.show_attempt_count);
    result.show_finished_runs_count = bool_value(
        settings,
        "ShowFinishedRunsCount",
        result.show_finished_runs_count,
    );
    result.single_line = bool_value(settings, "SingleLine", result.single_line);
    result.display_game_icon = bool_value(settings, "DisplayGameIcon", result.display_game_icon);
    result.show_region = bool_value(settings, "ShowRegion", result.show_region);
    result.show_platform = bool_value(settings, "ShowPlatform", result.show_platform);
    result.show_variables = bool_value(settings, "ShowVariables", result.show_variables);
    result.title_color = parse_color(
        child_text(settings, "TitleColor").as_deref(),
        result.title_color,
    );
    result.override_title_color = bool_value(
        settings,
        "OverrideTitleColor",
        result.override_title_color,
    );
    result.background = background_from_xml(settings, &result.background);

    if let Some(font) = child(settings, "TitleFont") {
        result.title_font = parse_font(font, &result.title_font);
    }
    result.override_title_font = bool_value(
        settings,
        "OverrideTitleFont",
        result.override_title_font,
    );
    // Very old layouts used the inverse flag instead.
    if child_text(settings, "OverrideTitleFont").is_none() {
        if let Some(use_layout_settings_font) = child_text(settings, "UseLayoutSettingsFont") {
            result.override_title_font =
                !parse_bool(Some(&use_layout_settings_font), true);
        }
    }

    if let Some(alignment) = child_text(settings, "TextAlignment") {
        result.text_alignment = parse_i32(Some(&alignment), result.text_alignment as i32)
            .clamp(0, 2) as u8;
    } else if result.display_game_icon && bool_value(settings, "CenterTitle", false) {
        result.text_alignment = 2;
    }

    result
}

pub fn title_to_xml(settings: &TitleSettings) -> XmlElement {
    let mut element = XmlElement::new("Settings");
    push_text(&mut element, "Version", "1.7.3");
    push_bool(&mut element, "ShowGameName", settings.show_game_name);
    push_bool(
        &mut element,
        "ShowCategoryName",
        settings.show_category_name,
    );
    push_bool(&mut element, "ShowAttemptCount", settings.show_attempt_count);
    push_bool(
        &mut element,
        "ShowFinishedRunsCount",
        settings.show_finished_runs_count,
    );
    push_bool(
        &mut element,
        "OverrideTitleColor",
        settings.override_title_color,
    );
    push_color(&mut element, "TitleColor", settings.title_color);
    push_bool(&mut element, "SingleLine", settings.single_line);
    push_background(&mut element, &settings.background);
    push_bool(&mut element, "DisplayGameIcon", settings.display_game_icon);
    push_bool(&mut element, "ShowRegion", settings.show_region);
    push_bool(&mut element, "ShowPlatform", settings.show_platform);
    push_bool(&mut element, "ShowVariables", settings.show_variables);
    push_text(
        &mut element,
        "TextAlignment",
        settings.text_alignment.clamp(0, 2).to_string(),
    );
    push_bool(
        &mut element,
        "OverrideTitleFont",
        settings.override_title_font,
    );
    push_font(&mut element, "TitleFont", &settings.title_font);
    element
}

pub fn draw_title(ui: &mut Ui, rect: Rect, ctx: &mut ComponentContext, settings: &TitleSettings) {
    paint_background(ui.painter(), rect, &settings.background);
    if rect.width() <= 1.0 || rect.height() <= 1.0 {
        return;
    }

    let scale = ctx.scale;
    let pad = 7.0 * scale;
    let run = &ctx.timer.run;
    let base_font = if settings.override_title_font {
        ctx.font(&settings.title_font)
    } else {
        ctx.text_font()
    };
    let color = if settings.override_title_color {
        settings.title_color
    } else {
        ctx.theme.text_color
    };
    let shadow = if ctx.theme.drop_shadows {
        Some(ctx.theme.shadows_color)
    } else {
        None
    };
    let outline = Some(ctx.theme.text_outline_color);

    let has_icon = settings.display_game_icon
        && run
            .game_icon
            .as_deref()
            .is_some_and(|bytes| !bytes.is_empty());
    let icon_size = (rect.height() - 4.0 * scale).max(1.0);
    let icon_texture = if has_icon {
        run.game_icon
            .as_deref()
            .and_then(|bytes| ctx.icons.texture(ui.ctx(), bytes))
    } else {
        None
    };

    let painter = ui.painter();
    if let Some(texture) = icon_texture {
        let aspect_ratio = ui
            .ctx()
            .tex_manager()
            .read()
            .meta(texture)
            .and_then(|meta| {
                let [width, height] = meta.size;
                (height > 0).then(|| width as f32 / height as f32)
            })
            .unwrap_or(1.0)
            .max(0.01);
        let (draw_width, draw_height) = if aspect_ratio > 1.0 {
            (icon_size, icon_size / aspect_ratio)
        } else {
            (icon_size * aspect_ratio, icon_size)
        };
        let icon_rect = Rect::from_center_size(
            egui::pos2(
                rect.left() + pad + icon_size / 2.0,
                rect.center().y,
            ),
            egui::vec2(draw_width, draw_height),
        );
        painter.image(
            texture,
            icon_rect,
            Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            Color32::WHITE,
        );
    }

    let content_left = rect.left() + pad + if has_icon { icon_size + 3.0 * scale } else { 0.0 };
    let content_right = rect.right() - pad;
    let game_name = if settings.show_game_name {
        run.game_name.trim()
    } else {
        ""
    };
    let category = if settings.show_category_name {
        extended_category_name(
            run,
            settings.show_region,
            settings.show_platform,
            settings.show_variables,
        )
    } else {
        String::new()
    };

    let two_line = !settings.single_line && !game_name.is_empty() && !category.is_empty();
    let unscaled_height = rect.height() / scale;
    let max_font_size = if two_line {
        unscaled_height * 0.45
    } else {
        unscaled_height * 0.72
    };
    let font = sized_font(
        &base_font,
        (base_font.size / scale).min(max_font_size),
        scale,
    );

    let count = title_count_text(ctx.timer, settings);
    let count_width = if count.is_empty() {
        0.0
    } else {
        text_width(painter, &count, &font) + 4.0 * scale
    };
    let text_right = (content_right - count_width).max(content_left);
    let available_width = (text_right - content_left).max(0.0);
    let centered = match settings.text_alignment {
        1 => false,
        2 => true,
        _ => !has_icon,
    };

    if settings.single_line && !game_name.is_empty() && !category.is_empty() {
        let text = format!("{game_name} - {category}");
        let fitted = fit_text(painter, &text, &font, available_width);
        let x = if centered {
            (content_left + text_right) / 2.0
        } else {
            content_left
        };
        paint_text(
            painter,
            egui::pos2(x, rect.center().y),
            if centered {
                Align2::CENTER_CENTER
            } else {
                Align2::LEFT_CENTER
            },
            &fitted,
            font.clone(),
            color,
            shadow,
            outline,
        );
    } else if !game_name.is_empty() && !category.is_empty() {
        let fitted_game = fit_text(painter, game_name, &font, available_width);
        let x = if centered {
            (content_left + text_right) / 2.0
        } else {
            content_left
        };
        paint_text(
            painter,
            egui::pos2(x, rect.top() + 1.0 * scale),
            if centered {
                Align2::CENTER_TOP
            } else {
                Align2::LEFT_TOP
            },
            &fitted_game,
            font.clone(),
            color,
            shadow,
            outline,
        );
        let fitted_category = fit_text(painter, &category, &font, available_width);
        paint_text(
            painter,
            egui::pos2(x, rect.bottom() - 1.0 * scale),
            if centered {
                Align2::CENTER_BOTTOM
            } else {
                Align2::LEFT_BOTTOM
            },
            &fitted_category,
            font.clone(),
            color,
            shadow,
            outline,
        );
    } else {
        let text = if !game_name.is_empty() {
            game_name
        } else {
            category.as_str()
        };
        if !text.is_empty() {
            let fitted = fit_text(painter, text, &font, available_width);
            let x = if centered {
                (content_left + text_right) / 2.0
            } else {
                content_left
            };
            paint_text(
                painter,
                egui::pos2(x, rect.center().y),
                if centered {
                    Align2::CENTER_CENTER
                } else {
                    Align2::LEFT_CENTER
                },
                &fitted,
                font.clone(),
                color,
                shadow,
                outline,
            );
        }
    }

    if !count.is_empty() {
        let fitted_count = fit_text(painter, &count, &font, (content_right - content_left).max(0.0));
        paint_text(
            painter,
            egui::pos2(content_right, rect.bottom() - 1.0 * scale),
            Align2::RIGHT_BOTTOM,
            &fitted_count,
            font,
            color,
            shadow,
            outline,
        );
    }
}

fn title_count_text(timer: &Timer, settings: &TitleSettings) -> String {
    if !settings.show_attempt_count && !settings.show_finished_runs_count {
        return String::new();
    }
    let mut finished = timer
        .run
        .attempts
        .iter()
        .filter(|attempt| attempt.time.real_time.is_some())
        .count();
    if timer.phase == TimerPhase::Ended {
        finished += 1;
    }
    match (
        settings.show_attempt_count,
        settings.show_finished_runs_count,
    ) {
        (true, true) => format!("{finished}/{}", timer.run.attempt_count),
        (true, false) => timer.run.attempt_count.to_string(),
        (false, true) => finished.to_string(),
        (false, false) => String::new(),
    }
}

fn extended_category_name(
    run: &Run,
    show_region: bool,
    show_platform: bool,
    show_variables: bool,
) -> String {
    let mut category = run.category_name.trim().to_string();
    if category.is_empty() {
        return String::new();
    }

    let mut parts = Vec::new();
    let mut suffix = String::new();
    if let Some(start) = category.find('(') {
        if let Some(relative_end) = category[start + 1..].find(')') {
            let end = start + 1 + relative_end;
            let inside = category[start + 1..end].trim();
            if !inside.is_empty() {
                parts.push(inside.to_string());
            }
            suffix = category[end + 1..].trim().to_string();
            category = category[..start].trim().to_string();
        }
    }

    if show_variables {
        for (name, value) in &run.metadata.variables {
            let name = name.trim_end_matches('?').trim();
            if value.eq_ignore_ascii_case("yes") {
                parts.push(name.to_string());
            } else if value.eq_ignore_ascii_case("no") {
                parts.push(format!("No {name}"));
            } else if !value.trim().is_empty() {
                parts.push(value.clone());
            }
        }
    }

    if show_region {
        if let Some(region) = run
            .metadata
            .region_name
            .as_deref()
            .map(str::trim)
            .filter(|region| !region.is_empty())
        {
            parts.push(region.to_string());
        }
    }

    if show_platform {
        if let Some(platform) = run
            .metadata
            .platform_name
            .as_deref()
            .map(str::trim)
            .filter(|platform| !platform.is_empty())
        {
            if run.metadata.uses_emulator {
                parts.push(format!("{platform} Emulator"));
            } else {
                parts.push(platform.to_string());
            }
        } else if run.metadata.uses_emulator {
            parts.push("Emulator".to_string());
        }
    }

    if parts.is_empty() {
        category
    } else {
        let suffix = if suffix.is_empty() {
            String::new()
        } else {
            format!(" {suffix}")
        };
        format!("{category} ({}){suffix}", parts.join(", "))
            .trim()
            .to_string()
    }
}

pub fn title_settings_ui(ui: &mut Ui, settings: &mut TitleSettings) {
    ui.label(egui::RichText::new("Title").strong());
    ui.checkbox(&mut settings.show_game_name, "Show game name");
    ui.checkbox(&mut settings.show_category_name, "Show category name");
    ui.checkbox(&mut settings.show_attempt_count, "Show attempt count");
    ui.checkbox(
        &mut settings.show_finished_runs_count,
        "Show finished runs count",
    );
    ui.checkbox(&mut settings.show_region, "Show region");
    ui.checkbox(&mut settings.show_platform, "Show platform");
    ui.checkbox(&mut settings.show_variables, "Show variables");
    ui.horizontal(|ui| {
        ui.label("Alignment:");
        egui::ComboBox::from_id_salt(ui.scope_id().with("title_alignment"))
            .selected_text(alignment_label(settings.text_alignment))
            .show_ui(ui, |ui| {
                for (value, label) in [(0u8, "Auto"), (1, "Left"), (2, "Center")] {
                    ui.selectable_value(&mut settings.text_alignment, value, label);
                }
            });
    });
    ui.checkbox(&mut settings.single_line, "Single line");
    ui.checkbox(&mut settings.display_game_icon, "Display game icon");

    ui.separator();
    color_setting(
        ui,
        "Override title color",
        &mut settings.override_title_color,
        &mut settings.title_color,
    );
    ui.separator();
    background_settings_ui(ui, &mut settings.background);
    ui.separator();
    ui.checkbox(&mut settings.override_title_font, "Override title font");
    let override_font = settings.override_title_font;
    let title_font = &mut settings.title_font;
    ui.add_enabled_ui(override_font, |ui| font_editor(ui, title_font));
}

fn alignment_label(alignment: u8) -> &'static str {
    match alignment {
        1 => "Left",
        2 => "Center",
        _ => "Auto",
    }
}

// ---------------------------------------------------------------------------
// Text
// ---------------------------------------------------------------------------

pub fn text_from_xml(settings: &XmlElement) -> TextSettings {
    let mut result = TextSettings::default();
    result.text1 = child_text(settings, "Text1").unwrap_or(result.text1);
    result.text2 = child_text(settings, "Text2").unwrap_or_default();
    result.text_color = parse_color(
        child_text(settings, "TextColor").as_deref(),
        result.text_color,
    );
    result.override_text_color = bool_value(
        settings,
        "OverrideTextColor",
        result.override_text_color,
    );
    result.time_color = parse_color(
        child_text(settings, "TimeColor").as_deref(),
        result.time_color,
    );
    result.override_time_color = bool_value(
        settings,
        "OverrideTimeColor",
        result.override_time_color,
    );
    result.background = background_from_xml(settings, &result.background);
    result.display_2_rows = bool_value(settings, "Display2Rows", result.display_2_rows);
    if bool_value(settings, "CustomVariable", false) {
        result.custom_variable = result.text2.clone();
    }
    result
}

pub fn text_to_xml(settings: &TextSettings) -> XmlElement {
    let mut element = XmlElement::new("Settings");
    push_text(&mut element, "Version", "1.5");
    push_color(&mut element, "TextColor", settings.text_color);
    push_bool(
        &mut element,
        "OverrideTextColor",
        settings.override_text_color,
    );
    push_color(&mut element, "TimeColor", settings.time_color);
    push_bool(
        &mut element,
        "OverrideTimeColor",
        settings.override_time_color,
    );
    push_background(&mut element, &settings.background);
    push_text(&mut element, "Text1", settings.text1.clone());
    let custom_variable = !settings.custom_variable.trim().is_empty();
    if custom_variable {
        // LiveSplit stores the custom variable's name in Text2.
        push_text(&mut element, "Text2", settings.custom_variable.clone());
    } else {
        push_text(&mut element, "Text2", settings.text2.clone());
    }
    push_bool(&mut element, "Display2Rows", settings.display_2_rows);
    push_bool(&mut element, "CustomVariable", custom_variable);
    element
}

pub fn draw_text(ui: &mut Ui, rect: Rect, ctx: &mut ComponentContext, settings: &TextSettings) {
    let text1 = substitute_variables(&settings.text1, ctx);
    let text2 = if settings.custom_variable.trim().is_empty() {
        substitute_variables(&settings.text2, ctx)
    } else {
        custom_variable_value(ctx, settings.custom_variable.trim())
            .unwrap_or_else(|| livesplit_core::format::DASH.to_string())
    };
    let name_color = if settings.override_text_color {
        settings.text_color
    } else {
        ctx.theme.text_color
    };
    let value_color = if settings.override_time_color {
        settings.time_color
    } else {
        ctx.theme.text_color
    };

    paint_background(ui.painter(), rect, &settings.background);
    draw_info(
        ui,
        rect,
        ctx,
        &text1,
        &text2,
        name_color,
        value_color,
        ctx.text_font(),
        settings.display_2_rows,
    );
}

fn custom_variable_value(ctx: &ComponentContext, name: &str) -> Option<String> {
    let name = name.trim().trim_start_matches('{').trim_end_matches('}');
    ctx.timer
        .run
        .metadata
        .custom_variables
        .get(name)
        .or_else(|| ctx.timer.run.metadata.variables.get(name))
        .cloned()
}

fn substitute_variables(text: &str, ctx: &ComponentContext) -> String {
    if !text.contains('{') {
        return text.to_string();
    }
    let mut result = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('{') {
        result.push_str(&rest[..start]);
        let Some(relative_end) = rest[start + 1..].find('}') else {
            result.push_str(&rest[start..]);
            return result;
        };
        let end = start + 1 + relative_end;
        let name = &rest[start + 1..end];
        if let Some(value) = custom_variable_value(ctx, name) {
            result.push_str(&value);
        } else {
            result.push_str(&rest[start..=end]);
        }
        rest = &rest[end + 1..];
    }
    result.push_str(rest);
    result
}

pub fn text_settings_ui(ui: &mut Ui, settings: &mut TextSettings) {
    ui.label(egui::RichText::new("Text").strong());
    ui.horizontal(|ui| {
        ui.label("Text 1:");
        ui.text_edit_singleline(&mut settings.text1);
    });
    ui.horizontal(|ui| {
        ui.label("Text 2:");
        ui.text_edit_singleline(&mut settings.text2);
    });
    let mut custom_variable = !settings.custom_variable.trim().is_empty();
    if ui
        .checkbox(
            &mut custom_variable,
            "Use custom variable for the value",
        )
        .changed()
        && !custom_variable
    {
        settings.custom_variable.clear();
    }
    if custom_variable {
        ui.horizontal(|ui| {
            ui.label("Variable name:");
            ui.text_edit_singleline(&mut settings.custom_variable);
        });
    }
    ui.label("Tip: {Variable} placeholders are replaced from the run's custom variables.");

    ui.separator();
    color_setting(
        ui,
        "Override text color",
        &mut settings.override_text_color,
        &mut settings.text_color,
    );
    color_setting(
        ui,
        "Override time color",
        &mut settings.override_time_color,
        &mut settings.time_color,
    );
    ui.checkbox(&mut settings.display_2_rows, "Display 2 rows");
    ui.separator();
    background_settings_ui(ui, &mut settings.background);
}

// ---------------------------------------------------------------------------
// Blank Space
// ---------------------------------------------------------------------------

pub fn blank_space_from_xml(settings: &XmlElement) -> BlankSpaceSettings {
    let mut result = BlankSpaceSettings::default();
    result.space_height = f32_value(settings, "SpaceHeight", result.space_height).max(0.0);
    result.space_width = f32_value(settings, "SpaceWidth", result.space_width).max(0.0);
    result.background = background_from_xml(settings, &result.background);
    result
}

pub fn blank_space_to_xml(settings: &BlankSpaceSettings) -> XmlElement {
    let mut element = XmlElement::new("Settings");
    push_text(&mut element, "Version", "1.7");
    push_f32(&mut element, "SpaceHeight", settings.space_height);
    push_f32(&mut element, "SpaceWidth", settings.space_width);
    push_background(&mut element, &settings.background);
    element
}

pub fn draw_blank_space(
    ui: &mut Ui,
    rect: Rect,
    _ctx: &mut ComponentContext,
    settings: &BlankSpaceSettings,
) {
    paint_background(ui.painter(), rect, &settings.background);
}

pub fn blank_space_settings_ui(ui: &mut Ui, settings: &mut BlankSpaceSettings) {
    ui.label(egui::RichText::new("Blank Space").strong());
    ui.horizontal(|ui| {
        ui.label("Height:");
        ui.add(
            egui::DragValue::new(&mut settings.space_height)
                .speed(1.0)
                .range(0.0..=1000.0),
        );
    });
    ui.horizontal(|ui| {
        ui.label("Width:");
        ui.add(
            egui::DragValue::new(&mut settings.space_width)
                .speed(1.0)
                .range(0.0..=1000.0),
        );
    });
    ui.separator();
    background_settings_ui(ui, &mut settings.background);
}

// ---------------------------------------------------------------------------
// Total Playtime
// ---------------------------------------------------------------------------

pub fn total_playtime_from_xml(settings: &XmlElement) -> TotalPlaytimeSettings {
    let mut result = TotalPlaytimeSettings::default();
    result.text_color = parse_color(
        child_text(settings, "TextColor").as_deref(),
        result.text_color,
    );
    result.override_text_color = bool_value(
        settings,
        "OverrideTextColor",
        result.override_text_color,
    );
    result.time_color = parse_color(
        child_text(settings, "TimeColor").as_deref(),
        result.time_color,
    );
    result.override_time_color = bool_value(
        settings,
        "OverrideTimeColor",
        result.override_time_color,
    );
    result.background = background_from_xml(settings, &result.background);
    result.display_2_rows = bool_value(settings, "Display2Rows", result.display_2_rows);
    result.show_total_hours = bool_value(settings, "ShowTotalHours", result.show_total_hours);
    result
}

pub fn total_playtime_to_xml(settings: &TotalPlaytimeSettings) -> XmlElement {
    let mut element = XmlElement::new("Settings");
    push_text(&mut element, "Version", "1.6");
    push_color(&mut element, "TextColor", settings.text_color);
    push_bool(
        &mut element,
        "OverrideTextColor",
        settings.override_text_color,
    );
    push_color(&mut element, "TimeColor", settings.time_color);
    push_bool(
        &mut element,
        "OverrideTimeColor",
        settings.override_time_color,
    );
    push_background(&mut element, &settings.background);
    push_bool(&mut element, "Display2Rows", settings.display_2_rows);
    push_bool(&mut element, "ShowTotalHours", settings.show_total_hours);
    element
}

pub fn draw_total_playtime(
    ui: &mut Ui,
    rect: Rect,
    ctx: &mut ComponentContext,
    settings: &TotalPlaytimeSettings,
) {
    let total = total_playtime_nanos(ctx.timer);
    let text = format_playtime(total, settings.show_total_hours);
    let name_color = if settings.override_text_color {
        settings.text_color
    } else {
        ctx.theme.text_color
    };
    let value_color = if settings.override_time_color {
        settings.time_color
    } else {
        ctx.theme.text_color
    };

    paint_background(ui.painter(), rect, &settings.background);
    draw_info(
        ui,
        rect,
        ctx,
        "Total Playtime",
        &text,
        name_color,
        value_color,
        ctx.times_font(),
        settings.display_2_rows,
    );
}

fn total_playtime_nanos(timer: &Timer) -> i64 {
    let mut total = 0i64;
    for attempt in &timer.run.attempts {
        if let Some(duration) = attempt.duration() {
            let pause = attempt.pause_time.map(TimeSpan::as_nanos).unwrap_or(0);
            total = total.saturating_add(duration.as_nanos().saturating_sub(pause));
        } else {
            // Older split files don't have attempt timestamps.  Their total is
            // the sum of the segment history for that attempt.
            for segment in &timer.run.segments {
                if let Some(history) = segment.segment_history.get(&attempt.index) {
                    if let Some(time) = history.real_time {
                        total = total.saturating_add(time.as_nanos());
                    }
                }
            }
        }
    }

    if let Some(duration) = timer.current_attempt_duration() {
        let pause = timer
            .pause_time()
            .map(TimeSpan::as_nanos)
            .unwrap_or(0);
        total = total.saturating_add(duration.as_nanos().saturating_sub(pause));
    }
    total
}

fn format_playtime(total_nanos: i64, show_total_hours: bool) -> String {
    let total_nanos = total_nanos.max(0);
    if show_total_hours {
        return GeneralTimeFormatter {
            accuracy: TimeAccuracy::Seconds,
            digits_format: livesplit_core::DigitsFormat::SingleDigitMinutes,
            null_format: livesplit_core::NullFormat::ZeroWithAccuracy,
            ..GeneralTimeFormatter::default()
        }
        .format(Some(TimeSpan::from_nanos(total_nanos)));
    }

    let total_seconds = total_nanos / 1_000_000_000;
    let days = total_seconds / 86_400;
    let hours = (total_seconds / 3_600) % 24;
    let minutes = (total_seconds / 60) % 60;
    let seconds = total_seconds % 60;
    let mut result = String::new();
    if days >= 1 {
        result.push_str(&format!("{days}d "));
    }
    if total_seconds / 3_600 >= 1 {
        result.push_str(&format!("{hours}:{minutes:02}:{seconds:02}"));
    } else {
        result.push_str(&format!("{minutes}:{seconds:02}"));
    }
    result
}

pub fn total_playtime_settings_ui(ui: &mut Ui, settings: &mut TotalPlaytimeSettings) {
    ui.label(egui::RichText::new("Total Playtime").strong());
    ui.checkbox(&mut settings.show_total_hours, "Show total hours");
    ui.checkbox(&mut settings.display_2_rows, "Display 2 rows");
    ui.separator();
    color_setting(
        ui,
        "Override text color",
        &mut settings.override_text_color,
        &mut settings.text_color,
    );
    color_setting(
        ui,
        "Override time color",
        &mut settings.override_time_color,
        &mut settings.time_color,
    );
    ui.separator();
    background_settings_ui(ui, &mut settings.background);
}

// ---------------------------------------------------------------------------
// Counter
// ---------------------------------------------------------------------------

pub fn counter_from_xml(settings: &XmlElement) -> CounterSettings {
    let mut result = CounterSettings::default();
    result.global_hotkeys_enabled = bool_value(
        settings,
        "GlobalHotkeysEnabled",
        result.global_hotkeys_enabled,
    );
    result.counter_text_color = parse_color(
        child_text(settings, "CounterTextColor").as_deref(),
        result.counter_text_color,
    );
    result.counter_value_color = parse_color(
        child_text(settings, "CounterValueColor").as_deref(),
        result.counter_value_color,
    );
    result.override_text_color = bool_value(
        settings,
        "OverrideTextColor",
        result.override_text_color,
    );
    result.background = background_from_xml(settings, &result.background);
    result.counter_text = child_text(settings, "CounterText").unwrap_or(result.counter_text);
    result.initial_value = i32_value(settings, "InitialValue", result.initial_value);
    result.increment = i32_value(settings, "Increment", result.increment);
    result.increment_key = child_text(settings, "IncrementKey").unwrap_or(result.increment_key);
    result.decrement_key = child_text(settings, "DecrementKey").unwrap_or(result.decrement_key);
    result.reset_key = child_text(settings, "ResetKey").unwrap_or(result.reset_key);
    result
}

pub fn counter_to_xml(settings: &CounterSettings) -> XmlElement {
    let mut element = XmlElement::new("Settings");
    push_text(&mut element, "Version", "1.0");
    push_bool(
        &mut element,
        "GlobalHotkeysEnabled",
        settings.global_hotkeys_enabled,
    );
    push_bool(
        &mut element,
        "OverrideTextColor",
        settings.override_text_color,
    );
    push_color(&mut element, "CounterTextColor", settings.counter_text_color);
    push_color(
        &mut element,
        "CounterValueColor",
        settings.counter_value_color,
    );
    push_background(&mut element, &settings.background);
    push_text(&mut element, "CounterText", settings.counter_text.clone());
    push_i32(&mut element, "InitialValue", settings.initial_value);
    push_i32(&mut element, "Increment", settings.increment);
    push_text(
        &mut element,
        "IncrementKey",
        settings.increment_key.clone(),
    );
    push_text(
        &mut element,
        "DecrementKey",
        settings.decrement_key.clone(),
    );
    push_text(&mut element, "ResetKey", settings.reset_key.clone());
    element
}

#[derive(Clone)]
struct CounterRuntime {
    count: i32,
    initial_value: i32,
}

fn counter_memory_id(settings: &CounterSettings) -> egui::Id {
    egui::Id::unique(("rustsplit.counter", settings.counter_text.as_str()))
}

fn counter_runtime(ui: &Ui, settings: &CounterSettings) -> CounterRuntime {
    let id = counter_memory_id(settings);
    let mut runtime = ui
        .ctx()
        .data_mut(|data| data.get_temp::<CounterRuntime>(id))
        .unwrap_or(CounterRuntime {
            count: settings.initial_value,
            initial_value: settings.initial_value,
        });
    if runtime.initial_value != settings.initial_value {
        runtime.count = settings.initial_value;
        runtime.initial_value = settings.initial_value;
    }
    ui.ctx()
        .data_mut(|data| data.insert_temp(id, runtime.clone()));
    runtime
}

pub fn draw_counter(
    ui: &mut Ui,
    rect: Rect,
    ctx: &mut ComponentContext,
    settings: &CounterSettings,
) {
    let runtime = counter_runtime(ui, settings);
    let name_color = if settings.override_text_color {
        settings.counter_text_color
    } else {
        ctx.theme.text_color
    };
    let value_color = if settings.override_text_color {
        settings.counter_value_color
    } else {
        ctx.theme.text_color
    };

    paint_background(ui.painter(), rect, &settings.background);
    draw_info(
        ui,
        rect,
        ctx,
        &settings.counter_text,
        &runtime.count.to_string(),
        name_color,
        value_color,
        ctx.text_font(),
        settings.display_2_rows,
    );
}

pub fn counter_settings_ui(ui: &mut Ui, settings: &mut CounterSettings) {
    ui.label(egui::RichText::new("Counter").strong());
    ui.horizontal(|ui| {
        ui.label("Text:");
        ui.text_edit_singleline(&mut settings.counter_text);
    });
    ui.horizontal(|ui| {
        ui.label("Initial value:");
        ui.add(egui::DragValue::new(&mut settings.initial_value).speed(1.0));
    });
    ui.horizontal(|ui| {
        ui.label("Increment:");
        ui.add(egui::DragValue::new(&mut settings.increment).speed(1.0));
    });
    ui.checkbox(
        &mut settings.global_hotkeys_enabled,
        "Global hotkeys enabled",
    );

    let id = counter_memory_id(settings);
    let current = ui
        .ctx()
        .data_mut(|data| data.get_temp::<CounterRuntime>(id))
        .map(|runtime| runtime.count);
    ui.horizontal(|ui| {
        ui.label(match current {
            Some(value) => format!("Current value: {value}"),
            None => "Current value: not initialized".to_string(),
        });
        if ui.button("Reset count").clicked() {
            ui.ctx().data_mut(|data| data.remove::<CounterRuntime>(id));
        }
    });

    ui.separator();
    color_setting(
        ui,
        "Override text colors",
        &mut settings.override_text_color,
        &mut settings.counter_text_color,
    );
    let override_colors = settings.override_text_color;
    let counter_value_color = &mut settings.counter_value_color;
    ui.add_enabled_ui(override_colors, |ui| {
        ui.horizontal(|ui| {
            ui.label("Value color:");
            ui.color_edit_button_srgba(counter_value_color);
        });
    });
    ui.checkbox(&mut settings.display_2_rows, "Display 2 rows");
    ui.separator();
    ui.label("Hotkeys (stored for compatibility):");
    ui.horizontal(|ui| {
        ui.label("Increment:");
        ui.text_edit_singleline(&mut settings.increment_key);
    });
    ui.horizontal(|ui| {
        ui.label("Decrement:");
        ui.text_edit_singleline(&mut settings.decrement_key);
    });
    ui.horizontal(|ui| {
        ui.label("Reset:");
        ui.text_edit_singleline(&mut settings.reset_key);
    });
    ui.label("Example keys: NumPad+, NumPad-, NumPad*.");
    ui.separator();
    background_settings_ui(ui, &mut settings.background);
}

// ---------------------------------------------------------------------------
// Hotkey Indicator
// ---------------------------------------------------------------------------

pub fn hotkey_indicator_from_xml(settings: &XmlElement) -> HotkeyIndicatorSettings {
    let mut result = HotkeyIndicatorSettings::default();
    result.indicator_height = f32_value(settings, "IndicatorHeight", result.indicator_height);
    result.indicator_width = f32_value(settings, "IndicatorWidth", result.indicator_width);
    result.on_color = parse_color(
        child_text(settings, "HotkeysOnColor").as_deref(),
        result.on_color,
    );
    result.off_color = parse_color(
        child_text(settings, "HotkeysOffColor").as_deref(),
        result.off_color,
    );
    result
}

pub fn hotkey_indicator_to_xml(settings: &HotkeyIndicatorSettings) -> XmlElement {
    let mut element = XmlElement::new("Settings");
    // LiveSplit's HotkeyIndicator settings do not contain a version element.
    push_color(&mut element, "HotkeysOnColor", settings.on_color);
    push_color(&mut element, "HotkeysOffColor", settings.off_color);
    push_f32(&mut element, "IndicatorHeight", settings.indicator_height);
    push_f32(&mut element, "IndicatorWidth", settings.indicator_width);
    element
}

pub fn draw_hotkey_indicator(
    ui: &mut Ui,
    rect: Rect,
    ctx: &mut ComponentContext,
    settings: &HotkeyIndicatorSettings,
) {
    if rect.width() <= 0.0 || rect.height() <= 0.0 {
        return;
    }
    // The overlay does not track LiveSplit's global-hotkey state yet, so the
    // indicator is always drawn in its "off" state.
    let color = if settings.off_color.a() > 0 {
        settings.off_color
    } else {
        ctx.theme.separators_color
    };
    ui.painter().rect_filled(rect, 0.0, color);
}

pub fn hotkey_indicator_settings_ui(ui: &mut Ui, settings: &mut HotkeyIndicatorSettings) {
    ui.label(egui::RichText::new("Hotkey Indicator").strong());
    ui.horizontal(|ui| {
        ui.label("Width:");
        ui.add(
            egui::DragValue::new(&mut settings.indicator_width)
                .speed(1.0)
                .range(0.0..=1000.0),
        );
    });
    ui.horizontal(|ui| {
        ui.label("Height:");
        ui.add(
            egui::DragValue::new(&mut settings.indicator_height)
                .speed(1.0)
                .range(0.0..=1000.0),
        );
    });
    ui.horizontal(|ui| {
        ui.label("Off color:");
        ui.color_edit_button_srgba(&mut settings.off_color);
    });
    ui.horizontal(|ui| {
        ui.label("On color:");
        ui.color_edit_button_srgba(&mut settings.on_color);
    });
    ui.label("The indicator is static in the overlay; global hotkey state is not tracked yet.");
}

// ---------------------------------------------------------------------------
// Comparison Time
// ---------------------------------------------------------------------------

pub fn comparison_time_from_xml(settings: &XmlElement) -> ComparisonTimeSettings {
    let mut result = ComparisonTimeSettings::default();
    result.text_color = parse_color(
        child_text(settings, "TextColor").as_deref(),
        result.text_color,
    );
    result.override_text_color = bool_value(
        settings,
        "OverrideTextColor",
        result.override_text_color,
    );
    result.time_color = parse_color(
        child_text(settings, "TimeColor").as_deref(),
        result.time_color,
    );
    result.override_time_color = bool_value(
        settings,
        "OverrideTimeColor",
        result.override_time_color,
    );
    result.accuracy = accuracy_from_xml(
        child_text(settings, "Accuracy").as_deref(),
        result.accuracy,
    );
    result.background = background_from_xml(settings, &result.background);
    result.comparison = child_text(settings, "Comparison").unwrap_or(result.comparison);
    result.timing_method = timing_method_from_xml(
        child_text(settings, "TimingMethod").as_deref(),
        result.timing_method,
    );
    result.display_2_rows = bool_value(settings, "Display2Rows", result.display_2_rows);
    result.kind = comparison_kind_from_xml(child_text(settings, "Type").as_deref());
    if let Some(index) = child_text(settings, "SplitIndex")
        .and_then(|index| index.trim().parse::<usize>().ok())
    {
        result.split_index = index;
    }
    result
}

pub fn comparison_time_to_xml(settings: &ComparisonTimeSettings) -> XmlElement {
    let mut element = XmlElement::new("Settings");
    push_text(&mut element, "Version", "1.4");
    push_color(&mut element, "TextColor", settings.text_color);
    push_bool(
        &mut element,
        "OverrideTextColor",
        settings.override_text_color,
    );
    push_color(&mut element, "TimeColor", settings.time_color);
    push_bool(
        &mut element,
        "OverrideTimeColor",
        settings.override_time_color,
    );
    push_text(
        &mut element,
        "Accuracy",
        accuracy_xml_name(settings.accuracy),
    );
    push_background(&mut element, &settings.background);
    push_text(&mut element, "Comparison", settings.comparison.clone());
    push_text(
        &mut element,
        "TimingMethod",
        timing_method_xml_name(settings.timing_method),
    );
    push_bool(&mut element, "Display2Rows", settings.display_2_rows);
    push_text(
        &mut element,
        "Type",
        comparison_kind_xml_name(settings.kind),
    );
    push_text(&mut element, "SplitIndex", settings.split_index.to_string());
    element
}

pub fn draw_comparison_time(
    ui: &mut Ui,
    rect: Rect,
    ctx: &mut ComponentContext,
    settings: &ComparisonTimeSettings,
) {
    let method = ctx.resolved_timing_method(settings.timing_method);
    let comparison = ctx.resolve_comparison(&settings.comparison).to_string();
    let value = comparison_time_value(
        ctx.timer,
        &comparison,
        method,
        settings.kind,
        settings.split_index,
    );
    let name = comparison_label(&comparison, settings.timing_method);
    let name_color = if settings.override_text_color {
        settings.text_color
    } else {
        ctx.theme.text_color
    };
    let value_color = if settings.override_time_color {
        settings.time_color
    } else {
        ctx.theme.text_color
    };

    paint_background(ui.painter(), rect, &settings.background);
    draw_info(
        ui,
        rect,
        ctx,
        &name,
        &format_time(value, settings.accuracy),
        name_color,
        value_color,
        ctx.times_font(),
        settings.display_2_rows,
    );
}

fn comparison_time_value(
    timer: &Timer,
    comparison: &str,
    method: TimingMethod,
    kind: i32,
    split_index: usize,
) -> Option<TimeSpan> {
    let segments = &timer.run.segments;
    if segments.is_empty() {
        return None;
    }
    let index = split_index.min(segments.len() - 1);
    match kind {
        1 => {
            if timer.phase == TimerPhase::NotRunning {
                return None;
            }
            segments
                .get(index)?
                .comparison(comparison)
                .get(method)
        }
        2 => {
            if timer.phase == TimerPhase::NotRunning {
                return None;
            }
            let current = segments
                .get(index)?
                .comparison(comparison)
                .get(method)?;
            let previous = if index == 0 {
                Some(TimeSpan::ZERO)
            } else {
                segments
                    .get(index - 1)?
                    .comparison(comparison)
                    .get(method)
            }?;
            Some(current - previous)
        }
        _ => segments
            .last()?
            .comparison(comparison)
            .get(method),
    }
}

fn comparison_label(name: &str, timing_method: TimingMethodSetting) -> String {
    let base = if name.trim().is_empty() {
        "Comparison"
    } else {
        name
    };
    match timing_method {
        TimingMethodSetting::Current => base.to_string(),
        other => format!("{base} ({})", other.label()),
    }
}

pub fn comparison_time_settings_ui(ui: &mut Ui, settings: &mut ComparisonTimeSettings) {
    ui.label(egui::RichText::new("Comparison Time").strong());
    comparison_combo(ui, "comparison_time_comparison", &mut settings.comparison);
    timing_method_combo(
        ui,
        "Timing method:",
        "comparison_time_timing",
        &mut settings.timing_method,
    );
    accuracy_combo(
        ui,
        "Accuracy:",
        "comparison_time_accuracy",
        &mut settings.accuracy,
    );
    ui.horizontal(|ui| {
        ui.label("Show:");
        egui::ComboBox::from_id_salt(ui.scope_id().with("comparison_time_kind"))
            .selected_text(comparison_kind_label(settings.kind))
            .show_ui(ui, |ui| {
                for (kind, label) in
                    [(0i32, "Total Time"), (1, "Split Time"), (2, "Segment Time")]
                {
                    ui.selectable_value(&mut settings.kind, kind, label);
                }
            });
    });
    let mut split_index = settings.split_index.min(i32::MAX as usize) as i32;
    ui.horizontal(|ui| {
        ui.label("Split index:");
        if ui
            .add(
                egui::DragValue::new(&mut split_index)
                    .speed(0.2)
                    .range(0..=1_000_000),
            )
            .changed()
        {
            settings.split_index = split_index.max(0) as usize;
        }
    });
    ui.checkbox(&mut settings.display_2_rows, "Display 2 rows");
    ui.separator();
    color_setting(
        ui,
        "Override text color",
        &mut settings.override_text_color,
        &mut settings.text_color,
    );
    color_setting(
        ui,
        "Override time color",
        &mut settings.override_time_color,
        &mut settings.time_color,
    );
    ui.separator();
    background_settings_ui(ui, &mut settings.background);
}

fn comparison_kind_label(kind: i32) -> &'static str {
    match kind {
        1 => "Split Time",
        2 => "Segment Time",
        _ => "Total Time",
    }
}

// ---------------------------------------------------------------------------
// World Record
// ---------------------------------------------------------------------------

pub fn world_record_from_xml(settings: &XmlElement) -> WorldRecordSettings {
    let mut result = WorldRecordSettings::default();
    result.text_color = parse_color(
        child_text(settings, "TextColor").as_deref(),
        result.text_color,
    );
    result.override_text_color = bool_value(
        settings,
        "OverrideTextColor",
        result.override_text_color,
    );
    result.time_color = parse_color(
        child_text(settings, "TimeColor").as_deref(),
        result.time_color,
    );
    result.override_time_color = bool_value(
        settings,
        "OverrideTimeColor",
        result.override_time_color,
    );
    result.background = background_from_xml(settings, &result.background);
    result.display_2_rows = bool_value(settings, "Display2Rows", result.display_2_rows);
    result.timing_method = timing_method_from_xml(
        child_text(settings, "TimingMethod").as_deref(),
        result.timing_method,
    );
    result.centered_text = bool_value(settings, "CenteredText", result.centered_text);
    result.precision_type = precision_from_xml(child_text(settings, "PrecisionType").as_deref());
    result.filter_platform = filter_from_xml(child_text(settings, "FilterPlatform"));
    result.filter_region = filter_from_xml(child_text(settings, "FilterRegion"));
    result.filter_subcategories = bool_value(
        settings,
        "FilterSubcategories",
        result.filter_subcategories,
    );
    result.filter_variables =
        bool_value(settings, "FilterVariables", result.filter_variables);
    result
}

pub fn world_record_to_xml(settings: &WorldRecordSettings) -> XmlElement {
    let mut element = XmlElement::new("Settings");
    push_text(&mut element, "Version", "1.6");
    push_color(&mut element, "TextColor", settings.text_color);
    push_bool(
        &mut element,
        "OverrideTextColor",
        settings.override_text_color,
    );
    push_color(&mut element, "TimeColor", settings.time_color);
    push_bool(
        &mut element,
        "OverrideTimeColor",
        settings.override_time_color,
    );
    push_background(&mut element, &settings.background);
    push_bool(&mut element, "Display2Rows", settings.display_2_rows);
    push_bool(&mut element, "CenteredText", settings.centered_text);
    push_bool(
        &mut element,
        "FilterRegion",
        filter_enabled(&settings.filter_region),
    );
    push_bool(
        &mut element,
        "FilterPlatform",
        filter_enabled(&settings.filter_platform),
    );
    push_bool(
        &mut element,
        "FilterVariables",
        settings.filter_variables,
    );
    push_bool(
        &mut element,
        "FilterSubcategories",
        settings.filter_subcategories,
    );
    push_text(
        &mut element,
        "TimingMethod",
        timing_method_xml_name(settings.timing_method),
    );
    push_text(
        &mut element,
        "PrecisionType",
        precision_xml_name(settings.precision_type),
    );
    element
}

pub fn draw_world_record(
    ui: &mut Ui,
    rect: Rect,
    ctx: &mut ComponentContext,
    settings: &WorldRecordSettings,
) {
    let method = ctx.resolved_timing_method(settings.timing_method);
    let value = cached_world_record(ctx.timer, method);
    let time_text = format_world_record(value, settings.precision_type);
    let name_color = if settings.override_text_color {
        settings.text_color
    } else {
        ctx.theme.text_color
    };
    let value_color = if settings.override_time_color {
        settings.time_color
    } else {
        ctx.theme.text_color
    };
    paint_background(ui.painter(), rect, &settings.background);

    if settings.centered_text && !settings.display_2_rows {
        let text = if value.is_some() {
            format!("World Record: {time_text}")
        } else {
            "No World Record".to_string()
        };
        let base_font = ctx.text_font();
        let max_size = (rect.height() / ctx.scale) * 0.72;
        let font = sized_font(
            &base_font,
            (base_font.size / ctx.scale).min(max_size),
            ctx.scale,
        );
        let pad = 7.0 * ctx.scale;
        let fitted = fit_text(
            ui.painter(),
            &text,
            &font,
            (rect.width() - 2.0 * pad).max(0.0),
        );
        paint_text(
            ui.painter(),
            rect.center(),
            Align2::CENTER_CENTER,
            &fitted,
            font,
            if value.is_some() {
                value_color
            } else {
                name_color
            },
            if ctx.theme.drop_shadows {
                Some(ctx.theme.shadows_color)
            } else {
                None
            },
            Some(ctx.theme.text_outline_color),
        );
    } else {
        draw_info(
            ui,
            rect,
            ctx,
            "World Record",
            &time_text,
            name_color,
            value_color,
            ctx.text_font(),
            settings.display_2_rows,
        );
    }
}

fn cached_world_record(timer: &Timer, method: TimingMethod) -> Option<TimeSpan> {
    let run = &timer.run;
    let comparison = run
        .custom_comparisons
        .iter()
        .find(|name| is_world_record_name(name))
        .cloned()
        .or_else(|| {
            run.segments
                .iter()
                .flat_map(|segment| segment.comparisons.keys())
                .find(|name| is_world_record_name(name))
                .cloned()
        })?;
    run.segments
        .last()?
        .comparison(&comparison)
        .get(method)
}

fn is_world_record_name(name: &str) -> bool {
    name.eq_ignore_ascii_case("World Record") || name.eq_ignore_ascii_case("WR")
}

fn format_world_record(value: Option<TimeSpan>, precision_type: i32) -> String {
    match precision_type {
        1 => format_time(value, TimeAccuracy::Seconds),
        2 => format_time(value, TimeAccuracy::Milliseconds),
        _ => GeneralTimeFormatter {
            accuracy: TimeAccuracy::Milliseconds,
            automatic_precision: true,
            ..GeneralTimeFormatter::default()
        }
        .format(value),
    }
}

pub fn world_record_settings_ui(ui: &mut Ui, settings: &mut WorldRecordSettings) {
    ui.label(egui::RichText::new("World Record").strong());
    ui.checkbox(&mut settings.display_2_rows, "Display 2 rows");
    let two_rows = settings.display_2_rows;
    let centered_text = &mut settings.centered_text;
    ui.add_enabled_ui(!two_rows, |ui| {
        ui.checkbox(centered_text, "Centered text");
    });
    timing_method_combo(
        ui,
        "Timing method:",
        "world_record_timing",
        &mut settings.timing_method,
    );
    ui.horizontal(|ui| {
        ui.label("Precision:");
        egui::ComboBox::from_id_salt(ui.scope_id().with("world_record_precision"))
            .selected_text(precision_label(settings.precision_type))
            .show_ui(ui, |ui| {
                for (precision, label) in [
                    (0i32, "From Leaderboard"),
                    (1, "Seconds"),
                    (2, "Milliseconds"),
                ] {
                    ui.selectable_value(&mut settings.precision_type, precision, label);
                }
            });
    });

    ui.separator();
    filter_checkbox(ui, "Filter by platform", &mut settings.filter_platform);
    filter_checkbox(ui, "Filter by region", &mut settings.filter_region);
    ui.checkbox(
        &mut settings.filter_subcategories,
        "Filter by subcategories",
    );
    ui.checkbox(&mut settings.filter_variables, "Filter by variables");
    ui.label("Filters are stored for layout compatibility; no network lookup is performed.");

    ui.separator();
    color_setting(
        ui,
        "Override text color",
        &mut settings.override_text_color,
        &mut settings.text_color,
    );
    color_setting(
        ui,
        "Override time color",
        &mut settings.override_time_color,
        &mut settings.time_color,
    );
    ui.separator();
    background_settings_ui(ui, &mut settings.background);
}

fn precision_label(precision: i32) -> &'static str {
    match precision {
        1 => "Seconds",
        2 => "Milliseconds",
        _ => "From Leaderboard",
    }
}

fn filter_checkbox(ui: &mut Ui, label: &str, value: &mut String) {
    let mut enabled = filter_enabled(value);
    if ui.checkbox(&mut enabled, label).changed() {
        *value = if enabled {
            "true".to_string()
        } else {
            String::new()
        };
    }
}

// ---------------------------------------------------------------------------
// Sum of Best
// ---------------------------------------------------------------------------

pub fn sum_of_best_from_xml(settings: &XmlElement) -> SumOfBestSettings {
    let mut result = SumOfBestSettings::default();
    result.text_color = parse_color(
        child_text(settings, "TextColor").as_deref(),
        result.text_color,
    );
    result.override_text_color = bool_value(
        settings,
        "OverrideTextColor",
        result.override_text_color,
    );
    result.time_color = parse_color(
        child_text(settings, "TimeColor").as_deref(),
        result.time_color,
    );
    result.override_time_color = bool_value(
        settings,
        "OverrideTimeColor",
        result.override_time_color,
    );
    result.accuracy = accuracy_from_xml(
        child_text(settings, "Accuracy").as_deref(),
        result.accuracy,
    );
    result.background = background_from_xml(settings, &result.background);
    result.display_2_rows = bool_value(settings, "Display2Rows", result.display_2_rows);
    result
}

pub fn sum_of_best_to_xml(settings: &SumOfBestSettings) -> XmlElement {
    let mut element = XmlElement::new("Settings");
    push_text(&mut element, "Version", "1.4");
    push_color(&mut element, "TextColor", settings.text_color);
    push_bool(
        &mut element,
        "OverrideTextColor",
        settings.override_text_color,
    );
    push_color(&mut element, "TimeColor", settings.time_color);
    push_bool(
        &mut element,
        "OverrideTimeColor",
        settings.override_time_color,
    );
    push_text(
        &mut element,
        "Accuracy",
        accuracy_xml_name(settings.accuracy),
    );
    push_background(&mut element, &settings.background);
    push_bool(&mut element, "Display2Rows", settings.display_2_rows);
    element
}

pub fn draw_sum_of_best(
    ui: &mut Ui,
    rect: Rect,
    ctx: &mut ComponentContext,
    settings: &SumOfBestSettings,
) {
    let value = ctx
        .timer
        .run
        .sum_of_best(ctx.timer.current_timing_method, &ctx.timer.comparison_settings);
    let name_color = if settings.override_text_color {
        settings.text_color
    } else {
        ctx.theme.text_color
    };
    let value_color = if settings.override_time_color {
        settings.time_color
    } else {
        ctx.theme.text_color
    };

    paint_background(ui.painter(), rect, &settings.background);
    draw_info(
        ui,
        rect,
        ctx,
        "Sum of Best Segments",
        &format_time(value, settings.accuracy),
        name_color,
        value_color,
        ctx.times_font(),
        settings.display_2_rows,
    );
}

pub fn sum_of_best_settings_ui(ui: &mut Ui, settings: &mut SumOfBestSettings) {
    ui.label(egui::RichText::new("Sum of Best").strong());
    accuracy_combo(
        ui,
        "Accuracy:",
        "sum_of_best_accuracy",
        &mut settings.accuracy,
    );
    ui.checkbox(&mut settings.display_2_rows, "Display 2 rows");
    ui.separator();
    color_setting(
        ui,
        "Override text color",
        &mut settings.override_text_color,
        &mut settings.text_color,
    );
    color_setting(
        ui,
        "Override time color",
        &mut settings.override_time_color,
        &mut settings.time_color,
    );
    ui.separator();
    background_settings_ui(ui, &mut settings.background);
}

// ---------------------------------------------------------------------------
// Separators and The Run
// ---------------------------------------------------------------------------

pub fn draw_separator(ui: &mut Ui, rect: Rect, ctx: &mut ComponentContext, thin: bool) {
    let color = if thin {
        ctx.theme.thin_separators_color
    } else {
        ctx.theme.separators_color
    };
    let thickness = (if thin { 1.0 } else { 2.0 }) * ctx.scale;
    let thickness = thickness.max(1.0);
    let y = rect.center().y;
    ui.painter().line_segment(
        [egui::pos2(rect.left(), y), egui::pos2(rect.right(), y)],
        egui::Stroke::new(thickness, color),
    );
}

pub fn draw_the_run(ui: &mut Ui, rect: Rect, ctx: &mut ComponentContext) {
    let name = ctx.timer.run.extended_name();
    if name.trim().is_empty() {
        return;
    }
    let base_font = ctx.text_font();
    let max_size = (rect.height() / ctx.scale) * 0.72;
    let font = sized_font(
        &base_font,
        (base_font.size / ctx.scale).min(max_size),
        ctx.scale,
    );
    let pad = 7.0 * ctx.scale;
    let painter = ui.painter();
    let fitted = fit_text(
        painter,
        &name,
        &font,
        (rect.width() - 2.0 * pad).max(0.0),
    );
    paint_text(
        painter,
        rect.center(),
        Align2::CENTER_CENTER,
        &fitted,
        font,
        ctx.theme.text_color,
        if ctx.theme.drop_shadows {
            Some(ctx.theme.shadows_color)
        } else {
            None
        },
        Some(ctx.theme.text_outline_color),
    );
}

pub fn draw_separator_component(
    ui: &mut Ui,
    rect: Rect,
    ctx: &mut ComponentContext,
    _settings: &SeparatorSettings,
) {
    draw_separator(ui, rect, ctx, false);
}

pub fn draw_thin_separator_component(
    ui: &mut Ui,
    rect: Rect,
    ctx: &mut ComponentContext,
    _settings: &ThinSeparatorSettings,
) {
    draw_separator(ui, rect, ctx, true);
}

// ---------------------------------------------------------------------------
// Shared drawing helpers
// ---------------------------------------------------------------------------

fn sized_font(font: &FontId, unscaled_size: f32, scale: f32) -> FontId {
    FontId::new((unscaled_size * scale).max(4.0), font.family.clone())
}

#[allow(clippy::too_many_arguments)]
fn draw_info(
    ui: &mut Ui,
    rect: Rect,
    ctx: &ComponentContext,
    name: &str,
    value: &str,
    name_color: Color32,
    value_color: Color32,
    value_font: FontId,
    display_2_rows: bool,
) {
    if rect.width() <= 0.0 || rect.height() <= 0.0 {
        return;
    }
    let pad = 7.0 * ctx.scale;
    let left = rect.left() + pad;
    let right = rect.right() - pad;
    let top = rect.top() + 1.0 * ctx.scale;
    let bottom = rect.bottom() - 1.0 * ctx.scale;
    if right <= left || bottom <= top {
        return;
    }

    let painter = ui.painter();
    let name_source = ctx.text_font();
    let unscaled_height = rect.height() / ctx.scale;
    let two_row_size = (unscaled_height * 0.45).max(4.0);
    let one_row_size = (unscaled_height * 0.72).max(4.0);
    let name_font = sized_font(
        &name_source,
        (name_source.size / ctx.scale).min(if display_2_rows {
            two_row_size
        } else {
            one_row_size
        }),
        ctx.scale,
    );
    let value_font = sized_font(
        &value_font,
        (value_font.size / ctx.scale).min(if display_2_rows {
            two_row_size
        } else {
            one_row_size
        }),
        ctx.scale,
    );
    let shadow = if ctx.theme.drop_shadows {
        Some(ctx.theme.shadows_color)
    } else {
        None
    };
    let outline = Some(ctx.theme.text_outline_color);
    let name_empty = name.is_empty();
    let value_empty = value.is_empty();

    if name_empty && value_empty {
        return;
    }

    if name_empty || value_empty {
        let (text, color, font) = if name_empty {
            (value, value_color, value_font)
        } else {
            (name, name_color, name_font)
        };
        let fitted = fit_text(painter, text, &font, (right - left).max(0.0));
        paint_text(
            painter,
            egui::pos2((left + right) / 2.0, (top + bottom) / 2.0),
            Align2::CENTER_CENTER,
            &fitted,
            font,
            color,
            shadow,
            outline,
        );
        return;
    }

    if display_2_rows {
        let fitted_name = fit_text(painter, name, &name_font, (right - left).max(0.0));
        paint_text(
            painter,
            egui::pos2(left, top),
            Align2::LEFT_TOP,
            &fitted_name,
            name_font,
            name_color,
            shadow,
            outline,
        );
        let fitted_value = fit_text(painter, value, &value_font, (right - left).max(0.0));
        paint_text(
            painter,
            egui::pos2(right, bottom),
            Align2::RIGHT_BOTTOM,
            &fitted_value,
            value_font,
            value_color,
            shadow,
            outline,
        );
    } else {
        let max_value_width = (right - left).max(0.0) * 0.65;
        let fitted_value = fit_text(painter, value, &value_font, max_value_width);
        let value_width = text_width(painter, &fitted_value, &value_font);
        paint_text(
            painter,
            egui::pos2(right, (top + bottom) / 2.0),
            Align2::RIGHT_CENTER,
            &fitted_value,
            value_font,
            value_color,
            shadow,
            outline,
        );
        let name_width = (right - left - value_width - pad).max(0.0);
        let fitted_name = fit_text(painter, name, &name_font, name_width);
        paint_text(
            painter,
            egui::pos2(left, (top + bottom) / 2.0),
            Align2::LEFT_CENTER,
            &fitted_name,
            name_font,
            name_color,
            shadow,
            outline,
        );
    }
}

// ---------------------------------------------------------------------------
// Shared settings UI helpers
// ---------------------------------------------------------------------------

const GRADIENTS: [GradientType; 7] = [
    GradientType::Plain,
    GradientType::Vertical,
    GradientType::Horizontal,
    GradientType::Alternating,
    GradientType::PlainWithDeltaColor,
    GradientType::VerticalWithDeltaColor,
    GradientType::HorizontalWithDeltaColor,
];

const ACCURACIES: [TimeAccuracy; 4] = [
    TimeAccuracy::Seconds,
    TimeAccuracy::Tenths,
    TimeAccuracy::Hundredths,
    TimeAccuracy::Milliseconds,
];

const COMPARISONS: [&str; 11] = [
    "Current Comparison",
    "Personal Best",
    "Best Segments",
    "Best Split Times",
    "Latest Run",
    "Average Segments",
    "Median Segments",
    "Worst Segments",
    "Balanced PB",
    "Golf HCP",
    "None",
];

fn color_setting(
    ui: &mut Ui,
    label: &str,
    override_flag: &mut bool,
    color: &mut Color32,
) {
    ui.horizontal(|ui| {
        ui.checkbox(override_flag, label);
        let enabled = *override_flag;
        ui.add_enabled_ui(enabled, |ui| {
            ui.color_edit_button_srgba(color);
        });
    });
}

fn font_editor(ui: &mut Ui, font: &mut FontSpec) {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label("Font:");
        changed |= ui.text_edit_singleline(&mut font.name).changed();
    });
    ui.horizontal(|ui| {
        ui.label("Size:");
        changed |= ui
            .add(
                egui::DragValue::new(&mut font.size)
                    .speed(0.1)
                    .range(1.0..=300.0),
            )
            .changed();
        changed |= ui.checkbox(&mut font.bold, "Bold").changed();
        changed |= ui.checkbox(&mut font.italic, "Italic").changed();
    });
    if changed {
        font.raw = None;
    }
}

fn background_settings_ui(ui: &mut Ui, background: &mut BackgroundStyle) {
    ui.label(egui::RichText::new("Background").strong());
    ui.horizontal(|ui| {
        ui.label("Gradient:");
        egui::ComboBox::from_id_salt(ui.scope_id().with("background_gradient"))
            .selected_text(background.gradient.label())
            .show_ui(ui, |ui| {
                for gradient in GRADIENTS {
                    ui.selectable_value(&mut background.gradient, gradient, gradient.label());
                }
            });
    });
    ui.horizontal(|ui| {
        ui.label("Color:");
        ui.color_edit_button_srgba(&mut background.color);
        if !matches!(
            background.gradient,
            GradientType::Plain | GradientType::PlainWithDeltaColor
        ) {
            ui.label("Color 2:");
            ui.color_edit_button_srgba(&mut background.color2);
        }
    });
}

fn accuracy_combo(
    ui: &mut Ui,
    label: &str,
    salt: &'static str,
    accuracy: &mut TimeAccuracy,
) {
    ui.horizontal(|ui| {
        ui.label(label);
        egui::ComboBox::from_id_salt(ui.scope_id().with(salt))
            .selected_text(accuracy.label())
            .show_ui(ui, |ui| {
                for option in ACCURACIES {
                    ui.selectable_value(accuracy, option, option.label());
                }
            });
    });
}

fn timing_method_combo(
    ui: &mut Ui,
    label: &str,
    salt: &'static str,
    method: &mut TimingMethodSetting,
) {
    const METHODS: [TimingMethodSetting; 3] = [
        TimingMethodSetting::Current,
        TimingMethodSetting::RealTime,
        TimingMethodSetting::GameTime,
    ];
    ui.horizontal(|ui| {
        ui.label(label);
        egui::ComboBox::from_id_salt(ui.scope_id().with(salt))
            .selected_text(method.label())
            .show_ui(ui, |ui| {
                for option in METHODS {
                    ui.selectable_value(method, option, option.label());
                }
            });
    });
}

fn comparison_combo(ui: &mut Ui, salt: &'static str, comparison: &mut String) {
    let mut value = comparison.clone();
    ui.horizontal(|ui| {
        ui.label("Comparison:");
        egui::ComboBox::from_id_salt(ui.scope_id().with(salt))
            .selected_text(if value.is_empty() {
                "(none)".to_string()
            } else {
                value.clone()
            })
            .show_ui(ui, |ui| {
                for option in COMPARISONS {
                    ui.selectable_value(&mut value, option.to_string(), option);
                }
            });
    });
    ui.horizontal(|ui| {
        ui.label("Name:");
        ui.text_edit_singleline(&mut value);
    });
    *comparison = value;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xml::{parse_xml, xml_to_string};

    fn round_trip(element: &XmlElement) -> XmlElement {
        let text = xml_to_string(element).expect("serialize settings");
        parse_xml(text.as_bytes()).expect("parse settings")
    }

    #[test]
    fn title_round_trip() {
        let mut settings = TitleSettings::default();
        settings.show_game_name = false;
        settings.show_attempt_count = false;
        settings.show_finished_runs_count = true;
        settings.text_alignment = 2;
        settings.single_line = true;
        settings.display_game_icon = false;
        settings.show_region = true;
        settings.show_platform = true;
        settings.show_variables = false;
        settings.override_title_color = true;
        settings.title_color = Color32::from_rgb(1, 2, 3);
        settings.background = BackgroundStyle::vertical(
            Color32::from_rgb(10, 20, 30),
            Color32::from_rgb(40, 50, 60),
        );
        settings.override_title_font = true;
        settings.title_font.size = 22.0;
        settings.title_font.bold = true;

        let parsed = title_from_xml(&round_trip(&title_to_xml(&settings)));
        assert!(!parsed.show_game_name);
        assert!(!parsed.show_attempt_count);
        assert!(parsed.show_finished_runs_count);
        assert_eq!(parsed.text_alignment, 2);
        assert!(parsed.single_line);
        assert!(!parsed.display_game_icon);
        assert!(parsed.show_region);
        assert!(parsed.show_platform);
        assert!(!parsed.show_variables);
        assert_eq!(parsed.title_color, settings.title_color);
        assert!(parsed.override_title_color);
        assert_eq!(parsed.background.gradient, GradientType::Vertical);
        assert_eq!(parsed.background.color, settings.background.color);
        assert_eq!(parsed.background.color2, settings.background.color2);
        assert!(parsed.override_title_font);
        assert_eq!(parsed.title_font.size, 22.0);
        assert!(parsed.title_font.bold);
    }

    #[test]
    fn text_counter_and_comparison_round_trip() {
        let mut text = TextSettings::default();
        text.text1 = "{Game}".to_string();
        text.text2 = "ignored".to_string();
        text.custom_variable = "Route".to_string();
        text.override_time_color = true;
        text.time_color = Color32::from_rgb(7, 8, 9);
        text.display_2_rows = true;
        let parsed = text_from_xml(&round_trip(&text_to_xml(&text)));
        assert_eq!(parsed.text1, "{Game}");
        assert_eq!(parsed.text2, "Route");
        assert_eq!(parsed.custom_variable, "Route");
        assert_eq!(parsed.time_color, text.time_color);
        assert!(parsed.override_time_color);
        assert!(parsed.display_2_rows);

        let mut counter = CounterSettings::default();
        counter.counter_text = "Deaths".to_string();
        counter.initial_value = -3;
        counter.increment = 2;
        counter.increment_key = "F1".to_string();
        counter.decrement_key = "F2".to_string();
        counter.reset_key = "F3".to_string();
        counter.global_hotkeys_enabled = true;
        let parsed = counter_from_xml(&round_trip(&counter_to_xml(&counter)));
        assert_eq!(parsed.counter_text, "Deaths");
        assert_eq!(parsed.initial_value, -3);
        assert_eq!(parsed.increment, 2);
        assert_eq!(parsed.increment_key, "F1");
        assert_eq!(parsed.decrement_key, "F2");
        assert_eq!(parsed.reset_key, "F3");
        assert!(parsed.global_hotkeys_enabled);

        let mut comparison = ComparisonTimeSettings::default();
        comparison.comparison = "Best Segments".to_string();
        comparison.timing_method = TimingMethodSetting::GameTime;
        comparison.accuracy = TimeAccuracy::Hundredths;
        comparison.kind = 2;
        comparison.split_index = 4;
        let parsed = comparison_time_from_xml(&round_trip(&comparison_time_to_xml(&comparison)));
        assert_eq!(parsed.comparison, "Best Segments");
        assert_eq!(parsed.timing_method, TimingMethodSetting::GameTime);
        assert_eq!(parsed.accuracy, TimeAccuracy::Hundredths);
        assert_eq!(parsed.kind, 2);
        assert_eq!(parsed.split_index, 4);
    }

    #[test]
    fn world_record_and_sum_of_best_round_trip() {
        let mut record = WorldRecordSettings::default();
        record.override_time_color = true;
        record.time_color = Color32::from_rgb(200, 100, 50);
        record.timing_method = TimingMethodSetting::RealTime;
        record.centered_text = true;
        record.precision_type = 2;
        record.filter_platform = "true".to_string();
        record.filter_region = String::new();
        record.filter_subcategories = true;
        record.filter_variables = true;
        let parsed = world_record_from_xml(&round_trip(&world_record_to_xml(&record)));
        assert_eq!(parsed.time_color, record.time_color);
        assert!(parsed.override_time_color);
        assert_eq!(parsed.timing_method, TimingMethodSetting::RealTime);
        assert!(parsed.centered_text);
        assert_eq!(parsed.precision_type, 2);
        assert!(filter_enabled(&parsed.filter_platform));
        assert!(!filter_enabled(&parsed.filter_region));
        assert!(parsed.filter_subcategories);
        assert!(parsed.filter_variables);

        let mut sum = SumOfBestSettings::default();
        sum.accuracy = TimeAccuracy::Tenths;
        sum.override_text_color = true;
        sum.text_color = Color32::from_rgb(1, 1, 1);
        sum.override_time_color = true;
        sum.time_color = Color32::from_rgb(2, 2, 2);
        sum.display_2_rows = true;
        let parsed = sum_of_best_from_xml(&round_trip(&sum_of_best_to_xml(&sum)));
        assert_eq!(parsed.accuracy, TimeAccuracy::Tenths);
        assert!(parsed.override_text_color);
        assert_eq!(parsed.text_color, sum.text_color);
        assert!(parsed.override_time_color);
        assert_eq!(parsed.time_color, sum.time_color);
        assert!(parsed.display_2_rows);
    }

    #[test]
    fn missing_settings_and_empty_runs_are_safe() {
        let empty = XmlElement::new("Settings");
        let _ = title_from_xml(&empty);
        let _ = text_from_xml(&empty);
        let _ = blank_space_from_xml(&empty);
        let _ = total_playtime_from_xml(&empty);
        let _ = counter_from_xml(&empty);
        let _ = hotkey_indicator_from_xml(&empty);
        let _ = comparison_time_from_xml(&empty);
        let _ = world_record_from_xml(&empty);
        let _ = sum_of_best_from_xml(&empty);

        let timer = Timer::with_system_clock(Run::default());
        assert_eq!(total_playtime_nanos(&timer), 0);
        assert_eq!(format_playtime(0, false), "0:00");
        assert_eq!(comparison_time_value(&timer, "Personal Best", TimingMethod::RealTime, 0, 0), None);
        assert_eq!(cached_world_record(&timer, TimingMethod::RealTime), None);
        assert_eq!(format_world_record(None, 0), "-");
    }
}
