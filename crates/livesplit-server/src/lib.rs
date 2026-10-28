//! LiveSplit Server compatible remote control over TCP.
//!
//! The server owns the socket and the connection threads, while the timer
//! itself stays on the application's main thread.  Every parsed command is
//! sent to the application through [`ServerHandle::try_recv`].  The
//! application applies it with [`handle_command`] and optionally sends a
//! response back with [`ServerRequest::respond`].
//!
//! ```no_run
//! use livesplit_server::{handle_command, ServerHandle};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let server = ServerHandle::start(16834)?;
//! // In the application's frame loop:
//! // if let Some(request) = server.try_recv() {
//! //     let outcome = handle_command(&mut timer, &request.command);
//! //     request.respond(outcome.response);
//! // }
//! # Ok(())
//! # }
//! ```
//!
//! Command names are case sensitive, exactly like LiveSplit's server.  A
//! command is terminated by `\n`; every response is terminated by `\n`.
//!
//! # Split indices
//!
//! Split indices in the text protocol are **one-based** (`1` is the first
//! split) and are converted to zero-based indices in [`ServerCommand`].  `0`
//! is also accepted as an alias for the first split for compatibility with
//! clients written for LiveSplit's zero-based indexing.  Negative indices
//! address splits from the end (`-1` is the last split), following
//! LiveSplit's convention.  [`ServerCommand::GetSplitIndex`] reports the
//! current split one-based, or `-1` while the timer is not running.

use std::io::{BufRead, BufReader, ErrorKind, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{
    sync_channel, Receiver, RecvTimeoutError, SyncSender, TrySendError,
};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use livesplit_core::analysis;
use livesplit_core::model::{TimerPhase, TimingMethod, BEST_SEGMENTS};
use livesplit_core::time::TimeSpan;
use livesplit_core::Timer;
use thiserror::Error;

/// Default TCP port used by LiveSplit Server.
pub const DEFAULT_PORT: u16 = 16834;

/// Default bind address.  Only local clients are accepted by default.
pub const DEFAULT_BIND_ADDRESS: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);

const REQUEST_QUEUE_SIZE: usize = 256;
const ACCEPT_POLL_INTERVAL: Duration = Duration::from_millis(10);
const CLIENT_READ_TIMEOUT: Duration = Duration::from_millis(100);
const REQUEST_SEND_TIMEOUT: Duration = Duration::from_secs(2);
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(5);
const RESPONSE_POLL_INTERVAL: Duration = Duration::from_millis(50);

/// Errors that can occur while starting the TCP server.
#[derive(Debug, Error)]
pub enum ServerError {
    /// The listening socket could not be bound.
    #[error("failed to bind LiveSplit server to {addr}: {source}")]
    Bind {
        addr: SocketAddr,
        #[source]
        source: std::io::Error,
    },
    /// Another socket or thread operation failed.
    #[error("LiveSplit server I/O error: {0}")]
    Io(#[from] std::io::Error),
}

/// A command received from a remote client.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ServerCommand {
    StartOrSplit,
    Start,
    Split,
    /// `undosplit` / `unsplit`
    Undo,
    Skip,
    Reset,
    Pause,
    Resume,
    TogglePause,
    UndoAllPauses,
    ResetAndSetAttemptAsPb,
    InitializeGameTime,
    SetGameTime(Option<TimeSpan>),
    SetLoadingTimes(TimeSpan),
    AddLoadingTimes(TimeSpan),
    PauseGameTime,
    ResumeGameTime,
    AlwaysPauseGameTime,
    SwitchToRealTime,
    SwitchToGameTime,
    ToggleTimingMethod,
    SwitchComparison(String),
    SwitchComparisonNext,
    SwitchComparisonPrevious,
    GetCurrentTime,
    GetCurrentRealTime,
    GetCurrentGameTime,
    GetLastSplitTime,
    GetCurrentSplitTime(Option<String>),
    GetComparisons,
    GetCurrentComparison,
    GetCurrentTimerPhase,
    GetSplitIndex,
    GetSplitNames,
    GetSplitCount,
    /// Zero-based split index.  The text protocol converts one-based positive
    /// indices before constructing this value.
    GetSplitName { index: i64 },
    /// Zero-based split index.  The text protocol converts one-based positive
    /// indices before constructing this value.
    SetSplitName { index: i64, name: String },
    SetCurrentSplitName(String),
    GetCurrentSplitName,
    GetPreviousSplitName,
    GetUpcomingSplitName,
    GetTimerState,
    GetDelta(Option<String>),
    GetFinalTime(Option<String>),
    GetPredictedTime(Option<String>),
    GetBestPossibleTime,
    GetPausedRealTime,
    GetPausedGameTime,
    GetOffset,
    GetTimingMethod,
    GetGameName,
    GetCategoryName,
    GetCategoryVariables,
    GetAttemptCount,
    GetCompletedCount,
    GetCustomVariableValue(String),
    SetCustomVariable { name: String, value: String },
    GetLayoutPath,
    SaveLayout,
    SaveLayoutAs(String),
    GetSplitsPath,
    SaveSplits,
    SaveSplitsAs(String),
    SwitchLayout(String),
    SwitchSplits(String),
    GetSplitsScreenshot,
    SaveSplitsScreenshot(String),
    GlobalHotkeysEnabled,
    EnableGlobalHotkeys,
    DisableGlobalHotkeys,
    SwitchHotkeyProfile(String),
    GetHotkeyProfile,
    GetAutoSplitterPath,
    AutoSplitterActivated,
    GetLiveSplitVersion,
    GetLiveSplitPath,
    GetServerType,
    Ping,
    /// Stop the server and/or the application.
    Quit,
}

/// Errors produced while parsing a command line.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum ServerCommandError {
    #[error("empty command")]
    Empty,
    #[error("unknown command: {0}")]
    UnknownCommand(String),
    #[error("command {command} requires an argument")]
    MissingArgument { command: String },
    #[error("invalid argument for {command}: {value}")]
    InvalidArgument {
        command: String,
        value: String,
    },
}

