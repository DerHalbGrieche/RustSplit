//! File dialog front-end: system dialogs with the built-in browser as a
//! fallback.
//!
//! [`FileDialogs`] has the same `open`/`show` interface as the built-in
//! [`FileDialog`]. When system dialogs are enabled and available, `open`
//! starts the platform dialog (the XDG desktop portal on Linux) on a
//! background thread, so the timer keeps running and repainting while it is
//! open. The chosen path comes back over a channel that `show` polls every
//! frame. If the system dialog cannot be shown, the same request is reopened
//! in the built-in browser.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use egui::Context;

use crate::file_dialog::{
    start_directory, with_default_extension, FileDialog, FileDialogAction, FileDialogMode,
};

/// A system dialog that comes back empty faster than this never really
/// opened. rfd reports a missing portal (and a missing zenity fallback) the
/// same way as a cancel, so a near-instant "cancel" is treated as a failure.
const FAST_FAILURE: Duration = Duration::from_millis(350);

/// Everything a dialog needs to know, resolved once for either backend.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DialogRequest {
    pub mode: FileDialogMode,
    pub title: String,
    /// The starting directory, already resolved to an existing directory.
    pub directory: PathBuf,
    /// The suggested file name for save dialogs.
    pub filename: String,
    /// Allowed extensions without the dot; the first is the save default.
    pub extensions: Vec<String>,
}

impl DialogRequest {
    fn new(
        mode: FileDialogMode,
        title: String,
        start: Option<&Path>,
        filename: String,
        extensions: &[&str],
    ) -> Self {
        Self {
            mode,
            title,
            directory: start_directory(start),
            filename,
            extensions: extensions
                .iter()
                .map(|extension| extension.to_string())
                .collect(),
        }
    }
}

/// The answer a system dialog thread sends back.
#[cfg_attr(not(feature = "native-dialogs"), allow(dead_code))]
pub struct NativeReply {
    id: u64,
    /// `Ok(None)` is a cancel (or a failure rfd could not tell apart);
    /// `Err` is a failure such as a panic in the dialog thread.
    result: Result<Option<PathBuf>, String>,
}

/// A way to show system file dialogs without blocking the UI thread.
pub trait NativeBackend {
    /// Whether the backend can plausibly show a dialog in this session.
    fn is_available(&self) -> bool;

    /// Start showing `request` and return immediately. The answer must be
    /// sent on `reply` with the given `id`, followed by a repaint of `ctx`.
    fn launch(
        &self,
        id: u64,
        request: DialogRequest,
        ctx: Option<Context>,
        reply: Sender<NativeReply>,
    ) -> Result<(), String>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Backend {
    Native,
    BuiltIn,
}

/// `native_available` is `None` when this build has no system dialogs.
fn choose_backend(prefer_native: bool, native_available: Option<bool>) -> Backend {
    if prefer_native && native_available == Some(true) {
        Backend::Native
    } else {
        Backend::BuiltIn
    }
}

#[derive(Debug, PartialEq, Eq)]
enum NativeOutcome {
    Picked(PathBuf),
    Cancelled,
    Failed(String),
}

fn classify(
    result: Result<Option<PathBuf>, String>,
    elapsed: Duration,
    fast_failure: Duration,
) -> NativeOutcome {
    match result {
        Ok(Some(path)) => NativeOutcome::Picked(path),
        Ok(None) if elapsed < fast_failure => {
            NativeOutcome::Failed("the system file dialog closed immediately".to_string())
        }
        Ok(None) => NativeOutcome::Cancelled,
        Err(error) => NativeOutcome::Failed(error),
    }
}

/// Turn a path picked in a system dialog into the action the built-in
/// browser would have produced for it.
fn picked_action(request: &DialogRequest, path: PathBuf) -> FileDialogAction {
    match request.mode {
        FileDialogMode::Open => FileDialogAction::Open(path),
        FileDialogMode::Save => {
            FileDialogAction::Save(with_default_extension(path, &request.extensions))
        }
    }
}

struct Pending {
    id: u64,
    request: DialogRequest,
    started: Instant,
}

/// Picks a system or built-in file dialog for each request.
pub struct FileDialogs {
    built_in: FileDialog,
    native: Option<Box<dyn NativeBackend>>,
    prefer_native: bool,
    pending: Option<Pending>,
    next_id: u64,
    sender: Sender<NativeReply>,
    receiver: Receiver<NativeReply>,
    ctx: Option<Context>,
    fast_failure: Duration,
}

impl Default for FileDialogs {
    fn default() -> Self {
        Self::with_native(native::default_backend())
    }
}

impl FileDialogs {
    fn with_native(native: Option<Box<dyn NativeBackend>>) -> Self {
        let (sender, receiver) = mpsc::channel();
        Self {
            built_in: FileDialog::default(),
            native,
            prefer_native: true,
            pending: None,
            next_id: 0,
            sender,
            receiver,
            ctx: None,
            fast_failure: FAST_FAILURE,
        }
    }

