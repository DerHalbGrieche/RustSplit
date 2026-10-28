//! Integration tests for LiveSplit `.lsl` layout parsing and saving.

use livesplit_ui::components::Component;
use livesplit_ui::layout::{FontSpec, GradientType, Layout, LayoutMode, LayoutSettings};
use livesplit_ui::layout_xml::{
    font_to_xml, layout_to_string, parse_dotnet_font, parse_layout, LayoutXmlError,
    EMBEDDED_IMAGE_PREFIX,
};
use livesplit_ui::xml::{child, parse_xml, text_trimmed};

fn default_layout() -> Layout {
    Layout::parse_default().expect("embedded default layout must parse")
}

fn roundtrip(layout: &Layout) -> Layout {
    let xml = layout_to_string(layout).expect("layout must serialize");
    parse_layout(xml.as_bytes()).expect("serialized layout must parse")
}

fn assert_font_fields_eq(actual: &FontSpec, expected: &FontSpec) {
    assert_eq!(actual.name, expected.name);
    assert_eq!(actual.size, expected.size);
    assert_eq!(actual.bold, expected.bold);
    assert_eq!(actual.italic, expected.italic);
}

fn assert_font_raw_eq(actual: &FontSpec, expected: &FontSpec) {
    assert_eq!(actual.raw, expected.raw);
}

fn assert_settings_fields_eq(actual: &LayoutSettings, expected: &LayoutSettings) {
    assert_eq!(actual.text_color, expected.text_color, "TextColor");
    assert_eq!(
        actual.background_color, expected.background_color,
        "BackgroundColor"
    );
    assert_eq!(
        actual.background_color2, expected.background_color2,
        "BackgroundColor2"
    );
    assert_eq!(
        actual.thin_separators_color, expected.thin_separators_color,
        "ThinSeparatorsColor"
    );
    assert_eq!(
        actual.separators_color, expected.separators_color,
        "SeparatorsColor"
    );
    assert_eq!(
        actual.personal_best_color, expected.personal_best_color,
        "PersonalBestColor"
    );
    assert_eq!(
        actual.ahead_gaining_time_color, expected.ahead_gaining_time_color,
        "AheadGainingTimeColor"
    );
    assert_eq!(
        actual.ahead_losing_time_color, expected.ahead_losing_time_color,
        "AheadLosingTimeColor"
    );
    assert_eq!(
        actual.behind_gaining_time_color, expected.behind_gaining_time_color,
        "BehindGainingTimeColor"
    );
    assert_eq!(
        actual.behind_losing_time_color, expected.behind_losing_time_color,
        "BehindLosingTimeColor"
    );
    assert_eq!(
        actual.best_segment_color, expected.best_segment_color,
        "BestSegmentColor"
    );
    assert_eq!(
        actual.not_running_color, expected.not_running_color,
        "NotRunningColor"
    );
    assert_eq!(actual.paused_color, expected.paused_color, "PausedColor");
    assert_eq!(
        actual.text_outline_color, expected.text_outline_color,
        "TextOutlineColor"
    );
    assert_eq!(
        actual.shadows_color, expected.shadows_color,
        "ShadowsColor"
    );
    assert_eq!(
        actual.background_gradient, expected.background_gradient,
        "BackgroundType/BackgroundGradient"
    );
    assert_eq!(
        actual.background_image, expected.background_image,
        "BackgroundImage"
    );
    assert_eq!(
        actual.image_opacity, expected.image_opacity,
        "ImageOpacity"
    );
    assert_eq!(actual.image_blur, expected.image_blur, "ImageBlur");
    assert_font_fields_eq(&actual.timer_font, &expected.timer_font);
    assert_font_fields_eq(&actual.times_font, &expected.times_font);
    assert_font_fields_eq(&actual.text_font, &expected.text_font);
    assert_eq!(
        actual.show_best_segments, expected.show_best_segments,
        "ShowBestSegments"
    );
    assert_eq!(
        actual.always_on_top, expected.always_on_top,
        "AlwaysOnTop"
    );
    assert_eq!(
        actual.anti_aliasing, expected.anti_aliasing,
        "AntiAliasing"
    );
    assert_eq!(
        actual.drop_shadows, expected.drop_shadows,
        "DropShadows"
    );
    assert_eq!(
        actual.use_rainbow_color, expected.use_rainbow_color,
        "UseRainbowColor"
    );
    assert_eq!(actual.opacity, expected.opacity, "Opacity");
    assert_eq!(
        actual.mouse_pass_through_while_running, expected.mouse_pass_through_while_running,
        "MousePassThroughWhileRunning"
    );
    assert_eq!(
        actual.allow_resizing, expected.allow_resizing,
        "AllowResizing"
    );
    assert_eq!(actual.allow_moving, expected.allow_moving, "AllowMoving");
}

