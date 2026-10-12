//! The service: owns the session, the journal and the platform, and runs one loop that handles
//! requests and the single deadline timer. Nothing here knows about sockets.

use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use tokio::sync::{mpsc, oneshot};
use vigilia_core::{
    ActionError, ActionId, ActionKind, Clock, Config, Journal, SessionError, SessionMachine,
    UndoRecord,
};
use vigilia_platform::{Capabilities, DeadlineTimer, Platform, PlatformError, TimerFired};
use vigilia_protocol::{
    ClientContext, ErrorBody, ErrorCode, Event, EventKind, HelloResult, PROTOCOL_VERSION, Request,
    Session, SessionExtendParams, SessionStartParams,
};

use crate::bus::{Bus, Subscription};
use crate::dispatch::{Dispatched, dispatch};

/// A deadline this close counts as reached, so jitter between clocks can't cause an extra
/// wake-up a few microseconds later.
const DEADLINE_SLACK: Duration = Duration::from_millis(50);

pub struct Deps {
    pub platform: Arc<dyn Platform>,
    pub clock: Arc<dyn Clock>,
    pub timer: Box<dyn DeadlineTimer>,
    pub config: Config,
    pub journal: Journal,
    /// The OS started us on the first connection (launchd socket activation).
    pub socket_activated: bool,
}

pub enum Msg {
    Request {
        ctx: ClientContext,
        req: Request,
        reply: oneshot::Sender<Dispatched>,
    },
    /// A connection that had subscribed has closed.
    SubscriberGone,
}

/// How transports talk to the service.
#[derive(Clone)]
pub struct ServiceHandle {
    tx: mpsc::Sender<Msg>,
}

impl ServiceHandle {
    pub fn channel() -> (Self, mpsc::Receiver<Msg>) {
        let (tx, rx) = mpsc::channel(64);
        (Self { tx }, rx)
    }

    /// `None` if the service has stopped.
    pub async fn call(&self, ctx: ClientContext, req: Request) -> Option<Dispatched> {
        let (reply, rx) = oneshot::channel();
        self.tx.send(Msg::Request { ctx, req, reply }).await.ok()?;
        rx.await.ok()
    }

