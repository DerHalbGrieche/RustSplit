//! Layout and global theme model.

use std::path::PathBuf;

use egui::Color32;

use crate::components::Component;

pub const DEFAULT_LAYOUT_LSL: &str =
    include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/default.lsl"));

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LayoutMode {
    #[default]
    Vertical,
    Horizontal,
}

impl LayoutMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Vertical => "Vertical",
            Self::Horizontal => "Horizontal",
        }
    }

    pub fn parse(value: &str, default: Self) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "vertical" => Self::Vertical,
            "horizontal" => Self::Horizontal,
            _ => default,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GradientType {
    #[default]
    Plain,
    Vertical,
    Horizontal,
    Alternating,
    PlainWithDeltaColor,
    VerticalWithDeltaColor,
    HorizontalWithDeltaColor,
}

impl GradientType {
    pub fn label(self) -> &'static str {
        match self {
            Self::Plain => "Plain",
            Self::Vertical => "Vertical",
            Self::Horizontal => "Horizontal",
            Self::Alternating => "Alternating",
            Self::PlainWithDeltaColor => "Plain With Delta Color",
            Self::VerticalWithDeltaColor => "Vertical With Delta Color",
            Self::HorizontalWithDeltaColor => "Horizontal With Delta Color",
        }
    }

    pub fn parse(value: &str, default: Self) -> Self {
        let normalized: String = value
            .chars()
            .filter(|ch| !ch.is_whitespace())
            .collect::<String>()
            .to_ascii_lowercase();
        match normalized.as_str() {
            "plain" => Self::Plain,
            "vertical" => Self::Vertical,
            "horizontal" => Self::Horizontal,
            "alternating" => Self::Alternating,
            "plainwithdeltacolor" => Self::PlainWithDeltaColor,
            "verticalwithdeltacolor" => Self::VerticalWithDeltaColor,
            "horizontalwithdeltacolor" => Self::HorizontalWithDeltaColor,
            _ => default,
        }
    }

    pub fn is_horizontal(self) -> bool {
        matches!(self, Self::Horizontal | Self::HorizontalWithDeltaColor)
    }
}

/// A font as configured in LiveSplit.  `raw` keeps the original .NET
/// BinaryFormatter blob so unchanged layouts can be saved without losing
/// information.
#[derive(Clone, Debug, PartialEq)]
pub struct FontSpec {
    pub name: String,
    pub size: f32,
    pub bold: bool,
    pub italic: bool,
    pub raw: Option<String>,
}

impl Default for FontSpec {
    fn default() -> Self {
        Self::text()
    }
}

impl FontSpec {
    pub fn text() -> Self {
        Self {
            name: "Segoe UI".to_string(),
            size: 16.0,
            bold: false,
            italic: false,
            raw: None,
        }
    }

    pub fn times() -> Self {
        Self {
            name: "Segoe UI".to_string(),
            size: 16.0,
            bold: true,
            italic: false,
            raw: None,
        }
    }

    pub fn timer() -> Self {
        Self {
            name: "Century Gothic".to_string(),
            size: 43.75,
            bold: true,
            italic: false,
            raw: None,
        }
    }

    pub fn label(&self) -> String {
        let weight = if self.bold { " Bold" } else { "" };
        format!("{} {}{}", self.name, self.size, weight)
    }
}

#[derive(Clone, Debug)]
pub struct LayoutSettings {
    pub text_color: Color32,
    pub background_color: Color32,
    pub background_color2: Color32,
    pub thin_separators_color: Color32,
    pub separators_color: Color32,
    pub personal_best_color: Color32,
    pub ahead_gaining_time_color: Color32,
    pub ahead_losing_time_color: Color32,
    pub behind_gaining_time_color: Color32,
    pub behind_losing_time_color: Color32,
    pub best_segment_color: Color32,
    pub not_running_color: Color32,
    pub paused_color: Color32,
    pub text_outline_color: Color32,
    pub shadows_color: Color32,
    pub background_gradient: GradientType,
    pub background_image: Option<PathBuf>,
    pub image_opacity: f32,
    pub image_blur: f32,
    pub timer_font: FontSpec,
    pub times_font: FontSpec,
    pub text_font: FontSpec,
    pub show_best_segments: bool,
    pub always_on_top: bool,
    pub anti_aliasing: bool,
    pub drop_shadows: bool,
    pub use_rainbow_color: bool,
    pub opacity: f32,
    pub mouse_pass_through_while_running: bool,
    pub allow_resizing: bool,
    pub allow_moving: bool,
}

