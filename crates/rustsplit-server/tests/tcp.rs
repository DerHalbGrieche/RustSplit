use std::io::{BufRead, BufReader, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use rustsplit_core::{Run, TestClock, Timer};
use rustsplit_server::{
    handle_command, ServerCommand, ServerHandle, ServerRequest, MAX_LINE_LENGTH,
};

struct Client {
    reader: BufReader<TcpStream>,
}

impl Client {
    fn connect(addr: SocketAddr) -> Self {
        let stream = TcpStream::connect_timeout(&addr, Duration::from_secs(2))
            .expect("TCP connection should succeed");
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        Self {
            reader: BufReader::new(stream),
        }
    }

    fn send(&mut self, line: &str) {
        self.reader
            .get_mut()
            .write_all(format!("{line}\n").as_bytes())
            .expect("command should be written");
        self.reader.get_mut().flush().unwrap();
    }

    fn receive(&mut self) -> String {
        let mut line = String::new();
        let read = self
            .reader
            .read_line(&mut line)
            .expect("response should be readable");
        assert!(read > 0, "connection closed before a response arrived");
        line.trim_end_matches(['\r', '\n']).to_string()
    }
}

fn receive_request(handle: &ServerHandle) -> ServerRequest {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if let Some(request) = handle.try_recv() {
            return request;
        }
        assert!(
            Instant::now() < deadline,
            "server did not deliver a request in time"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn ping(handle: &ServerHandle, client: &mut Client) {
    client.send("ping");
    let request = receive_request(handle);
    assert_eq!(request.command, ServerCommand::Ping);
    request.respond_str("pong");
    assert_eq!(client.receive(), "pong");
}

fn wait_until(what: &str, mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while !condition() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Assert that the server closed the connection.  A read timeout means the
/// connection is still open and fails the assertion.
fn assert_closed(client: &mut Client) {
    let mut line = String::new();
    match client.reader.read_line(&mut line) {
        Ok(0) => {}
        Ok(_) => panic!("unexpected data on a closed connection: {line:?}"),
        Err(error) => assert!(
            matches!(
                error.kind(),
                std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::ConnectionAborted
            ),
            "connection was not closed: {error}"
        ),
    }
}

fn local_addr() -> SocketAddr {
    SocketAddr::new(Ipv4Addr::LOCALHOST.into(), 0)
}

fn test_timer() -> Timer {
    let clock = TestClock::new();
    let run = Run::with_segments("Game", "Any%", ["One", "Two"]);
    Timer::new(run, Arc::new(clock))
}

#[test]
fn tcp_round_trip_and_clean_shutdown() {
    let handle = ServerHandle::start(0).expect("server should start on an ephemeral port");
    assert!(handle.is_running());
    assert_eq!(
        handle.bind_addr().ip(),
        std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST)
    );
    assert_ne!(handle.port(), 0);
    assert_eq!(handle.port(), handle.bind_addr().port());

    let mut client = Client::connect(handle.bind_addr());
    let mut timer = test_timer();

    client.send("ping");
    let request = receive_request(&handle);
    assert_eq!(request.command, ServerCommand::Ping);
    request.respond_str("pong");
    assert_eq!(client.receive(), "pong");

    client.send("getsplitname 1");
    let request = receive_request(&handle);
    assert_eq!(request.command, ServerCommand::GetSplitName { index: 0 });
    let outcome = handle_command(&mut timer, &request.command);
    request.respond(outcome.response);
    assert_eq!(client.receive(), "One");

    // Unknown commands are ignored but do not break the connection.
    client.send("definitely-not-a-command");
    client.send("ping");
    let request = receive_request(&handle);
    assert_eq!(request.command, ServerCommand::Ping);
    request.respond_str("pong");
    assert_eq!(client.receive(), "pong");

    // Dropping a request acknowledges it without a response, so the next
    // command is processed immediately.
    client.send("ping");
    let dropped = receive_request(&handle);
    assert_eq!(dropped.command, ServerCommand::Ping);
    drop(dropped);

    client.send("ping");
    let request = receive_request(&handle);
    request.respond_str("pong");
    assert_eq!(client.receive(), "pong");

    handle.stop();
    assert!(!handle.is_running());
}

#[test]
fn multiple_clients_are_served_independently() {
    let handle = ServerHandle::start(0).expect("server should start on an ephemeral port");

    let mut first = Client::connect(handle.bind_addr());
    let mut second = Client::connect(handle.bind_addr());

    first.send("ping");
    second.send("ping");

    let first_request = receive_request(&handle);
    let second_request = receive_request(&handle);
    assert_eq!(first_request.command, ServerCommand::Ping);
    assert_eq!(second_request.command, ServerCommand::Ping);
    first_request.respond_str("pong");
    second_request.respond_str("pong");

    // The responses belong to two independent connection threads.  Both
    // clients must receive their own newline-terminated response.
    assert_eq!(first.receive(), "pong");
    assert_eq!(second.receive(), "pong");

    first.send("getsplitcount");
    let request = receive_request(&handle);
    let mut timer = test_timer();
    let outcome = handle_command(&mut timer, &request.command);
    request.respond(outcome.response);
    assert_eq!(first.receive(), "2");

    handle.stop();
}

#[test]
fn stop_closes_connected_clients() {
    let handle = ServerHandle::start(0).expect("server should start on an ephemeral port");
    let mut client = Client::connect(handle.bind_addr());
    client.send("ping");
    let request = receive_request(&handle);
    request.respond_str("pong");
    assert_eq!(client.receive(), "pong");

    handle.stop();
    assert!(!handle.is_running());

    // The client thread notices the shutdown flag on its read timeout and
    // closes the socket, so the next read returns EOF.
    let mut line = String::new();
    let result = client.reader.read_line(&mut line);
    match result {
        Ok(0) => {}
        Ok(_) => panic!("unexpected data after shutdown: {line:?}"),
        Err(error) => {
            // A reset is also an acceptable way for the OS to report the
            // closed connection.
            assert!(
                matches!(
                    error.kind(),
                    std::io::ErrorKind::ConnectionReset
                        | std::io::ErrorKind::UnexpectedEof
                        | std::io::ErrorKind::TimedOut
                ),
                "unexpected shutdown error: {error}"
            );
        }
    }
}

#[test]
fn notifier_fires_when_a_command_is_queued() {
    let notifications = Arc::new(AtomicUsize::new(0));
    let (wake_sender, wake_receiver) = mpsc::sync_channel(16);
    let counter = Arc::clone(&notifications);
    let handle = ServerHandle::builder(local_addr())
        .notifier(move || {
            counter.fetch_add(1, Ordering::SeqCst);
            let _ = wake_sender.try_send(());
        })
        .start()
        .expect("server should start on an ephemeral port");

    let mut client = Client::connect(handle.bind_addr());
    // Connecting alone does not wake the application.
    wait_until("the connection to be accepted", || {
        handle.client_count() == 1
    });
    assert_eq!(notifications.load(Ordering::SeqCst), 0);

    client.send("ping");
    wake_receiver
        .recv_timeout(Duration::from_secs(3))
        .expect("the notifier should fire when a command arrives");
    // The request is queued before the notifier runs, so a woken
    // application finds it without polling.
    let request = handle
        .try_recv()
        .expect("request should be queued before the notification");
    assert_eq!(request.command, ServerCommand::Ping);
    request.respond_str("pong");
    assert_eq!(client.receive(), "pong");

    // Invalid commands never reach the application and do not wake it.
    client.send("definitely-not-a-command");
    ping(&handle, &mut client);
    assert_eq!(notifications.load(Ordering::SeqCst), 2);

    handle.stop();
}

#[test]
fn client_limit_is_enforced_and_slots_are_freed() {
    let handle = ServerHandle::builder(local_addr())
        .max_clients(2)
        .start()
        .expect("server should start on an ephemeral port");

    let mut first = Client::connect(handle.bind_addr());
    let mut second = Client::connect(handle.bind_addr());
    ping(&handle, &mut first);
    ping(&handle, &mut second);
    assert_eq!(handle.client_count(), 2);

    // The third client is accepted by the OS but closed by the server.
    let mut refused = Client::connect(handle.bind_addr());
    assert_closed(&mut refused);
    assert_eq!(handle.client_count(), 2);
    assert!(handle.is_running());

    // Disconnecting frees a slot for a new client.
    drop(first);
    wait_until("the slot to be freed", || handle.client_count() == 1);
    let mut third = Client::connect(handle.bind_addr());
    ping(&handle, &mut third);
    ping(&handle, &mut second);
    assert_eq!(handle.client_count(), 2);

    drop(second);
    drop(third);
    wait_until("all slots to be freed", || handle.client_count() == 0);
    handle.stop();
}

#[test]
fn over_long_line_closes_only_that_connection() {
    let handle = ServerHandle::start(0).expect("server should start on an ephemeral port");
    let mut bystander = Client::connect(handle.bind_addr());
    ping(&handle, &mut bystander);

    // A line exactly at the limit is still accepted.
    let mut client = Client::connect(handle.bind_addr());
    let prefix = "setcomparison ";
    let name = "x".repeat(MAX_LINE_LENGTH - prefix.len());
    client.send(&format!("{prefix}{name}"));
    let request = receive_request(&handle);
    assert_eq!(request.command, ServerCommand::SwitchComparison(name));
    drop(request);
    ping(&handle, &mut client);

    // One byte more closes the connection.  The server stops reading at the
    // limit, so the write itself may fail once the socket is reset.
    let mut abuser = Client::connect(handle.bind_addr());
    let line = format!("ping{}\n", " ".repeat(MAX_LINE_LENGTH - 3));
    let _ = abuser.reader.get_mut().write_all(line.as_bytes());
    assert_closed(&mut abuser);
    assert!(
        handle.try_recv().is_none(),
        "the long line must not be queued"
    );

    // The server and the other connections keep working.
    assert!(handle.is_running());
    ping(&handle, &mut bystander);
    ping(&handle, &mut client);
    let mut fresh = Client::connect(handle.bind_addr());
    ping(&handle, &mut fresh);
    wait_until("the closed connection to free its slot", || {
        handle.client_count() == 3
    });

    handle.stop();
}
