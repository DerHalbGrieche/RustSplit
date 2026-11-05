//! Local control socket used by compositor keybindings.
//!
//! Wayland has no portable global-hotkey API. Niri (and other compositors)
//! can spawn a command from a key binding instead, so RustSplit listens on a
//! Unix socket in the runtime directory. `rustsplit --command startorsplit`
//! connects to the running instance, applies the command and prints any reply.
//!
//! Commands use the same names as the LiveSplit Server protocol, plus a few
//! UI-only commands such as `editrun` and `editlayout`.

use std::path::PathBuf;

/// The Unix socket used to control a running RustSplit instance.
///
/// `RUSTSPLIT_CONTROL_SOCKET` overrides the default path, which is useful for
/// tests or for running isolated instances.
pub fn socket_path() -> PathBuf {
    if let Some(path) = std::env::var_os("RUSTSPLIT_CONTROL_SOCKET") {
        if !path.is_empty() {
            return PathBuf::from(path);
        }
    }
    let runtime_dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    runtime_dir.join("rustsplit").join("control.sock")
}

#[cfg(unix)]
mod platform {
    use std::io::{BufRead, BufReader, ErrorKind, Read, Write};
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
    use std::sync::{Arc, Mutex};
    use std::thread::{self, JoinHandle};
    use std::time::Duration;

    use egui::Context;

    use super::socket_path;

    const ACCEPT_POLL_INTERVAL: Duration = Duration::from_millis(10);
    const IO_TIMEOUT: Duration = Duration::from_secs(5);

    pub struct ControlRequest {
        pub command: String,
        respond: Option<SyncSender<Option<String>>>,
    }

    impl ControlRequest {
        pub fn respond(mut self, response: Option<String>) {
            if let Some(sender) = self.respond.take() {
                let _ = sender.send(response);
            }
        }
    }

    impl Drop for ControlRequest {
        fn drop(&mut self) {
            if let Some(sender) = self.respond.take() {
                let _ = sender.send(None);
            }
        }
    }

    pub struct ControlServer {
        receiver: Receiver<ControlRequest>,
        running: Arc<AtomicBool>,
        thread: Mutex<Option<JoinHandle<()>>>,
        path: PathBuf,
    }

    impl ControlServer {
        /// Start the local control socket.
        ///
        /// Returns `Ok(None)` when another RustSplit instance already owns the
        /// socket. That instance remains the one that `--command` talks to.
        pub fn start(ctx: Context) -> Result<Option<Self>, String> {
            let path = socket_path();
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).map_err(|error| {
                    format!(
                        "could not create the control socket directory {}: {error}",
                        parent.display()
                    )
                })?;
            }
            if path.exists() {
                if UnixStream::connect(&path).is_ok() {
                    log::debug!(
                        "another RustSplit instance owns the control socket at {}",
                        path.display()
                    );
                    return Ok(None);
                }
                let _ = std::fs::remove_file(&path);
            }

            let listener = UnixListener::bind(&path).map_err(|error| {
                format!(
                    "could not bind the control socket at {}: {error}",
                    path.display()
                )
            })?;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
            listener
                .set_nonblocking(true)
                .map_err(|error| error.to_string())?;

            let (sender, receiver) = sync_channel(64);
            let running = Arc::new(AtomicBool::new(true));
            let thread_running = Arc::clone(&running);
            let thread = thread::Builder::new()
                .name("rustsplit-control".to_string())
                .spawn(move || accept_loop(listener, sender, thread_running, ctx))
                .map_err(|error| format!("could not start the control thread: {error}"))?;

