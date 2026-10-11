//! The local transport end to end: a real Unix socket, real time, a fake platform.
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::net::UnixStream;
use tokio::net::unix::OwnedReadHalf;
use vigilia_core::{Config, Journal, SystemClock};
use vigilia_daemon::transports::local;
use vigilia_daemon::{Deps, Service, ServiceHandle};
use vigilia_platform::{FakePlatform, TokioTimer};
use vigilia_protocol::{Event, Response, ServerMessage, SessionResult, SessionState};

struct Client {
    lines: Lines<BufReader<OwnedReadHalf>>,
    write: tokio::net::unix::OwnedWriteHalf,
}

impl Client {
    async fn connect(path: &std::path::Path) -> Self {
        let (read, write) = UnixStream::connect(path).await.unwrap().into_split();
        Self {
            lines: BufReader::new(read).lines(),
            write,
        }
    }

    async fn send(&mut self, line: &str) {
        self.write.write_all(line.as_bytes()).await.unwrap();
        self.write.write_all(b"\n").await.unwrap();
    }

    async fn next(&mut self, wait: Duration) -> Option<ServerMessage> {
        let line = tokio::time::timeout(wait, self.lines.next_line())
            .await
            .ok()?
            .unwrap()?;
        Some(serde_json::from_str(&line).unwrap_or_else(|e| panic!("{e}: {line}")))
    }

    async fn response(&mut self) -> Response {
        match self.next(Duration::from_secs(2)).await {
            Some(ServerMessage::Response(r)) => r,
            other => panic!("expected a response, got {other:?}"),
        }
    }

    async fn event(&mut self, wait: Duration) -> Option<Event> {
        match self.next(wait).await {
            Some(ServerMessage::Event(e)) => Some(e),
            None => None,
            other => panic!("expected an event, got {other:?}"),
        }
    }
}

fn state(event: Option<Event>) -> Option<SessionState> {
    match event {
        Some(Event::SessionChanged { session }) => session.map(|s| s.state),
        other => panic!("expected session_changed, got {other:?}"),
    }
}

#[tokio::test]
async fn session_over_the_socket_with_a_watcher() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("vigilia.sock");
    let listener = local::bind(&socket).unwrap();
    assert_eq!(
        std::fs::metadata(&socket).unwrap().permissions().mode() & 0o777,
        0o600
    );
    listener.set_nonblocking(true).unwrap();
    let listener = tokio::net::UnixListener::from_std(listener).unwrap();

    let platform = FakePlatform::new();
    let (timer, fired) = TokioTimer::new(Arc::new(SystemClock));
    let service = Service::start(Deps {
        platform: Arc::new(platform.clone()),
        clock: Arc::new(SystemClock),
        timer: Box::new(timer),
        config: Config::default(),
        journal: Journal::open(dir.path().join("journal.jsonl")).unwrap(),
        socket_activated: false,
    });
    let (handle, rx) = ServiceHandle::channel();
    tokio::spawn(local::serve(listener, handle));
    tokio::spawn(service.run(rx, fired, std::future::pending()));

    // A watcher subscribes and gets the current state (no session).
    let mut watcher = Client::connect(&socket).await;
    watcher
        .send(r#"{"id":1,"v":1,"method":"subscribe","params":{"filter":["session_changed"]}}"#)
        .await;
    assert!(watcher.response().await.ok);
    assert_eq!(state(watcher.event(Duration::from_secs(1)).await), None);

    // Another client starts a 2-second session.
    let mut cli = Client::connect(&socket).await;
    cli.send(r#"{"id":7,"v":1,"method":"session.start","params":{"duration":"2s"}}"#)
        .await;
    let resp = cli.response().await;
    assert_eq!(resp.id, Some(7));
    let started: SessionResult = resp.into_result().unwrap();
    let ends_at = started.session.unwrap().ends_at;
    assert!(platform.awake());

    match watcher.event(Duration::from_secs(1)).await {
        Some(Event::SessionChanged {
            session: Some(session),
        }) => {
            assert_eq!(session.state, SessionState::Active);
            assert_eq!(
                session.ends_at, ends_at,
                "watchers get ends_at to count down from"
            );
        }
        other => panic!("expected the started session, got {other:?}"),
    }

    // Nothing is sent while the session runs: no per-second ticks.
    assert!(watcher.event(Duration::from_millis(1_500)).await.is_none());

    // Then the session ends on its own.
    assert_eq!(
        state(watcher.event(Duration::from_secs(2)).await),
        Some(SessionState::Ended)
    );
    assert!(!platform.awake());

    cli.send(r#"{"id":8,"v":1,"method":"session.status"}"#)
        .await;
    let status: SessionResult = cli.response().await.into_result().unwrap();
    assert_eq!(status.session, None);
}

#[tokio::test]
async fn malformed_requests_get_bad_request() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("vigilia.sock");
    let listener = local::bind(&socket).unwrap();
    listener.set_nonblocking(true).unwrap();
    let listener = tokio::net::UnixListener::from_std(listener).unwrap();
    let (timer, fired) = TokioTimer::new(Arc::new(SystemClock));
    let service = Service::start(Deps {
        platform: Arc::new(FakePlatform::new()),
        clock: Arc::new(SystemClock),
        timer: Box::new(timer),
        config: Config::default(),
        journal: Journal::open(dir.path().join("journal.jsonl")).unwrap(),
        socket_activated: false,
    });
    let (handle, rx) = ServiceHandle::channel();
    tokio::spawn(local::serve(listener, handle));
    tokio::spawn(service.run(rx, fired, std::future::pending()));

    let mut c = Client::connect(&socket).await;
    c.send("not json").await;
    let r = c.response().await;
    assert_eq!(
        (r.id, r.error.unwrap().code),
        (None, vigilia_protocol::ErrorCode::BadRequest)
    );

    c.send(r#"{"id":3,"v":1,"method":"session.explode"}"#).await;
    let r = c.response().await;
    assert_eq!(
        (r.id, r.error.unwrap().code),
        (Some(3), vigilia_protocol::ErrorCode::BadRequest)
    );

    // The connection still works afterwards.
    c.send(r#"{"id":4,"v":1,"method":"hello"}"#).await;
    assert!(c.response().await.ok);
}
