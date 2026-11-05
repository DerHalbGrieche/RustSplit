//! Persistent application settings.

use std::path::{Path, PathBuf};

use directories::ProjectDirs;
use serde::{Deserialize, Serialize};

use crate::auto_splitter::AutoSplitterConfig;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
#[derive(Default)]
pub struct AppConfig {
    pub general: GeneralConfig,
    pub hotkeys: HotkeyConfig,
    pub server: ServerConfig,
    pub window: WindowConfig,
    pub niri: NiriConfig,
    pub speedrun_com: SpeedrunComConfig,
    pub sounds: SoundConfig,
    pub auto_splitter: AutoSplitterConfig,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct GeneralConfig {
    pub last_run: Option<PathBuf>,
    pub last_layout: Option<PathBuf>,
    pub recent_runs: Vec<PathBuf>,
    pub autosave: bool,
    pub autosave_layout: bool,
    /// Use the platform's file dialogs (the XDG desktop portal on Linux)
    /// instead of the built-in browser when they are available.
    pub use_system_file_dialogs: bool,
}

impl Default for GeneralConfig {
    fn default() -> Self {
        Self {
            last_run: None,
            last_layout: None,
            recent_runs: Vec::new(),
            autosave: true,
            autosave_layout: true,
            use_system_file_dialogs: true,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
#[derive(Default)]
pub struct HotkeyConfig {
    pub enabled: bool,
    pub start_split: String,
    pub reset: String,
    pub undo: String,
    pub skip: String,
    pub pause: String,
    pub undo_all_pauses: String,
    pub switch_comparison_next: String,
    pub switch_comparison_previous: String,
    pub toggle_timing_method: String,
    pub edit_run: String,
    pub edit_layout: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct ServerConfig {
    pub enabled: bool,
    pub autostart: bool,
    pub port: u16,
    pub bind_address: String,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            autostart: true,
            port: 16834,
            bind_address: "127.0.0.1".to_string(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
#[derive(Default)]
pub struct WindowConfig {
    pub width: Option<f32>,
    pub height: Option<f32>,
    pub x: Option<f32>,
    pub y: Option<f32>,
}

/// Niri-specific overlay geometry.
///
/// Niri manages floating windows itself, so its position is kept separate
/// from the generic window position used on X11 and Windows. `x` and `y` are
/// relative to Niri's working area (below layer-shell panels such as a bar).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct NiriConfig {
    pub enabled: bool,
    pub auto_float: bool,
    pub x: Option<f32>,
    pub y: Option<f32>,
    pub width: Option<f32>,
    pub height: Option<f32>,
}

impl Default for NiriConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            auto_float: true,
            x: None,
            y: None,
            width: None,
            height: None,
        }
    }
}

/// speedrun.com integration. Network access is opt-in.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SpeedrunComConfig {
    /// Look up the world record for the World Record component.
    pub fetch_world_records: bool,
}

/// Sound files played on timer events, like LiveSplit's Sound Settings.
///
/// Every path is optional; unset events are silent. `volume` is the general
/// volume from 0 to 100, where 100 plays files at their recorded level.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SoundConfig {
    pub volume: u8,
    pub split: Option<PathBuf>,
    pub split_ahead: Option<PathBuf>,
    pub split_behind: Option<PathBuf>,
    pub best_segment: Option<PathBuf>,
    pub undo_split: Option<PathBuf>,
    pub skip_split: Option<PathBuf>,
    pub personal_best: Option<PathBuf>,
    pub not_a_personal_best: Option<PathBuf>,
    pub reset: Option<PathBuf>,
    pub pause: Option<PathBuf>,
    pub resume: Option<PathBuf>,
    pub start_timer: Option<PathBuf>,
}

impl Default for SoundConfig {
    fn default() -> Self {
        Self {
            volume: 100,
            split: None,
            split_ahead: None,
            split_behind: None,
            best_segment: None,
            undo_split: None,
            skip_split: None,
            personal_best: None,
            not_a_personal_best: None,
            reset: None,
            pause: None,
            resume: None,
            start_timer: None,
        }
    }
}

impl AppConfig {
    pub fn default_path() -> PathBuf {
        ProjectDirs::from("org", "RustSplit", "rustsplit")
            .map(|dirs| dirs.config_dir().to_path_buf())
            .unwrap_or_else(|| PathBuf::from(".rustsplit"))
            .join("settings.toml")
    }

