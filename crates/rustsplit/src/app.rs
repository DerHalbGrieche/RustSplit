//! The main RustSplit application window and event loop.

use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use egui::{
    Align2, Color32, Context, CursorIcon, Key, PointerButton, RichText, Sense, Ui,
    ViewportCommand, WindowLevel,
};
use livesplit_core::{load_run, save_run, Run, Timer, TimerPhase, TimingMethod};
use livesplit_server::{handle_command, ServerCommand, ServerHandle};
use livesplit_ui::{
    configure_fonts, load_layout, save_layout, Layout, LayoutMode, Renderer,
};

use crate::config::AppConfig;
use crate::file_dialog::{FileDialog, FileDialogAction, FileDialogMode};
use crate::hotkeys::{GlobalHotkeys, HotkeyAction};
use crate::layout_editor::{LayoutEditor, LayoutEditorEvent};
use crate::run_editor::{RunEditor, RunEditorEvent};

#[derive(Clone, Debug, PartialEq)]
enum Action {
    StartSplit,
    Reset,
    ResetAndSetPb,
    Undo,
    Skip,
    Pause,
    UndoAllPauses,
    SwitchComparisonNext,
    SwitchComparisonPrevious,
    ToggleTimingMethod,
    SetComparison(String),
    SetTimingMethod(TimingMethod),
    NewRun,
    EditRun,
    OpenRun,
    OpenRunPath(PathBuf),
    SaveRun,
    SaveRunAs,
    EditLayout,
    OpenLayout,
    SaveLayout,
    SaveLayoutAs,
    DefaultLayout,
    ToggleAlwaysOnTop,
    ToggleServer,
    Settings,
    About,
    Quit,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum FilePurpose {
    OpenRun,
    SaveRun,
    OpenLayout,
    SaveLayout,
}

pub struct RustSplitApp {
    timer: Timer,
    layout: Layout,
    renderer: Renderer,
    config: AppConfig,
    config_path: PathBuf,
    run_editor: RunEditor,
    layout_editor: LayoutEditor,
    file_dialog: FileDialog,
    file_purpose: Option<FilePurpose>,
    settings_open: bool,
    settings_draft: Option<AppConfig>,
    about_open: bool,
    status: Option<(String, Instant)>,
    hotkeys: GlobalHotkeys,
    server: Option<ServerHandle>,
    server_error: Option<String>,
    pending_actions: Vec<Action>,
    last_window_size: Option<egui::Vec2>,
    last_window_pos: Option<egui::Pos2>,
    passthrough: bool,
    resizable: bool,
    last_title: String,
    dialog_expanded: bool,
    overlay_size_before_dialog: Option<egui::Vec2>,
}

impl RustSplitApp {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        timer: Timer,
        layout: Layout,
        config: AppConfig,
        config_path: PathBuf,
    ) -> Self {
        configure_fonts(&cc.egui_ctx);
        let mut visuals = egui::Visuals::dark();
        visuals.window_corner_radius = egui::CornerRadius::same(6);
        visuals.menu_corner_radius = egui::CornerRadius::same(4);
        cc.egui_ctx.set_visuals(visuals);

        let resizable = layout.settings.allow_resizing;
        let mut app = Self {
            timer,
            layout,
            renderer: Renderer::default(),
            config,
            config_path,
            run_editor: RunEditor::default(),
            layout_editor: LayoutEditor::default(),
            file_dialog: FileDialog::default(),
            file_purpose: None,
            settings_open: false,
            settings_draft: None,
            about_open: false,
            status: Some((
                "Right-click for the LiveSplit menu. Space starts/splits.".to_string(),
                Instant::now(),
            )),
            hotkeys: GlobalHotkeys::default(),
            server: None,
            server_error: None,
            pending_actions: Vec::new(),
            last_window_size: None,
            last_window_pos: None,
            passthrough: false,
            resizable,
            last_title: String::new(),
            dialog_expanded: false,
            overlay_size_before_dialog: None,
        };
        app.hotkeys.re_register(&app.config.hotkeys);
        if app.config.server.enabled && app.config.server.autostart {
            app.start_server();
        }
        app
    }

    fn set_status(&mut self, message: impl Into<String>) {
        self.status = Some((message.into(), Instant::now()));
    }

    fn save_config(&self) {
        if let Err(error) = self.config.save(&self.config_path) {
            log::warn!("could not save settings: {error}");
        }
    }

    fn apply_actions(&mut self, ctx: &Context) {
        let actions = std::mem::take(&mut self.pending_actions);
        for action in actions {
            self.apply_action(ctx, action);
        }
    }