/// The result of applying a [`ServerCommand`] to a timer.
///
/// The response, if any, is sent back to the client by the application.
/// `quit` tells the application to shut the server down, while `save` asks it
/// to persist the current run/layout (the server itself never touches files
/// that belong to the application).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CommandOutcome {
    pub response: Option<String>,
    pub quit: bool,
    pub save: bool,
}

impl CommandOutcome {
    fn reply(response: impl Into<String>) -> Self {
        Self {
            response: Some(response.into()),
            quit: false,
            save: false,
        }
    }

    fn none() -> Self {
        Self::default()
    }
}

/// A command waiting to be handled by the application.
///
/// Dropping a request without responding acknowledges it implicitly, so a
/// client never has to wait for the full response timeout.
pub struct ServerRequest {
    pub command: ServerCommand,
    respond: Option<SyncSender<Option<String>>>,
}

impl ServerRequest {
    /// Send the optional response back to the client and close the request.
    pub fn respond(mut self, response: Option<String>) {
        if let Some(sender) = self.respond.take() {
            let _ = sender.send(response);
        }
    }

    /// Convenience wrapper for commands that always produce a response.
    pub fn respond_str(mut self, response: impl Into<String>) {
        if let Some(sender) = self.respond.take() {
            let _ = sender.send(Some(response.into()));
        }
    }
}

impl Drop for ServerRequest {
    fn drop(&mut self) {
        if let Some(sender) = self.respond.take() {
            // The client treats `None` as "no response" and moves on.
            let _ = sender.send(None);
        }
    }
}

impl std::fmt::Debug for ServerRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServerRequest")
            .field("command", &self.command)
            .finish_non_exhaustive()
    }
}

/// Handle to the background TCP server.
pub struct ServerHandle {
    receiver: Receiver<ServerRequest>,
    running: Arc<AtomicBool>,
    finished: Arc<AtomicBool>,
    thread: Mutex<Option<JoinHandle<()>>>,
    bind_addr: SocketAddr,
}

impl ServerHandle {
    /// Start a server on `127.0.0.1:port`.
    ///
    /// Pass port `0` to let the operating system choose an ephemeral port;
    /// the actual address is available through [`Self::bind_addr`].
    pub fn start(port: u16) -> Result<Self, ServerError> {
        Self::start_with_addr(SocketAddr::new(DEFAULT_BIND_ADDRESS, port))
    }

    /// Start a server on the default LiveSplit port.
    pub fn start_default() -> Result<Self, ServerError> {
        Self::start(DEFAULT_PORT)
    }

    /// Start a server with an explicit bind address.
    pub fn start_with_addr(addr: SocketAddr) -> Result<Self, ServerError> {
        let listener = TcpListener::bind(addr).map_err(|source| ServerError::Bind {
            addr,
            source,
        })?;
        listener.set_nonblocking(true)?;
        let bind_addr = listener.local_addr()?;

        let running = Arc::new(AtomicBool::new(true));
        let finished = Arc::new(AtomicBool::new(false));
        let (sender, receiver) = sync_channel(REQUEST_QUEUE_SIZE);

        let thread_running = Arc::clone(&running);
        let thread_finished = Arc::clone(&finished);
        let thread = thread::Builder::new()
            .name("livesplit-server".to_string())
            .spawn(move || {
                accept_loop(listener, sender, thread_running);
                thread_finished.store(true, Ordering::SeqCst);
            })?;

        Ok(Self {
            receiver,
            running,
            finished,
            thread: Mutex::new(Some(thread)),
            bind_addr,
        })
    }

    /// Receive the next pending command, if there is one.
    pub fn try_recv(&self) -> Option<ServerRequest> {
        self.receiver.try_recv().ok()
    }

    /// Stop the accept loop.  Already connected clients are disconnected on
    /// their next read timeout (within roughly 100 ms).
    pub fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
        if let Ok(mut guard) = self.thread.lock() {
            if let Some(handle) = guard.take() {
                // Joining ourselves would deadlock; this can only happen if a
                // callback on the accept thread calls stop, which the public
                // API does not expose.
                if handle.thread().id() != thread::current().id() {
                    let _ = handle.join();
                }
            }
        }
    }

    /// Whether the server is still accepting connections.
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst) && !self.finished.load(Ordering::SeqCst)
    }

    /// The actual TCP port the server is listening on.
    pub fn port(&self) -> u16 {
        self.bind_addr.port()
    }

    /// The actual bound address, including the ephemeral port for port `0`.
    pub fn bind_addr(&self) -> SocketAddr {
        self.bind_addr
    }
}

impl Drop for ServerHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

fn accept_loop(listener: TcpListener, sender: SyncSender<ServerRequest>, running: Arc<AtomicBool>) {
    while running.load(Ordering::SeqCst) {
        match listener.accept() {
            Ok((stream, peer)) => {
                let _ = stream.set_nodelay(true);
                log::debug!("[server] accepted connection from {peer}");
                let client_sender = sender.clone();
                let client_running = Arc::clone(&running);
                if let Err(error) = thread::Builder::new()
                    .name("livesplit-server-client".to_string())
                    .spawn(move || client_loop(stream, client_sender, client_running))
                {
                    log::warn!("[server] could not spawn client thread: {error}");
                }
            }
            Err(error) if error.kind() == ErrorKind::WouldBlock => {
                thread::sleep(ACCEPT_POLL_INTERVAL);
            }
            Err(error) if error.kind() == ErrorKind::Interrupted => {}
            Err(error) => {
                log::warn!("[server] accept failed: {error}");
                thread::sleep(ACCEPT_POLL_INTERVAL);
            }
        }
    }
}