    /// Whether system dialogs should be tried first (the user setting).
    pub fn set_prefer_native(&mut self, prefer: bool) {
        self.prefer_native = prefer;
    }

    /// Whether the built-in browser is showing inside the overlay. A pending
    /// system dialog lives in its own window and does not count.
    pub fn is_open(&self) -> bool {
        self.built_in.open
    }

    /// Open a dialog. A newer request replaces any dialog still open, so the
    /// caller's purpose always matches the result `show` returns.
    pub fn open(
        &mut self,
        mode: FileDialogMode,
        title: impl Into<String>,
        start: Option<&Path>,
        filename: impl Into<String>,
        extensions: &[&str],
    ) {
        let request = DialogRequest::new(mode, title.into(), start, filename.into(), extensions);
        self.next_id += 1;
        let id = self.next_id;
        // Any older system dialog is superseded; its answer will be ignored.
        self.pending = None;

        let available = self.native.as_ref().map(|native| native.is_available());
        if choose_backend(self.prefer_native, available) == Backend::BuiltIn {
            self.open_built_in(&request);
            return;
        }
        let Some(native) = &self.native else {
            self.open_built_in(&request);
            return;
        };
        match native.launch(id, request.clone(), self.ctx.clone(), self.sender.clone()) {
            Ok(()) => {
                self.built_in.close();
                self.pending = Some(Pending {
                    id,
                    request,
                    started: Instant::now(),
                });
            }
            Err(error) => {
                log::warn!("could not open the system file dialog: {error}");
                self.open_built_in(&request);
            }
        }
    }

    /// Show the built-in browser if it is open and return an action when a
    /// dialog of either kind is confirmed or cancelled.
    pub fn show(&mut self, ctx: &Context) -> Option<FileDialogAction> {
        if self.ctx.is_none() {
            self.ctx = Some(ctx.clone());
        }
        if let Some(action) = self.poll_native() {
            return Some(action);
        }
        self.built_in.show(ctx)
    }

    fn poll_native(&mut self) -> Option<FileDialogAction> {
        while let Ok(reply) = self.receiver.try_recv() {
            let Some(pending) = self.pending.take_if(|pending| pending.id == reply.id) else {
                continue;
            };
            match classify(reply.result, pending.started.elapsed(), self.fast_failure) {
                NativeOutcome::Picked(path) => return Some(picked_action(&pending.request, path)),
                NativeOutcome::Cancelled => return Some(FileDialogAction::Cancel),
                NativeOutcome::Failed(error) => {
                    log::warn!("{error}; using the built-in file browser");
                    self.open_built_in(&pending.request);
                }
            }
        }
        None
    }

    fn open_built_in(&mut self, request: &DialogRequest) {
        let extensions: Vec<&str> = request.extensions.iter().map(String::as_str).collect();
        self.built_in.open(
            request.mode,
            request.title.clone(),
            Some(&request.directory),
            request.filename.clone(),
            &extensions,
        );
    }
}

#[cfg(not(feature = "native-dialogs"))]
mod native {
    use super::NativeBackend;

    pub fn default_backend() -> Option<Box<dyn NativeBackend>> {
        None
    }
}

#[cfg(feature = "native-dialogs")]
mod native {
    use std::panic::AssertUnwindSafe;
    use std::path::PathBuf;
    use std::sync::mpsc::Sender;

