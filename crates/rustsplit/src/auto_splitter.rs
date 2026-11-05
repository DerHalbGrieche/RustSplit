//! WebAssembly auto splitter integration: the context menu entries, the
//! auto splitter window and the choice of auto splitter per split file.
//!
//! The runtime itself lives in the `rustsplit-auto-splitting` crate and is
//! only compiled with the `auto-splitting` cargo feature. Without it, this
//! module keeps the settings intact and shows a disabled menu entry.
//!
//! Which auto splitter belongs to a run is decided like this:
//! 1. `settings.toml` `[auto_splitter.runs]`, keyed by the split file path;
//! 2. otherwise the `<ScriptPath>` stored in the `.lss` (the same element
//!    LiveSplit One / LiveSplit's Auto Splitting Runtime component write).
//!
//! The auto splitter's settings are stored in the `.lss`
//! `<AutoSplitterSettings>` element, like livesplit-core does.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// `[auto_splitter]` in `settings.toml`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct AutoSplitterConfig {
    /// Run auto splitters at all.
    pub enabled: bool,
    /// The auto splitter (`.wasm`) chosen for each split file, keyed by the
    /// split file's path.
    pub runs: BTreeMap<String, PathBuf>,
}

impl Default for AutoSplitterConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            runs: BTreeMap::new(),
        }
    }
}

#[cfg_attr(not(feature = "auto-splitting"), allow(dead_code))]
impl AutoSplitterConfig {
    /// The key a split file is remembered under (canonicalized like the
    /// recent files list).
    pub fn run_key(path: &Path) -> String {
        std::fs::canonicalize(path)
            .unwrap_or_else(|_| path.to_path_buf())
            .to_string_lossy()
            .into_owned()
    }

    pub fn path_for_run(&self, run_path: &Path) -> Option<&PathBuf> {
        self.runs.get(&Self::run_key(run_path))
    }
}

/// What the app should do after [`AutoSplitterUi::update`].
#[derive(Debug, Default)]
pub struct AutoSplitterOutcome {
    /// An auto splitter command finished or reset the attempt.
    pub autosave: bool,
    pub status: Option<String>,
    /// `AppConfig` changed and should be saved.
    pub config_changed: bool,
}

#[cfg(feature = "auto-splitting")]
pub use enabled::AutoSplitterUi;

#[cfg(not(feature = "auto-splitting"))]
pub use disabled::AutoSplitterUi;

#[cfg(not(feature = "auto-splitting"))]
mod disabled {
    use egui::{Context, Ui};
    use rustsplit_core::Timer;

    use super::{AutoSplitterConfig, AutoSplitterOutcome};

    /// Stand-in used when RustSplit is built without auto splitting.
    pub struct AutoSplitterUi;

    impl AutoSplitterUi {
        pub fn new(_ctx: &Context) -> Self {
            Self
        }

        pub fn update(
            &mut self,
            _timer: &mut Timer,
            _config: &mut AutoSplitterConfig,
        ) -> AutoSplitterOutcome {
            AutoSplitterOutcome::default()
        }

        pub fn publish(&mut self, _timer: &Timer) {}

        pub fn menu(&mut self, ui: &mut Ui, _config: &AutoSplitterConfig) {
            ui.add_enabled(false, egui::Button::new("Auto Splitter (not built in)"))
                .on_disabled_hover_text(
                    "This build of RustSplit was compiled without the `auto-splitting` feature.",
                );
        }

        pub fn show(&mut self, _ctx: &Context, _config: &AutoSplitterConfig) {}

        pub fn window_open(&self) -> bool {
            false
        }
    }
}

#[cfg(feature = "auto-splitting")]
mod enabled {
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    use egui::{Color32, Context, RichText, Ui};
    use rustsplit_auto_splitting::settings::{FileFilter, Map, Value, Widget, WidgetKind};
    use rustsplit_auto_splitting::{
        wasi_path, AutoSplitterHost, HostEvent, HostStatus, LogSeverity, StoredSettings,
    };
    use rustsplit_core::Timer;

    use super::{AutoSplitterConfig, AutoSplitterOutcome};
    use crate::file_dialog::{FileDialog, FileDialogAction, FileDialogMode};

    enum Request {
        Load(PathBuf),
        Remove,
        SetEnabled(bool),
        Reload,
    }

    enum DialogPurpose {
        LoadAutoSplitter,
        /// A file select setting of the auto splitter.
        Setting(Arc<str>),
    }