fn client_loop(stream: TcpStream, sender: SyncSender<ServerRequest>, running: Arc<AtomicBool>) {
    let _ = stream.set_read_timeout(Some(CLIENT_READ_TIMEOUT));
    let _ = stream.set_write_timeout(Some(RESPONSE_TIMEOUT));
    let reader_stream = match stream.try_clone() {
        Ok(stream) => stream,
        Err(error) => {
            log::debug!("[server] could not clone client stream: {error}");
            return;
        }
    };
    let mut reader = BufReader::new(reader_stream);
    let mut writer = stream;
    let mut buffer = Vec::new();

    while running.load(Ordering::SeqCst) {
        match reader.read_until(b'\n', &mut buffer) {
            Ok(0) => break,
            Ok(_) => {
                // `read_until` only stops early on EOF; keep a partial line
                // buffered in that case and let the next read finish it.
                if !buffer.ends_with(b"\n") {
                    continue;
                }
                buffer.pop();
                if buffer.last() == Some(&b'\r') {
                    buffer.pop();
                }
                let line = String::from_utf8_lossy(&buffer).into_owned();
                buffer.clear();

                if line.trim().is_empty() {
                    continue;
                }

                match parse_command(&line) {
                    Ok(command) => {
                        if !dispatch_command(command, &sender, &running, &mut writer) {
                            break;
                        }
                    }
                    Err(error) => {
                        // LiveSplit silently ignores invalid commands; log it
                        // and leave the connection open.
                        log::warn!("[server] invalid command {line:?}: {error}");
                    }
                }
            }
            Err(error)
                if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) =>
            {
                continue;
            }
            Err(error) => {
                log::debug!("[server] client read failed: {error}");
                break;
            }
        }
    }
}

fn dispatch_command(
    command: ServerCommand,
    sender: &SyncSender<ServerRequest>,
    running: &AtomicBool,
    writer: &mut TcpStream,
) -> bool {
    let (response_sender, response_receiver) = sync_channel(1);
    let request = ServerRequest {
        command,
        respond: Some(response_sender),
    };

    let deadline = Instant::now() + REQUEST_SEND_TIMEOUT;
    let mut request = request;
    loop {
        match sender.try_send(request) {
            Ok(()) => break,
            Err(TrySendError::Full(returned)) => {
                if !running.load(Ordering::SeqCst) || Instant::now() >= deadline {
                    log::warn!(
                        "[server] command queue is full; dropping {:?}",
                        returned.command
                    );
                    return true;
                }
                request = returned;
                thread::sleep(Duration::from_millis(10));
            }
            Err(TrySendError::Disconnected(_)) => return false,
        }
    }

    match wait_for_response(&response_receiver, running) {
        Some(Some(response)) => {
            if let Err(error) = write_response(writer, &response) {
                log::debug!("[server] client write failed: {error}");
                return false;
            }
        }
        Some(None) => {}
        None => {
            log::warn!("[server] application did not answer a command in time");
        }
    }

    true
}

fn wait_for_response(
    receiver: &Receiver<Option<String>>,
    running: &AtomicBool,
) -> Option<Option<String>> {
    let deadline = Instant::now() + RESPONSE_TIMEOUT;
    loop {
        match receiver.recv_timeout(RESPONSE_POLL_INTERVAL) {
            Ok(response) => return Some(response),
            Err(RecvTimeoutError::Timeout) => {
                if !running.load(Ordering::SeqCst) || Instant::now() >= deadline {
                    return None;
                }
            }
            Err(RecvTimeoutError::Disconnected) => return Some(None),
        }
    }
}

fn write_response(writer: &mut TcpStream, response: &str) -> std::io::Result<()> {
    writer.write_all(response.as_bytes())?;
    writer.write_all(b"\n")?;
    writer.flush()
}

