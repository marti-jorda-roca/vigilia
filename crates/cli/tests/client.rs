//! The CLI's client against a real service on a real socket, with a fake platform.
#![cfg(unix)]

use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use chrono::{TimeDelta, Utc};
use tokio::sync::oneshot;
use vigilia::client::Client;
use vigilia::watch::{Mode, watch};
use vigilia::{CliError, commands};
use vigilia_core::{Config, Journal, SystemClock};
use vigilia_daemon::transports::local;
use vigilia_daemon::{Deps, Service, ServiceHandle};
use vigilia_platform::{FakePlatform, TokioTimer};
use vigilia_protocol::{ErrorCode, HumanDuration, SessionResult, SessionState};

/// A service on its own thread and runtime. Dropping it stops the service and closes every
/// connection.
struct TestServer {
    socket: PathBuf,
    platform: FakePlatform,
    shutdown: Option<oneshot::Sender<()>>,
    thread: Option<JoinHandle<()>>,
    _dir: tempfile::TempDir,
}

impl TestServer {
    fn start() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("vigilia.sock");
        let listener = local::bind(&socket).unwrap();
        listener.set_nonblocking(true).unwrap();
        let journal = Journal::open(dir.path().join("journal.jsonl")).unwrap();
        let platform = FakePlatform::new();
        let service_platform = Arc::new(platform.clone());
        let (shutdown, shutdown_rx) = oneshot::channel::<()>();
        let thread = std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            runtime.block_on(async move {
                let listener = tokio::net::UnixListener::from_std(listener).unwrap();
                let (timer, fired) = TokioTimer::new(Arc::new(SystemClock));
                let service = Service::start(Deps {
                    platform: service_platform,
                    clock: Arc::new(SystemClock),
                    timer: Box::new(timer),
                    config: Config::default(),
                    journal,
                    socket_activated: false,
                });
                let (handle, rx) = ServiceHandle::channel();
                tokio::spawn(local::serve(listener, handle));
                service
                    .run(rx, fired, async {
                        let _ = shutdown_rx.await;
                    })
                    .await;
            });
        });
        Self {
            socket,
            platform,
            shutdown: Some(shutdown),
            thread: Some(thread),
            _dir: dir,
        }
    }

    fn client(&self) -> Client {
        Client::connect(&self.socket).unwrap()
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn dur(s: &str) -> HumanDuration {
    s.parse().unwrap()
}

fn refused(result: Result<impl std::fmt::Debug, CliError>) -> ErrorCode {
    match result {
        Err(CliError::Refused(body)) => body.code,
        other => panic!("expected a refusal, got {other:?}"),
    }
}

#[test]
fn session_lifecycle() {
    let server = TestServer::start();
    let mut c = server.client();

    assert_eq!(commands::status(&mut c).unwrap(), None);
    let started = commands::start(&mut c, Some(dur("1h"))).unwrap().unwrap();
    assert_eq!(started.state, SessionState::Active);
    assert!(server.platform.awake());

    let status = commands::status(&mut c).unwrap().unwrap();
    assert_eq!(status.id, started.id);
    let extended = commands::extend(&mut c, dur("30m")).unwrap().unwrap();
    assert_eq!(
        extended.ends_at.unwrap() - started.ends_at.unwrap(),
        TimeDelta::minutes(30),
        "extend adds to the end time"
    );

    let stopped = commands::stop(&mut c).unwrap().unwrap();
    assert_eq!(stopped.state, SessionState::Ended);
    assert!(!server.platform.awake());
}

#[test]
fn refusals_and_stop_without_a_session() {
    let server = TestServer::start();
    let mut c = server.client();

    commands::start(&mut c, Some(dur("1h"))).unwrap();
    assert_eq!(
        refused(commands::start(&mut c, Some(dur("1h")))),
        ErrorCode::SessionActive
    );
    commands::stop(&mut c).unwrap();

    // Stopping again is fine; extending isn't.
    assert_eq!(commands::stop(&mut c).unwrap(), None);
    assert_eq!(
        refused(commands::extend(&mut c, dur("1m"))),
        ErrorCode::NoSession
    );
    assert_eq!(
        refused(commands::start(&mut c, Some(dur("31d")))),
        ErrorCode::BadRequest
    );
}

#[test]
fn start_with_extend_makes_the_session_last_at_least_that_long() {
    let server = TestServer::start();
    let mut c = server.client();

    // No session: starts one.
    let first = commands::start_or_extend(&mut c, dur("1h"))
        .unwrap()
        .unwrap();
    assert_eq!(first.state, SessionState::Active);

    // Already long enough: unchanged.
    let same = commands::start_or_extend(&mut c, dur("30m"))
        .unwrap()
        .unwrap();
    assert_eq!(
        (same.id.as_str(), same.ends_at),
        (first.id.as_str(), first.ends_at)
    );

    // Too short: pushed out to now + 2h, same session.
    let before = Utc::now();
    let longer = commands::start_or_extend(&mut c, dur("2h"))
        .unwrap()
        .unwrap();
    assert_eq!(longer.id, first.id);
    let wanted = before + TimeDelta::hours(2);
    let off = (longer.ends_at.unwrap() - wanted).num_milliseconds();
    assert!((-1_000..=1_500).contains(&off), "ends_at off by {off} ms");
}

#[test]
fn a_session_without_an_end_runs_until_stopped() {
    let server = TestServer::start();
    let mut c = server.client();

    let started = commands::start(&mut c, None).unwrap().unwrap();
    assert_eq!(started.ends_at, None);
    assert!(server.platform.awake());

    // Neither `extend` nor `start --extend` gives it an end.
    assert_eq!(
        commands::extend(&mut c, dur("30m")).unwrap(),
        Some(started.clone())
    );
    assert_eq!(
        commands::start_or_extend(&mut c, dur("2h")).unwrap(),
        Some(started)
    );

    let stopped = commands::stop(&mut c).unwrap().unwrap();
    assert_eq!(stopped.state, SessionState::Ended);
    assert!(!server.platform.awake());
}

/// A `Write` the test can read while the watch thread writes to it.
#[derive(Clone, Default)]
struct Shared(Arc<Mutex<Vec<u8>>>);

impl Write for Shared {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Shared {
    fn lines(&self) -> Vec<String> {
        String::from_utf8(self.0.lock().unwrap().clone())
            .unwrap()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    fn wait_for_lines(&self, n: usize) -> Vec<String> {
        let deadline = Instant::now() + Duration::from_secs(2);
        while self.lines().len() < n && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        self.lines()
    }
}

fn state(line: &str) -> Option<SessionState> {
    let result: SessionResult =
        serde_json::from_str(line).unwrap_or_else(|e| panic!("{e}: {line}"));
    result.session.map(|s| s.state)
}

#[test]
fn json_watch_prints_one_line_per_change() {
    let server = TestServer::start();
    let mut watcher = server.client();
    let out = Shared::default();
    let mut sink = out.clone();
    let watching = std::thread::spawn(move || watch(&mut watcher, Mode::Json, &mut sink));

    assert_eq!(
        out.wait_for_lines(1).len(),
        1,
        "the current state comes first"
    );
    let mut c = server.client();
    commands::start(&mut c, Some(dur("1h"))).unwrap();
    commands::extend(&mut c, dur("1m")).unwrap();
    commands::stop(&mut c).unwrap();
    let lines = out.wait_for_lines(4);

    // The watch ends when the service goes away.
    drop(server);
    assert!(matches!(
        watching.join().unwrap(),
        Err(CliError::Disconnected)
    ));
    let states: Vec<_> = lines.iter().map(|l| state(l)).collect();
    assert_eq!(
        states,
        [
            None,
            Some(SessionState::Active),
            Some(SessionState::Active),
            Some(SessionState::Ended)
        ]
    );
}

#[test]
fn lid_closed_session() {
    let server = TestServer::start();
    let mut c = server.client();
    let started = commands::start_lid(&mut c, Some(dur("1h")), true)
        .unwrap()
        .unwrap();
    assert!(started.lid_closed_awake);
    assert!(server.platform.sleep_disabled());
    assert!(server.platform.backstop_registered());
    commands::simulate_lid(&mut c, vigilia_protocol::LidState::Closed).unwrap();
    let stopped = commands::stop(&mut c).unwrap().unwrap();
    assert_eq!(stopped.state, SessionState::Ended);
    assert!(!server.platform.sleep_disabled());
    assert!(!server.platform.backstop_registered());
}
