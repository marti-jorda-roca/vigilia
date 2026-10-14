//! The service: owns the session, the journal and the platform, and runs one loop that handles
//! requests and the single deadline timer. Nothing here knows about sockets.

use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use tokio::sync::{mpsc, oneshot};
use vigilia_core::{
    ActionError, ActionId, ActionKind, Clock, Config, Journal, RuleAction, SessionError,
    SessionMachine, UndoRecord,
};
use vigilia_platform::{
    BACKSTOP_INTERVAL, Capabilities, DeadlineTimer, Platform, PlatformError, TimerFired,
};
use vigilia_protocol::{
    ClientContext, ConfigSetParams, ErrorBody, ErrorCode, Event, EventKind, HelloResult,
    HumanDuration, LidState, PROTOCOL_VERSION, PowerSnapshot, Request, RuleSettings, Session,
    SessionExtendParams, SessionStartParams, SessionState, Settings,
};

use crate::bus::{Bus, Subscription};
use crate::dispatch::{Dispatched, dispatch};

/// A deadline this close counts as reached, so jitter between clocks can't cause an extra
/// wake-up a few microseconds later.
const DEADLINE_SLACK: Duration = Duration::from_millis(50);

/// The next item on an optional event channel; pending forever when there is none, and
/// pending again after it closes so a dead event source can't spin the loop.
async fn recv_opt<T>(rx: &mut Option<tokio::sync::mpsc::UnboundedReceiver<T>>) -> Option<T> {
    match rx {
        Some(rx) => rx.recv().await,
        None => std::future::pending().await,
    }
}

pub struct Deps {
    pub platform: Arc<dyn Platform>,
    pub clock: Arc<dyn Clock>,
    pub timer: Box<dyn DeadlineTimer>,
    pub config: Config,
    pub journal: Journal,
    /// Where `config.set` persists `[rules]`; `None` keeps changes in memory only.
    pub config_path: Option<PathBuf>,
    /// The OS started us on the first connection (launchd socket activation).
    pub socket_activated: bool,
}

