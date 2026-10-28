//! RustSplit's layout model and egui renderer.

pub mod canvas;
pub mod components;
pub mod icon;
pub mod layout;
pub mod layout_xml;
pub mod render;

pub use components::{Component, ComponentContext};
pub use icon::IconCache;
pub use layout::{
    color_to_hex, parse_color, BackgroundStyle, FontOverrides, FontSpec, GradientType, Layout,
    LayoutComponent, LayoutMode, LayoutSettings,
};
pub use layout_xml::{load_layout, parse_layout, save_layout};
pub use livesplit_core::xml;
pub use render::Renderer;

use std::sync::Arc;

use egui::{FontData, FontDefinitions, FontFamily, FontId, TextStyle};

pub const FONT_TEXT: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/assets/fonts/DejaVuSans.ttf"
));
pub const FONT_BOLD: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/assets/fonts/DejaVuSans-Bold.ttf"
));
pub const FONT_MONO: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/assets/fonts/DejaVuSansMono.ttf"
));
pub const FONT_MONO_BOLD: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/assets/fonts/DejaVuSansMono-Bold.ttf"
));

pub const TEXT_FAMILY: &str = "livesplit_text";
pub const BOLD_FAMILY: &str = "livesplit_bold";
pub const TIMER_FAMILY: &str = "livesplit_timer";
pub const TIMER_BOLD_FAMILY: &str = "livesplit_timer_bold";

/// Register the bundled fonts with egui.  Safe to call once at startup.
pub fn configure_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    fonts
        .font_data
        .insert("ls_text".to_string(), Arc::new(FontData::from_static(FONT_TEXT)));
    fonts
        .font_data
        .insert("ls_bold".to_string(), Arc::new(FontData::from_static(FONT_BOLD)));
    fonts
        .font_data
        .insert("ls_mono".to_string(), Arc::new(FontData::from_static(FONT_MONO)));
    fonts.font_data.insert(
        "ls_mono_bold".to_string(),
        Arc::new(FontData::from_static(FONT_MONO_BOLD)),
    );

    fonts.families.insert(
        FontFamily::Name(Arc::from(TEXT_FAMILY)),
        vec!["ls_text".to_string(), "Ubuntu-Light".to_string()],
    );
    fonts.families.insert(
        FontFamily::Name(Arc::from(BOLD_FAMILY)),
        vec!["ls_bold".to_string(), "ls_text".to_string()],
    );
    fonts.families.insert(
        FontFamily::Name(Arc::from(TIMER_FAMILY)),
        vec!["ls_mono".to_string(), "Hack".to_string()],
    );
    fonts.families.insert(
        FontFamily::Name(Arc::from(TIMER_BOLD_FAMILY)),
        vec!["ls_mono_bold".to_string(), "ls_mono".to_string()],
    );
    // Use our bundled fonts for normal widgets as well.
    fonts
        .families
        .entry(FontFamily::Proportional)
        .or_default()
        .insert(0, "ls_text".to_string());
    fonts
        .families
        .entry(FontFamily::Monospace)
        .or_default()
        .insert(0, "ls_mono".to_string());

    ctx.set_fonts(fonts);

    ctx.all_styles_mut(|style| {
        style.text_styles.insert(
            TextStyle::Body,
            FontId::new(15.0, FontFamily::Name(Arc::from(TEXT_FAMILY))),
        );
        style.text_styles.insert(
            TextStyle::Button,
            FontId::new(15.0, FontFamily::Name(Arc::from(TEXT_FAMILY))),
        );
        style.text_styles.insert(
            TextStyle::Small,
            FontId::new(12.0, FontFamily::Name(Arc::from(TEXT_FAMILY))),
        );
        style.text_styles.insert(
            TextStyle::Heading,
            FontId::new(22.0, FontFamily::Name(Arc::from(BOLD_FAMILY))),
        );
        style.text_styles.insert(
            TextStyle::Monospace,
            FontId::new(14.0, FontFamily::Name(Arc::from(TIMER_FAMILY))),
        );
    });
}
