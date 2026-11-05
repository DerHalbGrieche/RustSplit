//! Layout editor: component order, per-component settings and global theme.

use std::path::{Path, PathBuf};

use egui::{Context, Grid, Key, RichText, ScrollArea, TextEdit};

use rustsplit_ui::components::{default_component_for_path, ADDABLE_COMPONENTS};
use rustsplit_ui::layout_xml::EMBEDDED_IMAGE_PREFIX;
use rustsplit_ui::{color_to_hex, parse_color, BackgroundType, Layout, LayoutMode, LayoutSettings};

use crate::config::AppConfig;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayoutEditorEvent {
    None,
    Apply,
    ApplyAndClose,
    Save,
    SaveAs,
    Cancel,
    ResetDefault,
    /// Pick a background image; the app answers with
    /// [`LayoutEditor::set_background_image`].
    BrowseBackgroundImage,
}

/// Image formats the renderer can decode.
pub const BACKGROUND_IMAGE_EXTENSIONS: &[&str] =
    &["png", "jpg", "jpeg", "gif", "bmp", "webp", "ico"];

pub struct LayoutEditor {
    pub open: bool,
    draft: Option<Layout>,
    selected: usize,
    add_path: String,
    show_global: bool,
    error: Option<String>,
    /// Set while the app's file dialog picks a background image, so the
    /// first Escape closes the built-in browser instead of the editor.
    picking_image: bool,
}

impl Default for LayoutEditor {
    fn default() -> Self {
        Self {
            open: false,
            draft: None,
            selected: 0,
            add_path: ADDABLE_COMPONENTS
                .first()
                .map(|(path, _)| path.to_string())
                .unwrap_or_default(),
            show_global: false,
            error: None,
            picking_image: false,
        }
    }
}

impl LayoutEditor {
    pub fn open(&mut self, layout: &Layout) {
        self.draft = Some(layout.clone());
        self.open = true;
        self.selected = 0;
        self.show_global = false;
        self.error = None;
        self.picking_image = false;
    }

    pub fn draft_cloned(&self) -> Option<Layout> {
        self.draft.clone()
    }

    pub fn close(&mut self) {
        self.open = false;
        self.draft = None;
    }

    /// Where the background image dialog starts: the current image file, or
    /// the layout file's directory.
    pub fn background_image_start(&self) -> Option<PathBuf> {
        let draft = self.draft.as_ref()?;
        draft
            .settings
            .background_image
            .clone()
            .filter(|path| !is_embedded(path))
            .or_else(|| draft.file_path.clone())
    }

    /// The background image dialog closed without a choice (or never
    /// opened), so Escape closes the editor again.
    pub fn cancel_background_image(&mut self) {
        self.picking_image = false;
    }

    /// Use `path` as the draft's background image.
    pub fn set_background_image(&mut self, path: PathBuf) {
        self.picking_image = false;
        if let Some(draft) = self.draft.as_mut() {
            draft.settings.background_image = Some(path);
            draft.settings.background_type = BackgroundType::Image;
        }
    }

