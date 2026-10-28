//! LiveSplit `.lsl` layout file parsing and saving.
//!
//! # Compatibility notes
//!
//! Layout files written by LiveSplit 1.3 through 1.7 are supported:
//!
//! * Up to 1.6.0 the global background mode lives in
//!   `<BackgroundGradient>Plain|Vertical|Horizontal</BackgroundGradient>`.
//!   1.6.1 and newer use
//!   `<BackgroundType>SolidColor|VerticalGradient|HorizontalGradient|Image</BackgroundType>`.
//!   Both spellings are understood regardless of the declared version.
//! * Before 1.3 the two non-timer fonts were called `<MainFont>` and
//!   `<SplitNamesFont>`.  Those names are accepted (and preferred when the
//!   newer names are missing) for every version.
//! * Component paths may contain a directory, a `.dll` suffix and a query
//!   suffix such as `LiveSplit.Title.dll?Version=1.6.1`.  Component matching
//!   ignores those decorations while the original path is kept verbatim so
//!   that saving does not rewrite the layout.
//! * Unknown components keep their complete `<Settings>` subtree and are
//!   written back unchanged.
//!
//! ## Fonts
//!
//! LiveSplit serializes `System.Drawing.Font` with .NET's `BinaryFormatter`
//! and stores the result as base64 inside a CDATA block.  The font name is a
//! BinaryFormatter string record (7-bit encoded byte length + UTF-8), followed
//! by the little-endian `f32` size and references to the `FontStyle` and
//! `GraphicsUnit` enum objects.  We parse that format directly.
//!
//! When a layout is saved unchanged, the original base64 blob is emitted
//! byte-for-byte.  When a font is changed we patch a known-good LiveSplit font
//! blob (the original blob when it parsed, otherwise a bundled template) with
//! the new name, size and bold/italic flags.  The result is still a valid
//! .NET `BinaryFormatter` font blob.  If a blob cannot be understood at all it
//! is preserved verbatim rather than being replaced.  Only if patching is
//! impossible do we fall back to RustSplit's `Name|Size|Bold|Italic` format.
//!
//! ## Background images
//!
//! LiveSplit stores the background image itself as a base64
//! `BinaryFormatter` blob in `<BackgroundImage>`.  Decoding .NET image
//! serialization is outside the scope of the layout loader, but the blob is
//! not discarded: it is represented by an opaque path prefixed with
//! [`EMBEDDED_IMAGE_PREFIX`] in [`LayoutSettings::background_image`].  Saving
//! writes the original blob back unchanged.  The image is not decoded for
//! rendering yet.  RustSplit's own `<BackgroundImagePath>` extension is still
//! understood and written when the image is a plain filesystem path.

use std::path::{Path, PathBuf};

use base64::Engine;
use thiserror::Error;
use xmltree::XMLNode;

use crate::components::Component;
use crate::layout::{
    parse_color, FontOverrides, FontSpec, GradientType, Layout, LayoutComponent, LayoutMode,
    LayoutSettings,
};
use crate::xml::{
    child, child_text, children_named, element_with_text, parse_bool, parse_f32, parse_i32,
    parse_xml, text_trimmed, xml_to_string, XmlElement,
};