    fn apply_action(&mut self, ctx: &Context, action: Action) {
        match action {
            Action::StartSplit => match self.timer.phase {
                TimerPhase::NotRunning => {
                    self.timer.start();
                }
                TimerPhase::Running | TimerPhase::Paused => {
                    self.timer.split();
                    if self.timer.phase == TimerPhase::Ended {
                        self.autosave_after_change();
                    }
                }
                TimerPhase::Ended => {}
            },
            Action::Reset => {
                if self.timer.phase != TimerPhase::NotRunning {
                    self.timer.reset();
                    if self.timer.run.file_path.is_some() {
                        self.set_status("Run reset.");
                    } else {
                        self.set_status("Run reset. Use Ctrl+S to save your splits.");
                    }
                    self.autosave_after_change();
                }
            }
            Action::ResetAndSetPb => {
                if self.timer.phase != TimerPhase::NotRunning {
                    self.timer.reset_and_set_attempt_as_pb();
                    self.set_status("Run reset and set as personal best.");
                    self.autosave_after_change();
                }
            }
            Action::Undo => self.timer.undo_split(),
            Action::Skip => self.timer.skip_split(),
            Action::Pause => {
                self.timer.pause();
                self.set_status(match self.timer.phase {
                    TimerPhase::Paused => "Paused.",
                    TimerPhase::Running => "Resumed.",
                    _ => "Timer running.",
                });
            }
            Action::UndoAllPauses => {
                self.timer.undo_all_pauses();
                self.set_status("Undid all pauses.");
            }
            Action::SwitchComparisonNext => self.timer.switch_comparison_next(),
            Action::SwitchComparisonPrevious => self.timer.switch_comparison_previous(),
            Action::ToggleTimingMethod => {
                self.timer.toggle_timing_method();
                self.set_status(format!(
                    "Timing method: {}",
                    self.timer.current_timing_method
                ));
            }
            Action::SetComparison(comparison) => self.timer.set_current_comparison(comparison),
            Action::SetTimingMethod(method) => {
                self.timer.set_timing_method(method);
                self.set_status(format!("Timing method: {method}"));
            }
            Action::NewRun => {
                let run = Run::with_segments("", "", ["Segment 1"]);
                self.timer.set_run(run);
                self.run_editor.open(&self.timer.run);
                self.set_status("New run created.");
            }
            Action::EditRun => {
                if self.timer.phase != TimerPhase::NotRunning {
                    self.set_status("Finish or reset the run before editing splits.");
                } else {
                    self.run_editor.open(&self.timer.run);
                }
            }
            Action::OpenRun => {
                let start = self
                    .timer
                    .run
                    .file_path
                    .clone()
                    .or_else(|| self.config.general.last_run.clone());
                self.file_dialog.open(
                    FileDialogMode::Open,
                    "Open Splits",
                    start.as_deref(),
                    "",
                    &["lss"],
                );
                self.file_purpose = Some(FilePurpose::OpenRun);
            }
            Action::OpenRunPath(path) => {
                let _ = self.open_run_path(ctx, &path);
            }
            Action::SaveRun => {
                if let Err(error) = self.save_current_run(false) {
                    self.set_status(format!("Could not save: {error}"));
                }
            }
            Action::SaveRunAs => {
                self.open_save_dialog_for_run();
            }
            Action::EditLayout => self.layout_editor.open(&self.layout),
            Action::OpenLayout => {
                let start = self
                    .layout
                    .file_path
                    .clone()
                    .or_else(|| self.config.general.last_layout.clone());
                self.file_dialog.open(
                    FileDialogMode::Open,
                    "Open Layout",
                    start.as_deref(),
                    "",
                    &["lsl"],
                );
                self.file_purpose = Some(FilePurpose::OpenLayout);
            }
            Action::SaveLayout => {
                if let Err(error) = self.save_current_layout(false) {
                    self.set_status(format!("Could not save layout: {error}"));
                }
            }
            Action::SaveLayoutAs => {
                self.file_dialog.open(
                    FileDialogMode::Save,
                    "Save Layout As",
                    self.layout.file_path.as_deref(),
                    format!("{}.lsl", default_layout_name(&self.layout)),
                    &["lsl"],
                );
                self.file_purpose = Some(FilePurpose::SaveLayout);
            }
            Action::DefaultLayout => {
                self.layout = Layout::parse_default().unwrap_or_else(|_| Layout::livesplit_default());
                self.layout.file_path = None;
                self.apply_layout_change(ctx);
                self.set_status("Loaded the default layout.");
            }
            Action::ToggleAlwaysOnTop => {
                self.layout.settings.always_on_top = !self.layout.settings.always_on_top;
                self.apply_layout_change(ctx);
            }
            Action::ToggleServer => {
                if self.server.is_some() {
                    self.stop_server();
                } else {
                    self.start_server();
                }
            }
            Action::Settings => {
                self.settings_draft = Some(self.config.clone());
                self.settings_open = true;
            }
            Action::About => self.about_open = true,
            Action::Quit => {
                self.save_config();
                ctx.send_viewport_cmd(ViewportCommand::Close);
            }
        }
    }

    fn open_run_path(&mut self, _ctx: &Context, path: &Path) -> bool {
        match load_run(path) {
            Ok(run) => {
                self.timer.set_run(run);
                self.timer.run.has_changed = false;
                self.config.remember_run(path);
                self.save_config();
                self.load_linked_layout(path);
                self.set_status(format!("Opened {}", path.display()));
                true
            }
            Err(error) => {
                self.set_status(format!("Could not open splits: {error}"));
                false
            }
        }
    }

    fn open_layout_path(&mut self, ctx: &Context, path: &Path) -> bool {
        match load_layout(path) {
            Ok(layout) => {
                self.layout = layout;
                self.layout.file_path = Some(path.to_path_buf());
                self.config.remember_layout(path);
                self.apply_layout_change(ctx);
                self.set_status(format!("Opened layout {}", path.display()));
                true
            }
            Err(error) => {
                self.set_status(format!("Could not open layout: {error}"));
                false
            }
        }
    }

    fn autosave_after_change(&mut self) {
        if !self.config.general.autosave {
            return;
        }
        if self.timer.run.file_path.is_some() {
            if let Err(error) = self.save_current_run(true) {
                self.set_status(format!("Could not save: {error}"));
            }
        }
    }