/// Parse a single command line (without its trailing newline).
///
/// Parsing is case sensitive, matching LiveSplit.  Commands that take a
/// comparison or a name consume the rest of the line, so those values may
/// contain spaces.
pub fn parse_command(line: &str) -> Result<ServerCommand, ServerCommandError> {
    let line = line.trim_end_matches(['\r', '\n']);
    if line.is_empty() {
        return Err(ServerCommandError::Empty);
    }

    let (command, argument) = match line.split_once(' ') {
        Some((command, argument)) => (command, Some(argument)),
        None => (line, None),
    };

    macro_rules! required_argument {
        () => {
            argument.ok_or(ServerCommandError::MissingArgument {
                command: command.to_string(),
            })?
        };
    }

    macro_rules! optional_argument {
        () => {
            argument.map(str::trim)
        };
    }

    let parsed = match command {
        "startorsplit" => ServerCommand::StartOrSplit,
        "start" | "starttimer" => ServerCommand::Start,
        "split" => ServerCommand::Split,
        "undosplit" | "unsplit" => ServerCommand::Undo,
        "skipsplit" => ServerCommand::Skip,
        "reset" => ServerCommand::Reset,
        "pause" => ServerCommand::Pause,
        "resume" => ServerCommand::Resume,
        "togglepause" => ServerCommand::TogglePause,
        "undoallpauses" => ServerCommand::UndoAllPauses,
        "resetandsetattemptaspb" | "resetattemptaspb" => {
            ServerCommand::ResetAndSetAttemptAsPb
        }
        "initializegametime" => ServerCommand::InitializeGameTime,
        "setgametime" => {
            let value = required_argument!();
            ServerCommand::SetGameTime(parse_optional_time(command, value)?)
        }
        "setloadingtimes" => {
            let value = required_argument!();
            ServerCommand::SetLoadingTimes(parse_required_time(command, value)?)
        }
        "addloadingtimes" => {
            let value = required_argument!();
            ServerCommand::AddLoadingTimes(parse_required_time(command, value)?)
        }
        "pausegametime" => ServerCommand::PauseGameTime,
        "unpausegametime" | "resumegametime" => ServerCommand::ResumeGameTime,
        "alwayspausegametime" => ServerCommand::AlwaysPauseGameTime,
        "switchtorealtime" => ServerCommand::SwitchToRealTime,
        "switchtogametime" => ServerCommand::SwitchToGameTime,
        "toggletimingmethod" | "toggletiming" => ServerCommand::ToggleTimingMethod,
        "switchto" => {
            let value = required_argument!().trim();
            match value {
                "realtime" => ServerCommand::SwitchToRealTime,
                "gametime" => ServerCommand::SwitchToGameTime,
                _ => {
                    return Err(ServerCommandError::InvalidArgument {
                        command: command.to_string(),
                        value: value.to_string(),
                    })
                }
            }
        }
        "settimingmethod" => {
            let value = required_argument!().trim();
            match value {
                "realtime" => ServerCommand::SwitchToRealTime,
                "gametime" => ServerCommand::SwitchToGameTime,
                _ => {
                    return Err(ServerCommandError::InvalidArgument {
                        command: command.to_string(),
                        value: value.to_string(),
                    })
                }
            }
        }
        "setcomparison" | "switchcomparison" => {
            ServerCommand::SwitchComparison(required_argument!().to_string())
        }
        "switchcomparisonnext" | "nextcomparison" => ServerCommand::SwitchComparisonNext,
        "switchcomparisonprevious" | "previouscomparison" => {
            ServerCommand::SwitchComparisonPrevious
        }
        "getcurrenttime" => ServerCommand::GetCurrentTime,
        "getcurrentrealtime" => ServerCommand::GetCurrentRealTime,
        "getcurrentgametime" => ServerCommand::GetCurrentGameTime,
        "getlastsplittime" | "getprevioussplittime" => ServerCommand::GetLastSplitTime,
        "getcurrentsplittime" | "getcomparisonsplittime" => {
            ServerCommand::GetCurrentSplitTime(optional_argument!().map(str::to_string))
        }
        "getcomparisons" => ServerCommand::GetComparisons,
        "getcomparisonname" | "getcurrentcomparison" => ServerCommand::GetCurrentComparison,
        "gettimerphase" | "getcurrenttimerphase" => ServerCommand::GetCurrentTimerPhase,
        "getsplitindex" => ServerCommand::GetSplitIndex,
        "getsplitnames" => ServerCommand::GetSplitNames,
        "getsplitcount" => ServerCommand::GetSplitCount,
        "getsplitname" => ServerCommand::GetSplitName {
            index: parse_split_index(command, required_argument!())?,
        },
        "setsplitname" => {
            let value = required_argument!().trim_start();
            let (index, name) = value.split_once(' ').ok_or(
                ServerCommandError::MissingArgument {
                    command: command.to_string(),
                },
            )?;
            ServerCommand::SetSplitName {
                index: parse_split_index(command, index)?,
                name: name.to_string(),
            }
        }
        "setcurrentsplitname" => {
            ServerCommand::SetCurrentSplitName(required_argument!().to_string())
        }
        "getcurrentsplitname" => ServerCommand::GetCurrentSplitName,
        "getlastsplitname" | "getprevioussplitname" => ServerCommand::GetPreviousSplitName,
        "getnextsplitname" | "getupcomingsplitname" => ServerCommand::GetUpcomingSplitName,
        "gettimerstate" | "getstate" => ServerCommand::GetTimerState,
        "getdelta" => ServerCommand::GetDelta(optional_argument!().map(str::to_string)),
        "getfinaltime" | "getfinalsplittime" => {
            ServerCommand::GetFinalTime(optional_argument!().map(str::to_string))
        }
        "getpredictedtime" => {
            ServerCommand::GetPredictedTime(optional_argument!().map(str::to_string))
        }
        "getbestpossibletime" => ServerCommand::GetBestPossibleTime,
        "getpausedrealtime" => ServerCommand::GetPausedRealTime,
        "getpausedgametime" => ServerCommand::GetPausedGameTime,
        "getoffset" => ServerCommand::GetOffset,
        "gettimingmethod" => ServerCommand::GetTimingMethod,
        "getgamename" => ServerCommand::GetGameName,
        "getcategoryname" => ServerCommand::GetCategoryName,
        "getcategoryvariables" => ServerCommand::GetCategoryVariables,
        "getattemptcount" => ServerCommand::GetAttemptCount,
        "getcompletedcount" => ServerCommand::GetCompletedCount,
        "getcustomvariablevalue" => {
            ServerCommand::GetCustomVariableValue(required_argument!().to_string())
        }
        "setcustomvariable" => {
            let value = required_argument!();
            let mut values = parse_json_string_array(value).map_err(|reason| {
                ServerCommandError::InvalidArgument {
                    command: command.to_string(),
                    value: format!("{value} ({reason})"),
                }
            })?;
            if values.len() < 2 {
                return Err(ServerCommandError::InvalidArgument {
                    command: command.to_string(),
                    value: value.to_string(),
                });
            }
            let value = values.pop().unwrap_or_default();
            let name = values.pop().unwrap_or_default();
            ServerCommand::SetCustomVariable { name, value }
        }
        "getlayoutpath" => ServerCommand::GetLayoutPath,
        "savelayout" => ServerCommand::SaveLayout,
        "savelayoutas" => ServerCommand::SaveLayoutAs(required_argument!().to_string()),
        "getsplitspath" => ServerCommand::GetSplitsPath,
        "savesplits" => ServerCommand::SaveSplits,
        "savesplitsas" => ServerCommand::SaveSplitsAs(required_argument!().to_string()),
        "switchlayout" => ServerCommand::SwitchLayout(required_argument!().to_string()),
        "switchsplits" => ServerCommand::SwitchSplits(required_argument!().to_string()),
        "getsplitsscreenshot" => ServerCommand::GetSplitsScreenshot,
        "savesplitsscreenshot" => {
            ServerCommand::SaveSplitsScreenshot(required_argument!().to_string())
        }
        "globalhotkeysenabled" => ServerCommand::GlobalHotkeysEnabled,
        "enableglobalhotkeys" => ServerCommand::EnableGlobalHotkeys,
        "disableglobalhotkeys" => ServerCommand::DisableGlobalHotkeys,
        "switchhotkeyprofile" => {
            ServerCommand::SwitchHotkeyProfile(required_argument!().to_string())
        }
        "gethotkeyprofile" => ServerCommand::GetHotkeyProfile,
        "getautosplitterpath" => ServerCommand::GetAutoSplitterPath,
        "autosplitteractivated" => ServerCommand::AutoSplitterActivated,
        "getlivesplitversion" => ServerCommand::GetLiveSplitVersion,
        "getlivesplitpath" => ServerCommand::GetLiveSplitPath,
        "getservertype" => ServerCommand::GetServerType,
        "ping" => ServerCommand::Ping,
        "quit" | "exit" => ServerCommand::Quit,
        _ => return Err(ServerCommandError::UnknownCommand(command.to_string())),
    };

    Ok(parsed)
}

