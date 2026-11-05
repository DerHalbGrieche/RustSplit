//! Niri compositor integration.
//!
//! Niri is a scrollable tiling Wayland compositor. Regular Wayland clients
//! cannot choose their position and open tiled by default, which is the wrong
//! shape for a speedrun overlay. When a Niri session is detected (through
//! `$NIRI_SOCKET`), RustSplit asks Niri over its IPC socket to move its window
//! to the floating layout and restores the last floating position and size.
//!
//! The IPC protocol is JSON, one request and one reply per line. See
//! <https://github.com/niri-wm/niri/wiki/IPC>.

use std::time::{Duration, Instant};

use serde::Deserialize;
use serde_json::json;

use crate::config::NiriConfig;

/// Wayland application id used for RustSplit's overlay window.
pub const APP_ID: &str = "rustsplit";

const PROBE_INTERVAL: Duration = Duration::from_millis(100);
const PROBE_TIMEOUT: Duration = Duration::from_secs(20);
const TRACK_INTERVAL: Duration = Duration::from_secs(2);
const IO_TIMEOUT: Duration = Duration::from_millis(500);
const DEFAULT_POSITION: f64 = 20.0;

/// How the overlay should be placed once it has been moved to the floating
/// layout. Sizes are in logical pixels and positions are relative to Niri's
/// working area (i.e. below any layer-shell panels such as a top bar).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DesiredWindow {
    pub x: Option<f64>,
    pub y: Option<f64>,
    pub width: i32,
    pub height: i32,
}

/// The floating geometry reported by Niri.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Geometry {
    pub x: f64,
    pub y: f64,
    pub width: i32,
    pub height: i32,
    pub floating: bool,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct WindowLayout {
    #[serde(default)]
    pub window_size: Option<[i32; 2]>,
    #[serde(default)]
    pub tile_size: Option<[f64; 2]>,
    #[serde(default)]
    pub tile_pos_in_workspace_view: Option<[f64; 2]>,
    #[serde(default)]
    pub window_offset_in_tile: Option<[f64; 2]>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Window {
    pub id: u64,
    #[serde(default)]
    pub app_id: Option<String>,
    #[serde(default)]
    pub pid: Option<i32>,
    #[serde(default)]
    pub is_floating: bool,
    #[serde(default)]
    pub layout: WindowLayout,
}

/// Whether RustSplit is running in a Niri session.
pub fn is_session() -> bool {
    std::env::var_os("NIRI_SOCKET").is_some_and(|path| !path.is_empty())
}

/// Find RustSplit's window in a Niri window list.
///
/// Pid is the primary key because it is unique per process; the app id is only
/// used as a fallback for compositor builds that do not report a pid.
pub fn find_window<'a>(windows: &'a [Window], pid: u32, app_id: &str) -> Option<&'a Window> {
    windows
        .iter()
        .find(|window| window.pid == Some(pid as i32) && window.app_id.as_deref() == Some(app_id))
        .or_else(|| windows.iter().find(|window| window.pid == Some(pid as i32)))
        .or_else(|| {
            windows
                .iter()
                .find(|window| window.app_id.as_deref() == Some(app_id))
        })
}

#[cfg(unix)]
mod platform {
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixStream;
    use std::path::PathBuf;

    use serde_json::Value;

    use super::IO_TIMEOUT;

    pub struct Niri {
        pub(super) socket: PathBuf,
    }

    impl Niri {
        pub fn connect() -> Result<Self, String> {
            let socket = std::env::var_os("NIRI_SOCKET")
                .filter(|path| !path.is_empty())
                .map(PathBuf::from)
                .ok_or_else(|| "NIRI_SOCKET is not set".to_string())?;
            Ok(Self { socket })
        }

