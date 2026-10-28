use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::Arc;
use std::time::{Duration, Instant};

use livesplit_core::{Run, TestClock, Timer};
use livesplit_server::{
    handle_command, ServerCommand, ServerHandle, ServerRequest,
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

fn test_timer() -> Timer {
    let clock = TestClock::new();
    let run = Run::with_segments("Game", "Any%", ["One", "Two"]);
    Timer::new(run, Arc::new(clock))
}

#[test]
fn tcp_round_trip_and_clean_shutdown() {
    let handle = ServerHandle::start(0).expect("server should start on an ephemeral port");
    assert!(handle.is_running());
    assert_eq!(handle.bind_addr().ip(), std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST));
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
    assert_eq!(
        request.command,
        ServerCommand::GetSplitName { index: 0 }
    );
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