    use egui::Context;

    use super::{DialogRequest, NativeBackend, NativeReply};
    use crate::file_dialog::FileDialogMode;

    pub fn default_backend() -> Option<Box<dyn NativeBackend>> {
        let available = probe();
        if !available {
            log::info!("no desktop portal or zenity found; using the built-in file browser");
        }
        Some(Box::new(RfdBackend { available }))
    }

    struct RfdBackend {
        available: bool,
    }

    impl NativeBackend for RfdBackend {
        fn is_available(&self) -> bool {
            self.available
        }

        fn launch(
            &self,
            id: u64,
            request: DialogRequest,
            ctx: Option<Context>,
            reply: Sender<NativeReply>,
        ) -> Result<(), String> {
            std::thread::Builder::new()
                .name("rustsplit-file-dialog".to_string())
                .spawn(move || {
                    let result = std::panic::catch_unwind(AssertUnwindSafe(|| run(&request)))
                        .map_err(|_| "the system file dialog panicked".to_string());
                    let _ = reply.send(NativeReply { id, result });
                    if let Some(ctx) = ctx {
                        ctx.request_repaint();
                    }
                })
                .map(drop)
                .map_err(|error| format!("could not start the dialog thread: {error}"))
        }
    }

    /// Show the dialog, blocking this (background) thread until it closes.
    fn run(request: &DialogRequest) -> Option<PathBuf> {
        let mut dialog = rfd::AsyncFileDialog::new()
            .set_title(&request.title)
            .set_directory(&request.directory);
        for filter in filters(&request.extensions) {
            dialog = dialog.add_filter(filter.name, &filter.extensions);
        }
        let handle = match request.mode {
            FileDialogMode::Open => pollster::block_on(dialog.pick_file()),
            FileDialogMode::Save => {
                pollster::block_on(dialog.set_file_name(&request.filename).save_file())
            }
        };
        handle.map(|handle| handle.path().to_path_buf())
    }

    #[derive(Debug, PartialEq, Eq)]
    pub(super) struct Filter {
        pub name: String,
        pub extensions: Vec<String>,
    }

    /// One filter matching the built-in browser's extension list. Portal
    /// globs are case-sensitive while the built-in browser is not, so both
    /// lower- and upper-case variants are listed.
    pub(super) fn filters(extensions: &[String]) -> Vec<Filter> {
        let Some(first) = extensions.first() else {
            return Vec::new();
        };
        let kind = filter_kind(first);
        let kind = if extensions
            .iter()
            .all(|extension| filter_kind(extension) == kind)
        {
            kind
        } else {
            "Supported Files"
        };
        let globs: Vec<String> = extensions
            .iter()
            .map(|extension| format!("*.{}", extension.to_ascii_lowercase()))
            .collect();
        let mut variants = Vec::new();
        for extension in extensions {
            for variant in [
                extension.to_ascii_lowercase(),
                extension.to_ascii_uppercase(),
            ] {
                if !variants.contains(&variant) {
                    variants.push(variant);
                }
            }
        }
        vec![Filter {
            name: format!("{kind} ({})", globs.join(" ")),
            extensions: variants,
        }]
    }

    fn filter_kind(extension: &str) -> &'static str {
        match extension.to_ascii_lowercase().as_str() {
            "lss" => "LiveSplit Splits",
            "lsl" => "LiveSplit Layout",
            "png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp" | "ico" | "tga" | "tif" | "tiff" => {
                "Images"
            }
            "wasm" => "Auto Splitters",
            _ => "Supported Files",
        }
    }