        pub fn request(&self, request: Value) -> Result<Value, String> {
            let mut stream = UnixStream::connect(&self.socket).map_err(|error| {
                format!(
                    "could not connect to niri at {}: {error}",
                    self.socket.display()
                )
            })?;
            stream
                .set_read_timeout(Some(IO_TIMEOUT))
                .map_err(|error| error.to_string())?;
            stream
                .set_write_timeout(Some(IO_TIMEOUT))
                .map_err(|error| error.to_string())?;
            stream
                .write_all(request.to_string().as_bytes())
                .and_then(|()| stream.write_all(b"\n"))
                .and_then(|()| stream.flush())
                .map_err(|error| format!("could not write to niri: {error}"))?;

            let mut line = String::new();
            BufReader::new(stream)
                .read_line(&mut line)
                .map_err(|error| format!("could not read from niri: {error}"))?;
            let reply: Value = serde_json::from_str(line.trim())
                .map_err(|error| format!("invalid reply from niri: {error}"))?;
            match reply {
                Value::Object(mut map) => match map.remove("Ok") {
                    Some(value) => Ok(value),
                    None => Err(map
                        .remove("Err")
                        .and_then(|value| value.as_str().map(str::to_string))
                        .unwrap_or_else(|| "unknown niri IPC error".to_string())),
                },
                _ => Err("invalid reply from niri".to_string()),
            }
        }

        pub fn windows(&self) -> Result<Vec<super::Window>, String> {
            let value = self.request(Value::String("Windows".to_string()))?;
            let windows = value.get("Windows").cloned().unwrap_or(value);
            serde_json::from_value(windows)
                .map_err(|error| format!("could not parse the niri window list: {error}"))
        }

        pub fn action(&self, action: Value) -> Result<(), String> {
            self.request(json_action(action)).map(|_| ())
        }
    }

    fn json_action(action: Value) -> Value {
        serde_json::json!({ "Action": action })
    }
}

#[cfg(not(unix))]
mod platform {
    use serde_json::Value;

    pub struct Niri;

    impl Niri {
        pub fn connect() -> Result<Self, String> {
            Err("Niri integration is only available on Unix".to_string())
        }

        pub fn request(&self, _request: Value) -> Result<Value, String> {
            Err("Niri integration is only available on Unix".to_string())
        }

        pub fn windows(&self) -> Result<Vec<super::Window>, String> {
            Err("Niri integration is only available on Unix".to_string())
        }

        pub fn action(&self, _action: Value) -> Result<(), String> {
            Err("Niri integration is only available on Unix".to_string())
        }
    }
}

use platform::Niri;

pub struct NiriIntegration {
    niri: Niri,
    pid: u32,
    window_id: Option<u64>,
    position_offset: Option<[f64; 2]>,
    last_requested_position: Option<[f64; 2]>,
    last_synced_surface: Option<(f32, f32)>,
    applied: bool,
    gave_up: bool,
    started: Instant,
    next_probe: Instant,
    next_track: Instant,
    next_surface_sync: Instant,
}

impl NiriIntegration {
    pub fn new(config: &NiriConfig) -> Option<Self> {
        if !config.enabled || !is_session() {
            return None;
        }
        let niri = match Niri::connect() {
            Ok(niri) => niri,
            Err(error) => {
                log::warn!("Niri integration disabled: {error}");
                return None;
            }
        };
        Some(Self {
            niri,
            pid: std::process::id(),
            window_id: None,
            position_offset: None,
            last_requested_position: None,
            last_synced_surface: None,
            applied: false,
            gave_up: false,
            started: Instant::now(),
            next_probe: Instant::now(),
            next_track: Instant::now(),
            next_surface_sync: Instant::now(),
        })
    }

    pub fn is_applied(&self) -> bool {
        self.applied
    }

