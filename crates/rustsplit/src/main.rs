//! RustSplit: a native Rust rewrite of LiveSplit.

mod app;
mod auto_splitter;
mod config;
mod control;
mod file_dialog;
mod file_dialogs;
mod hotkeys;
mod input;
mod layout_editor;
mod niri;
mod run_editor;
mod settings_window;
mod sounds;
#[cfg(feature = "speedrun-com")]
mod speedrun_com;

use std::path::{Path, PathBuf};

use clap::Parser;
use rustsplit_core::Timer;

use crate::app::RustSplitApp;
use crate::config::AppConfig;

#[derive(Debug, Parser)]
#[command(
    name = "rustsplit",
    version,
    about = "A native Rust rewrite of LiveSplit"
)]
struct Args {
    /// Split file to open (.lss)
    #[arg(short = 's', long = "splits", value_name = "FILE")]
    splits: Option<PathBuf>,

    /// Layout file to open (.lsl)
    #[arg(short = 'l', long = "layout", value_name = "FILE")]
    layout: Option<PathBuf>,

    /// Path to the configuration file
    #[arg(long = "config", value_name = "FILE")]
    config: Option<PathBuf>,

    /// Start the LiveSplit server
    #[arg(long = "server")]
    server: bool,

    /// Port for the LiveSplit server
    #[arg(long = "port", value_name = "PORT")]
    port: Option<u16>,

    /// Send a command to a running RustSplit instance (for compositor key
    /// bindings)
    #[arg(long = "command", value_name = "COMMAND")]
    command: Option<String>,
}

fn main() -> eframe::Result<()> {
    let mut logger =
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"));
    if let Some(directive) = rfd_log_directive(std::env::var("RUST_LOG").ok().as_deref()) {
        logger.parse_filters(directive);
    }
    logger.init();
    let args = Args::parse();

    if let Some(command) = args.command.as_deref() {
        match control::send_command(command) {
            Ok(response) => {
                if !response.is_empty() {
                    println!("{response}");
                }
                return Ok(());
            }
            Err(error) => {
                eprintln!("{error}");
                std::process::exit(1);
            }
        }
    }

    let config_path = args.config.clone().unwrap_or_else(AppConfig::default_path);
    let mut config = AppConfig::load(&config_path);
    if args.server {
        config.server.enabled = true;
        config.server.autostart = true;
    }
    if let Some(port) = args.port {
        config.server.port = port;
    }

    let run = app::load_initial_run(&mut config, args.splits.as_deref());
    let layout = app::load_initial_layout(&mut config, &run, args.layout.as_deref());
    let timer = Timer::with_system_clock(run);

    let mut options = app::native_options(&layout, &config, &timer);
    if let Some(icon) = load_icon() {
        options.viewport = options.viewport.clone().with_icon(icon);
    }

    eframe::run_native(
        "RustSplit",
        options,
        Box::new(move |cc| {
            let app = RustSplitApp::new(cc, timer, layout, config, config_path);
            Ok(Box::new(app))
        }),
    )
}

/// rfd only reports why a system file dialog could not be shown (no portal,
/// no zenity, a D-Bus error) in its log. Keep its warnings and errors on when
/// `RUST_LOG` neither sets a global level nor mentions rfd, e.g.
/// `RUST_LOG=rustsplit=debug`. The default filter (`info`) already has them.
fn rfd_log_directive(rust_log: Option<&str>) -> Option<&'static str> {
    let filter = rust_log?.split('/').next()?.trim();
    if filter.is_empty() {
        return None;
    }
    let covered = filter.split(',').map(str::trim).any(|directive| {
        directive.parse::<log::LevelFilter>().is_ok()
            || directive == "rfd"
            || directive.starts_with("rfd=")
            || directive.starts_with("rfd::")
    });
    (!covered).then_some("rfd=warn")
}

fn load_icon() -> Option<egui::IconData> {
    let bytes = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/icon.png"));
    let image = image::load_from_memory(bytes).ok()?.to_rgba8();
    let (width, height) = image.dimensions();
    Some(egui::IconData {
        rgba: image.into_raw(),
        width,
        height,
    })
}

/// Keep this helper available for tests and future headless modes.
#[allow(dead_code)]
pub fn run_with_paths(
    config_path: &Path,
    splits: Option<&Path>,
    layout: Option<&Path>,
) -> eframe::Result<()> {
    let mut config = AppConfig::load(config_path);
    let run = app::load_initial_run(&mut config, splits);
    let layout = app::load_initial_layout(&mut config, &run, layout);
    let timer = Timer::with_system_clock(run);
    let options = app::native_options(&layout, &config, &timer);
    let config_path = config_path.to_path_buf();
    eframe::run_native(
        "RustSplit",
        options,
        Box::new(move |cc| {
            Ok(Box::new(RustSplitApp::new(
                cc,
                timer,
                layout,
                config,
                config_path,
            )))
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::rfd_log_directive;

    #[test]
    fn rfd_warnings_stay_visible() {
        assert_eq!(rfd_log_directive(None), None);
        assert_eq!(rfd_log_directive(Some("")), None);
        assert_eq!(rfd_log_directive(Some("debug")), None);
        assert_eq!(rfd_log_directive(Some("off")), None);
        assert_eq!(rfd_log_directive(Some("rustsplit=debug,rfd=off")), None);
        assert_eq!(rfd_log_directive(Some("rustsplit=debug, rfd")), None);
        assert_eq!(rfd_log_directive(Some("rustsplit=debug")), Some("rfd=warn"));
        assert_eq!(
            rfd_log_directive(Some("rustsplit=trace,rfdx=info/split")),
            Some("rfd=warn")
        );
    }
}
