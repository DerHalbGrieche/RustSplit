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
    pub sounds: SoundConfig,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct GeneralConfig {
    pub last_run: Option<PathBuf>,
    pub last_layout: Option<PathBuf>,
    pub recent_runs: Vec<PathBuf>,
    pub autosave: bool,
    pub autosave_layout: bool,
}

impl Default for GeneralConfig {
    fn default() -> Self {
        Self {
            last_run: None,
            last_layout: None,
            recent_runs: Vec::new(),
            autosave: true,
            autosave_layout: true,
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
        let temporary = path.with_extension("toml.tmp");
        std::fs::write(&temporary, data)?;
        std::fs::rename(temporary, path)?;
        Ok(())
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
        let _ = std::fs::remove_dir_all(directory);
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