    /// Move the overlay to the floating layout and restore its geometry.
    ///
    /// This is called once per frame until the window is found and configured.
    /// It returns a short status message exactly once, when configuration
    /// succeeds.
    pub fn poll(
        &mut self,
        auto_float: bool,
        desired: DesiredWindow,
    ) -> Result<Option<String>, String> {
        if self.applied || self.gave_up || Instant::now() < self.next_probe {
            return Ok(None);
        }
        self.next_probe = Instant::now() + PROBE_INTERVAL;

        let Some(window) = self.find_window()? else {
            if self.started.elapsed() > PROBE_TIMEOUT {
                self.gave_up = true;
                log::warn!("Niri: could not find RustSplit's window; leaving it to the compositor");
            }
            return Ok(None);
        };

        if !window.is_floating {
            if auto_float {
                if self.window_id != Some(window.id) {
                    self.niri
                        .action(json!({ "MoveWindowToFloating": { "id": window.id } }))?;
                    self.window_id = Some(window.id);
                }
                // Give Niri a moment to finish the floating transition before
                // setting the size. Otherwise the transition can overwrite it
                // with the size the window had while tiled.
                self.next_probe = Instant::now() + Duration::from_millis(200);
                return Ok(None);
            }

            // The user asked us not to float the window (or a window rule
            // explicitly keeps it tiled). There is no useful geometry to
            // restore until it is floating.
            self.window_id = Some(window.id);
            self.applied = true;
            return Ok(Some(
                "Niri: leaving RustSplit in the tiling layout as configured.".to_string(),
            ));
        }

        self.window_id = Some(window.id);
        let width = desired.width.max(1);
        let height = desired.height.max(1);
        self.niri.action(json!({
            "SetWindowWidth": { "id": window.id, "change": { "SetFixed": width } }
        }))?;
        self.niri.action(json!({
            "SetWindowHeight": { "id": window.id, "change": { "SetFixed": height } }
        }))?;

        let x = desired.x.unwrap_or(DEFAULT_POSITION);
        let y = desired.y.unwrap_or(DEFAULT_POSITION);
        self.niri.action(json!({
            "MoveFloatingWindow": {
                "id": window.id,
                "x": { "SetFixed": x },
                "y": { "SetFixed": y }
            }
        }))?;

        // Niri's floating positions are relative to the working area, while
        // `tile_pos_in_workspace_view` is relative to the output. They differ
        // by the size of layer-shell panels (usually a top bar). Learn the
        // offset from a position we just set so geometry can be persisted
        // without drifting on every launch.
        if let Some(window) = self.find_window()? {
            if let Some(position) = window.layout.tile_pos_in_workspace_view {
                self.position_offset = Some([position[0] - x, position[1] - y]);
                self.last_requested_position = None;
            } else {
                self.last_requested_position = Some([x, y]);
            }
        }

        self.applied = true;
        self.next_track = Instant::now() + TRACK_INTERVAL;
        self.next_surface_sync = Instant::now() + Duration::from_millis(100);
        log::info!(
            "Niri: floating RustSplit window {} at {},{} ({}x{})",
            window.id,
            x,
            y,
            width,
            height
        );
        Ok(Some("Niri: floating overlay enabled.".to_string()))
    }

    /// Whether it is time to sample the current floating geometry.
    pub fn tracking_due(&self) -> bool {
        self.applied && !self.gave_up && Instant::now() >= self.next_track
    }

    /// Whether it is time to sync the client surface with Niri's visual tile.
    pub fn surface_sync_due(&self) -> bool {
        self.applied && !self.gave_up && Instant::now() >= self.next_surface_sync
    }

    /// Whether the client surface still needs to be resized for this Niri
    /// tile size.
    ///
    /// Niri can resize a floating window without sending an xdg configure
    /// event, so the client surface needs to be resized to this size manually
    /// or the OpenGL viewport keeps its old width and clips the overlay.
    pub fn take_surface_sync(&mut self, width: f32, height: f32) -> bool {
        let target = (width, height);
        if self
            .last_synced_surface
            .is_some_and(|last| (last.0 - width).abs() < 1.0 && (last.1 - height).abs() < 1.0)
        {
            return false;
        }
        self.last_synced_surface = Some(target);
        true
    }

