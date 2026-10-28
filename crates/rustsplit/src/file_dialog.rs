//! A small built-in file browser, avoiding platform-specific dialog
//! dependencies.

use std::path::{Path, PathBuf};

use egui::{Context, Key, RichText, ScrollArea};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileDialogMode {
    Open,
    Save,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileDialogAction {
    Open(PathBuf),
    Save(PathBuf),
    Cancel,
}

pub struct FileDialog {
    pub open: bool,
    mode: FileDialogMode,
    title: String,
    directory: PathBuf,
    filename: String,
    extensions: Vec<String>,
    entries: Vec<Entry>,
    error: Option<String>,
}

#[derive(Clone, Debug)]
struct Entry {
    name: String,
    path: PathBuf,
    is_dir: bool,
}

impl Default for FileDialog {
    fn default() -> Self {
        Self {
            open: false,
            mode: FileDialogMode::Open,
            title: String::new(),
            directory: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            filename: String::new(),
            extensions: vec!["lss".to_string()],
            entries: Vec::new(),
            error: None,
        }
    }
}

impl FileDialog {
    pub fn open(
        &mut self,
        mode: FileDialogMode,
        title: impl Into<String>,
        start: Option<&Path>,
        filename: impl Into<String>,
        extensions: &[&str],
    ) {
        self.open = true;
        self.mode = mode;
        self.title = title.into();
        self.filename = filename.into();
        self.extensions = extensions.iter().map(|extension| extension.to_string()).collect();
        self.directory = start
            .map(|path| {
                if path.is_dir() {
                    path.to_path_buf()
                } else {
                    path.parent().unwrap_or(Path::new(".")).to_path_buf()
                }
            })
            .filter(|path| path.is_dir())
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(|| PathBuf::from("."));
        self.refresh();
    }

    pub fn close(&mut self) {
        self.open = false;
        self.entries.clear();
        self.error = None;
    }

    fn refresh(&mut self) {
        self.entries.clear();
        self.error = None;
        let Ok(read_dir) = std::fs::read_dir(&self.directory) else {
            self.error = Some(format!("Cannot open {}", self.directory.display()));
            return;
        };
        for entry in read_dir.flatten() {
            let path = entry.path();
            let is_dir = path.is_dir();
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            if !is_dir && !self.matches_extension(&path) {
                continue;
            }
            self.entries.push(Entry { name, path, is_dir });
        }
        self.entries.sort_by(|a, b| {
            b.is_dir
                .cmp(&a.is_dir)
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
    }

    fn matches_extension(&self, path: &Path) -> bool {
        if self.extensions.is_empty() {
            return true;
        }
        path.extension()
            .and_then(|extension| extension.to_str())
            .map(|extension| {
                self.extensions
                    .iter()
                    .any(|allowed| allowed.eq_ignore_ascii_case(extension))
            })
            .unwrap_or(false)
    }

    fn selected_path(&self) -> Option<PathBuf> {
        let filename = self.filename.trim();
        if filename.is_empty() {
            return None;
        }
        let path = Path::new(filename);
        if path.is_absolute() {
            return Some(path.to_path_buf());
        }
        let mut path = self.directory.join(path);
        if self.mode == FileDialogMode::Save && path.extension().is_none() {
            if let Some(extension) = self.extensions.first() {
                path.set_extension(extension);
            }
        }
        Some(path)
    }

    /// Show the dialog and return an action when the user confirms/cancels.
    pub fn show(&mut self, ctx: &Context) -> Option<FileDialogAction> {
        if !self.open {
            return None;
        }
        let mut action = None;
        let mut refresh = false;
        let mut close = false;

        egui::Window::new(&self.title)
            .collapsible(false)
            .resizable(true)
            .default_width(560.0)
            .default_height(430.0)
            .default_pos(egui::pos2(12.0, 12.0))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if ui.button("⬆ Up").clicked() {
                        if let Some(parent) = self.directory.parent() {
                            self.directory = parent.to_path_buf();
                            refresh = true;
                        }
                    }
                    if ui.button("🏠 Home").clicked() {
                        if let Some(home) = std::env::var_os("HOME") {
                            self.directory = PathBuf::from(home);
                            refresh = true;
                        }
                    }
                    if ui.button("⟳ Refresh").clicked() {
                        refresh = true;
                    }
                    let mut path_text = self.directory.display().to_string();
                    let response = ui.add(
                        egui::TextEdit::singleline(&mut path_text)
                            .desired_width(f32::INFINITY)
                            .hint_text("Directory"),
                    );
                    if response.lost_focus() && ui.input(|input| input.key_pressed(Key::Enter)) {
                        let path = PathBuf::from(path_text);
                        if path.is_dir() {
                            self.directory = path;
                            refresh = true;
                        }
                    }
                });
                if let Some(error) = &self.error {
                    ui.colored_label(egui::Color32::LIGHT_RED, error);
                }
                ui.separator();
                ScrollArea::vertical()
                    .id_salt("file-dialog-list")
                    .max_height(280.0)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let entries = self.entries.clone();
                        for entry in entries {
                            if entry.is_dir {
                                if ui
                                    .add(
                                        egui::Button::new(
                                            RichText::new(format!("📁 {}", entry.name))
                                                .color(ui.visuals().hyperlink_color),
                                        )
                                        .frame(false),
                                    )
                                    .clicked()
                                {
                                    self.directory = entry.path;
                                    refresh = true;
                                }
                            } else {
                                let selected = self.filename == entry.name;
                                if ui
                                    .add(
                                        egui::Button::new(format!("📄 {}", entry.name))
                                            .selected(selected),
                                    )
                                    .clicked()
                                {
                                    self.filename = entry.name;
                                }
                            }
                        }
                    });
                ui.separator();
                ui.horizontal(|ui| {
                    ui.label("File:");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.filename)
                            .desired_width(360.0)
                            .hint_text("name.lss"),
                    );
                    let enabled = self.selected_path().is_some();
                    let label = match self.mode {
                        FileDialogMode::Open => "Open",
                        FileDialogMode::Save => "Save",
                    };
                    if ui.add_enabled(enabled, egui::Button::new(label)).clicked() {
                        if let Some(path) = self.selected_path() {
                            action = Some(match self.mode {
                                FileDialogMode::Open => FileDialogAction::Open(path),
                                FileDialogMode::Save => FileDialogAction::Save(path),
                            });
                            close = true;
                        }
                    }
                    if ui.button("Cancel").clicked() {
                        action = Some(FileDialogAction::Cancel);
                        close = true;
                    }
                });
                if self.mode == FileDialogMode::Open && !self.filename.is_empty() {
                    let path = self.directory.join(self.filename.trim());
                    if path.is_file() && ui.input(|input| input.key_pressed(Key::Enter)) {
                        action = Some(FileDialogAction::Open(path));
                        close = true;
                    }
                }
            });

        if refresh {
            self.refresh();
        }
        if ctx.input(|input| input.key_pressed(Key::Escape)) {
            action = Some(FileDialogAction::Cancel);
            close = true;
        }
        if close {
            self.close();
        }
        action
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_path_gets_an_extension() {
        let mut dialog = FileDialog::default();
        dialog.mode = FileDialogMode::Save;
        dialog.directory = PathBuf::from("/tmp");
        dialog.filename = "Run".to_string();
        dialog.extensions = vec!["lss".to_string()];
        assert_eq!(
            dialog.selected_path().unwrap(),
            PathBuf::from("/tmp/Run.lss")
        );
    }

    #[test]
    fn extension_filter_is_case_insensitive() {
        let mut dialog = FileDialog::default();
        dialog.extensions = vec!["lss".to_string()];
        assert!(dialog.matches_extension(Path::new("Run.LSS")));
        assert!(!dialog.matches_extension(Path::new("Run.lsl")));
    }
}