#[derive(Debug, Error)]
pub enum LayoutXmlError {
    #[error("could not read layout: {0}")]
    Io(#[from] std::io::Error),
    #[error("could not parse layout XML: {0}")]
    Xml(#[from] crate::xml::XmlError),
    #[error("invalid layout: {0}")]
    Invalid(String),
}

/// Prefix used to keep LiveSplit's serialized background image alive inside
/// [`LayoutSettings::background_image`] without decoding it.
pub const EMBEDDED_IMAGE_PREFIX: &str = "livesplit-embedded-image:";

pub fn load_layout(path: impl AsRef<Path>) -> Result<Layout, LayoutXmlError> {
    let path = path.as_ref();
    let data = std::fs::read(path)?;
    let mut layout = parse_layout(&data)?;
    layout.file_path = Some(path.to_path_buf());
    Ok(layout)
}

pub fn save_layout(layout: &Layout, path: impl AsRef<Path>) -> Result<(), LayoutXmlError> {
    let xml = layout_to_string(layout)?;
    std::fs::write(path, xml)?;
    Ok(())
}

pub fn parse_layout(data: impl AsRef<[u8]>) -> Result<Layout, LayoutXmlError> {
    let root = parse_xml(data)?;
    if root.name != "Layout" {
        return Err(LayoutXmlError::Invalid(format!(
            "expected <Layout>, found <{}>",
            root.name
        )));
    }
    let version = LayoutVersion::parse(root.attributes.get("version").map(String::as_str));
    let mut layout = Layout::default();
    layout.mode = LayoutMode::parse(
        child_text(&root, "Mode").as_deref().unwrap_or("Vertical"),
        LayoutMode::Vertical,
    );
    layout.x = parse_i32(child_text(&root, "X").as_deref(), 100);
    layout.y = parse_i32(child_text(&root, "Y").as_deref(), 100);
    layout.vertical_width = parse_i32(child_text(&root, "VerticalWidth").as_deref(), 286);
    layout.vertical_height = parse_i32(child_text(&root, "VerticalHeight").as_deref(), 519);
    layout.horizontal_width = parse_i32(child_text(&root, "HorizontalWidth").as_deref(), -1);
    layout.horizontal_height = parse_i32(child_text(&root, "HorizontalHeight").as_deref(), -1);
    if let Some(settings) = child(&root, "Settings") {
        layout.settings = parse_layout_settings(settings, &layout.settings, version);
    }
    if let Some(components) = child(&root, "Components") {
        for element in children_named(components, "Component") {
            let path = child_text(element, "Path")
                .ok_or_else(|| LayoutXmlError::Invalid("component without path".into()))?;
            let settings = child(element, "Settings")
                .cloned()
                .unwrap_or_else(|| XmlElement::new("Settings"));
            let component = Component::from_xml(&path, &settings);
            let mut layout_component = LayoutComponent {
                path,
                component,
                font_overrides: FontOverrides::default(),
            };
            if let Some(overrides) = child(element, "FontOverrides") {
                parse_font_overrides(overrides, &mut layout_component.font_overrides);
            }
            layout.components.push(layout_component);
        }
    }
    Ok(layout)
}

/// Version stored in the `<Layout version="...">` attribute.
///
/// Missing or malformed versions are treated like LiveSplit's
/// `new Version(1, 0, 0, 0)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct LayoutVersion {
    major: u32,
    minor: u32,
    patch: u32,
    build: u32,
}

impl LayoutVersion {
    fn parse(value: Option<&str>) -> Self {
        let mut parts = value
            .unwrap_or("1.0.0.0")
            .trim()
            .split('.')
            .map(|part| part.parse::<u32>().ok());
        Self {
            major: parts.next().flatten().unwrap_or(1),
            minor: parts.next().flatten().unwrap_or(0),
            patch: parts.next().flatten().unwrap_or(0),
            build: parts.next().flatten().unwrap_or(0),
        }
    }