    fn save_current_run(&mut self, silent: bool) -> Result<(), String> {
        let Some(path) = self.timer.run.file_path.clone() else {
            if !silent {
                self.open_save_dialog_for_run();
            }
            return Ok(());
        };
        match save_run(&self.timer.run, &path) {
            Ok(()) => {
                self.timer.run.has_changed = false;
                self.config.remember_run(&path);
                self.save_config();
                if !silent {
                    self.set_status(format!("Saved {}", path.display()));
                }
                Ok(())
            }
            Err(error) => Err(error.to_string()),
        }
    }

    fn open_save_dialog_for_run(&mut self) {
        let file_name = if self.timer.run.extended_name().trim().is_empty() {
            "Splits.lss".to_string()
        } else {
            format!("{}.lss", self.timer.run.extended_file_name())
        };
        self.file_dialog.open(
            FileDialogMode::Save,
            "Save Splits As",
            self.timer
                .run
                .file_path
                .as_deref()
                .or(self.config.general.last_run.as_deref()),
            file_name,
            &["lss"],
        );
        self.file_purpose = Some(FilePurpose::SaveRun);
    }

    fn save_current_layout(&mut self, silent: bool) -> Result<(), String> {
        let Some(path) = self.layout.file_path.clone() else {
            if !silent {
                self.file_dialog.open(
                    FileDialogMode::Save,
                    "Save Layout As",
                    None,
                    format!("{}.lsl", default_layout_name(&self.layout)),
                    &["lsl"],
                );
                self.file_purpose = Some(FilePurpose::SaveLayout);
            }
            return Ok(());
        };
        save_layout(&self.layout, &path).map_err(|error| error.to_string())?;
        self.layout.has_changed = false;
        self.config.remember_layout(&path);
        self.save_config();
        if !silent {
            self.set_status(format!("Saved layout {}", path.display()));
        }
        Ok(())
    }

    fn apply_layout_change(&mut self, ctx: &Context) {
        ctx.send_viewport_cmd(ViewportCommand::WindowLevel(
            if self.layout.settings.always_on_top {
                WindowLevel::AlwaysOnTop
            } else {
                WindowLevel::Normal
            },
        ));
        if self.resizable != self.layout.settings.allow_resizing {
            self.resizable = self.layout.settings.allow_resizing;
            ctx.send_viewport_cmd(ViewportCommand::Resizable(self.resizable));
        }
        if self.config.general.autosave_layout && self.layout.file_path.is_some() {
            let _ = self.save_current_layout(true);
        }
        self.save_config();
    }

    fn handle_file_action(&mut self, ctx: &Context, action: FileDialogAction) {
        let purpose = self.file_purpose.take();
        let (FileDialogAction::Open(path) | FileDialogAction::Save(path)) = action else {
            return;
        };
        match purpose {
            Some(FilePurpose::OpenRun) => {
                let _ = self.open_run_path(ctx, &path);
            }
            Some(FilePurpose::SaveRun) => {
                self.timer.run.file_path = Some(path.clone());
                match save_run(&self.timer.run, &path) {
                    Ok(()) => {
                        self.timer.run.has_changed = false;
                        self.config.remember_run(&path);
                        self.save_config();
                        self.set_status(format!("Saved {}", path.display()));
                    }
                    Err(error) => self.set_status(format!("Could not save: {error}")),
                }
            }
            Some(FilePurpose::OpenLayout) => {
                let _ = self.open_layout_path(ctx, &path);
            }
            Some(FilePurpose::SaveLayout) => {
                self.layout.file_path = Some(path.clone());
                match save_layout(&self.layout, &path) {
                    Ok(()) => {
                        self.layout.has_changed = false;
                        self.config.remember_layout(&path);
                        self.save_config();
                        self.set_status(format!("Saved layout {}", path.display()));
                    }
                    Err(error) => self.set_status(format!("Could not save layout: {error}")),
                }
            }
            None => {}
        }
    }

    fn load_linked_layout(&mut self, run_path: &Path) {
        let Some(link) = self.timer.run.layout_path.clone() else {
            return;
        };
        let path = if link == "?default" {
            None
        } else {
            let candidate = PathBuf::from(&link);
            if candidate.is_absolute() {
                Some(candidate)
            } else {
                Some(run_path.parent().unwrap_or(Path::new(".")).join(candidate))
            }
        };
        if let Some(path) = path {
            if let Ok(layout) = load_layout(&path) {
                self.layout = layout;
                self.layout.file_path = Some(path);
                return;
            }
        }
        if link == "?default" {
            self.layout = Layout::parse_default().unwrap_or_else(|_| Layout::livesplit_default());
        }
    }

    fn handle_editor_event(&mut self, ctx: &Context, event: RunEditorEvent) {
        match event {
            RunEditorEvent::None | RunEditorEvent::Cancel => {}
            RunEditorEvent::Apply | RunEditorEvent::ApplyAndClose => {
                if let Some(run) = self.run_editor.draft_cloned() {
                    let path = run.file_path.clone();
                    self.timer.set_run(run);
                    self.timer.run.file_path = path;
                    self.set_status("Splits applied.");
                    if self.config.general.autosave && self.timer.run.file_path.is_some() {
                        let _ = self.save_current_run(true);
                    }
                }
                if event == RunEditorEvent::ApplyAndClose {
                    self.run_editor.close();
                }
            }
            RunEditorEvent::Save => {
                if let Some(mut run) = self.run_editor.draft_cloned() {
                    run.file_path = self.timer.run.file_path.clone();
                    self.timer.set_run(run);
                    if self.timer.run.file_path.is_some() {
                        let _ = self.save_current_run(false);
                    } else {
                        self.open_save_dialog_for_run();
                    }
                }
            }
            RunEditorEvent::SaveAs => {
                if let Some(run) = self.run_editor.draft_cloned() {
                    self.timer.set_run(run);
                }
                self.open_save_dialog_for_run();
            }
        }
        let _ = ctx;
    }