    pub struct AutoSplitterUi {
        host: AutoSplitterHost,
        ctx: Context,
        requests: Vec<Request>,
        setting_changes: Vec<(Arc<str>, Value)>,
        window_open: bool,
        file_dialog: FileDialog,
        dialog_purpose: Option<DialogPurpose>,
        /// The run identity seen last frame, to notice when another split
        /// file was opened (from the menu, the server, drag and drop, …).
        seen_run_path: Option<PathBuf>,
        seen_run_settings: Option<String>,
        initialized: bool,
    }

    fn file_name(path: &Path) -> String {
        path.file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string())
    }

    impl AutoSplitterUi {
        pub fn new(ctx: &Context) -> Self {
            let repaint = ctx.clone();
            Self {
                host: AutoSplitterHost::new(move || repaint.request_repaint()),
                ctx: ctx.clone(),
                requests: Vec::new(),
                setting_changes: Vec::new(),
                window_open: false,
                file_dialog: FileDialog::default(),
                dialog_purpose: None,
                seen_run_path: None,
                seen_run_settings: None,
                initialized: false,
            }
        }

        fn request(&mut self, request: Request) {
            self.requests.push(request);
            self.ctx.request_repaint();
        }

        /// Applies auto splitter commands and UI requests. Call once per
        /// frame before anything reads the timer.
        pub fn update(
            &mut self,
            timer: &mut Timer,
            config: &mut AutoSplitterConfig,
        ) -> AutoSplitterOutcome {
            let mut outcome = AutoSplitterOutcome::default();

            let mut reconcile = !self.initialized
                || timer.run.file_path != self.seen_run_path
                || timer.run.auto_splitter_settings != self.seen_run_settings;
            for request in std::mem::take(&mut self.requests) {
                match request {
                    Request::Load(path) => {
                        if let Some(run_path) = &timer.run.file_path {
                            config
                                .runs
                                .insert(AutoSplitterConfig::run_key(run_path), path.clone());
                        }
                        config.enabled = true;
                        outcome.config_changed = true;
                        let settings =
                            stored_settings(timer).and_then(|stored| stored.settings_for(&path));
                        outcome.status =
                            Some(format!("Loading auto splitter {}…", file_name(&path)));
                        self.host.load(path, settings, timer);
                        reconcile = false;
                    }
                    Request::Remove => {
                        self.host.unload();
                        if let Some(run_path) = &timer.run.file_path {
                            if config
                                .runs
                                .remove(&AutoSplitterConfig::run_key(run_path))
                                .is_some()
                            {
                                outcome.config_changed = true;
                            }
                        }
                        // Only drop settings written for a WASM auto splitter,
                        // never an ASL payload from LiveSplit.
                        if stored_settings(timer).is_some_and(|stored| stored.script_path.is_some())
                        {
                            timer.run.auto_splitter_settings = None;
                            timer.run.has_changed = true;
                        }
                        outcome.status = Some("Auto splitter removed.".to_string());
                        reconcile = false;
                    }
                    Request::SetEnabled(enabled) => {
                        config.enabled = enabled;
                        outcome.config_changed = true;
                        reconcile = true;
                    }
                    Request::Reload => {
                        if let Some(path) = self.loaded_or_failed_path() {
                            let settings = self.host.settings_map().or_else(|| {
                                stored_settings(timer).and_then(|stored| stored.settings_for(&path))
                            });
                            self.host.load(path, settings, timer);
                        }
                        reconcile = false;
                    }
                }
            }
            if reconcile {
                self.reconcile(timer, config, &mut outcome);
            }
            self.initialized = true;

            for (key, value) in std::mem::take(&mut self.setting_changes) {
                self.host.set_setting(&key, value);
            }

            let poll = self.host.poll(timer);
            outcome.autosave = poll.autosave;
            for event in poll.events {
                outcome.status = Some(match event {
                    HostEvent::Loaded(path) => {
                        format!("Auto splitter {} running.", file_name(&path))
                    }
                    HostEvent::Failed(path, error) => {
                        format!("Auto splitter {} failed: {error}", file_name(&path))
                    }
                });
            }

            self.seen_run_path = timer.run.file_path.clone();
            self.seen_run_settings = timer.run.auto_splitter_settings.clone();
            outcome
        }

        /// Picks the auto splitter for the current run after the run changed.
        fn reconcile(
            &mut self,
            timer: &mut Timer,
            config: &mut AutoSplitterConfig,
            outcome: &mut AutoSplitterOutcome,
        ) {
            let stored = stored_settings(timer);
            let remembered = timer
                .run
                .file_path
                .as_deref()
                .and_then(|run_path| config.path_for_run(run_path))
                .cloned();
            let desired = remembered.clone().or_else(|| {
                stored
                    .as_ref()
                    .and_then(|stored| stored.script_path.as_deref())
                    .map(PathBuf::from)
            });
            // Remember a script path that came from the split file (or from
            // a "Save As" of a run with an auto splitter) for this run.
            if let (Some(run_path), Some(path), None) =
                (&timer.run.file_path, &desired, &remembered)
            {
                config
                    .runs
                    .insert(AutoSplitterConfig::run_key(run_path), path.clone());
                outcome.config_changed = true;
            }
            let desired = desired.filter(|_| config.enabled);
            let settings = desired
                .as_deref()
                .and_then(|path| stored.as_ref()?.settings_for(path));

            match desired {
                Some(path) if self.host.active_path() == Some(path.as_path()) => {
                    // Same auto splitter, possibly a reopened split file.
                    if let Some(settings) = settings {
                        if self.host.settings_map().as_ref() != Some(&settings) {
                            self.host.set_settings_map(settings);
                        }
                    }
                }
                Some(path) => {
                    outcome.status = Some(format!("Loading auto splitter {}…", file_name(&path)));
                    self.host.load(path, settings, timer);
                }
                None => {
                    if self.host.active_path().is_some() {
                        self.host.unload();
                        outcome.status = Some("Auto splitter stopped.".to_string());
                    }
                }
            }
        }

        fn loaded_or_failed_path(&self) -> Option<PathBuf> {
            match self.host.status() {
                HostStatus::Unloaded => None,
                HostStatus::Loading(path)
                | HostStatus::Running(path)
                | HostStatus::Failed(path, _) => Some(path.to_path_buf()),
            }
        }

        /// Republishes the timer snapshot for the auto splitter. Call at the
        /// end of every frame, after all other timer mutations.
        pub fn publish(&mut self, timer: &Timer) {
            self.host.publish(timer);
        }

        fn status_text(&self) -> (String, Option<Color32>) {
            match self.host.status() {
                HostStatus::Unloaded => ("No auto splitter loaded".to_string(), None),
                HostStatus::Loading(path) => (format!("Loading {}…", file_name(path)), None),
                HostStatus::Running(path) => (
                    format!("Running {}", file_name(path)),
                    Some(Color32::LIGHT_GREEN),
                ),
                HostStatus::Failed(path, _) => (
                    format!("{} failed", file_name(path)),
                    Some(Color32::LIGHT_RED),
                ),
            }
        }

        fn open_load_dialog(&mut self) {
            let start = self.loaded_or_failed_path();
            self.file_dialog.open(
                FileDialogMode::Open,
                "Load Auto Splitter",
                start.as_deref(),
                "",
                &["wasm"],
            );
            self.dialog_purpose = Some(DialogPurpose::LoadAutoSplitter);
        }

        /// The "Auto Splitter" submenu of the context menu.
        pub fn menu(&mut self, ui: &mut Ui, config: &AutoSplitterConfig) {
            ui.menu_button("Auto Splitter", |ui| {
                let (status, color) = self.status_text();
                let mut text = RichText::new(status).small();
                if let Some(color) = color {
                    text = text.color(color);
                }
                ui.label(text);
                ui.separator();
                if ui.button("Load Auto Splitter…").clicked() {
                    self.open_load_dialog();
                    ui.close();
                }
                let mut enabled = config.enabled;
                if ui.checkbox(&mut enabled, "Enable Auto Splitter").changed() {
                    self.request(Request::SetEnabled(enabled));
                }
                let loaded = self.loaded_or_failed_path().is_some();
                if ui
                    .add_enabled(loaded, egui::Button::new("Auto Splitter Settings…"))
                    .clicked()
                {
                    self.window_open = true;
                    ui.close();
                }
                if ui
                    .add_enabled(loaded, egui::Button::new("Remove Auto Splitter"))
                    .clicked()
                {
                    self.request(Request::Remove);
                    ui.close();
                }
            });
        }

        pub fn window_open(&self) -> bool {
            self.window_open || self.file_dialog.open
        }

        /// Shows the auto splitter window and file dialog.
        pub fn show(&mut self, ctx: &Context, config: &AutoSplitterConfig) {
            if let Some(action) = self.file_dialog.show(ctx) {
                let purpose = self.dialog_purpose.take();
                if let FileDialogAction::Open(path) | FileDialogAction::Save(path) = action {
                    match purpose {
                        Some(DialogPurpose::LoadAutoSplitter) => self.request(Request::Load(path)),
                        Some(DialogPurpose::Setting(key)) => {
                            if let Some(wasi) = wasi_path::from_native(&path) {
                                self.setting_changes
                                    .push((key, Value::String(Arc::from(wasi))));
                                ctx.request_repaint();
                            }
                        }
                        None => {}
                    }
                }
            }
            if !self.window_open {
                return;
            }
            let mut open = true;
            egui::Window::new("Auto Splitter")
                .open(&mut open)
                .collapsible(false)
                .resizable(true)
                .default_width(380.0)
                .default_pos(egui::pos2(12.0, 12.0))
                .show(ctx, |ui| self.window_contents(ui, config));
            if !open {
                self.window_open = false;
            }
            // Settings widgets can change on every auto splitter tick.
            if self.window_open && matches!(self.host.status(), HostStatus::Loading(_)) {
                ctx.request_repaint_after(std::time::Duration::from_millis(100));
            }
        }

        fn window_contents(&mut self, ui: &mut Ui, config: &AutoSplitterConfig) {
            let (status, color) = self.status_text();
            ui.label(RichText::new(status).color(color.unwrap_or(ui.visuals().text_color())));
            if let Some(path) = self.loaded_or_failed_path() {
                ui.label(
                    RichText::new(path.display().to_string())
                        .small()
                        .color(ui.visuals().weak_text_color()),
                );
            }
            if let HostStatus::Failed(_, error) = self.host.status() {
                ui.colored_label(Color32::LIGHT_RED, error);
            }
            ui.horizontal(|ui| {
                if ui.button("Load…").clicked() {
                    self.open_load_dialog();
                }
                let loaded = self.loaded_or_failed_path().is_some();
                if ui
                    .add_enabled(loaded, egui::Button::new("Reload"))
                    .clicked()
                {
                    self.request(Request::Reload);
                }
                if ui
                    .add_enabled(loaded, egui::Button::new("Remove"))
                    .clicked()
                {
                    self.request(Request::Remove);
                }
                let mut enabled = config.enabled;
                if ui.checkbox(&mut enabled, "Enabled").changed() {
                    self.request(Request::SetEnabled(enabled));
                }
            });
            ui.separator();

            match (self.host.settings_widgets(), self.host.settings_map()) {
                (Some(widgets), Some(map)) if !widgets.is_empty() => {
                    egui::ScrollArea::vertical()
                        .id_salt("auto-splitter-settings")
                        .max_height(360.0)
                        .show(ui, |ui| {
                            let mut indent = 0;
                            for widget in widgets.iter() {
                                if let WidgetKind::Title { heading_level } = widget.kind {
                                    indent = heading_level as usize;
                                }
                                let level = if matches!(widget.kind, WidgetKind::Title { .. }) {
                                    indent
                                } else {
                                    indent + 1
                                };
                                ui.horizontal(|ui| {
                                    ui.add_space(level.min(8) as f32 * 12.0);
                                    self.widget(ui, widget, &map);
                                });
                            }
                        });
                }
                (Some(_), _) => {
                    ui.label("This auto splitter has no settings.");
                }
                _ => {
                    ui.label("Settings appear once the auto splitter is running.");
                }
            }

            let logs = self.host.logs();
            if !logs.is_empty() {
                ui.separator();
                egui::CollapsingHeader::new(format!("Log ({})", logs.len()))
                    .id_salt("auto-splitter-log")
                    .show(ui, |ui| {
                        egui::ScrollArea::vertical()
                            .id_salt("auto-splitter-log-scroll")
                            .max_height(160.0)
                            .stick_to_bottom(true)
                            .show(ui, |ui| {
                                for line in logs {
                                    let color = match line.severity {
                                        LogSeverity::Error => Color32::LIGHT_RED,
                                        LogSeverity::Warning => Color32::YELLOW,
                                        _ if line.from_auto_splitter => ui.visuals().text_color(),
                                        _ => ui.visuals().weak_text_color(),
                                    };
                                    ui.label(
                                        RichText::new(&line.text).small().monospace().color(color),
                                    );
                                }
                            });
                    });
            }
        }

        fn widget(&mut self, ui: &mut Ui, widget: &Widget, map: &Map) {
            let current = map.get(&widget.key);
            let response = match &widget.kind {
                WidgetKind::Title { heading_level } => {
                    let text = RichText::new(&*widget.description).strong();
                    Some(ui.label(if *heading_level == 0 {
                        text.heading()
                    } else {
                        text
                    }))
                }
                WidgetKind::Bool { default_value } => {
                    let mut value = current.and_then(Value::to_bool).unwrap_or(*default_value);
                    let response = ui.checkbox(&mut value, &*widget.description);
                    if response.changed() {
                        self.setting_changes
                            .push((widget.key.clone(), Value::Bool(value)));
                    }
                    Some(response)
                }
                WidgetKind::Choice {
                    default_option_key,
                    options,
                } => {
                    let selected = current
                        .and_then(Value::as_string)
                        .filter(|key| options.iter().any(|option| option.key == **key))
                        .cloned()
                        .unwrap_or_else(|| default_option_key.clone());
                    let selected_text = options
                        .iter()
                        .find(|option| option.key == selected)
                        .map(|option| option.description.to_string())
                        .unwrap_or_else(|| selected.to_string());
                    ui.label(&*widget.description);
                    let response =
                        egui::ComboBox::from_id_salt(("auto-splitter-choice", &*widget.key))
                            .selected_text(selected_text)
                            .show_ui(ui, |ui| {
                                for option in options.iter() {
                                    if ui
                                        .selectable_label(
                                            option.key == selected,
                                            &*option.description,
                                        )
                                        .clicked()
                                        && option.key != selected
                                    {
                                        self.setting_changes.push((
                                            widget.key.clone(),
                                            Value::String(option.key.clone()),
                                        ));
                                    }
                                }
                            });
                    Some(response.response)
                }
                WidgetKind::TextInput { default_value } => {
                    let mut value = current
                        .and_then(Value::as_string)
                        .unwrap_or(default_value)
                        .to_string();
                    ui.label(&*widget.description);
                    let response = ui.text_edit_singleline(&mut value);
                    if response.changed() {
                        self.setting_changes
                            .push((widget.key.clone(), Value::String(value.into())));
                    }
                    Some(response)
                }
                WidgetKind::FileSelect { filters } => {
                    let value = current
                        .and_then(Value::as_string)
                        .and_then(|wasi| wasi_path::to_native(wasi, true))
                        .map(|path| path.display().to_string())
                        .unwrap_or_else(|| "(none)".to_string());
                    ui.label(&*widget.description);
                    ui.label(RichText::new(value).small().monospace());
                    let response = ui.button("Browse…");
                    if response.clicked() {
                        // Only `*.ext` name patterns map onto the built-in
                        // dialog; anything else shows all files.
                        let extensions: Vec<&str> = filters
                            .iter()
                            .filter_map(|filter| match filter {
                                FileFilter::Name { pattern, .. } => Some(&**pattern),
                                FileFilter::MimeType(_) => None,
                            })
                            .flat_map(str::split_whitespace)
                            .filter_map(|pattern| pattern.strip_prefix("*."))
                            .filter(|extension| !extension.contains(['*', '?']))
                            .collect();
                        self.file_dialog.open(
                            FileDialogMode::Open,
                            widget.description.to_string(),
                            None,
                            "",
                            &extensions,
                        );
                        self.dialog_purpose = Some(DialogPurpose::Setting(widget.key.clone()));
                    }
                    Some(response)
                }
            };
            if let (Some(response), Some(tooltip)) = (response, &widget.tooltip) {
                response.on_hover_text(&**tooltip);
            }
        }
    }

    fn stored_settings(timer: &Timer) -> Option<StoredSettings> {
        let xml = timer.run.auto_splitter_settings.as_deref()?;
        match StoredSettings::parse(xml) {
            Ok(stored) => Some(stored),
            Err(error) => {
                log::warn!("could not parse the stored auto splitter settings: {error}");
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::config::AppConfig;

    #[test]
    fn auto_splitter_config_roundtrips_through_toml() {
        let mut config = AppConfig::default();
        assert!(config.auto_splitter.enabled);
        config.auto_splitter.enabled = false;
        config.auto_splitter.runs.insert(
            r"C:\Runs\Game [Any%].lss".to_string(),
            PathBuf::from("/splitters/game.wasm"),
        );
        let text = toml::to_string_pretty(&config).unwrap();
        let parsed: AppConfig = toml::from_str(&text).unwrap();
        assert!(!parsed.auto_splitter.enabled);
        assert_eq!(parsed.auto_splitter.runs, config.auto_splitter.runs);

        // Settings files from before auto splitting keep working.
        let old: AppConfig = toml::from_str("[server]\nport = 1234\n").unwrap();
        assert!(old.auto_splitter.enabled);
        assert!(old.auto_splitter.runs.is_empty());
    }
}