    fn at_least(self, major: u32, minor: u32, patch: u32) -> bool {
        (self.major, self.minor, self.patch) >= (major, minor, patch)
    }
}

fn parse_layout_settings(
    element: &XmlElement,
    default: &LayoutSettings,
    version: LayoutVersion,
) -> LayoutSettings {
    let mut settings = default.clone();
    let value = |name: &str| child_text(element, name);
    settings.text_color = parse_color(value("TextColor").as_deref(), settings.text_color);
    settings.background_color = parse_color(
        value("BackgroundColor").as_deref(),
        settings.background_color,
    );
    if version.at_least(1, 3, 0) {
        settings.background_color2 = parse_color(
            value("BackgroundColor2").as_deref(),
            settings.background_color2,
        );
    } else {
        // Pre-1.3 layouts had a single background color.  LiveSplit keeps it
        // in both fields unless it was black, in which case both become
        // transparent (a black background meant "no background" back then).
        if settings.background_color == egui::Color32::BLACK {
            settings.background_color = egui::Color32::TRANSPARENT;
            settings.background_color2 = egui::Color32::TRANSPARENT;
        } else {
            settings.background_color2 = settings.background_color;
        }
    }
    settings.thin_separators_color = parse_color(
        value("ThinSeparatorsColor").as_deref(),
        settings.thin_separators_color,
    );
    settings.separators_color = parse_color(
        value("SeparatorsColor").as_deref(),
        settings.separators_color,
    );
    settings.personal_best_color = parse_color(
        value("PersonalBestColor").as_deref(),
        settings.personal_best_color,
    );
    settings.ahead_gaining_time_color = parse_color(
        value("AheadGainingTimeColor").as_deref(),
        settings.ahead_gaining_time_color,
    );
    settings.ahead_losing_time_color = parse_color(
        value("AheadLosingTimeColor").as_deref(),
        settings.ahead_losing_time_color,
    );
    settings.behind_gaining_time_color = parse_color(
        value("BehindGainingTimeColor").as_deref(),
        settings.behind_gaining_time_color,
    );
    settings.behind_losing_time_color = parse_color(
        value("BehindLosingTimeColor").as_deref(),
        settings.behind_losing_time_color,
    );
    settings.best_segment_color = parse_color(
        value("BestSegmentColor").as_deref(),
        settings.best_segment_color,
    );
    settings.not_running_color = parse_color(
        value("NotRunningColor").as_deref(),
        settings.not_running_color,
    );
    settings.paused_color = parse_color(value("PausedColor").as_deref(), settings.paused_color);
    settings.text_outline_color = parse_color(
        value("TextOutlineColor").as_deref(),
        settings.text_outline_color,
    );
    settings.shadows_color = parse_color(
        value("ShadowsColor").as_deref(),
        settings.shadows_color,
    );
    settings.background_gradient =
        parse_background_gradient(element, version, settings.background_gradient);
    settings.background_image = parse_background_image(element)
        .or_else(|| settings.background_image.clone());
    settings.image_opacity = parse_f32(value("ImageOpacity").as_deref(), settings.image_opacity);
    settings.image_blur = parse_f32(value("ImageBlur").as_deref(), settings.image_blur);
    if let Some(font) = child(element, "TimerFont") {
        settings.timer_font = parse_font(font, &settings.timer_font);
    }
    // `MainFont` / `SplitNamesFont` are the pre-1.3 names.  Accept them for
    // all versions, preferring the newer names when both are present.
    if let Some(font) = child(element, "TimesFont").or_else(|| child(element, "MainFont")) {
        settings.times_font = parse_font(font, &settings.times_font);
    }
    if let Some(font) = child(element, "TextFont").or_else(|| child(element, "SplitNamesFont")) {
        settings.text_font = parse_font(font, &settings.text_font);
    }
    settings.show_best_segments =
        parse_bool(value("ShowBestSegments").as_deref(), settings.show_best_segments);
    settings.always_on_top =
        parse_bool(value("AlwaysOnTop").as_deref(), settings.always_on_top);
    settings.anti_aliasing =
        parse_bool(value("AntiAliasing").as_deref(), settings.anti_aliasing);
    settings.drop_shadows = parse_bool(value("DropShadows").as_deref(), settings.drop_shadows);
    settings.use_rainbow_color = parse_bool(
        value("UseRainbowColor").as_deref(),
        settings.use_rainbow_color,
    );
    settings.opacity = parse_f32(value("Opacity").as_deref(), settings.opacity);
    settings.mouse_pass_through_while_running = parse_bool(
        value("MousePassThroughWhileRunning").as_deref(),
        settings.mouse_pass_through_while_running,
    );
    settings.allow_resizing = parse_bool(
        value("AllowResizing").as_deref(),
        settings.allow_resizing,
    );
    settings.allow_moving =
        parse_bool(value("AllowMoving").as_deref(), settings.allow_moving);
    settings
}

fn parse_background_gradient(
    element: &XmlElement,
    version: LayoutVersion,
    default: GradientType,
) -> GradientType {
    if version.at_least(1, 6, 1) {
        if let Some(value) = child_text(element, "BackgroundType") {
            return parse_background_type(&value, default);
        }
        if let Some(value) = child_text(element, "BackgroundGradient") {
            return parse_legacy_gradient(&value, default);
        }
    } else {
        if let Some(value) = child_text(element, "BackgroundGradient") {
            return parse_legacy_gradient(&value, default);
        }
        if let Some(value) = child_text(element, "BackgroundType") {
            return parse_background_type(&value, default);
        }
    }
    default
}

/// Modern `BackgroundType` enum.
fn parse_background_type(value: &str, default: GradientType) -> GradientType {
    match value.trim().to_ascii_lowercase().as_str() {
        "solidcolor" => GradientType::Plain,
        "verticalgradient" => GradientType::Vertical,
        "horizontalgradient" => GradientType::Horizontal,
        // `GradientType` has no image variant; the embedded background image
        // is tracked separately in `LayoutSettings::background_image`.
        "image" => GradientType::Plain,
        _ => default,
    }
}

/// Legacy `BackgroundGradient` was a three-value switch in LiveSplit:
/// `Plain`, `Vertical`, and everything else became a horizontal gradient.
fn parse_legacy_gradient(value: &str, default: GradientType) -> GradientType {
    match value.trim().to_ascii_lowercase().as_str() {
        "" => default,
        "plain" => GradientType::Plain,
        "vertical" => GradientType::Vertical,
        _ => GradientType::Horizontal,
    }
}

fn parse_background_image(element: &XmlElement) -> Option<PathBuf> {
    if let Some(raw) = child_text(element, "BackgroundImage").filter(|raw| !raw.is_empty()) {
        return Some(PathBuf::from(format!("{EMBEDDED_IMAGE_PREFIX}{raw}")));
    }
    child_text(element, "BackgroundImagePath")
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
}

fn parse_font_overrides(element: &XmlElement, overrides: &mut FontOverrides) {
    overrides.override_timer_font = parse_bool(
        child_text(element, "OverrideTimerFont").as_deref(),
        false,
    );
    if overrides.override_timer_font {
        if let Some(font) = child(element, "TimerFont") {
            overrides.timer_font = parse_font(font, &FontSpec::timer());
        }
    }
    overrides.override_times_font = parse_bool(
        child_text(element, "OverrideTimesFont").as_deref(),
        false,
    );
    if overrides.override_times_font {
        if let Some(font) = child(element, "TimesFont") {
            overrides.times_font = parse_font(font, &FontSpec::times());
        }
    }
    overrides.override_text_font = parse_bool(
        child_text(element, "OverrideTextFont").as_deref(),
        false,
    );
    if overrides.override_text_font {
        if let Some(font) = child(element, "TextFont") {
            overrides.text_font = parse_font(font, &FontSpec::text());
        }
    }
}

/// Parse either a LiveSplit .NET Font blob (base64) or RustSplit's simple
/// `Name|Size|Bold|Italic` format.
pub fn parse_font(element: &XmlElement, default: &FontSpec) -> FontSpec {
    let text = text_trimmed(element);
    if text.is_empty() {
        return default.clone();
    }
    if let Some(font) = parse_dotnet_font(&text) {
        return font;
    }
    if let Some((name, size, bold, italic)) = parse_simple_font_fields(&text) {
        return FontSpec {
            name,
            size,
            bold,
            italic,
            raw: Some(text),
        };
    }
    if looks_like_dotnet_font(&text) {
        // A BinaryFormatter blob we could not decode.  Keep it untouched so
        // an unmodified save cannot corrupt the user's font.
        let mut font = default.clone();
        font.raw = Some(text);
        return font;
    }
    FontSpec {
        name: text,
        ..default.clone()
    }
}

fn parse_simple_font_fields(text: &str) -> Option<(String, f32, bool, bool)> {
    let mut parts = text.split('|');
    let name = parts.next()?.trim().to_string();
    let size = parts.next()?.trim().parse::<f32>().ok()?;
    let bold = parse_bool(Some(parts.next()?.trim()), false);
    let italic = parse_bool(Some(parts.next()?.trim()), false);
    if parts.next().is_some() || name.is_empty() || !size.is_finite() || size <= 0.0 {
        return None;
    }
    Some((name, size, bold, italic))
}

fn looks_like_dotnet_font(text: &str) -> bool {
    let Some(bytes) = decode_base64(text) else {
        return false;
    };
    bytes.starts_with(&[0x00, 0x01, 0x00, 0x00, 0x00])
        && contains_bytes(&bytes, b"System.Drawing.Font")
}

/// Decode base64, ignoring whitespace the way `Convert.FromBase64String` does.
fn decode_base64(text: &str) -> Option<Vec<u8>> {
    let text = text.trim();
    if text.chars().any(char::is_whitespace) {
        let compact: String = text.chars().filter(|ch| !ch.is_whitespace()).collect();
        base64::engine::general_purpose::STANDARD
            .decode(compact)
            .ok()
    } else {
        base64::engine::general_purpose::STANDARD.decode(text).ok()
    }
}

/// Parse a LiveSplit .NET `BinaryFormatter` font blob (base64).
pub fn parse_dotnet_font(base64_text: &str) -> Option<FontSpec> {
    parse_dotnet_font_data(base64_text).map(|parsed| parsed.spec)
}

struct ParsedDotNetFont {
    spec: FontSpec,
    bytes: Vec<u8>,
    layout: DotNetFontLayout,
    unit: i32,
}

#[derive(Clone, Copy, Debug)]
struct DotNetFontLayout {
    /// Offset of the 7-bit encoded name length.
    name_len_offset: usize,
    name_offset: usize,
    name_len: usize,
    /// Offset of the little-endian f32 size.
    size_offset: usize,
    /// Offset of the little-endian i32 FontStyle enum value.
    style_value_offset: usize,
    /// Offset of the little-endian i32 GraphicsUnit enum value.
    unit_value_offset: usize,
}

const DOTNET_STYLE_REF: [u8; 5] = [0x05, 0xfc, 0xff, 0xff, 0xff];
const DOTNET_UNIT_REF: [u8; 5] = [0x05, 0xfb, 0xff, 0xff, 0xff];

fn parse_dotnet_font_data(base64_text: &str) -> Option<ParsedDotNetFont> {
    let text = base64_text.trim();
    if text.is_empty() {
        return None;
    }
    let bytes = decode_base64(text)?;
    let layout = locate_dotnet_font(&bytes)?;
    let name = std::str::from_utf8(
        bytes.get(layout.name_offset..layout.name_offset + layout.name_len)?,
    )
    .ok()?
    .to_string();
    let size = read_f32(&bytes, layout.size_offset)?;
    let style = read_i32(&bytes, layout.style_value_offset)?;
    let unit = read_i32(&bytes, layout.unit_value_offset)?;
    Some(ParsedDotNetFont {
        spec: FontSpec {
            name,
            size,
            bold: style & 1 != 0,
            italic: style & 2 != 0,
            raw: Some(text.to_string()),
        },
        bytes,
        layout,
        unit,
    })
}

/// Locate the `System.Drawing.Font` payload inside a BinaryFormatter stream.
///
/// A serialized font contains a string record (`0x06`, object id, 7-bit length,
/// UTF-8 bytes) for `Name`, immediately followed by the `Size` float.  The
/// `Style` and `Unit` fields are member references to separately serialized
/// enum objects.  We scan for the string record and validate the surrounding
/// structure instead of relying on fixed offsets, because the assembly name
/// (and therefore the header length) can vary between .NET versions.
fn locate_dotnet_font(bytes: &[u8]) -> Option<DotNetFontLayout> {
    if !bytes.starts_with(&[0x00, 0x01, 0x00, 0x00, 0x00])
        || !contains_bytes(bytes, b"System.Drawing.Font")
    {
        return None;
    }
    for index in 0..bytes.len() {
        let Some((name_len, len_bytes)) = read_7bit_encoded(bytes, index) else {
            continue;
        };
        if name_len == 0 || name_len > 4096 {
            continue;
        }
        let name_offset = index + len_bytes;
        let Some(size_offset) = name_offset.checked_add(name_len) else {
            continue;
        };
        if size_offset + 4 + DOTNET_STYLE_REF.len() > bytes.len() {
            continue;
        }
        // The string record marker sits five bytes before the length:
        // 0x06 followed by the four-byte object id.
        if index < 5 || bytes[index - 5] != 0x06 {
            continue;
        }
        let Ok(name) = std::str::from_utf8(&bytes[name_offset..size_offset]) else {
            continue;
        };
        if !valid_font_name(name) {
            continue;
        }
        let metadata_start = index.saturating_sub(512);
        let metadata = &bytes[metadata_start..index];
        if !contains_bytes(metadata, b"System.Drawing.Font")
            || !contains_bytes(metadata, b"Name")
            || !contains_bytes(metadata, b"Size")
        {
            continue;
        }
        let Some(size) = read_f32(bytes, size_offset) else {
            continue;
        };
        if !size.is_finite() || !(1.0..=1000.0).contains(&size) {
            continue;
        }
        let style_ref_offset = size_offset + 4;
        let (style_value_offset, unit_value_offset, style, unit);
        if bytes.get(style_ref_offset..style_ref_offset + DOTNET_STYLE_REF.len())
            == Some(DOTNET_STYLE_REF.as_slice())
        {
            let after_style_ref = style_ref_offset + DOTNET_STYLE_REF.len();
            let Some(unit_rel) = find_bytes(bytes.get(after_style_ref..)?, &DOTNET_UNIT_REF) else {
                continue;
            };
            let unit_ref_offset = after_style_ref + unit_rel;
            let Some(value_offset) = unit_ref_offset.checked_sub(4) else {
                continue;
            };
            if !contains_bytes(&bytes[style_ref_offset..value_offset], b"FontStyle") {
                continue;
            }
            if *bytes.last()? != 0x0b {
                continue;
            }
            let Some(unit_offset) = bytes.len().checked_sub(5) else {
                continue;
            };
            if unit_offset < unit_ref_offset + DOTNET_UNIT_REF.len()
                || !contains_bytes(&bytes[unit_ref_offset..unit_offset], b"GraphicsUnit")
            {
                continue;
            }
            style_value_offset = value_offset;
            unit_value_offset = unit_offset;
            let Some(parsed_style) = read_i32(bytes, style_value_offset) else {
                continue;
            };
            let Some(parsed_unit) = read_i32(bytes, unit_value_offset) else {
                continue;
            };
            style = parsed_style;
            unit = parsed_unit;
        } else {
            // Some serializers write the enum values inline instead of using
            // BinaryFormatter member references.
            let Some(inline_style) = read_i32(bytes, style_ref_offset) else {
                continue;
            };
            let inline_unit_offset = style_ref_offset + 4;
            let Some(inline_unit) = read_i32(bytes, inline_unit_offset) else {
                continue;
            };
            style_value_offset = style_ref_offset;
            unit_value_offset = inline_unit_offset;
            style = inline_style;
            unit = inline_unit;
        }
        if !(0..=15).contains(&style) || !(0..=6).contains(&unit) {
            continue;
        }
        return Some(DotNetFontLayout {
            name_len_offset: index,
            name_offset,
            name_len,
            size_offset,
            style_value_offset,
            unit_value_offset,
        });
    }
    None
}

fn valid_font_name(name: &str) -> bool {
    !name.is_empty()
        && !name.contains("System.")
        && !matches!(name, "Name" | "Size" | "Style" | "Unit" | "value__")
        && name.chars().all(|ch| !ch.is_control())
}

fn read_7bit_encoded(bytes: &[u8], start: usize) -> Option<(usize, usize)> {
    let mut value = 0usize;
    let mut shift = 0u32;
    let mut index = start;
    loop {
        let byte = *bytes.get(index)?;
        value |= ((byte & 0x7f) as usize) << shift;
        index += 1;
        if byte & 0x80 == 0 {
            return Some((value, index - start));
        }
        shift += 7;
        if shift > 28 {
            return None;
        }
    }
}

fn write_7bit_encoded(value: usize, output: &mut Vec<u8>) {
    let mut value = value;
    loop {
        let mut byte = (value & 0x7f) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        output.push(byte);
        if value == 0 {
            break;
        }
    }
}

fn read_f32(bytes: &[u8], offset: usize) -> Option<f32> {
    Some(f32::from_le_bytes(
        bytes.get(offset..offset + 4)?.try_into().ok()?,
    ))
}

fn read_i32(bytes: &[u8], offset: usize) -> Option<i32> {
    Some(i32::from_le_bytes(
        bytes.get(offset..offset + 4)?.try_into().ok()?,
    ))
}

fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    find_bytes(haystack, needle).is_some()
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// A known-good `System.Drawing.Font` blob used as a template when a font has
/// no parsed raw blob (for example a programmatically created default font).
/// It is the stock Segoe UI 16 pt/px regular font from LiveSplit's layout.
const DOTNET_FONT_TEMPLATE: &str = "AAEAAAD/////AQAAAAAAAAAMAgAAAFFTeXN0ZW0uRHJhd2luZywgVmVyc2lvbj00LjAuMC4wLCBDdWx0dXJlPW5ldXRyYWwsIFB1YmxpY0tleVRva2VuPWIwM2Y1ZjdmMTFkNTBhM2EFAQAAABNTeXN0ZW0uRHJhd2luZy5Gb250BAAAAAROYW1lBFNpemUFU3R5bGUEVW5pdAEABAQLGFN5c3RlbS5EcmF3aW5nLkZvbnRTdHlsZQIAAAAbU3lzdGVtLkRyYXdpbmcuR3JhcGhpY3NVbml0AgAAAAIAAAAGAwAAAAhTZWdvZSBVSQAAgEEF/P///xhTeXN0ZW0uRHJhd2luZy5Gb250U3R5bGUBAAAAB3ZhbHVlX18ACAIAAAAAAAAABfv///8bU3lzdGVtLkRyYXdpbmcuR3JhcGhpY3NVbml0AQAAAAd2YWx1ZV9fAAgCAAAAAgAAAAs=";

enum RawFont {
    DotNet(ParsedDotNetFont),
    Simple(FontSpec),
    Unknown,
}

fn classify_raw_font(raw: &str) -> RawFont {
    if let Some(parsed) = parse_dotnet_font_data(raw) {
        return RawFont::DotNet(parsed);
    }
    if let Some((name, size, bold, italic)) = parse_simple_font_fields(raw) {
        return RawFont::Simple(FontSpec {
            name,
            size,
            bold,
            italic,
            raw: Some(raw.to_string()),
        });
    }
    RawFont::Unknown
}

fn font_fields_match(font: &FontSpec, other: &FontSpec) -> bool {
    font.name == other.name
        && font.size == other.size
        && font.bold == other.bold
        && font.italic == other.italic
}

fn simple_font_text(font: &FontSpec) -> String {
    format!(
        "{}|{}|{}|{}",
        font.name,
        font.size,
        bool_text(font.bold),
        bool_text(font.italic)
    )
}

fn font_payload(font: &FontSpec) -> String {
    if let Some(raw) = &font.raw {
        match classify_raw_font(raw) {
            RawFont::DotNet(parsed) => {
                if font_fields_match(font, &parsed.spec) {
                    return raw.clone();
                }
                if let Some(encoded) = encode_dotnet_font_like(&parsed, font) {
                    return encoded;
                }
                return simple_font_text(font);
            }
            RawFont::Simple(parsed) => {
                if font_fields_match(font, &parsed) {
                    return raw.clone();
                }
                if let Some(encoded) = encode_dotnet_font(font) {
                    return encoded;
                }
                return simple_font_text(font);
            }
            // Unparseable blobs are preserved verbatim.  We cannot tell
            // whether they were edited, and corrupting them would be worse
            // than not persisting a font edit.
            RawFont::Unknown => return raw.clone(),
        }
    }
    encode_dotnet_font(font).unwrap_or_else(|| simple_font_text(font))
}

fn encode_dotnet_font(font: &FontSpec) -> Option<String> {
    let bytes = decode_base64(DOTNET_FONT_TEMPLATE)?;
    let layout = locate_dotnet_font(&bytes)?;
    let parsed = ParsedDotNetFont {
        spec: FontSpec::text(),
        bytes,
        layout,
        // LiveSplit's stock fonts use GraphicsUnit.Pixel (2).
        unit: 2,
    };
    encode_dotnet_font_like(&parsed, font)
}

/// Patch a valid LiveSplit font blob with new Name/Size/Style/Unit values.
///
/// Only the variable-length name and the four fixed-width scalar fields are
/// rewritten; all BinaryFormatter metadata (assembly names, object ids, enum
/// class records) is copied from the template byte-for-byte.
fn encode_dotnet_font_like(parsed: &ParsedDotNetFont, font: &FontSpec) -> Option<String> {
    if font.name.is_empty()
        || !font.size.is_finite()
        || font.size <= 0.0
        || font.name.len() > 4096
    {
        return None;
    }
    let bytes = &parsed.bytes;
    let layout = parsed.layout;
    // Keep additional FontStyle bits (underline/strikeout) that `FontSpec`
    // does not model; only bold and italic are owned by RustSplit.
    let mut style = read_i32(bytes, layout.style_value_offset)? & !3;
    if font.bold {
        style |= 1;
    }
    if font.italic {
        style |= 2;
    }
    let mut output = Vec::with_capacity(bytes.len() + font.name.len());
    output.extend_from_slice(bytes.get(..layout.name_len_offset)?);
    write_7bit_encoded(font.name.len(), &mut output);
    output.extend_from_slice(font.name.as_bytes());
    output.extend_from_slice(&font.size.to_le_bytes());
    output.extend_from_slice(bytes.get(layout.size_offset + 4..layout.style_value_offset)?);
    output.extend_from_slice(&style.to_le_bytes());
    output.extend_from_slice(
        bytes.get(layout.style_value_offset + 4..layout.unit_value_offset)?,
    );
    output.extend_from_slice(&parsed.unit.to_le_bytes());
    output.extend_from_slice(bytes.get(layout.unit_value_offset + 4..)?);
    Some(base64::engine::general_purpose::STANDARD.encode(output))
}

pub fn layout_to_string(layout: &Layout) -> Result<String, LayoutXmlError> {
    let mut root = XmlElement::new("Layout");
    root.attributes
        .insert("version".to_string(), "1.6.1".to_string());
    add_text(&mut root, "Mode", layout.mode.label());
    add_text(&mut root, "X", layout.x.to_string());
    add_text(&mut root, "Y", layout.y.to_string());
    add_text(
        &mut root,
        "VerticalWidth",
        layout.vertical_width.to_string(),
    );
    add_text(
        &mut root,
        "VerticalHeight",
        layout.vertical_height.to_string(),
    );
    add_text(
        &mut root,
        "HorizontalWidth",
        layout.horizontal_width.to_string(),
    );
    add_text(
        &mut root,
        "HorizontalHeight",
        layout.horizontal_height.to_string(),
    );
    root.children
        .push(XMLNode::Element(settings_to_xml(&layout.settings)));

    let mut components = XmlElement::new("Components");
    for component in &layout.components {
        let mut element = XmlElement::new("Component");
        add_text(&mut element, "Path", component.path.clone());
        let mut settings = component.component.to_xml();
        settings.name = "Settings".to_string();
        element.children.push(XMLNode::Element(settings));
        if component.font_overrides.has_overrides() {
            element
                .children
                .push(XMLNode::Element(font_overrides_to_xml(
                    &component.font_overrides,
                )));
        }
        components.children.push(XMLNode::Element(element));
    }
    root.children.push(XMLNode::Element(components));
    Ok(xml_to_string(&root)?)
}

fn settings_to_xml(settings: &LayoutSettings) -> XmlElement {
    let mut element = XmlElement::new("Settings");
    add_color(&mut element, "TextColor", settings.text_color);
    add_color(&mut element, "BackgroundColor", settings.background_color);
    add_color(&mut element, "BackgroundColor2", settings.background_color2);
    add_color(
        &mut element,
        "ThinSeparatorsColor",
        settings.thin_separators_color,
    );
    add_color(&mut element, "SeparatorsColor", settings.separators_color);
    add_color(&mut element, "PersonalBestColor", settings.personal_best_color);
    add_color(
        &mut element,
        "AheadGainingTimeColor",
        settings.ahead_gaining_time_color,
    );
    add_color(
        &mut element,
        "AheadLosingTimeColor",
        settings.ahead_losing_time_color,
    );
    add_color(
        &mut element,
        "BehindGainingTimeColor",
        settings.behind_gaining_time_color,
    );
    add_color(
        &mut element,
        "BehindLosingTimeColor",
        settings.behind_losing_time_color,
    );
    add_color(&mut element, "BestSegmentColor", settings.best_segment_color);
    add_bool(&mut element, "UseRainbowColor", settings.use_rainbow_color);
    add_color(&mut element, "NotRunningColor", settings.not_running_color);
    add_color(&mut element, "PausedColor", settings.paused_color);
    add_color(&mut element, "TextOutlineColor", settings.text_outline_color);
    add_color(&mut element, "ShadowsColor", settings.shadows_color);
    element
        .children
        .push(XMLNode::Element(font_to_xml("TimesFont", &settings.times_font)));
    element
        .children
        .push(XMLNode::Element(font_to_xml("TimerFont", &settings.timer_font)));
    element
        .children
        .push(XMLNode::Element(font_to_xml("TextFont", &settings.text_font)));
    add_bool(&mut element, "AlwaysOnTop", settings.always_on_top);
    add_bool(
        &mut element,
        "ShowBestSegments",
        settings.show_best_segments,
    );
    add_bool(&mut element, "AntiAliasing", settings.anti_aliasing);
    add_bool(&mut element, "DropShadows", settings.drop_shadows);
    add_text(
        &mut element,
        "BackgroundType",
        background_type_label(settings),
    );
    match embedded_background_image(settings) {
        Some(raw) => element
            .children
            .push(XMLNode::Element(cdata_element(
                "BackgroundImage",
                raw.to_string(),
            ))),
        None => add_text(&mut element, "BackgroundImage", ""),
    }
    if embedded_background_image(settings).is_none() {
        if let Some(path) = &settings.background_image {
            // RustSplit extension for ordinary filesystem paths.  LiveSplit
            // ignores unknown settings elements.
            add_text(
                &mut element,
                "BackgroundImagePath",
                path.display().to_string(),
            );
        }
    }
    add_text(&mut element, "ImageOpacity", settings.image_opacity.to_string());
    add_text(&mut element, "ImageBlur", settings.image_blur.to_string());
    add_text(&mut element, "Opacity", settings.opacity.to_string());
    add_bool(
        &mut element,
        "MousePassThroughWhileRunning",
        settings.mouse_pass_through_while_running,
    );
    add_bool(&mut element, "AllowResizing", settings.allow_resizing);
    add_bool(&mut element, "AllowMoving", settings.allow_moving);
    element
}

fn background_type_label(settings: &LayoutSettings) -> &'static str {
    if embedded_background_image(settings).is_some() {
        return "Image";
    }
    match settings.background_gradient {
        GradientType::Plain | GradientType::PlainWithDeltaColor | GradientType::Alternating => {
            "SolidColor"
        }
        GradientType::Vertical | GradientType::VerticalWithDeltaColor => "VerticalGradient",
        GradientType::Horizontal | GradientType::HorizontalWithDeltaColor => "HorizontalGradient",
    }
}