    fn handle_layout_editor_event(&mut self, ctx: &Context, event: LayoutEditorEvent) {
        match event {
            LayoutEditorEvent::None | LayoutEditorEvent::Cancel => {}
            LayoutEditorEvent::Apply | LayoutEditorEvent::ApplyAndClose => {
                if let Some(layout) = self.layout_editor.draft_cloned() {
                    let path = layout.file_path.clone();
                    self.layout = layout;
                    self.layout.file_path = path;
                    self.apply_layout_change(ctx);
                    self.set_status("Layout applied.");
                }
                if event == LayoutEditorEvent::ApplyAndClose {
                    self.layout_editor.close();
                }
            }
            LayoutEditorEvent::Save => {
                if let Some(mut layout) = self.layout_editor.draft_cloned() {
                    layout.file_path = self.layout.file_path.clone();
                    self.layout = layout;
                    self.apply_layout_change(ctx);
                    if self.layout.file_path.is_some() {
                        if let Err(error) = self.save_current_layout(false) {
                            self.set_status(format!("Could not save layout: {error}"));
                        }
                    } else {
                        self.file_dialog.open(
                            FileDialogMode::Save,
                            "Save Layout As",
                            None,
                            format!("{}.lsl", default_layout_name(&self.layout)),
                            &["lsl"],
                        );
                        self.file_purpose = Some(FilePurpose::SaveLayout);
                    }
                }
            }
            LayoutEditorEvent::SaveAs => {
                if let Some(mut layout) = self.layout_editor.draft_cloned() {
                    layout.file_path = None;
                    self.layout = layout;
                    self.apply_layout_change(ctx);
                }
                self.file_dialog.open(
                    FileDialogMode::Save,
                    "Save Layout As",
                    None,
                    format!("{}.lsl", default_layout_name(&self.layout)),
                    &["lsl"],
                );
                self.file_purpose = Some(FilePurpose::SaveLayout);
            }
            LayoutEditorEvent::ResetDefault => {
                self.layout = Layout::parse_default().unwrap_or_else(|_| Layout::livesplit_default());
                self.apply_layout_change(ctx);
            }
        }
    }

    fn handle_local_shortcuts(&mut self, ctx: &Context) {
        if ctx.egui_wants_keyboard_input() || ctx.any_popup_open() {
            return;
        }
        let (ctrl, shift, alt) = ctx.input(|input| {
            (
                input.modifiers.ctrl || input.modifiers.command,
                input.modifiers.shift,
                input.modifiers.alt,
            )
        });
        if ctrl {
            if ctx.input(|input| input.key_pressed(Key::Q)) {
                self.pending_actions.push(Action::Quit);
            }
            if ctx.input(|input| input.key_pressed(Key::S)) {
                self.pending_actions.push(if shift {
                    Action::SaveRunAs
                } else {
                    Action::SaveRun
                });
            }
            if ctx.input(|input| input.key_pressed(Key::O)) {
                self.pending_actions.push(Action::OpenRun);
            }
            if ctx.input(|input| input.key_pressed(Key::E)) {
                self.pending_actions.push(Action::EditRun);
            }
            if ctx.input(|input| input.key_pressed(Key::L)) {
                self.pending_actions.push(Action::EditLayout);
            }
            return;
        }
        if alt {
            return;
        }
        if ctx.input(|input| input.key_pressed(Key::Space)) {
            self.pending_actions.push(Action::StartSplit);
        }
        if ctx.input(|input| input.key_pressed(Key::R)) {
            self.pending_actions.push(Action::Reset);
        }
        if ctx.input(|input| input.key_pressed(Key::Backspace)) {
            self.pending_actions.push(Action::Undo);
        }
        if ctx.input(|input| input.key_pressed(Key::Delete)) {
            self.pending_actions.push(Action::Skip);
        }
        if ctx.input(|input| input.key_pressed(Key::P)) {
            self.pending_actions.push(Action::Pause);
        }
        if ctx.input(|input| input.key_pressed(Key::U)) {
            self.pending_actions.push(Action::UndoAllPauses);
        }
        if ctx.input(|input| input.key_pressed(Key::G)) {
            self.pending_actions.push(Action::ToggleTimingMethod);
        }
        if ctx.input(|input| input.key_pressed(Key::F1)) {
            self.pending_actions.push(Action::Settings);
        }
        if ctx.input(|input| input.key_pressed(Key::Tab)) {
            self.pending_actions.push(if shift {
                Action::SwitchComparisonPrevious
            } else {
                Action::SwitchComparisonNext
            });
        }
    }

    fn poll_global_hotkeys(&mut self) {
        for action in self.hotkeys.poll() {
            let action = match action {
                HotkeyAction::StartSplit => Action::StartSplit,
                HotkeyAction::Reset => Action::Reset,
                HotkeyAction::Undo => Action::Undo,
                HotkeyAction::Skip => Action::Skip,
                HotkeyAction::Pause => Action::Pause,
                HotkeyAction::UndoAllPauses => Action::UndoAllPauses,
                HotkeyAction::SwitchComparisonNext => Action::SwitchComparisonNext,
                HotkeyAction::SwitchComparisonPrevious => Action::SwitchComparisonPrevious,
                HotkeyAction::ToggleTimingMethod => Action::ToggleTimingMethod,
                HotkeyAction::EditRun => Action::EditRun,
                HotkeyAction::EditLayout => Action::EditLayout,
            };
            self.pending_actions.push(action);
        }
    }