#[test]
fn default_layout_parses_exact_components_and_settings() {
    let layout = default_layout();

    assert_eq!(layout.mode, LayoutMode::Vertical);
    assert_eq!(layout.x, 1022);
    assert_eq!(layout.y, 265);
    assert_eq!(layout.vertical_width, 286);
    assert_eq!(layout.vertical_height, 519);
    assert_eq!(layout.horizontal_width, -1);
    assert_eq!(layout.horizontal_height, -1);

    let paths: Vec<&str> = layout
        .components
        .iter()
        .map(|component| component.path.as_str())
        .collect();
    assert_eq!(
        paths,
        [
            "LiveSplit.Title.dll",
            "LiveSplit.Splits.dll",
            "LiveSplit.Timer.dll",
            "LiveSplit.PreviousSegment.dll",
        ]
    );
    assert!(matches!(
        &layout.components[0].component,
        Component::Title(_)
    ));
    assert!(matches!(
        &layout.components[1].component,
        Component::Splits(_)
    ));
    assert!(matches!(
        &layout.components[2].component,
        Component::Timer(_)
    ));
    assert!(matches!(
        &layout.components[3].component,
        Component::PreviousSegment(_)
    ));

    assert_settings_fields_eq(&layout.settings, &LayoutSettings::livesplit_default());

    // The three stock fonts are stored as BinaryFormatter blobs.
    assert_eq!(layout.settings.timer_font.name, "Century Gothic");
    assert_eq!(layout.settings.timer_font.size, 43.75);
    assert!(layout.settings.timer_font.bold);
    assert!(!layout.settings.timer_font.italic);

    assert_eq!(layout.settings.times_font.name, "Segoe UI");
    assert_eq!(layout.settings.times_font.size, 16.0);
    assert!(layout.settings.times_font.bold);
    assert!(!layout.settings.times_font.italic);

    assert_eq!(layout.settings.text_font.name, "Segoe UI");
    assert_eq!(layout.settings.text_font.size, 16.0);
    assert!(!layout.settings.text_font.bold);
    assert!(!layout.settings.text_font.italic);

    assert!(layout.settings.timer_font.raw.is_some());
    assert!(layout.settings.times_font.raw.is_some());
    assert!(layout.settings.text_font.raw.is_some());
}

#[test]
fn sample_font_blobs_parse() {
    let layout = default_layout();
    let cases = [
        (&layout.settings.text_font, "Segoe UI", 16.0, false, false),
        (&layout.settings.times_font, "Segoe UI", 16.0, true, false),
        (
            &layout.settings.timer_font,
            "Century Gothic",
            43.75,
            true,
            false,
        ),
    ];
    for (font, name, size, bold, italic) in cases {
        let raw = font.raw.as_ref().expect("sample font has a raw blob");
        let parsed = parse_dotnet_font(raw).expect("sample font blob must parse");
        assert_eq!(parsed.name, name);
        assert_eq!(parsed.size, size);
        assert_eq!(parsed.bold, bold);
        assert_eq!(parsed.italic, italic);
        assert_eq!(parsed.raw.as_deref(), Some(raw.as_str()));
    }
}