impl Default for LayoutSettings {
    fn default() -> Self {
        Self {
            text_color: Color32::WHITE,
            background_color: Color32::TRANSPARENT,
            background_color2: Color32::TRANSPARENT,
            thin_separators_color: Color32::from_rgba_unmultiplied(255, 255, 255, 9),
            separators_color: Color32::from_rgba_unmultiplied(255, 255, 255, 38),
            personal_best_color: Color32::from_rgb(22, 166, 255),
            ahead_gaining_time_color: Color32::from_rgb(41, 204, 84),
            ahead_losing_time_color: Color32::from_rgb(112, 204, 137),
            behind_gaining_time_color: Color32::from_rgb(204, 120, 112),
            behind_losing_time_color: Color32::from_rgb(204, 55, 41),
            best_segment_color: Color32::from_rgb(216, 175, 31),
            not_running_color: Color32::from_rgb(122, 122, 122),
            paused_color: Color32::from_rgb(122, 122, 122),
            text_outline_color: Color32::TRANSPARENT,
            shadows_color: Color32::from_rgba_unmultiplied(0, 0, 0, 128),
            background_gradient: GradientType::Plain,
            background_image: None,
            image_opacity: 1.0,
            image_blur: 0.0,
            timer_font: FontSpec::timer(),
            times_font: FontSpec::times(),
            text_font: FontSpec::text(),
            show_best_segments: true,
            always_on_top: true,
            anti_aliasing: true,
            drop_shadows: true,
            use_rainbow_color: false,
            opacity: 1.0,
            mouse_pass_through_while_running: false,
            allow_resizing: true,
            allow_moving: true,
        }
    }
}

impl LayoutSettings {
    /// Settings of LiveSplit's stock layout (`DefaultLayout.lsl`).
    pub fn livesplit_default() -> Self {
        Self {
            text_color: Color32::WHITE,
            background_color: Color32::from_rgb(15, 15, 15),
            background_color2: Color32::TRANSPARENT,
            thin_separators_color: Color32::from_rgba_unmultiplied(255, 255, 255, 3),
            separators_color: Color32::from_rgba_unmultiplied(255, 255, 255, 36),
            personal_best_color: Color32::from_rgb(22, 166, 255),
            ahead_gaining_time_color: Color32::from_rgb(0, 204, 54),
            ahead_losing_time_color: Color32::from_rgb(82, 204, 115),
            behind_gaining_time_color: Color32::from_rgb(204, 92, 82),
            behind_losing_time_color: Color32::from_rgb(204, 18, 0),
            best_segment_color: Color32::from_rgb(216, 175, 31),
            not_running_color: Color32::from_rgb(172, 172, 172),
            paused_color: Color32::from_rgb(122, 122, 122),
            text_outline_color: Color32::TRANSPARENT,
            shadows_color: Color32::from_rgba_unmultiplied(0, 0, 0, 128),
            background_gradient: GradientType::Plain,
            background_image: None,
            image_opacity: 1.0,
            image_blur: 0.0,
            timer_font: FontSpec::timer(),
            times_font: FontSpec::times(),
            text_font: FontSpec::text(),
            show_best_segments: true,
            always_on_top: true,
            anti_aliasing: true,
            drop_shadows: true,
            use_rainbow_color: false,
            opacity: 1.0,
            mouse_pass_through_while_running: false,
            allow_resizing: true,
            allow_moving: true,
        }
    }