/// One request on its way to the service, with where to send the outcome.
pub struct Msg {
    ctx: ClientContext,
    req: Request,
    reply: oneshot::Sender<Dispatched>,
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
        self.tx.send(Msg { ctx, req, reply }).await.ok()?;
        rx.await.ok()
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
    config_path: Option<PathBuf>,
    journal: Journal,
    bus: Bus,
    machine: SessionMachine,
    capabilities: Capabilities,
    /// The journaled keep-awake action of the current session.
    awake: Option<(ActionId, UndoRecord)>,
    /// The journaled backstop job of the current session, if one was registered.
    backstop: Option<(ActionId, UndoRecord)>,
    lid_rx: Option<tokio::sync::mpsc::UnboundedReceiver<LidState>>,
    power_rx: Option<tokio::sync::mpsc::UnboundedReceiver<PowerSnapshot>>,
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
            config_path,
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
        let lid_rx = platform.lid_events();
        let power_rx = platform.power_events();
        let now = clock.now();
        Self {
            platform,
            clock,
            timer,
            config,
            config_path,
            journal,
            bus: Bus::new(),
            machine: SessionMachine::new(),
            capabilities,
            awake: None,
            backstop: None,
            lid_rx,
            power_rx,
            last_activity: now.mono,
            armed: None,
        }
    }

    /// Handles requests and deadlines until the service should exit. On a signal or a closed
    /// channel, a running session is ended through the journal first.
    #[expect(
        clippy::cognitive_complexity,
        reason = "one select loop with a branch per event source; each branch is linear"
    )]
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
                msg = rx.recv() => {
                    let Some(Msg { ctx, req, reply }) = msg else {
                        break Exit::Closed;
                    };
                    self.touch();
                    let _ = reply.send(dispatch(&mut self, &ctx, req));
                }
                () = self.bus.subscriber_left() => self.touch(),
                lid = recv_opt(&mut self.lid_rx) => {
                    if let Some(state) = lid {
                        self.on_lid(state);
                    }
                }
                power = recv_opt(&mut self.power_rx) => {
                    if let Some(snapshot) = power {
                        self.on_power(snapshot);
                    }
                }
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
        if self.machine.current().is_some() {
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

    pub(crate) fn config_get(&self) -> Settings {
        Settings {
            rules: RuleSettings {
                battery_floor_percent: self.config.rules.battery_floor_percent,
                end_at_critical_heat: self.config.rules.end_at_critical_heat,
            },
        }
    }

    pub(crate) fn config_set(&mut self, p: ConfigSetParams) -> Result<Settings, ErrorBody> {
        let patch = p.rules.unwrap_or_default();
        let mut rules = self.config.rules.clone();
        if let Some(floor) = patch.battery_floor_percent {
            if floor > 100 {
                return Err(error(
                    ErrorCode::BadRequest,
                    format!("battery_floor_percent must be between 0 and 100, got {floor}"),
                ));
            }
            rules.battery_floor_percent = floor;
        }
        if let Some(heat) = patch.end_at_critical_heat {
            rules.end_at_critical_heat = heat;
        }
        if let Some(path) = &self.config_path {
            Config::save_rules(path, &rules)
                .map_err(|e| error(ErrorCode::Internal, e.to_string()))?;
        }
        self.config.rules = rules;
        let settings = self.config_get();
        self.bus.publish(Event::ConfigChanged { config: settings });
        self.apply_rules_now();
        Ok(settings)
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
            .start(
                now,
                p.duration.map(HumanDuration::as_duration),
                p.lid_closed_awake,
            )
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
        if let Err(e) = self.register_backstop(p.lid_closed_awake) {
            // Roll the keep-awake action back out through the journal.
            if let Some((id, rec)) = self.awake.clone() {
                let platform = &self.platform;
                let _ = self.journal.undo(id, || platform.release_awake(&rec));
                self.awake = None;
            }
            self.machine.discard();
            return Err(e);
        }
        tracing::info!(id = %session.id, ends_at = ?session.ends_at, lid_closed_awake = p.lid_closed_awake, "session started");
        self.publish_session(Some(session.clone()));
        if self.apply_rules_now() {
            return Ok(self.machine.current().cloned().unwrap_or(Session {
                state: SessionState::Ended,
                ..session
            }));
        }
        Ok(session)
    }

    /// Journals and registers the backstop job for this session. Unsupported platforms skip
    /// it, except for lid-closed sessions, where it is the crash-safety guarantee.
    fn register_backstop(&mut self, required: bool) -> Result<(), ErrorBody> {
        let program = std::env::current_exe().unwrap_or_else(|_| "vigilia".into());
        let platform = &self.platform;
        match self.journal.apply(ActionKind::Backstop, None, || {
            platform.register_backstop(&program, BACKSTOP_INTERVAL)
        }) {
            Ok(applied) => {
                self.backstop = Some(applied);
                Ok(())
            }
            Err(ActionError::Os(PlatformError::Unsupported(_))) if !required => {
                tracing::info!("no backstop on this platform; skipping");
                Ok(())
            }
            Err(e) => {
                tracing::error!(error = %e, "could not register the backstop");
                Err(action_error(e))
            }
        }
    }

    /// A lid event (real or simulated): published on the bus for later lid actions.
    pub(crate) fn on_lid(&mut self, state: LidState) {
        tracing::info!(?state, "lid changed");
        self.bus.publish(Event::LidChanged { state });
    }

    /// `vigilia debug simulate-lid`: the same path as a real lid event.
    pub(crate) fn simulate_lid(&mut self, state: LidState) {
        self.on_lid(state);
    }

    /// A power/thermal event: published, then the rules are evaluated. This is the only
    /// place rules run — there is no polling timer for them.
    pub(crate) fn on_power(&mut self, power: PowerSnapshot) {
        self.bus.publish(Event::PowerChanged { power });
        let active = self
            .machine
            .current()
            .is_some_and(|s| s.state == SessionState::Active);
        if !active {
            return;
        }
        if let Some(RuleAction::EndSession { reason }) =
            vigilia_core::rules::evaluate(&power, &self.config.rules)
                .into_iter()
                .next()
        {
            tracing::warn!(%reason, "rule ended the session");
            self.bus.publish(Event::Warning {
                kind: "rule".into(),
                detail: reason,
            });
            let _ = self.end_session();
        }
    }

    /// A rule the current power snapshot already breaks ends an active session straight away,
    /// through the normal undo (at session start and when the rules change). Returns whether
    /// it did.
    fn apply_rules_now(&mut self) -> bool {
        let active = self
            .machine
            .current()
            .is_some_and(|s| s.state == SessionState::Active);
        if active
            && let Some(snapshot) = self.platform.power_snapshot()
            && !vigilia_core::rules::evaluate(&snapshot, &self.config.rules).is_empty()
        {
            self.on_power(snapshot);
            return true;
        }
        false
    }

    pub(crate) fn extend(&mut self, p: SessionExtendParams) -> Result<Session, ErrorBody> {
        let session = self
            .machine
            .extend(self.clock.now(), p.duration.as_duration())
            .map_err(session_error)?;
        tracing::info!(ends_at = ?session.ends_at, "session extended");
        self.publish_session(Some(session.clone()));
        Ok(session)
    }

    pub(crate) fn subscribe(&self, filter: &[EventKind]) -> Subscription {
        self.bus.subscribe(filter)
    }

    /// Runs the undo steps and moves the session to `Ended`, or to `NeedsAttention` if an undo
    /// step failed (a later `stop` retries).
    pub(crate) fn end_session(&mut self) -> Result<Session, ErrorBody> {
        self.machine.begin_end().map_err(session_error)?;
        let mut failure = None;
        if let Some((id, rec)) = self.awake.clone() {
            let platform = &self.platform;
            match self.journal.undo(id, || platform.release_awake(&rec)) {
                Ok(()) => self.awake = None,
                Err(e) => failure = Some(e.to_string()),
            }
        }
        // The backstop goes away only once sleep is restored; while an undo has failed it
        // stays, so `vigilia recover` can finish the job after a crash.
        if failure.is_none()
            && let Some((id, rec)) = self.backstop.clone()
        {
            let platform = &self.platform;
            match self.journal.undo(id, || platform.unregister_backstop(&rec)) {
                Ok(()) => self.backstop = None,
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
                    detail: format!("could not undo the session's changes: {detail}"),
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
            // The idle countdown starts when the session ends, not at the last request.
            self.touch();
            return false;
        }
        self.idle_deadline()
            .is_some_and(|at| at <= now.mono + DEADLINE_SLACK)
    }

    /// When the service may exit, if it is idle: no session and no subscribers.
    fn idle_deadline(&self) -> Option<Instant> {
        if self.machine.current().is_some() || self.bus.subscribers() > 0 {
            return None;
        }
        // `None` for an idle exit too far away to represent: never exit.
        self.last_activity
            .checked_add(self.config.energy.idle_exit())
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
        self.platform.announce_session_change();
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
            config_path: None,
            socket_activated: false,
        });
        (svc, platform)
    }
}