    pub fn show(&mut self, ctx: &Context, config: &AppConfig) -> LayoutEditorEvent {
        if !self.open {
            return LayoutEditorEvent::None;
        }
        let Some(draft) = self.draft.as_mut() else {
            self.open = false;
            return LayoutEditorEvent::Cancel;
        };
        let mut event = LayoutEditorEvent::None;
        let mut open = self.open;
        let mut remove_index = None;
        let mut move_up = None;
        let mut move_down = None;
        let selected = &mut self.selected;
        let add_path = &mut self.add_path;
        let show_global = &mut self.show_global;
        let error = &mut self.error;
        let picking_image = &mut self.picking_image;

        egui::Window::new("Layout Editor")
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_width(880.0)
            .default_height(650.0)
            .default_pos(egui::pos2(12.0, 12.0))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Mode:");
                    egui::ComboBox::from_id_salt("layout-mode")
                        .selected_text(draft.mode.label())
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut draft.mode, LayoutMode::Vertical, "Vertical");
                            ui.selectable_value(
                                &mut draft.mode,
                                LayoutMode::Horizontal,
                                "Horizontal",
                            );
                        });
                    ui.label("X");
                    ui.add(egui::DragValue::new(&mut draft.x));
                    ui.label("Y");
                    ui.add(egui::DragValue::new(&mut draft.y));
                    ui.label("Width");
                    ui.add(egui::DragValue::new(&mut draft.vertical_width).range(50..=5000));
                    ui.label("Height");
                    ui.add(egui::DragValue::new(&mut draft.vertical_height).range(20..=5000));
                });
                ui.horizontal(|ui| {
                    if ui.selectable_label(!*show_global, "Components").clicked() {
                        *show_global = false;
                    }
                    if ui.selectable_label(*show_global, "Appearance").clicked() {
                        *show_global = true;
                    }
                });
                ui.separator();

                if *show_global {
                    ScrollArea::vertical()
                        .id_salt("layout-editor-global")
                        .show(ui, |ui| {
                            if global_settings_ui(ui, &mut draft.settings) {
                                event = LayoutEditorEvent::BrowseBackgroundImage;
                                *picking_image = true;
                            }
                        });
                } else {
                    ui.horizontal(|ui| {
                        ui.label("Add:");
                        egui::ComboBox::from_id_salt("add-component")
                            .selected_text(
                                ADDABLE_COMPONENTS
                                    .iter()
                                    .find(|(path, _)| path == add_path)
                                    .map(|(_, label)| *label)
                                    .unwrap_or("Component"),
                            )
                            .show_ui(ui, |ui| {
                                for (path, label) in ADDABLE_COMPONENTS {
                                    ui.selectable_value(add_path, path.to_string(), *label);
                                }
                            });
                        if ui.button("+ Add component").clicked() {
                            draft.add_component(default_component_for_path(add_path));
                            *selected = draft.components.len().saturating_sub(1);
                        }
                    });
                    ui.separator();
                    ui.horizontal_top(|ui| {
                        ui.vertical(|ui| {
                            ui.set_min_width(230.0);
                            ui.set_max_width(230.0);
                            ScrollArea::vertical()
                                .id_salt("layout-editor-components")
                                .max_width(230.0)
                                .max_height(480.0)
                                .show(ui, |ui| {
                                    for (index, component) in draft.components.iter().enumerate() {
                                        let selected_now = index == *selected;
                                        if ui
                                            .selectable_label(
                                                selected_now,
                                                format!("{}  ({})", component.name(), index + 1),
                                            )
                                            .clicked()
                                        {
                                            *selected = index;
                                        }
                                    }
                                });
                            ui.separator();
                            if ui
                                .add_enabled(*selected > 0, egui::Button::new("↑ Move up"))
                                .clicked()
                            {
                                move_up = Some(*selected);
                            }
                            if ui
                                .add_enabled(
                                    *selected + 1 < draft.components.len(),
                                    egui::Button::new("↓ Move down"),
                                )
                                .clicked()
                            {
                                move_down = Some(*selected);
                            }
                            if ui
                                .add_enabled(
                                    !draft.components.is_empty(),
                                    egui::Button::new("✕ Remove"),
                                )
                                .clicked()
                            {
                                remove_index = Some(*selected);
                            }
                        });
                        ui.separator();
                        ui.vertical(|ui| {
                            ui.set_min_width(580.0);
                            ui.set_max_width(600.0);
                            ScrollArea::vertical()
                                .id_salt("layout-editor-settings")
                                .max_width(600.0)
                                .max_height(480.0)
                                .show(ui, |ui| {
                                    if let Some(component) = draft.components.get_mut(*selected) {
                                        ui.heading(component.name());
                                        ui.label(
                                            RichText::new(&component.path)
                                                .small()
                                                .color(ui.visuals().weak_text_color()),
                                        );
                                        ui.separator();
                                        component.component.settings_ui(ui, &mut draft.settings);
                                        ui.separator();
                                        ui.collapsing("Font overrides", |ui| {
                                            let overrides = &mut component.font_overrides;
                                            font_override_ui(
                                                ui,
                                                "Timer font",
                                                "timer",
                                                &mut overrides.override_timer_font,
                                                &mut overrides.timer_font,
                                            );
                                            font_override_ui(
                                                ui,
                                                "Times font",
                                                "times",
                                                &mut overrides.override_times_font,
                                                &mut overrides.times_font,
                                            );
                                            font_override_ui(
                                                ui,
                                                "Text font",
                                                "text",
                                                &mut overrides.override_text_font,
                                                &mut overrides.text_font,
                                            );
                                        });
                                    } else {
                                        ui.label("No component selected.");
                                    }
                                });
                        });
                    });
                }

                if let Some(message) = error.as_ref() {
                    ui.colored_label(egui::Color32::LIGHT_RED, message);
                }
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("Apply").clicked() {
                        event = LayoutEditorEvent::Apply;
                    }
                    if ui.button("OK").clicked() {
                        event = LayoutEditorEvent::ApplyAndClose;
                    }
                    if ui.button("Cancel").clicked() {
                        event = LayoutEditorEvent::Cancel;
                    }
                    ui.separator();
                    if ui.button("Save").clicked() {
                        event = LayoutEditorEvent::Save;
                    }
                    if ui.button("Save As…").clicked() {
                        event = LayoutEditorEvent::SaveAs;
                    }
                    if ui.button("Reset to default").clicked() {
                        event = LayoutEditorEvent::ResetDefault;
                    }
                    if config.general.autosave_layout {
                        ui.label(
                            RichText::new("Autosave is on")
                                .small()
                                .color(ui.visuals().weak_text_color()),
                        );
                    }
                });
                if ctx.input(|input| input.key_pressed(Key::Escape)) {
                    if *picking_image {
                        *picking_image = false;
                    } else {
                        event = LayoutEditorEvent::Cancel;
                    }
                }
            });

        if let Some(index) = move_up {
            if let Some(draft) = self.draft.as_mut() {
                draft.move_component_up(index);
                *selected = index.saturating_sub(1);
            }
        }
        if let Some(index) = move_down {
            if let Some(draft) = self.draft.as_mut() {
                draft.move_component_down(index);
                *selected = (index + 1).min(draft.components.len().saturating_sub(1));
            }
        }
        if let Some(index) = remove_index {
            if let Some(draft) = self.draft.as_mut() {
                draft.remove_component(index);
                *selected = (*selected).min(draft.components.len().saturating_sub(1));
            }
        }
        if event == LayoutEditorEvent::ResetDefault {
            if let Some(draft) = self.draft.as_mut() {
                let default =
                    Layout::parse_default().unwrap_or_else(|_| Layout::livesplit_default());
                let file_path = draft.file_path.clone();
                *draft = default;
                draft.file_path = file_path;
            }
            event = LayoutEditorEvent::Apply;
        }

        if !open || event == LayoutEditorEvent::Cancel {
            self.close();
            return LayoutEditorEvent::Cancel;
        }
        self.open = open;
        event
    }
}