    #[cfg(any(
        target_os = "linux",
        target_os = "freebsd",
        target_os = "dragonfly",
        target_os = "netbsd",
        target_os = "openbsd"
    ))]
    fn probe() -> bool {
        let var = std::env::var_os;
        unix_probe(
            var("DBUS_SESSION_BUS_ADDRESS").as_deref(),
            var("XDG_RUNTIME_DIR").as_deref(),
            var("PATH").as_deref(),
        )
    }

    /// Windows and macOS always have a system dialog.
    #[cfg(not(any(
        target_os = "linux",
        target_os = "freebsd",
        target_os = "dragonfly",
        target_os = "netbsd",
        target_os = "openbsd"
    )))]
    fn probe() -> bool {
        true
    }

    /// rfd talks to the portal over the D-Bus session bus and falls back to
    /// `zenity`. Without either, there is nothing to show a dialog with.
    #[cfg(any(
        target_os = "linux",
        target_os = "freebsd",
        target_os = "dragonfly",
        target_os = "netbsd",
        target_os = "openbsd"
    ))]
    pub(super) fn unix_probe(
        bus_address: Option<&std::ffi::OsStr>,
        runtime_dir: Option<&std::ffi::OsStr>,
        path: Option<&std::ffi::OsStr>,
    ) -> bool {
        let session_bus = bus_address.is_some_and(|address| !address.is_empty())
            || runtime_dir
                .is_some_and(|directory| std::path::Path::new(directory).join("bus").exists());
        let zenity = path.is_some_and(|path| {
            std::env::split_paths(path).any(|directory| directory.join("zenity").is_file())
        });
        session_bus || zenity
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;

    #[derive(Default)]
    struct FakeState {
        available: bool,
        fail_launch: bool,
        launched: Vec<(u64, DialogRequest)>,
        reply: Option<Sender<NativeReply>>,
    }

    struct FakeBackend(Arc<Mutex<FakeState>>);

    impl NativeBackend for FakeBackend {
        fn is_available(&self) -> bool {
            self.0.lock().unwrap().available
        }

        fn launch(
            &self,
            id: u64,
            request: DialogRequest,
            _ctx: Option<Context>,
            reply: Sender<NativeReply>,
        ) -> Result<(), String> {
            let mut state = self.0.lock().unwrap();
            if state.fail_launch {
                return Err("no portal".to_string());
            }
            state.launched.push((id, request));
            state.reply = Some(reply);
            Ok(())
        }
    }

    fn fake(available: bool) -> (FileDialogs, Arc<Mutex<FakeState>>) {
        let state = Arc::new(Mutex::new(FakeState {
            available,
            ..FakeState::default()
        }));
        let dialogs = FileDialogs::with_native(Some(Box::new(FakeBackend(state.clone()))));
        (dialogs, state)
    }

    fn reply(state: &Arc<Mutex<FakeState>>, id: u64, result: Result<Option<PathBuf>, String>) {
        let state = state.lock().unwrap();
        state
            .reply
            .as_ref()
            .unwrap()
            .send(NativeReply { id, result })
            .unwrap();
    }

    fn open_save_splits(dialogs: &mut FileDialogs, start: &Path) {
        dialogs.open(
            FileDialogMode::Save,
            "Save Splits As",
            Some(start),
            "Game - Any%.lss",
            &["lss"],
        );
    }

    fn temp_dir(name: &str) -> PathBuf {
        let directory = std::env::temp_dir().join(format!(
            "rustsplit-file-dialogs-{name}-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        directory
    }

    #[test]
    fn backend_selection() {
        assert_eq!(choose_backend(true, Some(true)), Backend::Native);
        assert_eq!(choose_backend(true, Some(false)), Backend::BuiltIn);
        assert_eq!(choose_backend(true, None), Backend::BuiltIn);
        assert_eq!(choose_backend(false, Some(true)), Backend::BuiltIn);
        assert_eq!(choose_backend(false, None), Backend::BuiltIn);
    }

    #[test]
    fn classifies_native_results() {
        let path = PathBuf::from("/runs/a.lss");
        let fast = Duration::from_millis(10);
        let slow = Duration::from_secs(5);
        assert_eq!(
            classify(Ok(Some(path.clone())), fast, FAST_FAILURE),
            NativeOutcome::Picked(path)
        );
        assert!(matches!(
            classify(Ok(None), fast, FAST_FAILURE),
            NativeOutcome::Failed(_)
        ));
        assert_eq!(
            classify(Ok(None), slow, FAST_FAILURE),
            NativeOutcome::Cancelled
        );
        assert_eq!(
            classify(Err("boom".to_string()), slow, FAST_FAILURE),
            NativeOutcome::Failed("boom".to_string())
        );
    }

    #[test]
    fn request_resolves_start_directory_and_defaults() {
        let directory = temp_dir("request");
        let file = directory.join("Run.lss");
        std::fs::write(&file, "").unwrap();

        let request = DialogRequest::new(
            FileDialogMode::Save,
            "Save Splits As".to_string(),
            Some(&file),
            "Run.lss".to_string(),
            &["lss"],
        );
        assert_eq!(request.directory, directory);
        assert_eq!(request.filename, "Run.lss");
        assert_eq!(request.extensions, vec!["lss".to_string()]);

        let from_dir = DialogRequest::new(
            FileDialogMode::Open,
            String::new(),
            Some(&directory),
            String::new(),
            &["lsl"],
        );
        assert_eq!(from_dir.directory, directory);

        let missing = DialogRequest::new(
            FileDialogMode::Open,
            String::new(),
            Some(Path::new("/definitely/not/here/Run.lss")),
            String::new(),
            &[],
        );
        assert_eq!(missing.directory, std::env::current_dir().unwrap());
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn save_results_get_the_default_extension() {
        let request = DialogRequest::new(
            FileDialogMode::Save,
            String::new(),
            None,
            String::new(),
            &["lsl"],
        );
        assert_eq!(
            picked_action(&request, PathBuf::from("/layouts/Mine")),
            FileDialogAction::Save(PathBuf::from("/layouts/Mine.lsl"))
        );
        assert_eq!(
            picked_action(&request, PathBuf::from("/layouts/Mine.LSL")),
            FileDialogAction::Save(PathBuf::from("/layouts/Mine.LSL"))
        );
        let open = DialogRequest {
            mode: FileDialogMode::Open,
            ..request
        };
        assert_eq!(
            picked_action(&open, PathBuf::from("/layouts/Mine")),
            FileDialogAction::Open(PathBuf::from("/layouts/Mine"))
        );
    }

    #[test]
    fn native_request_matches_the_built_in_one() {
        let directory = temp_dir("native-request");
        let (mut dialogs, state) = fake(true);
        open_save_splits(&mut dialogs, &directory);
        assert!(!dialogs.is_open());
        let launched = &state.lock().unwrap().launched;
        assert_eq!(launched.len(), 1);
        assert_eq!(
            launched[0].1,
            DialogRequest {
                mode: FileDialogMode::Save,
                title: "Save Splits As".to_string(),
                directory: directory.clone(),
                filename: "Game - Any%.lss".to_string(),
                extensions: vec!["lss".to_string()],
            }
        );
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn uses_built_in_when_disabled_unavailable_or_failing() {
        let directory = std::env::temp_dir();

        let (mut dialogs, state) = fake(true);
        dialogs.set_prefer_native(false);
        open_save_splits(&mut dialogs, &directory);
        assert!(dialogs.is_open());
        assert!(state.lock().unwrap().launched.is_empty());

        let (mut dialogs, state) = fake(false);
        open_save_splits(&mut dialogs, &directory);
        assert!(dialogs.is_open());
        assert!(state.lock().unwrap().launched.is_empty());

        let (mut dialogs, state) = fake(true);
        state.lock().unwrap().fail_launch = true;
        open_save_splits(&mut dialogs, &directory);
        assert!(dialogs.is_open());

        let mut dialogs = FileDialogs::with_native(None);
        open_save_splits(&mut dialogs, &directory);
        assert!(dialogs.is_open());
    }

    #[test]
    fn native_results_are_dispatched() {
        let directory = std::env::temp_dir();
        let (mut dialogs, state) = fake(true);
        dialogs.fast_failure = Duration::ZERO;

        open_save_splits(&mut dialogs, &directory);
        assert_eq!(dialogs.poll_native(), None);
        reply(&state, 1, Ok(Some(PathBuf::from("/runs/Game"))));
        assert_eq!(
            dialogs.poll_native(),
            Some(FileDialogAction::Save(PathBuf::from("/runs/Game.lss")))
        );
        assert_eq!(dialogs.poll_native(), None);

        open_save_splits(&mut dialogs, &directory);
        reply(&state, 2, Ok(None));
        assert_eq!(dialogs.poll_native(), Some(FileDialogAction::Cancel));
        assert!(!dialogs.is_open());
    }

    #[test]
    fn failed_native_dialog_falls_back_to_built_in() {
        let directory = std::env::temp_dir();
        let (mut dialogs, state) = fake(true);

        // An immediate empty answer means the portal never showed a dialog.
        open_save_splits(&mut dialogs, &directory);
        reply(&state, 1, Ok(None));
        assert_eq!(dialogs.poll_native(), None);
        assert!(dialogs.is_open());

        let (mut dialogs, state) = fake(true);
        dialogs.fast_failure = Duration::ZERO;
        open_save_splits(&mut dialogs, &directory);
        reply(&state, 1, Err("panicked".to_string()));
        assert_eq!(dialogs.poll_native(), None);
        assert!(dialogs.is_open());
    }

    #[test]
    fn superseded_native_results_are_ignored() {
        let directory = std::env::temp_dir();
        let (mut dialogs, state) = fake(true);
        dialogs.fast_failure = Duration::ZERO;

        open_save_splits(&mut dialogs, &directory);
        dialogs.open(FileDialogMode::Open, "Open Layout", None, "", &["lsl"]);
        reply(&state, 1, Ok(Some(PathBuf::from("/runs/Old.lss"))));
        assert_eq!(dialogs.poll_native(), None);
        assert!(!dialogs.is_open());
        reply(&state, 2, Ok(Some(PathBuf::from("/layouts/New.lsl"))));
        assert_eq!(
            dialogs.poll_native(),
            Some(FileDialogAction::Open(PathBuf::from("/layouts/New.lsl")))
        );
    }

    #[cfg(feature = "native-dialogs")]
    #[test]
    fn native_filters_mirror_the_extension_list() {
        use native::{filters, Filter};
        let owned = |list: &[&str]| list.iter().map(|item| item.to_string()).collect::<Vec<_>>();

        assert!(filters(&[]).is_empty());
        assert_eq!(
            filters(&owned(&["lss"])),
            vec![Filter {
                name: "LiveSplit Splits (*.lss)".to_string(),
                extensions: owned(&["lss", "LSS"]),
            }]
        );
        assert_eq!(
            filters(&owned(&["lsl"]))[0].name,
            "LiveSplit Layout (*.lsl)"
        );
        assert_eq!(
            filters(&owned(&["wasm"]))[0].name,
            "Auto Splitters (*.wasm)"
        );
        let images = filters(&owned(&["png", "JPG", "jpeg"]));
        assert_eq!(images[0].name, "Images (*.png *.jpg *.jpeg)");
        assert_eq!(
            images[0].extensions,
            owned(&["png", "PNG", "jpg", "JPG", "jpeg", "JPEG"])
        );
        assert_eq!(
            filters(&owned(&["lss", "lsl"]))[0].name,
            "Supported Files (*.lss *.lsl)"
        );
    }

    #[cfg(all(
        feature = "native-dialogs",
        any(
            target_os = "linux",
            target_os = "freebsd",
            target_os = "dragonfly",
            target_os = "netbsd",
            target_os = "openbsd"
        )
    ))]
    #[test]
    fn unix_probe_needs_a_session_bus_or_zenity() {
        use std::ffi::OsStr;

        use native::unix_probe;

        let empty = temp_dir("probe-empty");
        assert!(!unix_probe(None, None, None));
        assert!(!unix_probe(
            Some(OsStr::new("")),
            Some(empty.as_os_str()),
            Some(empty.as_os_str())
        ));
        assert!(unix_probe(
            Some(OsStr::new("unix:path=/run/user/1000/bus")),
            None,
            None
        ));

        let runtime = temp_dir("probe-runtime");
        std::fs::write(runtime.join("bus"), "").unwrap();
        assert!(unix_probe(None, Some(runtime.as_os_str()), None));

        let bin = temp_dir("probe-bin");
        std::fs::write(bin.join("zenity"), "").unwrap();
        let path = std::env::join_paths([empty.clone(), bin.clone()]).unwrap();
        assert!(unix_probe(None, None, Some(&path)));

        for directory in [empty, runtime, bin] {
            let _ = std::fs::remove_dir_all(directory);
        }
    }
}
