//! The Settings window.
//!
//! It edits a draft of the settings. OK applies and closes, Apply keeps the
//! window open, and Cancel or the close button discard the edits that weren't
//! applied. Applying only copies what was edited in the window (see
//! [`AppConfig::apply_settings_edits`]), so changes made elsewhere while it
//! was open, like a new auto splitter or the server menu toggle, are kept.

use egui::{Color32, Context, RichText};

use crate::config::{AppConfig, SoundConfig};
use crate::hotkeys::HotkeyAction;
use crate::niri;
use crate::sounds::{self, SoundEvent, SoundPlayer};

/// What the user did in the window this frame.
#[derive(Debug, Default)]
pub struct SettingsResponse {
    /// OK or Apply.
    pub apply: bool,
    /// OK, Cancel or the close button.
    pub close: bool,
    /// The "Browse…" button of this sound was clicked.
    pub browse_sound: Option<SoundEvent>,
}

/// The draft and the settings it started from.
pub struct SettingsWindow {
    base: AppConfig,
    draft: AppConfig,
}

impl SettingsWindow {
    pub fn new(config: &AppConfig) -> Self {
        Self {
            base: config.clone(),
            draft: config.clone(),
        }
    }

    pub fn draft_sounds(&self) -> &SoundConfig {
        &self.draft.sounds
    }

    pub fn draft_sounds_mut(&mut self) -> &mut SoundConfig {
        &mut self.draft.sounds
    }

    /// Applies the edits to `config`. Further edits start from the result.
    pub fn apply_to(&mut self, config: &mut AppConfig) {
        config.apply_settings_edits(&self.base, &self.draft);
        *self = Self::new(config);
    }

    pub fn show(
        &mut self,
        ctx: &Context,
        sound_player: &mut SoundPlayer,
        hotkey_error: Option<&str>,
        server_error: Option<&str>,
    ) -> SettingsResponse {
        let draft = &mut self.draft;
        let mut open = true;
        let mut apply = false;
        let mut close = false;
        let mut browse_sound = None;
        egui::Window::new("Settings")
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_width(560.0)
            .default_height(520.0)
            .default_pos(egui::pos2(12.0, 12.0))
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("settings-scroll")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                ui.heading("Hotkeys");
                ui.checkbox(
                    &mut draft.hotkeys.enabled,
                    "Enable global hotkeys (X11 only; use compositor binds on Wayland)",
                );
                if let Some(error) = hotkey_error {
                    ui.colored_label(Color32::LIGHT_RED, error);
                }
                egui::Grid::new("hotkey-grid")
                    .num_columns(2)
                    .striped(true)
                    .show(ui, |ui| {
                        for action in HotkeyAction::all() {
                            ui.label(action.label());
                            let value = action.config_value_mut(&mut draft.hotkeys);
                            ui.add(
                                egui::TextEdit::singleline(value)
                                    .hint_text("e.g. F1 or control+shift+F12")
                                    .desired_width(220.0),
                            );
                            ui.end_row();
                        }
                    });
                ui.label(
                    RichText::new("Use names like Space, F1, control+KeyS, shift+alt+KeyR.")
                        .small()
                        .color(ui.visuals().weak_text_color()),
                );
                ui.label(
                    RichText::new(
                        "Wayland/Niri: bind `rustsplit --command startorsplit` in your compositor instead.",
                    )
                    .small()
                    .color(ui.visuals().weak_text_color()),
                );
                ui.separator();
                ui.heading("LiveSplit Server");
                ui.checkbox(&mut draft.server.enabled, "Enable server");
                ui.checkbox(&mut draft.server.autostart, "Start automatically");
                ui.horizontal(|ui| {
                    ui.label("Port:");
                    ui.add(egui::DragValue::new(&mut draft.server.port).range(1..=65535));
                    ui.label("Bind:");
                    ui.add(
                        egui::TextEdit::singleline(&mut draft.server.bind_address)
                            .desired_width(120.0),
                    );
                });
                if let Some(error) = server_error {
                    ui.colored_label(Color32::LIGHT_RED, error);
                }
                ui.separator();
                ui.heading("Behavior");
                ui.checkbox(&mut draft.general.autosave, "Autosave splits on reset/finish");
                ui.checkbox(
                    &mut draft.general.autosave_layout,
                    "Autosave layout after applying",
                );
                ui.add_enabled(
                    cfg!(feature = "native-dialogs"),
                    egui::Checkbox::new(
                        &mut draft.general.use_system_file_dialogs,
                        "Use system file dialogs",
                    ),
                )
                .on_hover_text(
                    "Use the desktop's file chooser (XDG portal on Linux). \
                     Falls back to the built-in browser when it is unavailable.",
                )
                .on_disabled_hover_text("This build has no system file dialogs.");
                #[cfg(feature = "speedrun-com")]
                {
                    ui.separator();
                    ui.heading("speedrun.com");
                    ui.checkbox(
                        &mut draft.speedrun_com.fetch_world_records,
                        "Fetch world records from speedrun.com",
                    );
                    ui.label(
                        RichText::new(
                            "Used by the World Record component. Sends the game, category and filters to speedrun.com.",
                        )
                        .small()
                        .color(ui.visuals().weak_text_color()),
                    );
                }
                ui.separator();
                ui.heading("Sounds");
                browse_sound = sounds::settings_ui(ui, &mut draft.sounds, sound_player);
                if niri::is_session() {
                    ui.separator();
                    ui.heading("Niri");
                    ui.checkbox(&mut draft.niri.enabled, "Enable Niri integration");
                    ui.checkbox(
                        &mut draft.niri.auto_float,
                        "Float the overlay automatically",
                    );
                    ui.label(
                        RichText::new(
                            "Uses Niri's IPC socket to float the overlay and remember its position and size.",
                        )
                        .small()
                        .color(ui.visuals().weak_text_color()),
                    );
                }
                ui.horizontal(|ui| {
                    if ui.button("OK").clicked() {
                        apply = true;
                        close = true;
                    }
                    if ui.button("Apply").clicked() {
                        apply = true;
                    }
                    if ui.button("Cancel").clicked() {
                        close = true;
                    }
                });
                ui.separator();
                ui.label(
                    RichText::new("RustSplit — a native Rust rewrite of LiveSplit")
                        .small()
                        .color(ui.visuals().weak_text_color()),
                );
                    });
            });
        SettingsResponse {
            apply,
            close: close || !open,
            browse_sound,
        }
    }
}