    fn start_server(&mut self) {
        if self.server.is_some() {
            return;
        }
        let address = self
            .config
            .server
            .bind_address
            .parse::<IpAddr>()
            .unwrap_or(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST));
        let socket = SocketAddr::new(address, self.config.server.port);
        match ServerHandle::start_with_addr(socket) {
            Ok(server) => {
                self.config.server.enabled = true;
                self.server_error = None;
                self.set_status(format!("LiveSplit Server listening on port {}", server.port()));
                self.server = Some(server);
                self.save_config();
            }
            Err(error) => {
                self.server_error = Some(error.to_string());
                self.set_status(format!("Could not start server: {error}"));
            }
        }
    }

    fn stop_server(&mut self) {
        if let Some(server) = self.server.take() {
            server.stop();
            self.set_status("LiveSplit Server stopped.");
        }
        self.config.server.enabled = false;
        self.save_config();
    }

    fn poll_server(&mut self, ctx: &Context) {
        loop {
            let request = {
                let Some(server) = &self.server else {
                    break;
                };
                server.try_recv()
            };
            let Some(request) = request else {
                break;
            };
            match &request.command {
                ServerCommand::SwitchSplits(path) => {
                    let path = PathBuf::from(path);
                    let ok = self.open_run_path(ctx, &path);
                    request.respond(Some(if ok { "True" } else { "False" }.to_string()));
                    continue;
                }
                ServerCommand::SwitchLayout(path) => {
                    let path = PathBuf::from(path);
                    let ok = self.open_layout_path(ctx, &path);
                    request.respond(Some(if ok { "True" } else { "False" }.to_string()));
                    continue;
                }
                _ => {}
            }
            let autosave = server_command_mutates_run(&request.command);
            let outcome = handle_command(&mut self.timer, &request.command);
            request.respond(outcome.response.clone());
            if outcome.save {
                let _ = self.save_current_run(false);
            }
            if autosave {
                self.autosave_after_change();
            }
            if outcome.quit {
                self.save_config();
                std::process::exit(0);
            }
        }
    }

    fn show_settings(&mut self, ctx: &Context) {
        if !self.settings_open {
            return;
        }
        let Some(draft) = self.settings_draft.as_mut() else {
            self.settings_open = false;
            return;
        };
        let mut open = self.settings_open;
        let mut apply = false;
        let hotkey_error = self.hotkeys.error.clone();
        let server_error = self.server_error.clone();
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
                ui.checkbox(&mut draft.hotkeys.enabled, "Enable global hotkeys (Linux: X11 only)");
                if let Some(error) = &hotkey_error {
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
                if let Some(error) = &server_error {
                    ui.colored_label(Color32::LIGHT_RED, error);
                }
                ui.separator();
                ui.heading("Behavior");
                ui.checkbox(&mut draft.general.autosave, "Autosave splits on reset/finish");
                ui.checkbox(
                    &mut draft.general.autosave_layout,
                    "Autosave layout after applying",
                );
                if ui.button("Apply").clicked() {
                    apply = true;
                }
                ui.separator();
                ui.label(
                    RichText::new("RustSplit — a native Rust rewrite of LiveSplit")
                        .small()
                        .color(ui.visuals().weak_text_color()),
                );
                    });
            });
        self.settings_open = open;
        if apply || !open {
            let old_config = self.config.clone();
            if let Some(draft) = self.settings_draft.take() {
                self.config = draft;
            }
            self.hotkeys.re_register(&self.config.hotkeys);
            if self.config.server.port != old_config.server.port
                || self.config.server.enabled != old_config.server.enabled
                || self.config.server.bind_address != old_config.server.bind_address
            {
                let wanted = self.config.server.enabled;
                self.stop_server();
                if wanted {
                    self.start_server();
                }
            }
            self.save_config();
            if !self.settings_open {
                self.settings_draft = None;
            }
        }
    }

    fn show_about(&mut self, ctx: &Context) {
        if !self.about_open {
            return;
        }
        let mut open = self.about_open;
        egui::Window::new("About RustSplit")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_pos(egui::pos2(12.0, 12.0))
            .show(ctx, |ui| {
                ui.heading("RustSplit");
                ui.label(format!("Version {}", env!("CARGO_PKG_VERSION")));
                ui.label("A native Rust rewrite of LiveSplit, inspired by the original C# application and livesplit-core.");
                ui.separator();
                ui.label("Timer semantics, comparisons and .lss files are compatible with LiveSplit.");
                ui.hyperlink_to("LiveSplit", "https://livesplit.org");
                ui.hyperlink_to("livesplit-core", "https://github.com/LiveSplit/livesplit-core");
                ui.separator();
                ui.label("Licensed under the MIT license.");
            });
        self.about_open = open;
    }

    fn show_file_dialog(&mut self, ctx: &Context) {
        let action = self.file_dialog.show(ctx);
        if let Some(action) = action {
            if action == FileDialogAction::Cancel {
                self.file_purpose = None;
            } else {
                self.handle_file_action(ctx, action);
            }
        }
    }

    fn handle_dropped_files(&mut self, ctx: &Context) {
        let dropped = ctx.input(|input| input.raw.dropped_files.clone());
        for file in dropped {
            let path = file.path().to_path_buf();
            match path
                .extension()
                .and_then(|extension| extension.to_str())
                .map(|extension| extension.to_ascii_lowercase())
                .as_deref()
            {
                Some("lss") => {
                    let _ = self.open_run_path(ctx, &path);
                }
                Some("lsl") => {
                    let _ = self.open_layout_path(ctx, &path);
                }
                _ => self.set_status("Drop an .lss split file or .lsl layout file."),
            }
        }
    }

    fn draw_context_menu(&mut self, ui: &mut Ui) {
        let mut actions = Vec::new();
        ui.set_min_width(240.0);
        if ui.button("Start / Split        Space").clicked() {
            actions.push(Action::StartSplit);
            ui.close();
        }
        if ui.button("Pause / Resume       P").clicked() {
            actions.push(Action::Pause);
            ui.close();
        }
        if ui.button("Undo Split           Backspace").clicked() {
            actions.push(Action::Undo);
            ui.close();
        }
        if ui.button("Skip Split           Delete").clicked() {
            actions.push(Action::Skip);
            ui.close();
        }
        ui.separator();
        if ui.button("Reset                R").clicked() {
            actions.push(Action::Reset);
            ui.close();
        }
        if ui.button("Reset and Set as Personal Best").clicked() {
            actions.push(Action::ResetAndSetPb);
            ui.close();
        }
        if ui.button("Undo All Pauses      U").clicked() {
            actions.push(Action::UndoAllPauses);
            ui.close();
        }
        ui.separator();
        ui.menu_button("Comparison", |ui| {
            let comparisons = self.timer.run.comparisons();
            for comparison in comparisons {
                let selected = comparison == self.timer.current_comparison;
                if ui.selectable_label(selected, &comparison).clicked() {
                    actions.push(Action::SetComparison(comparison));
                    ui.close();
                }
            }
        });
        ui.menu_button("Timing Method", |ui| {
            for method in [TimingMethod::RealTime, TimingMethod::GameTime] {
                let selected = method == self.timer.current_timing_method;
                if ui.selectable_label(selected, method.to_string()).clicked() {
                    actions.push(Action::SetTimingMethod(method));
                    ui.close();
                }
            }
        });
        ui.separator();
        if ui.button("Open Splits…         Ctrl+O").clicked() {
            actions.push(Action::OpenRun);
            ui.close();
        }
        if !self.config.general.recent_runs.is_empty() {
            ui.menu_button("Open Recent", |ui| {
                for path in self.config.general.recent_runs.clone() {
                    let label = path
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_else(|| path.display().to_string());
                    if ui
                        .button(label)
                        .on_hover_text(path.display().to_string())
                        .clicked()
                    {
                        actions.push(Action::OpenRunPath(path));
                        ui.close();
                    }
                }
            });
        }
        if ui.button("New Splits").clicked() {
            actions.push(Action::NewRun);
            ui.close();
        }
        if ui.button("Edit Splits…         Ctrl+E").clicked() {
            actions.push(Action::EditRun);
            ui.close();
        }
        if ui.button("Save Splits          Ctrl+S").clicked() {
            actions.push(Action::SaveRun);
            ui.close();
        }
        if ui.button("Save Splits As…").clicked() {
            actions.push(Action::SaveRunAs);
            ui.close();
        }
        ui.separator();
        ui.menu_button("Layout", |ui| {
            if ui.button("Edit Layout…         Ctrl+L").clicked() {
                actions.push(Action::EditLayout);
                ui.close();
            }
            if ui.button("Open Layout…").clicked() {
                actions.push(Action::OpenLayout);
                ui.close();
            }
            if ui.button("Save Layout").clicked() {
                actions.push(Action::SaveLayout);
                ui.close();
            }
            if ui.button("Save Layout As…").clicked() {
                actions.push(Action::SaveLayoutAs);
                ui.close();
            }
            if ui.button("Load Default Layout").clicked() {
                actions.push(Action::DefaultLayout);
                ui.close();
            }
        });
        let mut always_on_top = self.layout.settings.always_on_top;
        if ui.checkbox(&mut always_on_top, "Always on Top").changed() {
            actions.push(Action::ToggleAlwaysOnTop);
        }
        ui.separator();
        if ui.button("Settings…            F1").clicked() {
            actions.push(Action::Settings);
            ui.close();
        }
        if ui
            .button(if self.server.is_some() {
                "Stop LiveSplit Server"
            } else {
                "Start LiveSplit Server"
            })
            .clicked()
        {
            actions.push(Action::ToggleServer);
            ui.close();
        }
        if ui.button("About RustSplit").clicked() {
            actions.push(Action::About);
            ui.close();
        }
        ui.separator();
        if ui.button("Quit").clicked() {
            actions.push(Action::Quit);
            ui.close();
        }
        self.pending_actions.extend(actions);
    }

    fn draw_overlay_ui(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let rect = ui.max_rect();
        self.renderer.draw(ui, &self.layout, &self.timer);

        // Whole-window drag support.  Buttons and dialogs sit above this
        // interaction in egui's layer order.
        if self.layout.settings.allow_moving {
            let response = ui.interact(rect, ui.scope_id().with("rustsplit-drag"), Sense::click_and_drag());
            if response.drag_started_by(PointerButton::Primary)
                && !self.layout_editor.open
                && !self.run_editor.open
            {
                ctx.send_viewport_cmd(ViewportCommand::StartDrag);
            }
            response.context_menu(|ui| self.draw_context_menu(ui));
        } else {
            let response = ui.interact(rect, ui.scope_id().with("rustsplit-menu"), Sense::click());
            response.context_menu(|ui| self.draw_context_menu(ui));
        }

        // Resize grip in the bottom-right corner.
        if self.layout.settings.allow_resizing && !self.layout_editor.open {
            let grip = egui::Rect::from_min_size(
                rect.right_bottom() - egui::vec2(18.0, 18.0),
                egui::vec2(18.0, 18.0),
            );
            let response = ui.interact(grip, ui.scope_id().with("rustsplit-resize"), Sense::drag());
            if response.hovered() {
                ctx.set_cursor_icon(CursorIcon::ResizeSouthEast);
            }
            ui.painter().add(egui::Shape::convex_polygon(
                vec![
                    grip.right_bottom(),
                    grip.right_bottom() - egui::vec2(12.0, 0.0),
                    grip.right_bottom() - egui::vec2(0.0, 12.0),
                ],
                Color32::from_white_alpha(60),
                egui::Stroke::NONE,
            ));
            if response.drag_started() {
                ctx.send_viewport_cmd(ViewportCommand::BeginResize(
                    egui::ResizeDirection::SouthEast,
                ));
            }
        }

        if let Some((message, time)) = &self.status {
            if time.elapsed() < Duration::from_secs(5) {
                let pos = rect.left_bottom() + egui::vec2(10.0, -10.0);
                ui.painter().text(
                    pos,
                    Align2::LEFT_BOTTOM,
                    message,
                    egui::FontId::proportional(13.0),
                    Color32::from_white_alpha(220),
                );
            }
        }

        // Mouse pass-through while running.
        let hotkeys_ready = self.config.hotkeys.enabled && self.hotkeys.error.is_none();
        let wanted_passthrough = self.layout.settings.mouse_pass_through_while_running
            && self.timer.phase == TimerPhase::Running
            && (hotkeys_ready || self.server.is_some());
        if wanted_passthrough != self.passthrough {
            self.passthrough = wanted_passthrough;
            ctx.send_viewport_cmd(ViewportCommand::MousePassthrough(wanted_passthrough));
        }
    }

    fn any_modal_open(&self) -> bool {
        self.run_editor.open
            || self.layout_editor.open
            || self.file_dialog.open
            || self.settings_open
            || self.about_open
    }

    fn ensure_dialog_space(&mut self, ctx: &Context) {
        if self.dialog_expanded {
            return;
        }
        let current = ctx.input(|input| input.viewport().inner_rect.map(|rect| rect.size()));
        let monitor = ctx
            .input(|input| input.viewport().monitor_size)
            .unwrap_or_else(|| egui::vec2(1920.0, 1080.0));
        let target = egui::vec2(
            (monitor.x - 24.0).min(1060.0).max(640.0),
            (monitor.y - 48.0).min(820.0).max(480.0),
        );
        let needs_resize = current.is_none_or(|size| size.x < target.x || size.y < target.y);
        if needs_resize {
            self.overlay_size_before_dialog = current;
            ctx.send_viewport_cmd(ViewportCommand::InnerSize(target));
        }
        self.dialog_expanded = true;
    }

    fn restore_overlay_size(&mut self, ctx: &Context) {
        if !self.dialog_expanded {
            return;
        }
        if let Some(size) = self.overlay_size_before_dialog.take() {
            ctx.send_viewport_cmd(ViewportCommand::InnerSize(size));
        }
        self.dialog_expanded = false;
    }

    fn update_window_metadata(&mut self, ctx: &Context) {
        let title = if self.timer.run.extended_name().trim().is_empty() {
            "RustSplit".to_string()
        } else {
            format!("{} — RustSplit", self.timer.run.extended_name())
        };
        if title != self.last_title {
            self.last_title = title.clone();
            ctx.send_viewport_cmd(ViewportCommand::Title(title));
        }
        if let Some(rect) = ctx.input(|input| input.viewport().inner_rect) {
            self.last_window_size = Some(rect.size());
            self.last_window_pos = Some(rect.min);
        }
    }

    fn backup_current_run(&mut self) {
        let directory = crate::config::data_dir().join("backups");
        if std::fs::create_dir_all(&directory).is_err() {
            return;
        }
        let name = if self.timer.run.extended_file_name().trim().is_empty() {
            "Untitled".to_string()
        } else {
            self.timer.run.extended_file_name()
        };
        let timestamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
        let path = directory.join(format!("{name}-{timestamp}.lss"));
        if let Err(error) = save_run(&self.timer.run, &path) {
            log::warn!("could not write backup {}: {error}", path.display());
        } else {
            log::info!("saved backup to {}", path.display());
        }
    }

    fn store_window_geometry(&mut self) {
        if let Some(size) = self.last_window_size {
            self.config.window.width = Some(size.x);
            self.config.window.height = Some(size.y);
        }
        if let Some(pos) = self.last_window_pos {
            self.config.window.x = Some(pos.x);
            self.config.window.y = Some(pos.y);
        }
    }
}