#[test]
fn save_roundtrip_preserves_layout_and_raw_font_blobs() {
    let layout = default_layout();
    let raw_fonts = [
        layout.settings.timer_font.raw.clone().unwrap(),
        layout.settings.times_font.raw.clone().unwrap(),
        layout.settings.text_font.raw.clone().unwrap(),
    ];

    let xml = layout_to_string(&layout).expect("layout must serialize");
    assert!(xml.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>"));
    assert!(xml.contains("<Layout version=\"1.6.1\">"));
    assert!(xml.contains("<Mode>Vertical</Mode>"));
    let document = parse_xml(xml.as_bytes()).unwrap();
    let global_settings = child(&document, "Settings").unwrap();
    assert_eq!(
        child(global_settings, "BackgroundType")
            .map(text_trimmed)
            .as_deref(),
        Some("SolidColor")
    );
    assert!(child(global_settings, "BackgroundGradient").is_none());
    assert!(xml.contains("<UseRainbowColor>False</UseRainbowColor>"));
    assert!(xml.contains("<AlwaysOnTop>True</AlwaysOnTop>"));
    // Colors are AARRGGBB with unmultiplied components (LiveSplit's format).
    assert!(xml.contains("<ThinSeparatorsColor>03FFFFFF</ThinSeparatorsColor>"));
    assert!(xml.contains("<SeparatorsColor>24FFFFFF</SeparatorsColor>"));
    assert!(xml.contains("<ShadowsColor>80000000</ShadowsColor>"));
    assert!(xml.contains("<BackgroundColor>FF0F0F0F</BackgroundColor>"));
    // Unchanged fonts must be emitted byte-for-byte.
    for raw in &raw_fonts {
        assert!(xml.contains(raw), "raw font blob was not preserved");
    }

    let reparsed = parse_layout(xml.as_bytes()).expect("saved layout must parse");
    assert_settings_fields_eq(&reparsed.settings, &layout.settings);
    assert_font_raw_eq(&reparsed.settings.timer_font, &layout.settings.timer_font);
    assert_font_raw_eq(&reparsed.settings.times_font, &layout.settings.times_font);
    assert_font_raw_eq(&reparsed.settings.text_font, &layout.settings.text_font);

    assert_eq!(reparsed.mode, layout.mode);
    assert_eq!(reparsed.x, layout.x);
    assert_eq!(reparsed.y, layout.y);
    assert_eq!(reparsed.vertical_width, layout.vertical_width);
    assert_eq!(reparsed.vertical_height, layout.vertical_height);
    assert_eq!(reparsed.horizontal_width, layout.horizontal_width);
    assert_eq!(reparsed.horizontal_height, layout.horizontal_height);
    assert_eq!(reparsed.components.len(), layout.components.len());
    let paths = |layout: &Layout| {
        layout
            .components
            .iter()
            .map(|component| component.path.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(paths(&reparsed), paths(&layout));
}

#[test]
fn changed_font_is_encoded_as_livesplit_blob() {
    let mut layout = default_layout();
    let original_raw = layout.settings.text_font.raw.clone().unwrap();

    for (name, size, bold, italic) in [
        ("Arial", 18.0_f32, true, false),
        ("A Very Long Custom Font Family Name", 9.5, false, true),
        ("S", 7.0, true, true),
    ] {
        layout.settings.text_font = FontSpec {
            name: name.to_string(),
            size,
            bold,
            italic,
            raw: Some(original_raw.clone()),
        };
        let reparsed = roundtrip(&layout);
        let font = &reparsed.settings.text_font;
        assert_eq!(font.name, name);
        assert_eq!(font.size, size);
        assert_eq!(font.bold, bold);
        assert_eq!(font.italic, italic);
        let raw = font.raw.as_ref().expect("changed font must be encoded");
        assert!(raw.starts_with("AAEAAAD"), "not a BinaryFormatter blob");
        assert_ne!(raw, &original_raw);
        let parsed = parse_dotnet_font(raw).expect("encoded font must parse");
        assert_eq!(parsed.name, name);
        assert_eq!(parsed.size, size);
        assert_eq!(parsed.bold, bold);
        assert_eq!(parsed.italic, italic);
    }
}

#[test]
fn long_and_unicode_font_names_roundtrip() {
    let long_name = format!("Font {}", "A".repeat(140));
    for name in [long_name, "Sëgoe UI Ω".to_string()] {
        let font = FontSpec {
            name: name.clone(),
            size: 13.5,
            bold: true,
            italic: false,
            raw: None,
        };
        let encoded = text_trimmed(&font_to_xml("TextFont", &font));
        let parsed = parse_dotnet_font(&encoded).expect("encoded font must parse");
        assert_eq!(parsed.name, name);
        assert_eq!(parsed.size, 13.5);
        assert!(parsed.bold);
        assert!(!parsed.italic);
    }
}

#[test]
fn font_without_raw_and_unparseable_raw_handling() {
    let font = FontSpec {
        name: "Arial".to_string(),
        size: 12.0,
        bold: false,
        italic: true,
        raw: None,
    };
    let element = font_to_xml("TextFont", &font);
    let encoded = text_trimmed(&element);
    assert!(encoded.starts_with("AAEAAAD"));
    let parsed = parse_dotnet_font(&encoded).expect("new font blob must parse");
    assert_eq!(parsed.name, "Arial");
    assert_eq!(parsed.size, 12.0);
    assert!(!parsed.bold);
    assert!(parsed.italic);

    // A blob we cannot understand must never be overwritten.
    let opaque = FontSpec {
        raw: Some("not-a-real-font-blob".to_string()),
        ..FontSpec::text()
    };
    let element = font_to_xml("TextFont", &opaque);
    assert_eq!(text_trimmed(&element), "not-a-real-font-blob");
}

#[test]
fn legacy_layout_with_main_font_and_split_names_font() {
    let defaults = default_layout();
    let times_raw = defaults.settings.times_font.raw.as_ref().unwrap();
    let timer_raw = defaults.settings.timer_font.raw.as_ref().unwrap();
    let text_raw = defaults.settings.text_font.raw.as_ref().unwrap();
    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<Layout version="1.3">
  <Mode>Vertical</Mode>
  <X>10</X>
  <Y>20</Y>
  <VerticalWidth>286</VerticalWidth>
  <VerticalHeight>519</VerticalHeight>
  <HorizontalWidth>-1</HorizontalWidth>
  <HorizontalHeight>-1</HorizontalHeight>
  <Settings>
    <TextColor>FFFFFFFF</TextColor>
    <BackgroundColor>FF101010</BackgroundColor>
    <MainFont><![CDATA[{times_raw}]]></MainFont>
    <TimerFont><![CDATA[{timer_raw}]]></TimerFont>
    <SplitNamesFont><![CDATA[{text_raw}]]></SplitNamesFont>
    <BackgroundGradient>Vertical</BackgroundGradient>
  </Settings>
  <Components>
    <Component>
      <Path>LiveSplit.Title.dll</Path>
      <Settings><Version>1.3</Version></Settings>
    </Component>
  </Components>
</Layout>"#
    );
    let layout = parse_layout(xml.as_bytes()).expect("legacy layout must parse");
    assert_eq!(layout.settings.times_font.name, "Segoe UI");
    assert!(layout.settings.times_font.bold);
    assert_eq!(layout.settings.text_font.name, "Segoe UI");
    assert!(!layout.settings.text_font.bold);
    assert_eq!(layout.settings.timer_font.name, "Century Gothic");
    assert_eq!(
        layout.settings.background_gradient,
        GradientType::Vertical
    );
    assert_eq!(layout.components.len(), 1);
    assert!(matches!(
        &layout.components[0].component,
        Component::Title(_)
    ));

    let reparsed = roundtrip(&layout);
    assert_settings_fields_eq(&reparsed.settings, &layout.settings);
}

#[test]
fn pre_13_black_background_becomes_transparent() {
    let xml = r#"<Layout version="1.2">
  <Settings>
    <BackgroundColor>FF000000</BackgroundColor>
    <BackgroundGradient>Plain</BackgroundGradient>
  </Settings>
</Layout>"#;
    let layout = parse_layout(xml.as_bytes()).expect("1.2 layout must parse");
    assert_eq!(layout.settings.background_color, egui::Color32::TRANSPARENT);
    assert_eq!(layout.settings.background_color2, egui::Color32::TRANSPARENT);
    assert_eq!(layout.settings.background_gradient, GradientType::Plain);

    let xml = r#"<Layout version="1.2">
  <Settings>
    <BackgroundColor>FF112233</BackgroundColor>
  </Settings>
</Layout>"#;
    let layout = parse_layout(xml.as_bytes()).expect("1.2 layout must parse");
    assert_eq!(
        layout.settings.background_color,
        egui::Color32::from_rgb(0x11, 0x22, 0x33)
    );
    assert_eq!(
        layout.settings.background_color2,
        egui::Color32::from_rgb(0x11, 0x22, 0x33)
    );
}

#[test]
fn parses_layout_versions_1_3_through_1_7() {
    let legacy_values = ["Plain", "Vertical", "Horizontal"];
    let expected_legacy = [
        GradientType::Plain,
        GradientType::Vertical,
        GradientType::Horizontal,
    ];
    for version in ["1.3", "1.4", "1.5", "1.6"] {
        for (value, expected) in legacy_values.iter().zip(expected_legacy) {
            let xml = format!(
                "<Layout version=\"{version}\"><Settings><BackgroundGradient>{value}</BackgroundGradient></Settings></Layout>"
            );
            let layout = parse_layout(xml.as_bytes()).expect("legacy gradient must parse");
            assert_eq!(
                layout.settings.background_gradient, expected,
                "version {version} value {value}"
            );
        }
    }

    let modern_values = [
        ("SolidColor", GradientType::Plain),
        ("VerticalGradient", GradientType::Vertical),
        ("HorizontalGradient", GradientType::Horizontal),
        ("Image", GradientType::Plain),
    ];
    for version in ["1.6.1", "1.7"] {
        for (value, expected) in modern_values {
            let xml = format!(
                "<Layout version=\"{version}\"><Settings><BackgroundType>{value}</BackgroundType></Settings></Layout>"
            );
            let layout = parse_layout(xml.as_bytes()).expect("modern gradient must parse");
            assert_eq!(
                layout.settings.background_gradient, expected,
                "version {version} value {value}"
            );
        }
    }

    // The legacy fallback also has to work when a newer file still uses it.
    let xml = r#"<Layout version="1.7"><Settings><BackgroundGradient>Horizontal</BackgroundGradient></Settings></Layout>"#;
    let layout = parse_layout(xml.as_bytes()).unwrap();
    assert_eq!(
        layout.settings.background_gradient,
        GradientType::Horizontal
    );
}

#[test]
fn custom_layout_settings_roundtrip() {
    let defaults = default_layout();
    let times_raw = defaults.settings.times_font.raw.as_ref().unwrap();
    let timer_raw = defaults.settings.timer_font.raw.as_ref().unwrap();
    let text_raw = defaults.settings.text_font.raw.as_ref().unwrap();
    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<Layout version="1.7">
  <Mode>Horizontal</Mode>
  <X>7</X>
  <Y>9</Y>
  <VerticalWidth>300</VerticalWidth>
  <VerticalHeight>500</VerticalHeight>
  <HorizontalWidth>640</HorizontalWidth>
  <HorizontalHeight>480</HorizontalHeight>
  <Settings>
    <TextColor>FF112233</TextColor>
    <BackgroundColor>FF445566</BackgroundColor>
    <BackgroundColor2>00000000</BackgroundColor2>
    <ThinSeparatorsColor>FF778899</ThinSeparatorsColor>
    <SeparatorsColor>FFAABBCC</SeparatorsColor>
    <PersonalBestColor>FFDDEEFF</PersonalBestColor>
    <AheadGainingTimeColor>FF010203</AheadGainingTimeColor>
    <AheadLosingTimeColor>FF040506</AheadLosingTimeColor>
    <BehindGainingTimeColor>FF070809</BehindGainingTimeColor>
    <BehindLosingTimeColor>FF0A0B0C</BehindLosingTimeColor>
    <BestSegmentColor>FF0D0E0F</BestSegmentColor>
    <UseRainbowColor>True</UseRainbowColor>
    <NotRunningColor>FF101112</NotRunningColor>
    <PausedColor>FF131415</PausedColor>
    <TextOutlineColor>FF161718</TextOutlineColor>
    <ShadowsColor>FF191A1B</ShadowsColor>
    <TimesFont><![CDATA[{times_raw}]]></TimesFont>
    <TimerFont><![CDATA[{timer_raw}]]></TimerFont>
    <TextFont><![CDATA[{text_raw}]]></TextFont>
    <AlwaysOnTop>False</AlwaysOnTop>
    <ShowBestSegments>False</ShowBestSegments>
    <AntiAliasing>False</AntiAliasing>
    <DropShadows>False</DropShadows>
    <BackgroundType>HorizontalGradient</BackgroundType>
    <BackgroundImagePath>/tmp/background.png</BackgroundImagePath>
    <ImageOpacity>0.25</ImageOpacity>
    <ImageBlur>3.25</ImageBlur>
    <Opacity>0.5</Opacity>
    <MousePassThroughWhileRunning>True</MousePassThroughWhileRunning>
    <AllowResizing>False</AllowResizing>
    <AllowMoving>False</AllowMoving>
  </Settings>
  <Components />
</Layout>"#
    );
    let layout = parse_layout(xml.as_bytes()).expect("custom layout must parse");
    assert_eq!(layout.mode, LayoutMode::Horizontal);
    assert_eq!(
        (
            layout.x,
            layout.y,
            layout.vertical_width,
            layout.vertical_height,
            layout.horizontal_width,
            layout.horizontal_height
        ),
        (7, 9, 300, 500, 640, 480)
    );
    let settings = &layout.settings;
    assert_eq!(settings.text_color, egui::Color32::from_rgb(0x11, 0x22, 0x33));
    assert_eq!(
        settings.background_color,
        egui::Color32::from_rgb(0x44, 0x55, 0x66)
    );
    assert_eq!(settings.background_color2, egui::Color32::TRANSPARENT);
    assert_eq!(settings.background_gradient, GradientType::Horizontal);
    assert_eq!(
        settings.background_image.as_deref(),
        Some(std::path::Path::new("/tmp/background.png"))
    );
    assert_eq!(settings.image_opacity, 0.25);
    assert_eq!(settings.image_blur, 3.25);
    assert!(settings.use_rainbow_color);
    assert!(!settings.always_on_top);
    assert!(!settings.show_best_segments);
    assert!(!settings.anti_aliasing);
    assert!(!settings.drop_shadows);
    assert_eq!(settings.opacity, 0.5);
    assert!(settings.mouse_pass_through_while_running);
    assert!(!settings.allow_resizing);
    assert!(!settings.allow_moving);
    assert_eq!(settings.times_font.name, "Segoe UI");
    assert_eq!(settings.timer_font.name, "Century Gothic");
    assert_eq!(settings.text_font.name, "Segoe UI");

    let reparsed = roundtrip(&layout);
    assert_settings_fields_eq(&reparsed.settings, settings);
    assert_font_raw_eq(&reparsed.settings.timer_font, &settings.timer_font);
    assert_font_raw_eq(&reparsed.settings.times_font, &settings.times_font);
    assert_font_raw_eq(&reparsed.settings.text_font, &settings.text_font);
    assert_eq!(reparsed.mode, layout.mode);
    assert_eq!(reparsed.x, layout.x);
    assert_eq!(reparsed.y, layout.y);
}

#[test]
fn font_overrides_roundtrip() {
    let defaults = default_layout();
    let timer_raw = defaults.settings.timer_font.raw.as_ref().unwrap();
    let text_raw = defaults.settings.text_font.raw.as_ref().unwrap();
    let xml = format!(
        r#"<Layout version="1.7">
  <Settings />
  <Components>
    <Component>
      <Path>LiveSplit.Title.dll</Path>
      <Settings><Version>1.7</Version></Settings>
      <FontOverrides>
        <OverrideTimerFont>True</OverrideTimerFont>
        <TimerFont><![CDATA[{timer_raw}]]></TimerFont>
        <OverrideTimesFont>False</OverrideTimesFont>
        <OverrideTextFont>True</OverrideTextFont>
        <TextFont><![CDATA[{text_raw}]]></TextFont>
      </FontOverrides>
    </Component>
  </Components>
</Layout>"#
    );
    let layout = parse_layout(xml.as_bytes()).expect("font overrides must parse");
    let overrides = &layout.components[0].font_overrides;
    assert!(overrides.override_timer_font);
    assert_eq!(overrides.timer_font.name, "Century Gothic");
    assert_eq!(overrides.timer_font.raw.as_ref(), Some(timer_raw));
    assert!(!overrides.override_times_font);
    assert!(overrides.override_text_font);
    assert_eq!(overrides.text_font.raw.as_ref(), Some(text_raw));
    assert!(overrides.has_overrides());

    let xml = layout_to_string(&layout).expect("layout must serialize");
    let document = parse_xml(xml.as_bytes()).unwrap();
    let overrides_element = child(
        child(child(&document, "Components").unwrap(), "Component").unwrap(),
        "FontOverrides",
    )
    .expect("FontOverrides must be written");
    assert_eq!(
        child(overrides_element, "OverrideTimesFont")
            .map(text_trimmed)
            .as_deref(),
        Some("False")
    );
    assert!(child(overrides_element, "TimesFont").is_none());

    let reparsed = parse_layout(xml.as_bytes()).expect("saved layout must parse");
    let reparsed_overrides = &reparsed.components[0].font_overrides;
    assert!(reparsed_overrides.override_timer_font);
    assert!(!reparsed_overrides.override_times_font);
    assert!(reparsed_overrides.override_text_font);
    assert_eq!(
        reparsed_overrides.timer_font.raw,
        overrides.timer_font.raw
    );
    assert_eq!(reparsed_overrides.text_font.raw, overrides.text_font.raw);
}

#[test]
fn unknown_components_and_path_suffixes_are_preserved() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<Layout version="1.7">
  <Settings />
  <Components>
    <Component>
      <Path>Custom.Components.Fancy.dll</Path>
      <Settings>
        <Foo>bar &amp; baz</Foo>
        <Nested enabled="true"><Child>42</Child></Nested>
      </Settings>
    </Component>
    <Component>
      <Path>LiveSplit.Title.dll?Version=1.6.1</Path>
      <Settings><Version>1.6.1</Version></Settings>
    </Component>
    <Component>
      <Path>Components\LiveSplit.Timer.dll</Path>
      <Settings><Version>1.6.1</Version></Settings>
    </Component>
  </Components>
</Layout>"#;
    let layout = parse_layout(xml.as_bytes()).expect("layout must parse");
    assert_eq!(layout.components.len(), 3);
    assert_eq!(layout.components[0].path, "Custom.Components.Fancy.dll");
    assert!(matches!(
        &layout.components[0].component,
        Component::Unknown { .. }
    ));
    assert_eq!(
        layout.components[1].path,
        "LiveSplit.Title.dll?Version=1.6.1"
    );
    assert!(matches!(
        &layout.components[1].component,
        Component::Title(_)
    ));
    assert_eq!(
        layout.components[2].path,
        "Components\\LiveSplit.Timer.dll"
    );
    assert!(matches!(
        &layout.components[2].component,
        Component::Timer(_)
    ));

    let saved = layout_to_string(&layout).expect("layout must serialize");
    assert!(saved.contains("<Path>Custom.Components.Fancy.dll</Path>"));
    assert!(saved.contains("<Path>LiveSplit.Title.dll?Version=1.6.1</Path>"));
    assert!(saved.contains("<Path>Components\\LiveSplit.Timer.dll</Path>"));

    let reparsed = parse_layout(saved.as_bytes()).expect("saved layout must parse");
    let unknown = match &reparsed.components[0].component {
        Component::Unknown { path, settings } => (path, settings),
        other => panic!("expected unknown component, got {other:?}"),
    };
    assert_eq!(unknown.0, "Custom.Components.Fancy.dll");
    let foo = child(unknown.1, "Foo")
        .map(text_trimmed)
        .expect("Foo must survive");
    assert_eq!(foo, "bar & baz");
    let nested = child(unknown.1, "Nested").expect("Nested must survive");
    assert_eq!(
        nested.attributes.get("enabled").map(String::as_str),
        Some("true")
    );
    assert_eq!(
        child(nested, "Child").map(text_trimmed).as_deref(),
        Some("42")
    );
}