    pub async fn subscriber_gone(&self) {
        let _ = self.tx.send(Msg::SubscriberGone).await;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exit {
    /// No session and no subscribers for `idle_exit_minutes`.
    Idle,
    /// SIGTERM / SIGINT.
    Signal,
    /// Every handle was dropped (tests).
    Closed,
}

pub struct Service {
    platform: Arc<dyn Platform>,
    clock: Arc<dyn Clock>,
    timer: Box<dyn DeadlineTimer>,
    config: Config,
    journal: Journal,
    bus: Bus,
    machine: SessionMachine,
    capabilities: Capabilities,
    /// The journaled keep-awake action of the current session.
    awake: Option<(ActionId, UndoRecord)>,
    subscribers: usize,
    last_activity: Instant,
    armed: Option<DateTime<Utc>>,
}

impl Service {
    /// Builds the service and replays the journal, undoing anything a previous process left.
    pub fn start(deps: Deps) -> Self {
        let Deps {
            platform,
            clock,
            timer,
            config,
            mut journal,
            socket_activated,
        } = deps;
        match journal.replay(|action| platform.restore(action)) {
            Ok(report) => {
                for action in &report.undone {
                    tracing::info!(id = action.id, kind = ?action.kind, "replayed journal entry");
                }
                for (action, error) in &report.failed {
                    tracing::error!(id = action.id, kind = ?action.kind, %error, "journal replay failed");
                }
            }
            Err(error) => tracing::error!(%error, "could not replay the journal"),
        }
        let capabilities = Capabilities {
            socket_activation: socket_activated,
            ..platform.capabilities()
        };
        let now = clock.now();
        Self {
            platform,
            clock,
            timer,
            config,
            journal,
            bus: Bus::new(),
            machine: SessionMachine::new(),
            capabilities,
            awake: None,
            subscribers: 0,
            last_activity: now.mono,
            armed: None,
        }
    }

    /// Handles requests and deadlines until the service should exit. On a signal or a closed
    /// channel, a running session is ended through the journal first.
    pub async fn run(
        mut self,
        mut rx: mpsc::Receiver<Msg>,
        mut fired: TimerFired,
        shutdown: impl Future<Output = ()>,
    ) -> Exit {
        tokio::pin!(shutdown);
        self.rearm();
        let exit = loop {
            tokio::select! {
                msg = rx.recv() => match msg {
                    Some(Msg::Request { ctx, req, reply }) => {
                        self.touch();
                        let _ = reply.send(dispatch(&mut self, &ctx, req));
                    }
                    Some(Msg::SubscriberGone) => {
                        self.subscribers = self.subscribers.saturating_sub(1);
                        self.touch();
                    }
                    None => break Exit::Closed,
                },
                Some(()) = fired.recv() => {
                    self.armed = None;
                    if self.on_deadline() {
                        break Exit::Idle;
                    }
                }
                () = &mut shutdown => break Exit::Signal,
            }
            self.rearm();
        };
        if exit != Exit::Idle && self.machine.current().is_some() {
            tracing::info!("shutting down; ending the session first");
            let _ = self.end_session();
        }
        self.timer.disarm();
        exit
    }

    pub(crate) fn hello(&self, ctx: &ClientContext) -> HelloResult {
        HelloResult {
            protocol_version: PROTOCOL_VERSION,
            server_version: env!("CARGO_PKG_VERSION").into(),
            capabilities: self.capabilities.clone(),
            scopes: ctx.scopes.iter().cloned().collect(),
        }
    }

    pub(crate) fn status(&self) -> Option<Session> {
        self.machine.current().cloned()
    }

    pub(crate) fn start_session(&mut self, p: SessionStartParams) -> Result<Session, ErrorBody> {
        if p.lid_closed_awake && !self.capabilities.keep_awake_lid_closed {
            return Err(error(
                ErrorCode::Unsupported,
                "keeping awake with the lid closed is not supported yet",
            ));
        }
        let now = self.clock.now();
        let session = self
            .machine
            .start(now, p.duration.as_duration(), p.lid_closed_awake)
            .map_err(session_error)?;
        let platform = &self.platform;
        match self.journal.apply(ActionKind::KeepAwake, None, || {
            platform.keep_awake(p.lid_closed_awake)
        }) {
            Ok(applied) => self.awake = Some(applied),
            Err(e) => {
                self.machine.discard();
                tracing::error!(error = %e, "could not keep the machine awake");
                return Err(action_error(e));
            }
        }
        tracing::info!(id = %session.id, ends_at = %session.ends_at, "session started");
        self.publish_session(Some(session.clone()));
        Ok(session)
    }

    pub(crate) fn extend(&mut self, p: SessionExtendParams) -> Result<Session, ErrorBody> {
        let session = self
            .machine
            .extend(self.clock.now(), p.duration.as_duration())
            .map_err(session_error)?;
        tracing::info!(ends_at = %session.ends_at, "session extended");
        self.publish_session(Some(session.clone()));
        Ok(session)
    }

    pub(crate) fn stop(&mut self) -> Result<Session, ErrorBody> {
        self.end_session()
    }

    pub(crate) fn subscribe(&mut self, filter: &[EventKind]) -> Subscription {
        self.subscribers += 1;
        self.bus.subscribe(filter)
    }

    /// Runs the undo steps and moves the session to `Ended`, or to `NeedsAttention` if an undo
    /// step failed (a later `stop` retries).
    fn end_session(&mut self) -> Result<Session, ErrorBody> {
        self.machine.begin_end().map_err(session_error)?;
        let mut failure = None;
        if let Some((id, rec)) = self.awake.clone() {
            let platform = &self.platform;
            match self.journal.undo(id, || platform.release_awake(&rec)) {
                Ok(()) => self.awake = None,
                Err(e) => failure = Some(e.to_string()),
            }
        }
        let session = self
            .machine
            .finish_end(failure.is_none())
            .expect("a session was ending");
        self.publish_session(Some(session.clone()));
        match failure {
            None => tracing::info!(id = %session.id, "session ended"),
            Some(detail) => {
                tracing::error!(id = %session.id, %detail, "session needs attention");
                self.bus.publish(Event::NeedsAttention {
                    detail: format!("could not release the keep-awake assertion: {detail}"),
                });
            }
        }
        Ok(session)
    }

    /// Returns `true` when the service should exit.
    fn on_deadline(&mut self) -> bool {
        let now = self.clock.now();
        let session_due = self.machine.is_due(now)
            || self
                .machine
                .deadline(now)
                .is_some_and(|d| d <= now.wall + DEADLINE_SLACK);
        if session_due {
            let _ = self.end_session();
            return false;
        }
        self.idle_deadline()
            .is_some_and(|at| at <= now.mono + DEADLINE_SLACK)
    }

    /// When the service may exit, if it is idle: no session and no subscribers.
    fn idle_deadline(&self) -> Option<Instant> {
        if self.machine.current().is_some() || self.subscribers > 0 {
            return None;
        }
        Some(self.last_activity + self.config.energy.idle_exit())
    }

    /// Arms the one timer for the next deadline: the session end, or the idle exit.
    fn rearm(&mut self) {
        let now = self.clock.now();
        let idle = self
            .idle_deadline()
            .map(|at| now.wall + at.saturating_duration_since(now.mono));
        let next = match (self.machine.deadline(now), idle) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        if next == self.armed {
            return;
        }
        match next {
            Some(at) => self.timer.arm(at, self.config.energy.timer_leeway()),
            None => self.timer.disarm(),
        }
        self.armed = next;
    }

    fn touch(&mut self) {
        self.last_activity = self.clock.now().mono;
    }

    fn publish_session(&self, session: Option<Session>) {
        self.bus.publish(Event::SessionChanged { session });
    }
}

fn error(code: ErrorCode, message: impl Into<String>) -> ErrorBody {
    ErrorBody {
        code,
        message: message.into(),
    }
}

fn session_error(e: SessionError) -> ErrorBody {
    let code = match e {
        SessionError::SessionActive | SessionError::NeedsAttention | SessionError::Ending => {
            ErrorCode::SessionActive
        }
        SessionError::NoSession => ErrorCode::NoSession,
        SessionError::ZeroDuration | SessionError::TooLong => ErrorCode::BadRequest,
    };
    error(code, e.to_string())
}

fn action_error(e: ActionError<PlatformError>) -> ErrorBody {
    let code = match &e {
        ActionError::Os(PlatformError::Unsupported(_)) => ErrorCode::Unsupported,
        _ => ErrorCode::Internal,
    };
    error(code, e.to_string())
}

#[cfg(test)]
pub(crate) mod testing {
    use super::*;
    use vigilia_platform::{FakePlatform, TokioClock, TokioTimer};

    /// A service on a fake platform with a temp journal. Must run inside a tokio runtime.
    pub fn service() -> (Service, FakePlatform) {
        let dir = tempfile::tempdir().unwrap().keep();
        let platform = FakePlatform::new();
        let clock = Arc::new(TokioClock::new(Utc::now()));
        let (timer, _fired) = TokioTimer::new(clock.clone());
        let svc = Service::start(Deps {
            platform: Arc::new(platform.clone()),
            clock,
            timer: Box::new(timer),
            config: Config::default(),
            journal: Journal::open(dir.join("journal.jsonl")).unwrap(),
            socket_activated: false,
        });
        (svc, platform)
    }
}