impl eframe::App for RustSplitApp {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.poll_global_hotkeys();
        self.poll_server(&ctx);
        self.handle_dropped_files(&ctx);

        let modal_open = self.run_editor.open
            || self.layout_editor.open
            || self.file_dialog.open
            || self.settings_open
            || self.about_open;
        if !modal_open {
            self.handle_local_shortcuts(&ctx);
        }
        self.apply_actions(&ctx);

        if self.any_modal_open() {
            self.ensure_dialog_space(&ctx);
        } else {
            self.restore_overlay_size(&ctx);
        }

        self.draw_overlay_ui(ui);

        let event = self.run_editor.show(&ctx, &self.config);
        self.handle_editor_event(&ctx, event);
        let event = self.layout_editor.show(&ctx, &self.config);
        self.handle_layout_editor_event(&ctx, event);
        self.show_settings(&ctx);
        self.show_about(&ctx);
        self.show_file_dialog(&ctx);
        self.update_window_metadata(&ctx);
        self.store_window_geometry();

        let repaint = if self.server.is_some() {
            // Poll the remote-control queue promptly even while the timer is
            // idle.  Without this the server responses can feel sluggish
            // because egui only repaints every 250 ms when nothing changes.
            Duration::from_millis(33)
        } else {
            match self.timer.phase {
                TimerPhase::Running => Duration::from_millis(16),
                TimerPhase::Paused => Duration::from_millis(100),
                _ => Duration::from_millis(250),
            }
        };
        ctx.request_repaint_after(repaint);
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.store_window_geometry();
        self.save_config();
        if self.config.general.autosave && self.timer.run.has_changed {
            if self.timer.run.file_path.is_some() {
                let _ = self.save_current_run(true);
            } else {
                self.backup_current_run();
            }
        }
        if let Some(server) = self.server.take() {
            server.stop();
        }
    }
}