#[test]
fn background_image_blob_is_preserved() {
    let image_blob = "AAEAAAD/////AQAAAAAAAAA=";
    let xml = format!(
        r#"<Layout version="1.6.1">
  <Settings>
    <BackgroundType>Image</BackgroundType>
    <BackgroundImage><![CDATA[{image_blob}]]></BackgroundImage>
  </Settings>
  <Components />
</Layout>"#
    );
    let layout = parse_layout(xml.as_bytes()).expect("background image must parse");
    let image = layout
        .settings
        .background_image
        .as_ref()
        .expect("background image must be preserved");
    assert!(image
        .to_string_lossy()
        .starts_with(EMBEDDED_IMAGE_PREFIX));
    assert!(image.to_string_lossy().ends_with(image_blob));

    let saved = layout_to_string(&layout).expect("layout must serialize");
    assert!(saved.contains("<BackgroundType>Image</BackgroundType>"));
    assert!(saved.contains(image_blob));
    assert!(!saved.contains("<BackgroundImagePath>"));

    let reparsed = parse_layout(saved.as_bytes()).expect("saved layout must parse");
    assert_eq!(reparsed.settings.background_image, layout.settings.background_image);
    assert_eq!(
        reparsed.settings.background_gradient,
        GradientType::Plain
    );
}