fn parse_optional_time(
    command: &str,
    value: &str,
) -> Result<Option<TimeSpan>, ServerCommandError> {
    let value = value.trim();
    if value == "-" || value.is_empty() {
        return Ok(None);
    }
    TimeSpan::parse(value)
        .map(Some)
        .map_err(|_| ServerCommandError::InvalidArgument {
            command: command.to_string(),
            value: value.to_string(),
        })
}

fn parse_required_time(
    command: &str,
    value: &str,
) -> Result<TimeSpan, ServerCommandError> {
    Ok(parse_optional_time(command, value)?.unwrap_or(TimeSpan::ZERO))
}

fn parse_split_index(command: &str, value: &str) -> Result<i64, ServerCommandError> {
    let value = value.trim();
    let parsed = value
        .parse::<i64>()
        .map_err(|_| ServerCommandError::InvalidArgument {
            command: command.to_string(),
            value: value.to_string(),
        })?;

    // Positive values are one-based on the wire and zero-based internally.
    // Zero is accepted as an alias for the first split; negative values keep
    // their LiveSplit meaning of counting from the end.
    if parsed > 0 {
        Ok(parsed - 1)
    } else {
        Ok(parsed)
    }
}

fn resolve_split_index(index: i64, split_count: usize) -> Option<usize> {
    let count = i64::try_from(split_count).ok()?;
    let resolved = if index < 0 {
        count.checked_add(index)?
    } else {
        index
    };
    if resolved >= 0 && resolved < count {
        Some(resolved as usize)
    } else {
        None
    }
}