    pub fn load(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(data) => toml::from_str(&data).unwrap_or_else(|error| {
                log::warn!("could not parse {}: {error}", path.display());
                Self::default()
            }),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Self::default(),
            Err(error) => {
                log::warn!("could not read {}: {error}", path.display());
                Self::default()
            }
        }
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let data = toml::to_string_pretty(self)
            .map_err(|error| std::io::Error::other(error.to_string()))?;
        rustsplit_core::fs::write_atomic(path, data)
    }

    pub fn remember_run(&mut self, path: &Path) {
        let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        self.general.last_run = Some(path.clone());
        self.general.recent_runs.retain(|item| item != &path);
        self.general.recent_runs.insert(0, path);
        self.general.recent_runs.truncate(10);
    }

    pub fn remember_layout(&mut self, path: &Path) {
        let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        self.general.last_layout = Some(path);
    }

    /// Applies the edits of the Settings window: every setting it shows that
    /// differs between `draft` and `base` (the settings the window started
    /// from) is copied into `self`.
    ///
    /// Everything else may have changed while the window was open (recent
    /// files, window geometry, the auto splitter choices, the server menu
    /// toggle) and is kept, instead of being reset to the stale copy in the
    /// draft. A setting added to the Settings window must be listed here.
    pub fn apply_settings_edits(&mut self, base: &AppConfig, draft: &AppConfig) {
        macro_rules! edited {
            ($($($field:ident).+),* $(,)?) => {$(
                if draft.$($field).+ != base.$($field).+ {
                    self.$($field).+ = draft.$($field).+.clone();
                }
            )*};
        }
        edited!(
            hotkeys.enabled,
            hotkeys.start_split,
            hotkeys.reset,
            hotkeys.undo,
            hotkeys.skip,
            hotkeys.pause,
            hotkeys.undo_all_pauses,
            hotkeys.switch_comparison_next,
            hotkeys.switch_comparison_previous,
            hotkeys.toggle_timing_method,
            hotkeys.edit_run,
            hotkeys.edit_layout,
            server.enabled,
            server.autostart,
            server.port,
            server.bind_address,
            general.autosave,
            general.autosave_layout,
            general.use_system_file_dialogs,
            speedrun_com.fetch_world_records,
            sounds.volume,
            sounds.split,
            sounds.split_ahead,
            sounds.split_behind,
            sounds.best_segment,
            sounds.undo_split,
            sounds.skip_split,
            sounds.personal_best,
            sounds.not_a_personal_best,
            sounds.reset,
            sounds.pause,
            sounds.resume,
            sounds.start_timer,
            niri.enabled,
            niri.auto_float,
        );
    }
}

