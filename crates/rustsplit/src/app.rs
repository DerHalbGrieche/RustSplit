//! The main RustSplit application window and event loop.

use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use egui::{
    Align2, Color32, Context, CursorIcon, Key, PointerButton, Sense, Ui, ViewportCommand,
    WindowLevel,
};
use rustsplit_core::{load_run, save_run, Run, Timer, TimerPhase, TimingMethod};
use rustsplit_server::{handle_command, parse_command, ServerCommand, ServerHandle};
use rustsplit_ui::{configure_fonts, load_layout, save_layout, Layout, LayoutMode, Renderer};

use crate::auto_splitter::AutoSplitterUi;
use crate::config::AppConfig;
use crate::control::ControlServer;
use crate::file_dialog::{FileDialogAction, FileDialogMode};
use crate::file_dialogs::FileDialogs;
use crate::hotkeys::{GlobalHotkeys, HotkeyAction};
use crate::input::{apply_timer_action, TimerAction};
use crate::layout_editor::{LayoutEditor, LayoutEditorEvent, BACKGROUND_IMAGE_EXTENSIONS};
use crate::niri::{self, DesiredWindow, NiriIntegration};
use crate::run_editor::{RunEditor, RunEditorEvent};
use crate::settings_window::SettingsWindow;
use crate::sounds::{self, SoundEvent, SoundPlayer};

#[derive(Clone, Debug, PartialEq)]
enum Action {
    Timer(TimerAction),
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
    Sound(SoundEvent),
    BackgroundImage,
}