/// Apply a command to `timer` and produce the protocol response.
pub fn handle_command(timer: &mut Timer, command: &ServerCommand) -> CommandOutcome {
    match command {
        ServerCommand::StartOrSplit => {
            if timer.phase == TimerPhase::Running {
                timer.split();
            } else {
                timer.start();
            }
            CommandOutcome::none()
        }
        ServerCommand::Start => {
            timer.start();
            CommandOutcome::none()
        }
        ServerCommand::Split => {
            timer.split();
            CommandOutcome::none()
        }
        ServerCommand::Undo => {
            timer.undo_split();
            CommandOutcome::none()
        }
        ServerCommand::Skip => {
            timer.skip_split();
            CommandOutcome::none()
        }
        ServerCommand::Reset => {
            timer.reset();
            CommandOutcome::none()
        }
        ServerCommand::Pause => {
            if timer.phase != TimerPhase::Paused {
                timer.pause();
            }
            CommandOutcome::none()
        }
        ServerCommand::Resume => {
            if timer.phase == TimerPhase::Paused {
                timer.pause();
            }
            CommandOutcome::none()
        }
        ServerCommand::TogglePause => {
            timer.pause();
            CommandOutcome::none()
        }
        ServerCommand::UndoAllPauses => {
            timer.undo_all_pauses();
            CommandOutcome::none()
        }
        ServerCommand::ResetAndSetAttemptAsPb => {
            timer.reset_and_set_attempt_as_pb();
            CommandOutcome::none()
        }
        ServerCommand::InitializeGameTime => {
            timer.initialize_game_time();
            CommandOutcome::none()
        }
        ServerCommand::SetGameTime(time) => {
            timer.set_game_time(*time);
            CommandOutcome::none()
        }
        ServerCommand::SetLoadingTimes(time) => {
            timer.set_loading_times(*time);
            CommandOutcome::none()
        }
        ServerCommand::AddLoadingTimes(time) => {
            timer.set_loading_times(timer.loading_times + *time);
            CommandOutcome::none()
        }
        ServerCommand::PauseGameTime | ServerCommand::AlwaysPauseGameTime => {
            timer.set_game_time_paused(true);
            CommandOutcome::none()
        }
        ServerCommand::ResumeGameTime => {
            timer.set_game_time_paused(false);
            CommandOutcome::none()
        }
        ServerCommand::SwitchToRealTime => {
            timer.set_timing_method(TimingMethod::RealTime);
            CommandOutcome::none()
        }
        ServerCommand::SwitchToGameTime => {
            timer.set_timing_method(TimingMethod::GameTime);
            CommandOutcome::none()
        }
        ServerCommand::ToggleTimingMethod => {
            timer.toggle_timing_method();
            CommandOutcome::none()
        }
        ServerCommand::SwitchComparison(comparison) => {
            timer.set_current_comparison(comparison.clone());
            CommandOutcome::none()
        }
        ServerCommand::SwitchComparisonNext => {
            timer.switch_comparison_next();
            CommandOutcome::none()
        }
        ServerCommand::SwitchComparisonPrevious => {
            timer.switch_comparison_previous();
            CommandOutcome::none()
        }
        ServerCommand::GetCurrentTime => CommandOutcome::reply(format_time(
            current_time_for(timer, timer.current_timing_method, true),
        )),
        ServerCommand::GetCurrentRealTime => {
            CommandOutcome::reply(format_time(current_time_for(timer, TimingMethod::RealTime, false)))
        }
        ServerCommand::GetCurrentGameTime => {
            CommandOutcome::reply(format_time(current_time_for(timer, TimingMethod::GameTime, true)))
        }
        ServerCommand::GetLastSplitTime => {
            let index = timer.current_split_index - 1;
            let time = if index >= 0 {
                timer
                    .run
                    .segments
                    .get(index as usize)
                    .and_then(|segment| segment.split_time.get(timer.current_timing_method))
            } else {
                None
            };
            CommandOutcome::reply(format_time(time))
        }
        ServerCommand::GetCurrentSplitTime(comparison) => {
            let comparison = comparison
                .as_deref()
                .unwrap_or(&timer.current_comparison);
            let time = analysis::comparison_time_for_split(
                timer,
                timer.current_split_index,
                comparison,
                timer.current_timing_method,
            );
            CommandOutcome::reply(format_time(time))
        }
        ServerCommand::GetComparisons => {
            CommandOutcome::reply(timer.run.comparisons().join("\n"))
        }
        ServerCommand::GetCurrentComparison => {
            CommandOutcome::reply(timer.current_comparison.clone())
        }
        ServerCommand::GetCurrentTimerPhase => CommandOutcome::reply(phase_name(timer.phase)),
        ServerCommand::GetSplitIndex => {
            let index = if timer.phase == TimerPhase::NotRunning || timer.current_split_index < 0 {
                -1
            } else {
                timer.current_split_index + 1
            };
            CommandOutcome::reply(index.to_string())
        }
        ServerCommand::GetSplitNames => CommandOutcome::reply(
            timer
                .run
                .segments
                .iter()
                .map(|segment| segment.name.as_str())
                .collect::<Vec<_>>()
                .join("\n"),
        ),
        ServerCommand::GetSplitCount => CommandOutcome::reply(timer.run.segments.len().to_string()),
        ServerCommand::GetSplitName { index } => {
            let name = resolve_split_index(*index, timer.run.segments.len())
                .and_then(|index| timer.run.segments.get(index))
                .map(|segment| segment.name.as_str())
                .unwrap_or("-");
            CommandOutcome::reply(name)
        }
        ServerCommand::SetSplitName { index, name } => {
            if let Some(index) = resolve_split_index(*index, timer.run.segments.len()) {
                if let Some(segment) = timer.run.segments.get_mut(index) {
                    segment.name = name.clone();
                    timer.run.has_changed = true;
                }
                CommandOutcome::none()
            } else {
                CommandOutcome::reply("-")
            }
        }
        ServerCommand::SetCurrentSplitName(name) => {
            if let Some(segment) = timer.current_split_mut() {
                segment.name = name.clone();
                timer.run.has_changed = true;
            }
            CommandOutcome::none()
        }
        ServerCommand::GetCurrentSplitName => CommandOutcome::reply(
            timer
                .current_split()
                .map(|segment| segment.name.as_str())
                .unwrap_or("-"),
        ),
        ServerCommand::GetPreviousSplitName => {
            let index = timer.current_split_index - 1;
            let name = if index >= 0 {
                timer
                    .run
                    .segments
                    .get(index as usize)
                    .map(|segment| segment.name.as_str())
                    .unwrap_or("-")
            } else {
                "-"
            };
            CommandOutcome::reply(name)
        }
        ServerCommand::GetUpcomingSplitName => {
            let index = timer.current_split_index + 1;
            let name = timer
                .run
                .segments
                .get(index as usize)
                .map(|segment| segment.name.as_str())
                .unwrap_or("-");
            CommandOutcome::reply(name)
        }
        ServerCommand::GetTimerState => CommandOutcome::reply(timer_state(timer)),
        ServerCommand::GetDelta(comparison) => {
            let comparison = comparison
                .as_deref()
                .unwrap_or(&timer.current_comparison);
            let method = timer.current_timing_method;
            let delta = match timer.phase {
                TimerPhase::Running | TimerPhase::Paused => {
                    analysis::get_last_delta(timer, timer.current_split_index, comparison, method)
                }
                TimerPhase::Ended => timer.run.segments.last().and_then(|segment| {
                    let split = segment.split_time.get(method)?;
                    let comparison = segment.comparison(comparison).get(method)?;
                    Some(split - comparison)
                }),
                TimerPhase::NotRunning => None,
            };
            CommandOutcome::reply(format_time(delta))
        }
        ServerCommand::GetFinalTime(comparison) => {
            let method = timer.current_timing_method;
            let time = if timer.phase == TimerPhase::Ended {
                timer.current_time().get(method)
            } else {
                let comparison = comparison
                    .as_deref()
                    .unwrap_or(&timer.current_comparison);
                timer
                    .run
                    .segments
                    .last()
                    .and_then(|segment| segment.comparison(comparison).get(method))
            };
            CommandOutcome::reply(format_time(time))
        }
        ServerCommand::GetPredictedTime(comparison) => {
            let comparison = comparison
                .as_deref()
                .unwrap_or(&timer.current_comparison);
            CommandOutcome::reply(format_time(predict_time(
                timer,
                comparison,
                timer.current_timing_method,
            )))
        }
        ServerCommand::GetBestPossibleTime => CommandOutcome::reply(format_time(predict_time(
            timer,
            BEST_SEGMENTS,
            timer.current_timing_method,
        ))),
        ServerCommand::GetPausedRealTime => {
            CommandOutcome::reply(format_time(timer.pause_time()))
        }
        ServerCommand::GetPausedGameTime => {
            CommandOutcome::reply(format_time(timer.game_time_pause_time))
        }
        ServerCommand::GetOffset => CommandOutcome::reply(format_time(Some(timer.run.offset))),
        ServerCommand::GetTimingMethod => {
            CommandOutcome::reply(timing_method_name(timer.current_timing_method))
        }
        ServerCommand::GetGameName => CommandOutcome::reply(timer.run.game_name.clone()),
        ServerCommand::GetCategoryName => CommandOutcome::reply(timer.run.category_name.clone()),
        ServerCommand::GetCategoryVariables => {
            CommandOutcome::reply(category_variables_json(timer))
        }
        ServerCommand::GetAttemptCount => {
            CommandOutcome::reply(timer.run.attempt_count.to_string())
        }
        ServerCommand::GetCompletedCount => CommandOutcome::reply(
            timer
                .run
                .attempts
                .iter()
                .filter(|attempt| attempt.time.real_time.is_some())
                .count()
                .to_string(),
        ),
        ServerCommand::GetCustomVariableValue(name) => {
            let value = timer
                .run
                .metadata
                .custom_variables
                .get(name)
                .map(|value| {
                    value
                        .replace("\r\n", " ")
                        .replace(['\r', '\n'], " ")
                })
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| "-".to_string());
            CommandOutcome::reply(value)
        }
        ServerCommand::SetCustomVariable { name, value } => {
            timer
                .run
                .metadata
                .custom_variables
                .insert(name.clone(), value.clone());
            timer.run.has_changed = true;
            CommandOutcome::none()
        }
        ServerCommand::GetLayoutPath => CommandOutcome::reply(
            timer
                .run
                .layout_path
                .clone()
                .filter(|path| !path.is_empty())
                .unwrap_or_else(|| "-".to_string()),
        ),
        ServerCommand::SaveLayout => CommandOutcome {
            response: Some("True".to_string()),
            quit: false,
            save: false,
        },
        ServerCommand::SaveLayoutAs(path) => {
            timer.run.layout_path = Some(path.clone());
            timer.run.has_changed = true;
            CommandOutcome {
                response: Some("True".to_string()),
                quit: false,
                save: false,
            }
        }
        ServerCommand::GetSplitsPath => CommandOutcome::reply(
            timer
                .run
                .file_path
                .as_ref()
                .map(|path| path.display().to_string())
                .filter(|path| !path.is_empty())
                .unwrap_or_else(|| "-".to_string()),
        ),
        ServerCommand::SaveSplits => CommandOutcome {
            response: Some("True".to_string()),
            quit: false,
            save: true,
        },
        ServerCommand::SaveSplitsAs(path) => {
            timer.run.file_path = Some(path.into());
            timer.run.has_changed = true;
            CommandOutcome {
                response: Some("True".to_string()),
                quit: false,
                save: true,
            }
        }
        ServerCommand::SwitchLayout(_) => {
            // Layouts belong to the UI, which the server crate cannot touch.
            CommandOutcome::reply("False")
        }
        ServerCommand::SwitchSplits(path) => match livesplit_core::load_run(path) {
            Ok(run) => {
                timer.set_run(run);
                CommandOutcome::reply("True")
            }
            Err(error) => {
                log::warn!("[server] could not load splits from {path}: {error}");
                CommandOutcome::reply("False")
            }
        },
        ServerCommand::GetSplitsScreenshot => CommandOutcome::reply("-"),
        ServerCommand::SaveSplitsScreenshot(_) => CommandOutcome::reply("False"),
        ServerCommand::GlobalHotkeysEnabled => CommandOutcome::reply("False"),
        ServerCommand::EnableGlobalHotkeys
        | ServerCommand::DisableGlobalHotkeys
        | ServerCommand::SwitchHotkeyProfile(_) => CommandOutcome::none(),
        ServerCommand::GetHotkeyProfile => CommandOutcome::reply("Default"),
        ServerCommand::GetAutoSplitterPath => CommandOutcome::reply("-"),
        ServerCommand::AutoSplitterActivated => CommandOutcome::reply("False"),
        ServerCommand::GetLiveSplitVersion => {
            CommandOutcome::reply(env!("CARGO_PKG_VERSION"))
        }
        ServerCommand::GetLiveSplitPath => CommandOutcome::reply(
            std::env::current_exe()
                .map(|path| path.display().to_string())
                .unwrap_or_else(|_| "-".to_string()),
        ),
        ServerCommand::GetServerType => CommandOutcome::reply("TCP"),
        ServerCommand::Ping => CommandOutcome::reply("pong"),
        ServerCommand::Quit => CommandOutcome {
            response: None,
            quit: true,
            save: false,
        },
    }
}