    pub fn color_for_split(&self, color: livesplit_core::SplitColor) -> Color32 {
        match color {
            livesplit_core::SplitColor::AheadGaining => self.ahead_gaining_time_color,
            livesplit_core::SplitColor::AheadLosing => self.ahead_losing_time_color,
            livesplit_core::SplitColor::BehindGaining => self.behind_gaining_time_color,
            livesplit_core::SplitColor::BehindLosing => self.behind_losing_time_color,
            livesplit_core::SplitColor::BestSegment => self.best_segment_color,
            livesplit_core::SplitColor::NotRunning => self.not_running_color,
            livesplit_core::SplitColor::Paused => self.paused_color,
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct BackgroundStyle {
    pub color: Color32,
    pub color2: Color32,
    pub gradient: GradientType,
}

impl BackgroundStyle {
    pub fn transparent() -> Self {
        Self {
            color: Color32::TRANSPARENT,
            color2: Color32::TRANSPARENT,
            gradient: GradientType::Plain,
        }
    }

    pub fn vertical(top: Color32, bottom: Color32) -> Self {
        Self {
            color: top,
            color2: bottom,
            gradient: GradientType::Vertical,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct FontOverrides {
    pub override_timer_font: bool,
    pub timer_font: FontSpec,
    pub override_times_font: bool,
    pub times_font: FontSpec,
    pub override_text_font: bool,
    pub text_font: FontSpec,
}

impl FontOverrides {
    pub fn has_overrides(&self) -> bool {
        self.override_timer_font || self.override_times_font || self.override_text_font
    }
}

#[derive(Clone, Debug)]
pub struct LayoutComponent {
    pub path: String,
    pub component: Component,
    pub font_overrides: FontOverrides,
}

impl LayoutComponent {
    pub fn new(component: Component) -> Self {
        Self {
            path: component.path().to_string(),
            component,
            font_overrides: FontOverrides::default(),
        }
    }

    pub fn name(&self) -> &'static str {
        self.component.name()
    }
}

#[derive(Clone, Debug)]
pub struct Layout {
    pub mode: LayoutMode,
    pub x: i32,
    pub y: i32,
    pub vertical_width: i32,
    pub vertical_height: i32,
    pub horizontal_width: i32,
    pub horizontal_height: i32,
    pub settings: LayoutSettings,
    pub components: Vec<LayoutComponent>,
    pub file_path: Option<PathBuf>,
    pub has_changed: bool,
}

impl Default for Layout {
    fn default() -> Self {
        Self {
            mode: LayoutMode::Vertical,
            x: 100,
            y: 100,
            vertical_width: 286,
            vertical_height: 519,
            horizontal_width: -1,
            horizontal_height: -1,
            settings: LayoutSettings::default(),
            components: Vec::new(),
            file_path: None,
            has_changed: false,
        }
    }
}

impl Layout {
    /// A programmatic copy of LiveSplit's stock layout.
    pub fn livesplit_default() -> Self {
        use crate::components::settings::*;
        Self {
            mode: LayoutMode::Vertical,
            x: 100,
            y: 100,
            vertical_width: 286,
            vertical_height: 519,
            horizontal_width: -1,
            horizontal_height: -1,
            settings: LayoutSettings::livesplit_default(),
            components: vec![
                LayoutComponent::new(Component::Title(TitleSettings::default())),
                LayoutComponent::new(Component::Splits(SplitsSettings::default())),
                LayoutComponent::new(Component::Timer(TimerSettings::default())),
                LayoutComponent::new(Component::PreviousSegment(
                    PreviousSegmentSettings::default(),
                )),
            ],
            file_path: None,
            has_changed: false,
        }
    }

    pub fn parse_default() -> Result<Self, crate::layout_xml::LayoutXmlError> {
        crate::layout_xml::parse_layout(DEFAULT_LAYOUT_LSL.as_bytes())
    }

    pub fn component_index(&self, path: &str) -> Option<usize> {
        self.components.iter().position(|component| component.path == path)
    }

    pub fn move_component_up(&mut self, index: usize) {
        if index > 0 && index < self.components.len() {
            self.components.swap(index, index - 1);
            self.has_changed = true;
        }
    }

    pub fn move_component_down(&mut self, index: usize) {
        if index + 1 < self.components.len() {
            self.components.swap(index, index + 1);
            self.has_changed = true;
        }
    }

    pub fn remove_component(&mut self, index: usize) {
        if index < self.components.len() {
            self.components.remove(index);
            self.has_changed = true;
        }
    }

    pub fn add_component(&mut self, component: Component) {
        self.components.push(LayoutComponent::new(component));
        self.has_changed = true;
    }
}

/// Parse a LiveSplit ARGB hex color (`AARRGGBB` or `RRGGBB`).
pub fn parse_color(value: Option<&str>, default: Color32) -> Color32 {
    let Some(value) = value else {
        return default;
    };
    let value = value.trim().trim_start_matches('#');
    let parse = |text: &str| u8::from_str_radix(text, 16).ok();
    match value.len() {
        8 => {
            let a = parse(&value[0..2]);
            let r = parse(&value[2..4]);
            let g = parse(&value[4..6]);
            let b = parse(&value[6..8]);
            match (a, r, g, b) {
                (Some(a), Some(r), Some(g), Some(b)) => {
                    Color32::from_rgba_unmultiplied(r, g, b, a)
                }
                _ => default,
            }
        }
        6 => {
            let r = parse(&value[0..2]);
            let g = parse(&value[2..4]);
            let b = parse(&value[4..6]);
            match (r, g, b) {
                (Some(r), Some(g), Some(b)) => Color32::from_rgb(r, g, b),
                _ => default,
            }
        }
        _ => default,
    }
}

/// Serialize a color the way LiveSplit does (`AARRGGBB`).
pub fn color_to_hex(color: Color32) -> String {
    format!(
        "{:02X}{:02X}{:02X}{:02X}",
        color.a(),
        color.r(),
        color.g(),
        color.b()
    )
}

/// Basic alpha blending of a color with a global opacity.
pub fn with_opacity(color: Color32, opacity: f32) -> Color32 {
    let alpha = (color.a() as f32 * opacity.clamp(0.0, 1.0)).round() as u8;
    Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), alpha)
}