fn server_command_mutates_run(command: &ServerCommand) -> bool {
    matches!(
        command,
        ServerCommand::StartOrSplit
            | ServerCommand::Start
            | ServerCommand::Split
            | ServerCommand::Undo
            | ServerCommand::Skip
            | ServerCommand::Reset
            | ServerCommand::ResetAndSetAttemptAsPb
            | ServerCommand::UndoAllPauses
            | ServerCommand::SetSplitName { .. }
            | ServerCommand::SetCurrentSplitName { .. }
            | ServerCommand::SetCustomVariable { .. }
            | ServerCommand::SwitchComparison(_)
            | ServerCommand::SwitchComparisonNext
            | ServerCommand::SwitchComparisonPrevious
    )
}

fn default_layout_name(layout: &Layout) -> String {
    layout
        .file_path
        .as_ref()
        .and_then(|path| path.file_stem())
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Layout".to_string())
}

/// Build the initial eframe options from the layout and saved window geometry.
pub fn native_options(
    layout: &Layout,
    config: &AppConfig,
    timer: &Timer,
) -> eframe::NativeOptions {
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("RustSplit")
        .with_decorations(false)
        .with_transparent(true)
        .with_always_on_top()
        .with_min_inner_size(egui::vec2(80.0, 40.0))
        .with_resizable(layout.settings.allow_resizing);

    let renderer = Renderer::default();
    let natural_height = renderer.natural_height(layout, timer);
    let natural_width = renderer.natural_width(layout, timer);
    let width = config.window.width.unwrap_or(match layout.mode {
        LayoutMode::Vertical => layout.vertical_width.max(200) as f32,
        LayoutMode::Horizontal => natural_width.max(200.0),
    });
    let height = config.window.height.unwrap_or(match layout.mode {
        LayoutMode::Vertical => natural_height.max(80.0),
        LayoutMode::Horizontal => layout.horizontal_height.max(80) as f32,
    });
    viewport = viewport.with_inner_size(egui::vec2(width, height));
    if let (Some(x), Some(y)) = (config.window.x, config.window.y) {
        viewport = viewport.with_position(egui::pos2(x, y));
    }
    if !layout.settings.always_on_top {
        viewport = viewport.with_window_level(WindowLevel::Normal);
    }

    eframe::NativeOptions {
        viewport,
        ..Default::default()
    }
}