    /// Niri's visual size for the floating window.
    pub fn visual_size(&mut self) -> Result<Option<(f32, f32)>, String> {
        self.next_surface_sync = Instant::now() + Duration::from_millis(100);
        let Some(window) = self.find_window()? else {
            self.window_id = None;
            return Ok(None);
        };
        if !window.is_floating {
            return Ok(None);
        }
        let Some([width, height]) = window.layout.tile_size else {
            return Ok(None);
        };
        let [offset_x, offset_y] = window.layout.window_offset_in_tile.unwrap_or([0.0, 0.0]);
        let width = width - 2.0 * offset_x.max(0.0);
        let height = height - 2.0 * offset_y.max(0.0);
        if !width.is_finite() || !height.is_finite() || width <= 1.0 || height <= 1.0 {
            return Ok(None);
        }
        Ok(Some((width as f32, height as f32)))
    }

    /// Read back the current floating geometry.
    pub fn geometry(&mut self) -> Result<Option<Geometry>, String> {
        self.next_track = Instant::now() + TRACK_INTERVAL;
        let Some(window_id) = self.window_id else {
            return Ok(None);
        };
        let Some(window) = self.find_window()? else {
            self.window_id = None;
            return Ok(None);
        };
        if window.id != window_id || !window.is_floating {
            return Ok(None);
        }
        let Some(size) = window.layout.window_size else {
            return Ok(None);
        };
        let Some(position) = window.layout.tile_pos_in_workspace_view else {
            return Ok(None);
        };
        let offset = match self.position_offset {
            Some(offset) => offset,
            None => match self.last_requested_position.take() {
                Some(requested) => {
                    let offset = [position[0] - requested[0], position[1] - requested[1]];
                    self.position_offset = Some(offset);
                    offset
                }
                None => [0.0, 0.0],
            },
        };
        Ok(Some(Geometry {
            x: position[0] - offset[0],
            y: position[1] - offset[1],
            width: size[0],
            height: size[1],
            floating: true,
        }))
    }

    fn find_window(&self) -> Result<Option<Window>, String> {
        let windows = self.niri.windows()?;
        Ok(find_window(&windows, self.pid, APP_ID).cloned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(id: u64, pid: Option<i32>, app_id: &str, floating: bool) -> Window {
        Window {
            id,
            app_id: Some(app_id.to_string()),
            pid,
            is_floating: floating,
            layout: WindowLayout {
                window_size: Some([286, 400]),
                tile_size: Some([286.0, 400.0]),
                tile_pos_in_workspace_view: Some([100.0, 120.0]),
                window_offset_in_tile: Some([0.0, 0.0]),
            },
        }
    }

    #[test]
    fn finds_window_by_pid_and_app_id() {
        let windows = vec![
            window(1, Some(7), "other", false),
            window(2, Some(42), APP_ID, true),
        ];
        assert_eq!(find_window(&windows, 42, APP_ID).map(|w| w.id), Some(2));
    }

    #[test]
    fn falls_back_to_pid_then_app_id() {
        let by_pid = vec![window(3, Some(42), "renamed", true)];
        assert_eq!(find_window(&by_pid, 42, APP_ID).map(|w| w.id), Some(3));

        let by_app_id = vec![window(4, None, APP_ID, true)];
        assert_eq!(find_window(&by_app_id, 42, APP_ID).map(|w| w.id), Some(4));
    }

    #[test]
    fn parses_niri_window_json() {
        let value = serde_json::json!([
            {
                "id": 9,
                "title": "RustSplit",
                "app_id": "rustsplit",
                "pid": 1234,
                "workspace_id": 2,
                "is_focused": true,
                "is_floating": true,
                "is_urgent": false,
                "layout": {
                    "pos_in_scrolling_layout": null,
                    "tile_size": [500.0, 300.0],
                    "window_size": [500, 300],
                    "tile_pos_in_workspace_view": [100.0, 144.0],
                    "window_offset_in_tile": [0.0, 0.0]
                }
            }
        ]);
        let windows: Vec<Window> = serde_json::from_value(value).unwrap();
        let found = find_window(&windows, 1234, APP_ID).unwrap();
        assert!(found.is_floating);
        assert_eq!(found.layout.window_size, Some([500, 300]));
        assert_eq!(found.layout.tile_size, Some([500.0, 300.0]));
        assert_eq!(
            found.layout.tile_pos_in_workspace_view,
            Some([100.0, 144.0])
        );
    }
}
