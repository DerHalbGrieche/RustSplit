//! Persistent application settings.

use std::path::{Path, PathBuf};

use directories::ProjectDirs;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub general: GeneralConfig,
    pub hotkeys: HotkeyConfig,
    pub server: ServerConfig,
    pub window: WindowConfig,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            general: GeneralConfig::default(),
            hotkeys: HotkeyConfig::default(),
            server: ServerConfig::default(),
            window: WindowConfig::default(),
        }
    }
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

impl Default for HotkeyConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            start_split: String::new(),
            reset: String::new(),
            undo: String::new(),
            skip: String::new(),
            pause: String::new(),
            undo_all_pauses: String::new(),
            switch_comparison_next: String::new(),
            switch_comparison_previous: String::new(),
            toggle_timing_method: String::new(),
            edit_run: String::new(),
            edit_layout: String::new(),
        }
    }
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
pub struct WindowConfig {
    pub width: Option<f32>,
    pub height: Option<f32>,
    pub x: Option<f32>,
    pub y: Option<f32>,
}

impl Default for WindowConfig {
    fn default() -> Self {
        Self {
            width: None,
            height: None,
            x: None,
            y: None,
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
        let directory = std::env::temp_dir().join(format!(
            "rustsplit-config-test-{}",
            std::process::id()
        ));
        let path = directory.join("settings.toml");
        config.save(&path).unwrap();
        let loaded = AppConfig::load(&path);
        assert!(!loaded.general.autosave);
        assert_eq!(loaded.server.port, 23456);
        assert_eq!(loaded.hotkeys.start_split, "F1");
        let _ = std::fs::remove_dir_all(directory);
    }
}