fn current_time_for(
    timer: &Timer,
    method: TimingMethod,
    fallback_uninitialized_game_time: bool,
) -> Option<TimeSpan> {
    if timer.phase == TimerPhase::NotRunning {
        return Some(timer.run.offset);
    }
    let time = timer.current_time();
    match method {
        TimingMethod::RealTime => time.real_time,
        TimingMethod::GameTime => {
            if fallback_uninitialized_game_time && !timer.is_game_time_initialized {
                time.real_time
            } else {
                time.game_time
            }
        }
    }
}

fn predict_time(timer: &Timer, comparison: &str, method: TimingMethod) -> Option<TimeSpan> {
    let last_comparison = timer
        .run
        .segments
        .last()?
        .comparison(comparison)
        .get(method);

    match timer.phase {
        TimerPhase::Running | TimerPhase::Paused => {
            let mut delta =
                analysis::get_last_delta(timer, timer.current_split_index, comparison, method)
                    .unwrap_or(TimeSpan::ZERO);
            let live_delta = timer.current_time().get(method).zip(
                timer
                    .current_split()
                    .and_then(|segment| segment.comparison(comparison).get(method)),
            );
            if let Some((current, comparison_time)) = live_delta {
                let live = current - comparison_time;
                if live > delta {
                    delta = live;
                }
            }
            last_comparison.map(|last| delta + last)
        }
        TimerPhase::Ended => timer
            .run
            .segments
            .last()
            .and_then(|segment| segment.split_time.get(method))
            .or_else(|| timer.current_time().get(method)),
        TimerPhase::NotRunning => last_comparison,
    }
}

fn format_time(time: Option<TimeSpan>) -> String {
    time.map(|time| time.to_precise_string())
        .unwrap_or_else(|| "-".to_string())
}

fn phase_name(phase: TimerPhase) -> &'static str {
    match phase {
        TimerPhase::NotRunning => "NotRunning",
        TimerPhase::Running => "Running",
        TimerPhase::Paused => "Paused",
        TimerPhase::Ended => "Ended",
    }
}

