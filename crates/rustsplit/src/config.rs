//! Persistent application settings.

use std::path::{Path, PathBuf};

use directories::ProjectDirs;
use serde::{Deserialize, Serialize};

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
}