            Ok(Some(Self {
                receiver,
                running,
                thread: Mutex::new(Some(thread)),
                path,
            }))
        }

        pub fn try_recv(&self) -> Option<ControlRequest> {
            self.receiver.try_recv().ok()
        }

        pub fn stop(&self) {
            self.running.store(false, Ordering::SeqCst);
            if let Ok(mut guard) = self.thread.lock() {
                if let Some(handle) = guard.take() {
                    if handle.thread().id() != thread::current().id() {
                        let _ = handle.join();
                    }
                }
            }
            let _ = std::fs::remove_file(&self.path);
        }
    }

    impl Drop for ControlServer {
        fn drop(&mut self) {
            self.stop();
        }
    }

    fn accept_loop(
        listener: UnixListener,
        sender: SyncSender<ControlRequest>,
        running: Arc<AtomicBool>,
        ctx: Context,
    ) {
        while running.load(Ordering::SeqCst) {
            match listener.accept() {
                Ok((stream, _)) => handle_client(stream, &sender, &ctx),
                Err(error) if error.kind() == ErrorKind::WouldBlock => {
                    thread::sleep(ACCEPT_POLL_INTERVAL);
                }
                Err(error) if error.kind() == ErrorKind::Interrupted => {}
                Err(error) => {
                    log::warn!("control socket accept failed: {error}");
                    thread::sleep(ACCEPT_POLL_INTERVAL);
                }
            }
        }
    }

    /// Prepares an accepted client socket for blocking reads and writes.
    fn configure_client(stream: &UnixStream) -> std::io::Result<()> {
        // On macOS and the BSDs an accepted socket inherits the listener's
        // non-blocking mode, so the timeouts would do nothing and a command
        // that has not arrived yet would fail the read at once.
        stream.set_nonblocking(false)?;
        stream.set_read_timeout(Some(IO_TIMEOUT))?;
        stream.set_write_timeout(Some(IO_TIMEOUT))
    }

    fn handle_client(mut stream: UnixStream, sender: &SyncSender<ControlRequest>, ctx: &Context) {
        if let Err(error) = configure_client(&stream) {
            log::debug!("control socket could not configure the client stream: {error}");
            return;
        }
        let reader_stream = match stream.try_clone() {
            Ok(stream) => stream,
            Err(error) => {
                log::debug!("control socket could not clone the client stream: {error}");
                return;
            }
        };
        let mut reader = BufReader::new(reader_stream);
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) => return,
            Ok(_) => {}
            Err(error) => {
                log::debug!("control socket read failed: {error}");
                return;
            }
        }
        let command = line.trim_end_matches(['\r', '\n']).trim().to_string();
        if command.is_empty() {
            return;
        }

        let (response_sender, response_receiver) = sync_channel(1);
        if sender
            .send(ControlRequest {
                command,
                respond: Some(response_sender),
            })
            .is_err()
        {
            return;
        }
        ctx.request_repaint();

        match response_receiver.recv_timeout(IO_TIMEOUT) {
            Ok(Some(response)) => {
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.write_all(b"\n");
                let _ = stream.flush();
            }
            Ok(None) => {}
            Err(_) => {
                log::warn!("RustSplit did not answer a control command in time");
            }
        }
    }

    pub fn send_command(command: &str) -> Result<String, String> {
        let command = command.trim();
        if command.is_empty() {
            return Err("empty command".to_string());
        }
        if command.contains('\r') || command.contains('\n') {
            return Err("control commands must be a single line".to_string());
        }

        let path = socket_path();
        let mut stream = UnixStream::connect(&path).map_err(|error| {
            format!(
                "could not connect to RustSplit at {}: {error}. Is RustSplit running?",
                path.display()
            )
        })?;
        stream
            .set_read_timeout(Some(IO_TIMEOUT))
            .map_err(|error| error.to_string())?;
        stream
            .set_write_timeout(Some(IO_TIMEOUT))
            .map_err(|error| error.to_string())?;
        stream
            .write_all(command.as_bytes())
            .and_then(|()| stream.write_all(b"\n"))
            .and_then(|()| stream.flush())
            .map_err(|error| format!("could not send the command: {error}"))?;

        let mut response = String::new();
        BufReader::new(stream)
            .read_to_string(&mut response)
            .map_err(|error| format!("could not read the command response: {error}"))?;
        Ok(response.trim_end_matches(['\r', '\n']).to_string())
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn client_sockets_wait_for_late_commands() {
            let (server, mut client) = UnixStream::pair().unwrap();
            // What macOS and the BSDs hand out from a non-blocking listener.
            server.set_nonblocking(true).unwrap();
            configure_client(&server).unwrap();

            let writer = thread::spawn(move || {
                thread::sleep(Duration::from_millis(50));
                client.write_all(b"split\n").unwrap();
            });
            let mut line = String::new();
            BufReader::new(&server).read_line(&mut line).unwrap();
            assert_eq!(line, "split\n");
            writer.join().unwrap();
        }
    }
}

#[cfg(not(unix))]
mod platform {
    use egui::Context;

    pub struct ControlRequest {
        pub command: String,
    }

    impl ControlRequest {
        pub fn respond(self, _response: Option<String>) {}
    }

    pub struct ControlServer;

    impl ControlServer {
        pub fn start(_ctx: Context) -> Result<Option<Self>, String> {
            Ok(None)
        }

        pub fn try_recv(&self) -> Option<ControlRequest> {
            None
        }

        pub fn stop(&self) {}
    }

    pub fn send_command(_command: &str) -> Result<String, String> {
        Err("the control socket is only available on Unix".to_string())
    }
}

pub use platform::{send_command, ControlServer};