fn timing_method_name(method: TimingMethod) -> &'static str {
    match method {
        TimingMethod::RealTime => "RealTime",
        TimingMethod::GameTime => "GameTime",
    }
}

fn timer_state(timer: &Timer) -> String {
    let time = current_time_for(timer, timer.current_timing_method, true);
    let real_time = current_time_for(timer, TimingMethod::RealTime, false);
    let game_time = current_time_for(timer, TimingMethod::GameTime, true);
    let split_index = if timer.phase == TimerPhase::NotRunning || timer.current_split_index < 0 {
        -1
    } else {
        timer.current_split_index + 1
    };

    format!(
        "phase={}\ncurrent_time={}\ncurrent_real_time={}\ncurrent_game_time={}\nsplit_index={}\nsplit_count={}\ncurrent_comparison={}\ntiming_method={}",
        phase_name(timer.phase),
        format_time(time),
        format_time(real_time),
        format_time(game_time),
        split_index,
        timer.run.segments.len(),
        timer.current_comparison,
        timing_method_name(timer.current_timing_method),
    )
}

fn category_variables_json(timer: &Timer) -> String {
    let metadata = &timer.run.metadata;
    let region = json_string(metadata.region_name.as_deref());
    let platform = json_string(metadata.platform_name.as_deref());
    let mut variables = String::new();
    for (index, (name, value)) in metadata.variables.iter().enumerate() {
        if index > 0 {
            variables.push(',');
        }
        variables.push('"');
        variables.push_str(&json_escape(name));
        variables.push_str("\":\"");
        variables.push_str(&json_escape(value));
        variables.push('"');
    }
    format!(
        "{{\"region\":{region},\"platform\":{platform},\"usesEmulator\":{},\"variables\":{{{variables}}}}}",
        metadata.uses_emulator
    )
}

fn json_string(value: Option<&str>) -> String {
    match value {
        Some(value) => format!("\"{}\"", json_escape(value)),
        None => "null".to_string(),
    }
}

fn json_escape(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            '\u{08}' => escaped.push_str("\\b"),
            '\u{0c}' => escaped.push_str("\\f"),
            character if (character as u32) < 0x20 => {
                escaped.push_str(&format!("\\u{:04x}", character as u32));
            }
            character => escaped.push(character),
        }
    }
    escaped
}

/// Parse the `["name", "value"]` JSON array used by `setcustomvariable`.
fn parse_json_string_array(input: &str) -> Result<Vec<String>, String> {
    let characters: Vec<char> = input.chars().collect();
    let mut position = 0;

    skip_json_whitespace(&characters, &mut position);
    expect_json_char(&characters, &mut position, '[')?;

    let mut values = Vec::new();
    skip_json_whitespace(&characters, &mut position);
    if peek_json_char(&characters, position) == Some(']') {
        position += 1;
    } else {
        loop {
            skip_json_whitespace(&characters, &mut position);
            values.push(parse_json_string(&characters, &mut position)?);
            skip_json_whitespace(&characters, &mut position);
            match next_json_char(&characters, &mut position)? {
                ',' => continue,
                ']' => break,
                _ => return Err("expected ',' or ']'".to_string()),
            }
        }
    }

    skip_json_whitespace(&characters, &mut position);
    if position != characters.len() {
        return Err("trailing characters".to_string());
    }

    Ok(values)
}

fn parse_json_string(characters: &[char], position: &mut usize) -> Result<String, String> {
    expect_json_char(characters, position, '"')?;
    let mut value = String::new();
    loop {
        let character = next_json_char(characters, position)?;
        match character {
            '"' => return Ok(value),
            '\\' => {
                match next_json_char(characters, position)? {
                    '"' => value.push('"'),
                    '\\' => value.push('\\'),
                    '/' => value.push('/'),
                    'b' => value.push('\u{08}'),
                    'f' => value.push('\u{0c}'),
                    'n' => value.push('\n'),
                    'r' => value.push('\r'),
                    't' => value.push('\t'),
                    'u' => {
                        let mut code = parse_json_hex4(characters, position)?;
                        if (0xd800..=0xdbff).contains(&code) {
                            expect_json_char(characters, position, '\\')?;
                            expect_json_char(characters, position, 'u')?;
                            let low = parse_json_hex4(characters, position)?;
                            if !(0xdc00..=0xdfff).contains(&low) {
                                return Err("invalid surrogate pair".to_string());
                            }
                            code = 0x10000 + ((code - 0xd800) << 10) + (low - 0xdc00);
                        }
                        let character = char::from_u32(code)
                            .ok_or_else(|| "invalid unicode escape".to_string())?;
                        value.push(character);
                    }
                    _ => return Err("invalid escape sequence".to_string()),
                }
            }
            character if (character as u32) < 0x20 => {
                return Err("control character in string".to_string())
            }
            character => value.push(character),
        }
    }
}

fn parse_json_hex4(characters: &[char], position: &mut usize) -> Result<u32, String> {
    let mut value = 0u32;
    for _ in 0..4 {
        let character = next_json_char(characters, position)?;
        let digit = character
            .to_digit(16)
            .ok_or_else(|| "invalid unicode escape".to_string())?;
        value = (value << 4) | digit;
    }
    Ok(value)
}

fn skip_json_whitespace(characters: &[char], position: &mut usize) {
    while peek_json_char(characters, *position).is_some_and(char::is_whitespace) {
        *position += 1;
    }
}

fn peek_json_char(characters: &[char], position: usize) -> Option<char> {
    characters.get(position).copied()
}

fn next_json_char(characters: &[char], position: &mut usize) -> Result<char, String> {
    let character = peek_json_char(characters, *position)
        .ok_or_else(|| "unexpected end of input".to_string())?;
    *position += 1;
    Ok(character)
}

fn expect_json_char(
    characters: &[char],
    position: &mut usize,
    expected: char,
) -> Result<(), String> {
    let character = next_json_char(characters, position)?;
    if character != expected {
        return Err(format!("expected '{expected}'"));
    }
    Ok(())
}