pub struct RustSplitApp {
    timer: Timer,
    layout: Layout,
    renderer: Renderer,
    config: AppConfig,
    config_path: PathBuf,
    run_editor: RunEditor,
    layout_editor: LayoutEditor,
    file_dialog: FileDialogs,
    file_purpose: Option<FilePurpose>,
    settings: Option<SettingsWindow>,
    about_open: bool,
    status: Option<(String, Instant)>,
    hotkeys: GlobalHotkeys,
    server: Option<ServerHandle>,
    server_error: Option<String>,
    control: Option<ControlServer>,
    auto_splitter: AutoSplitterUi,
    niri: Option<NiriIntegration>,
    /// Actions with the moment they were triggered.
    pending_actions: Vec<(Action, Instant)>,
    last_window_size: Option<egui::Vec2>,
    last_window_pos: Option<egui::Pos2>,
    passthrough: bool,
    resizable: bool,
    last_title: String,
    dialog_expanded: bool,
    overlay_size_before_dialog: Option<egui::Vec2>,
    #[cfg(feature = "speedrun-com")]
    world_records: crate::speedrun_com::WorldRecordService,
    sounds: SoundPlayer,
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
        let niri = NiriIntegration::new(&config.niri);
        let control = match ControlServer::start(cc.egui_ctx.clone()) {
            Ok(server) => server,
            Err(error) => {
                log::warn!("could not start the control socket: {error}");
                None
            }
        };
        let mut app = Self {
            timer,
            layout,
            renderer: Renderer::default(),
            config,
            config_path,
            run_editor: RunEditor::default(),
            layout_editor: LayoutEditor::default(),
            file_dialog: FileDialogs::default(),
            file_purpose: None,
            settings: None,
            about_open: false,
            status: Some((
                "Right-click for the LiveSplit menu. Space starts/splits.".to_string(),
                Instant::now(),
            )),
            hotkeys: GlobalHotkeys::new(cc.egui_ctx.clone()),
            server: None,
            server_error: None,
            control,
            auto_splitter: AutoSplitterUi::new(&cc.egui_ctx),
            niri,
            pending_actions: Vec::new(),
            last_window_size: None,
            last_window_pos: None,
            passthrough: false,
            resizable,
            last_title: String::new(),
            dialog_expanded: false,
            overlay_size_before_dialog: None,
            #[cfg(feature = "speedrun-com")]
            world_records: Default::default(),
            sounds: SoundPlayer::default(),
        };
        app.hotkeys.re_register(&app.config.hotkeys);
        app.file_dialog
            .set_prefer_native(app.config.general.use_system_file_dialogs);
        app.sounds.preload(&app.config.sounds);
        if app.config.server.enabled && app.config.server.autostart {
            app.start_server(&cc.egui_ctx);
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

    fn push_action(&mut self, action: Action) {
        self.pending_actions.push((action, Instant::now()));
    }

    fn apply_actions(&mut self, ctx: &Context) {
        for (action, received) in std::mem::take(&mut self.pending_actions) {
            self.apply_action(ctx, action, received);
        }
    }

    /// Applies a timer action at the moment it was received, plays its sound
    /// and only then autosaves, so writing the split file can't delay the
    /// sound.
    fn run_timer_action(&mut self, action: &TimerAction, received: Instant) {
        let before = sounds::Snapshot::capture(&self.timer);
        let outcome = apply_timer_action(&mut self.timer, action, received);
        self.play_timer_sounds(&before);
        if let Some(status) = outcome.status {
            self.set_status(status);
        }
        if outcome.autosave {
            self.autosave_after_change();
        }
    }

    /// The single hook for timer sounds. Every timer action (shortcuts,
    /// hotkeys, menu, server, control socket and auto splitter) captures a
    /// snapshot before it runs and calls this right after the timer changed,
    /// before anything is saved.
    fn play_timer_sounds(&mut self, before: &sounds::Snapshot) {
        self.sounds
            .play_timer_change(before, &self.timer, &self.config.sounds);
    }

    fn apply_action(&mut self, ctx: &Context, action: Action, received: Instant) {
        match action {
            Action::Timer(action) => self.run_timer_action(&action, received),
            Action::NewRun => {
                let run = Run::with_segments("", "", ["Segment 1"]);
                self.switch_run(run);
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
                self.open_file_dialog(
                    FilePurpose::OpenRun,
                    FileDialogMode::Open,
                    "Open Splits",
                    start.as_deref(),
                    "",
                    &["lss"],
                );
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
                self.open_file_dialog(
                    FilePurpose::OpenLayout,
                    FileDialogMode::Open,
                    "Open Layout",
                    start.as_deref(),
                    "",
                    &["lsl"],
                );
            }
            Action::SaveLayout => {
                if let Err(error) = self.save_current_layout(false) {
                    self.set_status(format!("Could not save layout: {error}"));
                }
            }
            Action::SaveLayoutAs => {
                self.open_save_dialog_for_layout(self.layout.file_path.clone());
            }
            Action::DefaultLayout => {
                self.layout =
                    Layout::parse_default().unwrap_or_else(|_| Layout::livesplit_default());
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
                    self.start_server(ctx);
                }
            }
            Action::Settings => {
                // Opening it again keeps the edits made so far.
                if self.settings.is_none() {
                    self.settings = Some(SettingsWindow::new(&self.config));
                }
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
                self.switch_run(run);
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

    /// Replaces the run with another one (opened, new or switched to by the
    /// server). Like LiveSplit, an attempt in progress is reset first, which
    /// records it in the old run, and the old run is autosaved. Switching
    /// runs plays no sound.
    fn switch_run(&mut self, run: Run) {
        if self.timer.phase != TimerPhase::NotRunning {
            self.timer.reset();
            self.autosave_after_change();
        }
        self.timer.set_run(run);
    }

    /// Opens a file dialog for `purpose`. While a system dialog is still
    /// open, the request is refused instead of stacking a second dialog.
    fn open_file_dialog(
        &mut self,
        purpose: FilePurpose,
        mode: FileDialogMode,
        title: impl Into<String>,
        start: Option<&Path>,
        filename: impl Into<String>,
        extensions: &[&str],
    ) {
        if !self
            .file_dialog
            .open(mode, title, start, filename, extensions)
        {
            self.set_status("A file dialog is already open.");
            if purpose == FilePurpose::BackgroundImage {
                self.layout_editor.cancel_background_image();
            }
            return;
        }
        // A built-in browser that was still open for another purpose is
        // replaced.
        let previous = self.file_purpose.replace(purpose.clone());
        if previous == Some(FilePurpose::BackgroundImage) && purpose != FilePurpose::BackgroundImage
        {
            self.layout_editor.cancel_background_image();
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
        let start = self
            .timer
            .run
            .file_path
            .clone()
            .or_else(|| self.config.general.last_run.clone());
        self.open_file_dialog(
            FilePurpose::SaveRun,
            FileDialogMode::Save,
            "Save Splits As",
            start.as_deref(),
            file_name,
            &["lss"],
        );
    }

    fn open_save_dialog_for_layout(&mut self, start: Option<PathBuf>) {
        let file_name = format!("{}.lsl", default_layout_name(&self.layout));
        self.open_file_dialog(
            FilePurpose::SaveLayout,
            FileDialogMode::Save,
            "Save Layout As",
            start.as_deref(),
            file_name,
            &["lsl"],
        );
    }

    fn save_current_layout(&mut self, silent: bool) -> Result<(), String> {
        let Some(path) = self.layout.file_path.clone() else {
            if !silent {
                self.open_save_dialog_for_layout(None);
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
            if let Err(error) = self.save_current_layout(true) {
                self.set_status(format!("Could not save layout: {error}"));
            }
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
            Some(FilePurpose::Sound(event)) => {
                if let Some(settings) = self.settings.as_mut() {
                    *event.path_mut(settings.draft_sounds_mut()) = Some(path);
                } else {
                    // The Settings window was closed while the system dialog
                    // was open. The choice was explicit, so keep it.
                    self.set_status(format!("{} sound set.", event.label()));
                    *event.path_mut(&mut self.config.sounds) = Some(path);
                    self.sounds.preload(&self.config.sounds);
                    self.save_config();
                }
            }
            Some(FilePurpose::BackgroundImage) => self.layout_editor.set_background_image(path),
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
        if event != RunEditorEvent::None
            && event != RunEditorEvent::Cancel
            && self.timer.phase != TimerPhase::NotRunning
        {
            // The draft was copied before the attempt started; applying it
            // would throw the attempt away. The editor stays open.
            self.set_status("Finish or reset the run before applying split changes.");
            return;
        }
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
                        self.open_save_dialog_for_layout(None);
                    }
                }
            }
            LayoutEditorEvent::SaveAs => {
                if let Some(mut layout) = self.layout_editor.draft_cloned() {
                    layout.file_path = None;
                    self.layout = layout;
                    self.apply_layout_change(ctx);
                }
                self.open_save_dialog_for_layout(None);
            }
            LayoutEditorEvent::ResetDefault => {
                self.layout =
                    Layout::parse_default().unwrap_or_else(|_| Layout::livesplit_default());
                self.apply_layout_change(ctx);
            }
            LayoutEditorEvent::BrowseBackgroundImage => {
                let start = self.layout_editor.background_image_start();
                self.open_file_dialog(
                    FilePurpose::BackgroundImage,
                    FileDialogMode::Open,
                    "Choose Background Image",
                    start.as_deref(),
                    "",
                    BACKGROUND_IMAGE_EXTENSIONS,
                );
            }
        }
    }

    /// Keyboard shortcuts of the overlay. egui has no per-event timestamps,
    /// so these are applied at "now", within the frame they arrived in.
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
        let mut actions = Vec::new();
        if ctrl {
            if key_pressed_once(ctx, Key::Q) {
                actions.push(Action::Quit);
            }
            if key_pressed_once(ctx, Key::S) {
                actions.push(if shift {
                    Action::SaveRunAs
                } else {
                    Action::SaveRun
                });
            }
            if key_pressed_once(ctx, Key::O) {
                actions.push(Action::OpenRun);
            }
            if key_pressed_once(ctx, Key::E) {
                actions.push(Action::EditRun);
            }
            if key_pressed_once(ctx, Key::L) {
                actions.push(Action::EditLayout);
            }
        } else if !alt {
            let keys = [
                (Key::Space, TimerAction::StartSplit),
                (Key::R, TimerAction::Reset),
                (Key::Backspace, TimerAction::Undo),
                (Key::Delete, TimerAction::Skip),
                (Key::P, TimerAction::Pause),
                (Key::U, TimerAction::UndoAllPauses),
                (Key::G, TimerAction::ToggleTimingMethod),
            ];
            for (key, action) in keys {
                if key_pressed_once(ctx, key) {
                    actions.push(Action::Timer(action));
                }
            }
            if key_pressed_once(ctx, Key::F1) {
                actions.push(Action::Settings);
            }
            if key_pressed_once(ctx, Key::Tab) {
                actions.push(Action::Timer(if shift {
                    TimerAction::SwitchComparisonPrevious
                } else {
                    TimerAction::SwitchComparisonNext
                }));
            }
        }
        for action in actions {
            self.push_action(action);
        }
    }

    fn poll_global_hotkeys(&mut self) {
        for (action, pressed) in self.hotkeys.poll() {
            let action = match action {
                HotkeyAction::StartSplit => Action::Timer(TimerAction::StartSplit),
                HotkeyAction::Reset => Action::Timer(TimerAction::Reset),
                HotkeyAction::Undo => Action::Timer(TimerAction::Undo),
                HotkeyAction::Skip => Action::Timer(TimerAction::Skip),
                HotkeyAction::Pause => Action::Timer(TimerAction::Pause),
                HotkeyAction::UndoAllPauses => Action::Timer(TimerAction::UndoAllPauses),
                HotkeyAction::SwitchComparisonNext => {
                    Action::Timer(TimerAction::SwitchComparisonNext)
                }
                HotkeyAction::SwitchComparisonPrevious => {
                    Action::Timer(TimerAction::SwitchComparisonPrevious)
                }
                HotkeyAction::ToggleTimingMethod => Action::Timer(TimerAction::ToggleTimingMethod),
                HotkeyAction::EditRun => Action::EditRun,
                HotkeyAction::EditLayout => Action::EditLayout,
            };
            self.pending_actions.push((action, pressed));
        }
    }

    fn sync_niri(&mut self) {
        if !self.config.niri.enabled {
            self.niri = None;
            return;
        }
        if self.niri.is_none() {
            self.niri = NiriIntegration::new(&self.config.niri);
        }
    }

    fn start_server(&mut self, ctx: &Context) {
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
        // Wake the UI as soon as a request is queued; the client thread is
        // blocked until `poll_server` answers it.
        let ctx = ctx.clone();
        let server = ServerHandle::builder(socket)
            .notifier(move || ctx.request_repaint())
            .start();
        match server {
            Ok(server) => {
                self.config.server.enabled = true;
                self.server_error = None;
                self.set_status(format!(
                    "LiveSplit Server listening on port {}",
                    server.port()
                ));
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
            let response = self.apply_server_command(ctx, &request.command, request.received);
            request.respond(response);
        }
    }

    /// Applies a server or control socket command at the moment it was
    /// `received`.
    fn apply_server_command(
        &mut self,
        ctx: &Context,
        command: &ServerCommand,
        received: Instant,
    ) -> Option<String> {
        match command {
            ServerCommand::SwitchSplits(path) => {
                let ok = self.open_run_path(ctx, &PathBuf::from(path));
                return Some(if ok { "True" } else { "False" }.to_string());
            }
            ServerCommand::SwitchLayout(path) => {
                let ok = self.open_layout_path(ctx, &PathBuf::from(path));
                return Some(if ok { "True" } else { "False" }.to_string());
            }
            _ => {}
        }

        let autosave = server_command_mutates_run(command);
        let before = sounds::Snapshot::capture(&self.timer);
        let at = self.timer.timestamp_of(received);
        let outcome = self.timer.at(at, |timer| handle_command(timer, command));
        self.play_timer_sounds(&before);
        let response = outcome.response.clone();
        if outcome.save {
            let _ = self.save_current_run(false);
        }
        if autosave {
            self.autosave_after_change();
        }
        if outcome.quit {
            self.save_config();
            ctx.send_viewport_cmd(ViewportCommand::Close);
        }
        response
    }

    fn poll_control(&mut self, ctx: &Context) {
        loop {
            let request = {
                let Some(control) = &self.control else {
                    break;
                };
                control.try_recv()
            };
            let Some(request) = request else {
                break;
            };
            let response = self.apply_control_command(ctx, &request.command, request.received);
            request.respond(response);
        }
    }

    /// Applies commands from the auto splitter thread (see `auto_splitter.rs`).
    fn poll_auto_splitter(&mut self) {
        let before = sounds::Snapshot::capture(&self.timer);
        let outcome = self
            .auto_splitter
            .update(&mut self.timer, &mut self.config.auto_splitter);
        self.play_timer_sounds(&before);
        if outcome.autosave {
            self.autosave_after_change();
        }
        if outcome.config_changed {
            self.save_config();
        }
        if let Some(status) = outcome.status {
            self.set_status(status);
        }
    }

    fn apply_control_command(
        &mut self,
        ctx: &Context,
        command: &str,
        received: Instant,
    ) -> Option<String> {
        let command = command.trim();
        let action = match command.to_ascii_lowercase().as_str() {
            "editrun" | "edit-run" | "editsplits" => Some(Action::EditRun),
            "editlayout" | "edit-layout" => Some(Action::EditLayout),
            "settings" => Some(Action::Settings),
            _ => None,
        };
        if let Some(action) = action {
            self.pending_actions.push((action, received));
            return Some("ok".to_string());
        }

        match parse_command(command) {
            Ok(command) => self.apply_server_command(ctx, &command, received),
            Err(error) => Some(format!("error: {error}")),
        }
    }

    fn poll_niri(&mut self, ctx: &Context) {
        let desired = self.desired_niri_window();
        let auto_float = self.config.niri.auto_float;
        let Some(niri) = self.niri.as_mut() else {
            return;
        };
        match niri.poll(auto_float, desired) {
            Ok(Some(message)) => log::info!("{message}"),
            Ok(None) => {}
            Err(error) => log::warn!("Niri integration: {error}"),
        }
        // Keep probing until the window is configured, but stop waking the
        // app once the integration gave up on finding it.
        if niri.is_probing() {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }

    fn desired_niri_window(&self) -> DesiredWindow {
        let (fallback_width, fallback_height) = computed_window_size(&self.layout, &self.timer);
        let width = self.config.niri.width.unwrap_or(fallback_width).max(80.0);
        let height = self.config.niri.height.unwrap_or(fallback_height).max(40.0);
        DesiredWindow {
            x: self.config.niri.x.map(f64::from),
            y: self.config.niri.y.map(f64::from),
            width: width.round() as i32,
            height: height.round() as i32,
        }
    }

    fn show_settings(&mut self, ctx: &Context) {
        let Some(settings) = self.settings.as_mut() else {
            return;
        };
        let response = settings.show(
            ctx,
            &mut self.sounds,
            self.hotkeys.error.as_deref(),
            self.server_error.as_deref(),
        );
        if response.apply {
            self.apply_settings(ctx);
        }
        if response.close {
            self.settings = None;
            return;
        }
        if let Some(event) = response.browse_sound {
            let start = self
                .settings
                .as_ref()
                .and_then(|settings| event.path(settings.draft_sounds()))
                .map(Path::to_path_buf);
            self.open_file_dialog(
                FilePurpose::Sound(event),
                FileDialogMode::Open,
                format!("Choose Sound: {}", event.label()),
                start.as_deref(),
                "",
                &["wav", "ogg", "oga", "mp3", "flac"],
            );
        }
    }

    /// Applies the Settings window's edits and keeps the window open.
    fn apply_settings(&mut self, ctx: &Context) {
        let Some(settings) = self.settings.as_mut() else {
            return;
        };
        let old_config = self.config.clone();
        settings.apply_to(&mut self.config);
        self.hotkeys.re_register(&self.config.hotkeys);
        if self.config.server.port != old_config.server.port
            || self.config.server.enabled != old_config.server.enabled
            || self.config.server.bind_address != old_config.server.bind_address
        {
            let wanted = self.config.server.enabled;
            self.stop_server();
            if wanted {
                self.start_server(ctx);
            }
        }
        self.sync_niri();
        self.sounds.preload(&self.config.sounds);
        self.save_config();
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
        self.file_dialog
            .set_prefer_native(self.config.general.use_system_file_dialogs);
        let action = self.file_dialog.show(ctx);
        if let Some(action) = action {
            if action == FileDialogAction::Cancel {
                if self.file_purpose.take() == Some(FilePurpose::BackgroundImage) {
                    self.layout_editor.cancel_background_image();
                }
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
            actions.push(Action::Timer(TimerAction::StartSplit));
            ui.close();
        }
        if ui.button("Pause / Resume       P").clicked() {
            actions.push(Action::Timer(TimerAction::Pause));
            ui.close();
        }
        if ui.button("Undo Split           Backspace").clicked() {
            actions.push(Action::Timer(TimerAction::Undo));
            ui.close();
        }
        if ui.button("Skip Split           Delete").clicked() {
            actions.push(Action::Timer(TimerAction::Skip));
            ui.close();
        }
        ui.separator();
        if ui.button("Reset                R").clicked() {
            actions.push(Action::Timer(TimerAction::Reset));
            ui.close();
        }
        if ui.button("Reset and Set as Personal Best").clicked() {
            actions.push(Action::Timer(TimerAction::ResetAndSetPb));
            ui.close();
        }
        if ui.button("Undo All Pauses      U").clicked() {
            actions.push(Action::Timer(TimerAction::UndoAllPauses));
            ui.close();
        }
        ui.separator();
        ui.menu_button("Comparison", |ui| {
            let comparisons = self.timer.run.comparisons();
            for comparison in comparisons {
                let selected = comparison == self.timer.current_comparison;
                if ui.selectable_label(selected, &comparison).clicked() {
                    actions.push(Action::Timer(TimerAction::SetComparison(comparison)));
                    ui.close();
                }
            }
        });
        ui.menu_button("Timing Method", |ui| {
            for method in [TimingMethod::RealTime, TimingMethod::GameTime] {
                let selected = method == self.timer.current_timing_method;
                if ui.selectable_label(selected, method.to_string()).clicked() {
                    actions.push(Action::Timer(TimerAction::SetTimingMethod(method)));
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
        self.auto_splitter.menu(ui, &self.config.auto_splitter);
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
        for action in actions {
            self.push_action(action);
        }
    }

    fn draw_overlay_ui(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let rect = ui.max_rect();
        self.renderer.draw(ui, &self.layout, &self.timer);

        // Whole-window drag support.  Buttons and dialogs sit above this
        // interaction in egui's layer order.
        if self.layout.settings.allow_moving {
            let response = ui.interact(
                rect,
                ui.scope_id().with("rustsplit-drag"),
                Sense::click_and_drag(),
            );
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
            && (hotkeys_ready || self.server.is_some() || self.control.is_some());
        if wanted_passthrough != self.passthrough {
            self.passthrough = wanted_passthrough;
            ctx.send_viewport_cmd(ViewportCommand::MousePassthrough(wanted_passthrough));
        }
    }

    fn any_modal_open(&self) -> bool {
        self.run_editor.open
            || self.layout_editor.open
            || self.file_dialog.is_open()
            || self.settings.is_some()
            || self.about_open
            || self.auto_splitter.window_open()
    }

    fn ensure_dialog_space(&mut self, ctx: &Context) {
        if self.dialog_expanded {
            return;
        }
        let current = ctx.input(|input| input.viewport().inner_rect.map(|rect| rect.size()));
        let monitor = ctx
            .input(|input| input.viewport().monitor_size)
            .unwrap_or_else(|| egui::vec2(1920.0, 1080.0));
        // Not `f32::clamp`: min/max maps a NaN monitor size to the upper bound
        // instead of propagating NaN into the viewport size.
        #[expect(clippy::manual_clamp, reason = "keeps the NaN handling of min/max")]
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
        let inner_rect = ctx.input(|input| input.viewport().inner_rect);
        if let Some(rect) = inner_rect {
            self.last_window_pos = Some(rect.min);
        }
        // Wayland compositors do not expose a window position, which makes
        // egui's `inner_rect` unavailable there. `content_rect` still tracks
        // the content size, so use it as a fallback.
        let size = inner_rect
            .map(|rect| rect.size())
            .unwrap_or_else(|| ctx.content_rect().size());
        if size.x > 1.0 && size.y > 1.0 {
            self.last_window_size = Some(size);
        }
        self.track_niri_geometry(false);
        self.sync_niri_surface(ctx);
    }

    /// Niri can resize floating windows without sending a new xdg configure
    /// event. Keep the client surface in sync with Niri's visual tile size,
    /// otherwise Niri scales/letterboxes the old surface.
    fn sync_niri_surface(&mut self, ctx: &Context) {
        if self.any_modal_open() {
            return;
        }
        let Some(niri) = self.niri.as_mut() else {
            return;
        };
        if !niri.is_applied() {
            return;
        }
        ctx.request_repaint_after(Duration::from_millis(100));
        if !niri.surface_sync_due() {
            return;
        }
        match niri.visual_size() {
            Ok(Some((width, height))) => {
                if niri.take_surface_sync(width, height) {
                    log::debug!("Niri: resizing client surface to {width}x{height}");
                    ctx.send_viewport_cmd(ViewportCommand::InnerSize(egui::vec2(width, height)));
                }
            }
            Ok(None) => {}
            Err(error) => log::debug!("could not read Niri window size: {error}"),
        }
    }

    fn track_niri_geometry(&mut self, force: bool) {
        let modal_open = self.any_modal_open();
        let Some(niri) = self.niri.as_mut() else {
            return;
        };
        if modal_open || (!force && !niri.tracking_due()) {
            return;
        }
        match niri.geometry() {
            Ok(Some(geometry)) if geometry.floating => {
                self.config.niri.x = Some(geometry.x as f32);
                self.config.niri.y = Some(geometry.y as f32);
                self.config.niri.width = Some(geometry.width as f32);
                self.config.niri.height = Some(geometry.height as f32);
            }
            Ok(_) => {}
            Err(error) => log::debug!("could not track Niri geometry: {error}"),
        }
    }

    fn backup_current_run(&mut self) {
        let directory = crate::config::data_dir().join("backups");
        if let Err(error) = std::fs::create_dir_all(&directory) {
            log::error!("could not create {}: {error}", directory.display());
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
            log::error!("could not write backup {}: {error}", path.display());
        } else {
            log::info!("saved backup to {}", path.display());
        }
    }

    fn store_window_geometry(&mut self) {
        if let Some(size) = self.last_window_size {
            self.config.window.width = Some(size.x);
            self.config.window.height = Some(size.y);
        }
        // Wayland clients cannot read their position and Niri tracks floating
        // windows itself, so don't overwrite its saved geometry with egui's
        // meaningless (0, 0) origin.
        if self.niri.is_none() {
            if let Some(pos) = self.last_window_pos {
                self.config.window.x = Some(pos.x);
                self.config.window.y = Some(pos.y);
            }
        }
    }
}

impl eframe::App for RustSplitApp {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        // The overlay background is painted by the layout. Without this,
        // eframe clears the window with a semi-opaque dark color, which shows
        // up as an opaque box whenever the layout background is transparent.
        egui::Color32::TRANSPARENT.to_normalized_gamma_f32()
    }

    /// Everything that isn't drawing: input from other threads, the timer
    /// actions it triggers, autosaves and background lookups. eframe calls
    /// this before every `ui`, and also while the window is minimized or
    /// hidden (then about every 100 ms, or when another thread requests a
    /// repaint), when `ui` is not called at all. Input is timestamped where
    /// it arrives, so that delay does not change the recorded times.
    fn logic(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        self.poll_global_hotkeys();
        self.poll_server(ctx);
        self.poll_control(ctx);
        // Hotkeys, UI commands of the control socket and menu actions of the
        // last frame.
        self.apply_actions(ctx);
        self.poll_auto_splitter();
        #[cfg(feature = "speedrun-com")]
        {
            self.renderer.world_record = self.world_records.update(
                ctx,
                self.config.speedrun_com.fetch_world_records,
                &self.layout,
                &self.timer,
            );
        }
        self.auto_splitter.publish(&self.timer);

        // The server, control socket, hotkeys and the auto splitter request
        // a repaint when something arrives, so the idle cadence only has to
        // follow the timer (and the auto splitter's game time updates).
        let repaint = match self.timer.phase {
            TimerPhase::Running => Duration::from_millis(16),
            TimerPhase::Paused => Duration::from_millis(100),
            _ => Duration::from_millis(250),
        };
        ctx.request_repaint_after(repaint);
    }

    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.handle_dropped_files(&ctx);
        if !self.any_modal_open() {
            self.handle_local_shortcuts(&ctx);
        }
        self.apply_actions(&ctx);
        self.poll_niri(&ctx);

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
        self.auto_splitter.show(&ctx, &self.config.auto_splitter);
        // After every other timer mutation of this frame.
        self.auto_splitter.publish(&self.timer);
        self.update_window_metadata(&ctx);
        self.store_window_geometry();
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.track_niri_geometry(true);
        self.store_window_geometry();
        self.save_config();
        if let Some(control) = self.control.take() {
            control.stop();
        }
        if self.config.general.autosave && self.timer.run.has_changed {
            if self.timer.run.file_path.is_some() {
                // Keep the unsaved changes somewhere if the split file can't
                // be written.
                if let Err(error) = self.save_current_run(true) {
                    log::error!("could not save splits on exit: {error}");
                    self.backup_current_run();
                }
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

/// Ignore OS key-repeat events so holding a key cannot trigger a split
/// multiple times in a row.
fn key_pressed_once(ctx: &Context, key: Key) -> bool {
    ctx.input(|input| {
        input.events.iter().any(|event| {
            matches!(
                event,
                egui::Event::Key {
                    key: pressed_key,
                    pressed: true,
                    repeat: false,
                    ..
                } if *pressed_key == key
            )
        })
    })
}

fn default_layout_name(layout: &Layout) -> String {
    layout
        .file_path
        .as_ref()
        .and_then(|path| path.file_stem())
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Layout".to_string())
}

/// The initial size for the overlay.
///
/// LiveSplit stores the window size in the layout, so use those values when
/// they are present. Fall back to the components' natural size for layouts
/// that leave an axis at -1.
fn computed_window_size(layout: &Layout, timer: &Timer) -> (f32, f32) {
    let renderer = Renderer::default();
    let natural_height = renderer.natural_height(layout, timer);
    let natural_width = renderer.natural_width(layout, timer);
    let dimension = |stored: i32, natural: f32, minimum: f32| {
        if stored > 0 {
            (stored as f32).max(minimum)
        } else {
            natural.max(minimum)
        }
    };
    match layout.mode {
        LayoutMode::Vertical => (
            dimension(layout.vertical_width, natural_width, 200.0),
            dimension(layout.vertical_height, natural_height, 80.0),
        ),
        LayoutMode::Horizontal => (
            dimension(layout.horizontal_width, natural_width, 200.0),
            dimension(layout.horizontal_height, natural_height, 80.0),
        ),
    }
}

/// Build the initial eframe options from the layout and saved window geometry.
pub fn native_options(layout: &Layout, config: &AppConfig, timer: &Timer) -> eframe::NativeOptions {
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("RustSplit")
        .with_app_id(niri::APP_ID)
        .with_decorations(false)
        .with_transparent(true)
        .with_always_on_top()
        .with_min_inner_size(egui::vec2(80.0, 40.0))
        .with_resizable(layout.settings.allow_resizing);

    let niri_active = config.niri.enabled && niri::is_session();
    let (fallback_width, fallback_height) = computed_window_size(layout, timer);
    let (saved_width, saved_height) = if niri_active {
        (config.niri.width, config.niri.height)
    } else {
        (config.window.width, config.window.height)
    };
    let width = saved_width.unwrap_or(fallback_width);
    let height = saved_height.unwrap_or(fallback_height);
    viewport = viewport.with_inner_size(egui::vec2(width, height));
    if !niri_active {
        if let (Some(x), Some(y)) = (config.window.x, config.window.y) {
            viewport = viewport.with_position(egui::pos2(x, y));
        }
    }
    if !layout.settings.always_on_top {
        viewport = viewport.with_window_level(WindowLevel::Normal);
    }

    eframe::NativeOptions {
        viewport,
        // RustSplit persists its own window/overlay geometry. eframe's
        // generic window persistence would restore stale tiled sizes and
        // fight the Niri integration.
        persist_window: false,
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

pub fn load_initial_layout(config: &mut AppConfig, run: &Run, cli_path: Option<&Path>) -> Layout {
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
                run_path.parent().unwrap_or(Path::new(".")).join(candidate)
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