fn font_override_ui(
    ui: &mut egui::Ui,
    label: &str,
    id: &str,
    enabled: &mut bool,
    font: &mut rustsplit_ui::FontSpec,
) {
    ui.horizontal(|ui| {
        ui.checkbox(enabled, label);
        ui.add_enabled_ui(*enabled, |ui| {
            if ui
                .add(TextEdit::singleline(&mut font.name).desired_width(130.0))
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
            let _ = id;
        });
    });
}

fn is_embedded(path: &Path) -> bool {
    path.to_str()
        .is_some_and(|path| path.starts_with(EMBEDDED_IMAGE_PREFIX))
}

/// Returns whether the user asked to browse for a background image.
fn global_settings_ui(ui: &mut egui::Ui, settings: &mut LayoutSettings) -> bool {
    let mut browse = false;
    Grid::new("layout-global-grid")
        .num_columns(2)
        .striped(true)
        .show(ui, |ui| {
            ui.strong("General");
            ui.label("");
            ui.end_row();
            ui.label("Text color");
            color_edit(ui, &mut settings.text_color);
            ui.end_row();
            ui.label("Transparent background");
            ui.horizontal(|ui| {
                let changed = ui
                    .checkbox(&mut settings.transparent_background, "")
                    .changed();
                ui.label(
                    RichText::new("High-contrast text over any background")
                        .small()
                        .color(ui.visuals().weak_text_color()),
                );
                if changed && !settings.transparent_background && !settings.has_background() {
                    settings.background_color = egui::Color32::from_rgb(15, 15, 15);
                    settings.background_color2 = egui::Color32::TRANSPARENT;
                    settings.background_type = BackgroundType::SolidColor;
                }
            });
            ui.end_row();
            ui.label("Background type");
            egui::ComboBox::from_id_salt("layout-bg-type")
                .selected_text(settings.background_type.label())
                .show_ui(ui, |ui| {
                    for background_type in BackgroundType::ALL {
                        ui.selectable_value(
                            &mut settings.background_type,
                            background_type,
                            background_type.label(),
                        );
                    }
                });
            ui.end_row();
            if settings.background_type == BackgroundType::Image {
                ui.label("Background image");
                ui.horizontal(|ui| {
                    match settings.background_image.as_deref() {
                        Some(path) if is_embedded(path) => {
                            ui.label("Embedded in the layout");
                        }
                        _ => {
                            let mut path = settings
                                .background_image
                                .as_ref()
                                .map(|path| path.display().to_string())
                                .unwrap_or_default();
                            if ui.text_edit_singleline(&mut path).changed() {
                                settings.background_image = if path.trim().is_empty() {
                                    None
                                } else {
                                    Some(path.into())
                                };
                            }
                        }
                    }
                    browse = ui.button("Browse…").clicked();
                    if ui
                        .add_enabled(
                            settings.background_image.is_some(),
                            egui::Button::new("Remove"),
                        )
                        .clicked()
                    {
                        settings.background_image = None;
                    }
                });
                ui.end_row();
                ui.label("Image opacity").on_hover_text(
                    "Like LiveSplit, the image is drawn over black, so lower values \
                     darken it. With a transparent background the desktop shows \
                     through instead.",
                );
                ui.add(egui::Slider::new(&mut settings.image_opacity, 0.0..=1.0));
                ui.end_row();
                ui.label("Image blur");
                // LiveSplit's blur is a 0..1 fraction of the image size.
                ui.add(egui::Slider::new(&mut settings.image_blur, 0.0..=1.0));
                ui.end_row();
            } else {
                ui.label("Background color");
                color_edit(ui, &mut settings.background_color);
                ui.end_row();
                if settings.background_type != BackgroundType::SolidColor {
                    ui.label("Background color 2");
                    color_edit(ui, &mut settings.background_color2);
                    ui.end_row();
                }
            }
            ui.label("Opacity");
            ui.add(egui::Slider::new(&mut settings.opacity, 0.0..=1.0));
            ui.end_row();

            ui.strong("Colors");
            ui.label("");
            ui.end_row();
            for (label, color) in [
                ("Thin separators", &mut settings.thin_separators_color),
                ("Separators", &mut settings.separators_color),
                ("Personal best", &mut settings.personal_best_color),
                ("Ahead gaining", &mut settings.ahead_gaining_time_color),
                ("Ahead losing", &mut settings.ahead_losing_time_color),
                ("Behind gaining", &mut settings.behind_gaining_time_color),
                ("Behind losing", &mut settings.behind_losing_time_color),
                ("Best segment", &mut settings.best_segment_color),
                ("Not running", &mut settings.not_running_color),
                ("Paused", &mut settings.paused_color),
                ("Text outline", &mut settings.text_outline_color),
                ("Shadows", &mut settings.shadows_color),
            ] {
                ui.label(label);
                color_edit(ui, color);
                ui.end_row();
            }

            ui.strong("Fonts");
            ui.label("");
            ui.end_row();
            for (label, font) in [
                ("Timer", &mut settings.timer_font),
                ("Times", &mut settings.times_font),
                ("Text", &mut settings.text_font),
            ] {
                ui.label(label);
                ui.horizontal(|ui| {
                    if ui
                        .add(TextEdit::singleline(&mut font.name).desired_width(130.0))
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
                });
                ui.end_row();
            }

            ui.strong("Behavior");
            ui.label("");
            ui.end_row();
            ui.label("Always on top");
            ui.checkbox(&mut settings.always_on_top, "");
            ui.end_row();
            ui.label("Show best segments");
            ui.checkbox(&mut settings.show_best_segments, "");
            ui.end_row();
            ui.label("Drop shadows");
            ui.checkbox(&mut settings.drop_shadows, "");
            ui.end_row();
            ui.label("Anti-aliasing");
            ui.checkbox(&mut settings.anti_aliasing, "");
            ui.end_row();
            ui.label("Rainbow best segment");
            ui.checkbox(&mut settings.use_rainbow_color, "");
            ui.end_row();
            ui.label("Allow resizing");
            ui.checkbox(&mut settings.allow_resizing, "");
            ui.end_row();
            ui.label("Allow moving");
            ui.checkbox(&mut settings.allow_moving, "");
            ui.end_row();
            ui.label("Mouse pass-through while running");
            ui.checkbox(&mut settings.mouse_pass_through_while_running, "");
            ui.end_row();
        });
    ui.separator();
    ui.label(
        RichText::new(format!(
            "Text {}  |  Separators {}  |  PB {}",
            color_to_hex(settings.text_color),
            color_to_hex(settings.separators_color),
            color_to_hex(settings.personal_best_color)
        ))
        .small()
        .color(ui.visuals().weak_text_color()),
    );
    browse
}

fn color_edit(ui: &mut egui::Ui, color: &mut egui::Color32) {
    ui.horizontal(|ui| {
        ui.color_edit_button_srgba(color);
        let mut hex = color_to_hex(*color);
        if ui
            .add(TextEdit::singleline(&mut hex).desired_width(80.0))
            .changed()
        {
            *color = parse_color(Some(&hex), *color);
        }
    });
}
