//! Smoketests for the packaged example files and layout parser.

use std::io::Write;
use std::net::TcpStream;
use std::path::Path;
use std::time::{Duration, Instant};

use rustsplit_core::{load_run, Run, TimeSpan, Timer, TimerPhase, PERSONAL_BEST};
use rustsplit_server::{handle_command, ServerHandle, ServerRequest};
use rustsplit_ui::layout::DEFAULT_LAYOUT_LSL;
use rustsplit_ui::parse_layout;

#[test]
fn demo_run_loads_and_has_expected_data() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/demo.lss");
    let run = load_run(&path).expect("demo.lss should parse");
    assert_eq!(run.game_name, "RustSplit Demo");
    assert_eq!(run.category_name, "Any%");
    assert_eq!(run.segments.len(), 5);
    assert_eq!(run.attempts.len(), 3);
    assert!(run
        .custom_comparisons
        .iter()
        .any(|comparison| comparison == PERSONAL_BEST));
    assert!(run
        .custom_comparisons
        .iter()
        .any(|comparison| comparison == "Glitchless"));
    assert!(run.segments[0].personal_best().real_time.is_some());
    assert!(run.segments[0].best_segment_time.real_time.is_some());
    assert_eq!(run.segments[0].segment_history.len(), 3);
}

#[test]
fn bundled_layout_parses() {
    let layout = parse_layout(DEFAULT_LAYOUT_LSL.as_bytes()).expect("default layout parses");
    assert_eq!(layout.components.len(), 4);
    assert_eq!(layout.components[0].component.name(), "Title");
    assert_eq!(layout.components[1].component.name(), "Splits");
    assert_eq!(layout.components[2].component.name(), "Timer");
    assert_eq!(layout.components[3].component.name(), "Previous Segment");
    assert_eq!(layout.vertical_width, 286);
    assert!(layout.settings.always_on_top);
}

fn next_request(server: &ServerHandle) -> ServerRequest {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if let Some(request) = server.try_recv() {
            return request;
        }
        assert!(Instant::now() < deadline, "no request arrived");
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// The path the app takes for server commands: the request carries the
/// moment it was read and is applied at that moment, however late the UI
/// thread gets to it (a hidden window only runs its logic every 100 ms).
#[test]
fn late_server_commands_keep_their_receipt_time() {
    let server = ServerHandle::start(0).expect("the server should start");
    let mut client = TcpStream::connect(server.bind_addr()).unwrap();
    let mut timer = Timer::with_system_clock(Run::with_segments("Game", "Any%", ["One"]));

    client.write_all(b"starttimer\n").unwrap();
    let request = next_request(&server);
    std::thread::sleep(Duration::from_millis(100));
    let at = timer.timestamp_of(request.received);
    timer.at(at, |timer| handle_command(timer, &request.command));
    request.respond(None);
    assert_eq!(timer.phase, TimerPhase::Running);
    let elapsed = timer.current_time().real_time.unwrap();
    assert!(elapsed >= TimeSpan::from_millis(100), "{elapsed:?}");

    let sent = timer.current_time().real_time.unwrap();
    client.write_all(b"split\n").unwrap();
    let request = next_request(&server);
    let arrived = timer.current_time().real_time.unwrap();
    std::thread::sleep(Duration::from_millis(150));
    let at = timer.timestamp_of(request.received);
    timer.at(at, |timer| handle_command(timer, &request.command));
    request.respond(None);
    assert_eq!(timer.phase, TimerPhase::Ended);
    let split = timer.run.segments[0].split_time.real_time.unwrap();
    assert!(
        sent - TimeSpan::from_millis(1) <= split && split <= arrived + TimeSpan::from_millis(1),
        "the split was recorded at {split:?}, but arrived between {sent:?} and {arrived:?}"
    );
    server.stop();
}