fn embedded_background_image(settings: &LayoutSettings) -> Option<&str> {
    settings
        .background_image
        .as_ref()?
        .to_str()?
        .strip_prefix(EMBEDDED_IMAGE_PREFIX)
        .filter(|raw| !raw.is_empty())
}

fn font_overrides_to_xml(overrides: &FontOverrides) -> XmlElement {
    let mut element = XmlElement::new("FontOverrides");
    add_bool(
        &mut element,
        "OverrideTimerFont",
        overrides.override_timer_font,
    );
    if overrides.override_timer_font {
        element
            .children
            .push(XMLNode::Element(font_to_xml("TimerFont", &overrides.timer_font)));
    }
    add_bool(
        &mut element,
        "OverrideTimesFont",
        overrides.override_times_font,
    );
    if overrides.override_times_font {
        element
            .children
            .push(XMLNode::Element(font_to_xml("TimesFont", &overrides.times_font)));
    }
    add_bool(
        &mut element,
        "OverrideTextFont",
        overrides.override_text_font,
    );
    if overrides.override_text_font {
        element
            .children
            .push(XMLNode::Element(font_to_xml("TextFont", &overrides.text_font)));
    }
    element
}

pub fn font_to_xml(name: &str, font: &FontSpec) -> XmlElement {
    cdata_element(name, font_payload(font))
}

fn cdata_element(name: &str, value: String) -> XmlElement {
    let mut element = XmlElement::new(name);
    element.children.push(XMLNode::CData(value));
    element
}

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

fn add_color(parent: &mut XmlElement, name: &str, color: egui::Color32) {
    add_text(parent, name, color_to_livesplit_hex(color));
}

/// Serialize an ARGB color as `AARRGGBB`.
///
/// `Color32` stores premultiplied components, so use the unmultiplied view;
/// otherwise translucent colors such as `03FFFFFF` would be written as
/// `03030303`.
fn color_to_livesplit_hex(color: egui::Color32) -> String {
    let [r, g, b, a] = color.to_srgba_unmultiplied();
    format!("{a:02X}{r:02X}{g:02X}{b:02X}")
}