pub fn load_initial_run(config: &mut AppConfig, cli_path: Option<&Path>) -> Run {
    let path = cli_path
        .map(Path::to_path_buf)
        .or_else(|| config.general.last_run.clone());
    if let Some(path) = path {
        match load_run(&path) {
            Ok(run) => {
                config.remember_run(&path);
                return run;
            }
            Err(error) => log::warn!("could not load {}: {error}", path.display()),
        }
    }
    Run::with_segments("", "", ["Segment 1"])
}

pub fn load_initial_layout(
    config: &mut AppConfig,
    run: &Run,
    cli_path: Option<&Path>,
) -> Layout {
    if let Some(path) = cli_path {
        if let Ok(layout) = load_layout(path) {
            config.remember_layout(path);
            return layout;
        }
    }
    if let (Some(link), Some(run_path)) = (&run.layout_path, &run.file_path) {
        let path = if link == "?default" {
            None
        } else {
            let candidate = PathBuf::from(link);
            Some(if candidate.is_absolute() {
                candidate
            } else {
                run_path
                    .parent()
                    .unwrap_or(Path::new("."))
                    .join(candidate)
            })
        };
        if let Some(path) = path {
            if let Ok(layout) = load_layout(&path) {
                config.remember_layout(&path);
                return layout;
            }
        }
        if link == "?default" {
            return Layout::parse_default().unwrap_or_else(|_| Layout::livesplit_default());
        }
    }
    if let Some(path) = &config.general.last_layout {
        if let Ok(layout) = load_layout(path) {
            return layout;
        }
    }
    Layout::parse_default().unwrap_or_else(|_| Layout::livesplit_default())
}

impl std::fmt::Debug for RustSplitApp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RustSplitApp")
            .field("phase", &self.timer.phase)
            .field("layout", &self.layout.file_path)
            .finish()
    }
}
