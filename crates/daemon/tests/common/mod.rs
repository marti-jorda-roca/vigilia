#![allow(dead_code)]

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use chrono::Utc;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use vigilia_core::{Clock, Config, Journal};
use vigilia_daemon::dispatch::Dispatched;
use vigilia_daemon::{Deps, Exit, Service, ServiceHandle};
use vigilia_platform::{FakePlatform, TokioClock, TokioTimer};
use vigilia_protocol::{Call, ClientContext, HumanDuration, Request, SessionStartParams};

pub struct Harness {
    pub handle: ServiceHandle,
    pub platform: FakePlatform,
    pub fires: Arc<AtomicUsize>,
    pub task: JoinHandle<Exit>,
    pub dir: tempfile::TempDir,
    shutdown: Option<oneshot::Sender<()>>,
    next_id: u64,
}

impl Harness {
    /// A service on a fake platform whose clock and timer follow tokio time.
    pub fn spawn() -> Self {
        Self::spawn_with(
            tempfile::tempdir().unwrap(),
            FakePlatform::new(),
            Config::default(),
        )
    }

    pub fn spawn_with(dir: tempfile::TempDir, platform: FakePlatform, config: Config) -> Self {
        let clock: Arc<dyn Clock> = Arc::new(TokioClock::new(Utc::now()));
        let (timer, fired) = TokioTimer::new(clock.clone());
        let fires = timer.fire_counter();
        let service = Service::start(Deps {
            platform: Arc::new(platform.clone()),
            clock,
            timer: Box::new(timer),
            config,
            journal: Journal::open(journal_path(&dir)).unwrap(),
            socket_activated: false,
        });
        let (handle, rx) = ServiceHandle::channel();
        let (shutdown, shutdown_rx) = oneshot::channel();
        let task = tokio::spawn(service.run(rx, fired, async {
            let _ = shutdown_rx.await;
        }));
        Self {
            handle,
            platform,
            fires,
            task,
            dir,
            shutdown: Some(shutdown),
            next_id: 1,
        }
    }

    pub async fn call(&mut self, call: Call) -> Dispatched {
        self.next_id += 1;
        self.handle
            .call(ClientContext::local(), Request::new(self.next_id, call))
            .await
            .expect("service is running")
    }

    pub async fn start(&mut self, duration: &str) -> Dispatched {
        self.call(Call::SessionStart(SessionStartParams {
            duration: duration.parse::<HumanDuration>().unwrap(),
            lid_closed_awake: false,
        }))
        .await
    }

    pub fn fires(&self) -> usize {
        self.fires.load(Ordering::Relaxed)
    }

    pub fn signal(&mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
    }

    pub fn journal_path(&self) -> PathBuf {
        journal_path(&self.dir)
    }
}

pub fn journal_path(dir: &tempfile::TempDir) -> PathBuf {
    dir.path().join("journal.jsonl")
}
