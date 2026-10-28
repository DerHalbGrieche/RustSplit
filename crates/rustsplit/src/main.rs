//! RustSplit: a native Rust rewrite of LiveSplit.

mod app;
mod config;
mod file_dialog;
mod hotkeys;
mod layout_editor;
mod run_editor;

use std::path::{Path, PathBuf};

use clap::Parser;
use livesplit_core::Timer;

use crate::app::RustSplitApp;
use crate::config::AppConfig;

#[derive(Debug, Parser)]
#[command(name = "rustsplit", version, about = "A native Rust rewrite of LiveSplit")]
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
}

fn main() -> eframe::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let args = Args::parse();

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