#[test]
fn background_image_path_extension_roundtrips() {
    let xml = r#"<Layout version="1.6.1">
  <Settings><BackgroundImagePath>relative/background.png</BackgroundImagePath></Settings>
  <Components />
</Layout>"#;
    let layout = parse_layout(xml.as_bytes()).expect("image path must parse");
    assert_eq!(
        layout.settings.background_image.as_deref(),
        Some(std::path::Path::new("relative/background.png"))
    );
    let saved = layout_to_string(&layout).unwrap();
    assert!(saved.contains("<BackgroundImagePath>relative/background.png</BackgroundImagePath>"));
    let reparsed = parse_layout(saved.as_bytes()).unwrap();
    assert_eq!(
        reparsed.settings.background_image.as_deref(),
        Some(std::path::Path::new("relative/background.png"))
    );
}

#[test]
fn invalid_xml_reports_an_error() {
    let error = parse_layout(b"<Layout><Mode>Vertical</Mode>").unwrap_err();
    assert!(
        matches!(&error, LayoutXmlError::Xml(_)),
        "expected XML error, got {error:?}"
    );

    let error = parse_layout(b"<NotLayout />").unwrap_err();
    assert!(
        matches!(&error, LayoutXmlError::Invalid(_)),
        "expected invalid layout error, got {error:?}"
    );
}