/// Directory where RustSplit stores layouts and backups.
pub fn data_dir() -> PathBuf {
    ProjectDirs::from("org", "RustSplit", "rustsplit")
        .map(|dirs| dirs.data_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from(".rustsplit"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_roundtrips() {
        let mut config = AppConfig::default();
        config.general.autosave = false;
        config.server.port = 23456;
        config.hotkeys.start_split = "F1".to_string();
        config.niri.auto_float = false;
        config.niri.x = Some(42.0);
        config.speedrun_com.fetch_world_records = true;
        let directory =
            std::env::temp_dir().join(format!("rustsplit-config-test-{}", std::process::id()));
        let path = directory.join("settings.toml");
        config.save(&path).unwrap();
        let loaded = AppConfig::load(&path);
        assert!(!loaded.general.autosave);
        assert_eq!(loaded.server.port, 23456);
        assert_eq!(loaded.hotkeys.start_split, "F1");
        assert!(!loaded.niri.auto_float);
        assert_eq!(loaded.niri.x, Some(42.0));
        assert!(loaded.speedrun_com.fetch_world_records);
        assert!(!AppConfig::default().speedrun_com.fetch_world_records);
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn system_file_dialogs_default_on_and_roundtrip() {
        let old: AppConfig = toml::from_str("[general]\nautosave = false\n").unwrap();
        assert!(old.general.use_system_file_dialogs);
        let mut config = AppConfig::default();
        config.general.use_system_file_dialogs = false;
        let data = toml::to_string_pretty(&config).unwrap();
        assert!(data.contains("use_system_file_dialogs = false"));
        let loaded: AppConfig = toml::from_str(&data).unwrap();
        assert!(!loaded.general.use_system_file_dialogs);
    }

    #[test]
    fn sound_config_roundtrips() {
        let mut config = AppConfig::default();
        config.sounds.volume = 35;
        config.sounds.split = Some(PathBuf::from("/sounds/split.wav"));
        config.sounds.personal_best = Some(PathBuf::from("/sounds/pb.ogg"));
        let data = toml::to_string_pretty(&config).unwrap();
        assert!(data.contains("[sounds]"));
        let loaded: AppConfig = toml::from_str(&data).unwrap();
        assert_eq!(loaded.sounds, config.sounds);
        assert_eq!(loaded.sounds.reset, None);
    }

    #[test]
    fn settings_edits_keep_changes_made_elsewhere() {
        let base = AppConfig::default();
        let mut draft = base.clone();
        draft.hotkeys.start_split = "F1".to_string();
        draft.sounds.split = Some(PathBuf::from("/sounds/split.wav"));
        draft.server.port = 23456;

        // While the window was open: an auto splitter was chosen, a run
        // opened, the window moved and the server started from the menu.
        let mut config = base.clone();
        config
            .auto_splitter
            .runs
            .insert("/runs/a.lss".to_string(), PathBuf::from("/a.wasm"));
        config.general.last_run = Some(PathBuf::from("/runs/a.lss"));
        config.niri.x = Some(10.0);
        config.server.enabled = true;

        config.apply_settings_edits(&base, &draft);
        assert_eq!(config.hotkeys.start_split, "F1");
        assert_eq!(
            config.sounds.split,
            Some(PathBuf::from("/sounds/split.wav"))
        );
        assert_eq!(config.server.port, 23456);
        assert_eq!(config.auto_splitter.runs.len(), 1);
        assert_eq!(config.general.last_run, Some(PathBuf::from("/runs/a.lss")));
        assert_eq!(config.niri.x, Some(10.0));
        assert!(config.server.enabled, "the draft did not touch the toggle");

        // Unchecking a setting is an edit too.
        let base = config.clone();
        let mut draft = base.clone();
        draft.server.enabled = false;
        draft.general.autosave = false;
        config.apply_settings_edits(&base, &draft);
        assert!(!config.server.enabled);
        assert!(!config.general.autosave);
    }

    #[test]
    fn config_without_sounds_uses_defaults() {
        let loaded: AppConfig =
            toml::from_str("[general]\nautosave = false\n\n[server]\nport = 23456\n").unwrap();
        assert!(!loaded.general.autosave);
        assert_eq!(loaded.sounds, SoundConfig::default());
        assert_eq!(loaded.sounds.volume, 100);

        let partial: AppConfig = toml::from_str("[sounds]\nreset = \"reset.mp3\"\n").unwrap();
        assert_eq!(partial.sounds.reset, Some(PathBuf::from("reset.mp3")));
        assert_eq!(partial.sounds.volume, 100);
    }
}
