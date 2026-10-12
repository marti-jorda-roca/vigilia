//! The session state machine. Pure: callers pass in the time, and side effects (power
//! assertions, journal writes) happen outside.
//!
//! ```text
//! (none) --start--> Active --extend--> Active
//!                     |
//!          stop / deadline (none for a session without an end)
//!                     v
//!                  Ending --undo ok--> Ended --> (none)
//!                     |
//!                 undo failed
//!                     v
//!              NeedsAttention --stop (retry)--> Ending
//! ```

use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use vigilia_protocol::{Session, SessionState};

use crate::clock::Now;

/// Longest a session with an end may last, counting extensions. A session without one runs
/// until stopped.
pub const MAX_SESSION: Duration = Duration::from_secs(30 * 86_400);

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SessionError {
    #[error("a session is already running")]
    SessionActive,
    #[error("a session needs attention; run `vigilia stop` to retry the undo")]
    NeedsAttention,
    #[error("no session is running")]
    NoSession,
    #[error("the session is already ending")]
    Ending,
    #[error("duration must be more than zero")]
    ZeroDuration,
    #[error("sessions can last at most 30 days")]
    TooLong,
}

#[derive(Debug, Clone)]
struct Live {
    session: Session,
    /// The same deadline on the monotonic clock. The session ends at whichever comes first.
    /// `None` when the session runs until stopped.
    mono_deadline: Option<Instant>,
}

#[derive(Debug, Default)]
pub struct SessionMachine {
    live: Option<Live>,
}

impl SessionMachine {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn current(&self) -> Option<&Session> {
        self.live.as_ref().map(|l| &l.session)
    }

    /// Starts a session that ends after `duration`, or runs until stopped when it is `None`.
    pub fn start(
        &mut self,
        now: Now,
        duration: Option<Duration>,
        lid_closed_awake: bool,
    ) -> Result<Session, SessionError> {
        if let Some(live) = &self.live {
            return Err(match live.session.state {
                SessionState::NeedsAttention => SessionError::NeedsAttention,
                _ => SessionError::SessionActive,
            });
        }
        if let Some(duration) = duration {
            check_duration(duration)?;
        }
        let session = Session {
            id: uuid::Uuid::new_v4().to_string(),
            started_at: now.wall,
            ends_at: duration.map(|d| now.wall + d),
            state: SessionState::Active,
            lid_closed_awake,
        };
        self.live = Some(Live {
            session: session.clone(),
            mono_deadline: duration.map(|d| now.mono + d),
        });
        Ok(session)
    }

    /// Adds `duration` to the remaining time. A session without an end is left as it is.
    pub fn extend(&mut self, now: Now, duration: Duration) -> Result<Session, SessionError> {
        let live = self.active_mut()?;
        if duration.is_zero() {
            return Err(SessionError::ZeroDuration);
        }
        let (Some(ends_at), Some(mono_deadline)) =
            (&mut live.session.ends_at, &mut live.mono_deadline)
        else {
            return Ok(live.session.clone());
        };
        let remaining = mono_deadline.saturating_duration_since(now.mono);
        if remaining.saturating_add(duration) > MAX_SESSION {
            return Err(SessionError::TooLong);
        }
        *ends_at += duration;
        *mono_deadline += duration;
        Ok(live.session.clone())
    }

    /// Drops a session that just started because applying it to the OS failed. There is nothing
    /// to undo, so it never goes through `Ending`.
    pub fn discard(&mut self) {
        self.live = None;
    }

    /// Moves to `Ending` so the caller can run the undo steps. Also used to retry after
    /// `NeedsAttention`.
    pub fn begin_end(&mut self) -> Result<Session, SessionError> {
        let live = self.live.as_mut().ok_or(SessionError::NoSession)?;
        match live.session.state {
            SessionState::Active | SessionState::NeedsAttention => {
                live.session.state = SessionState::Ending;
                Ok(live.session.clone())
            }
            SessionState::Ending | SessionState::Ended => Err(SessionError::Ending),
        }
    }

    /// Records the outcome of the undo steps started by [`begin_end`](Self::begin_end).
    /// On success the session is gone and its final `Ended` snapshot is returned.
    pub fn finish_end(&mut self, undo_ok: bool) -> Option<Session> {
        let live = self.live.as_mut()?;
        if undo_ok {
            let mut session = self.live.take()?.session;
            session.state = SessionState::Ended;
            Some(session)
        } else {
            live.session.state = SessionState::NeedsAttention;
            Some(live.session.clone())
        }
    }

    /// Whether an active session has reached its end on either clock.
    pub fn is_due(&self, now: Now) -> bool {
        match &self.live {
            Some(Live {
                session:
                    Session {
                        state: SessionState::Active,
                        ends_at: Some(ends_at),
                        ..
                    },
                mono_deadline: Some(mono_deadline),
            }) => now.wall >= *ends_at || now.mono >= *mono_deadline,
            _ => false,
        }
    }

    /// The wall-clock time at which the active session is due, taking the earlier of the two
    /// clocks. `None` when no session is active, or it runs until stopped.
    pub fn deadline(&self, now: Now) -> Option<DateTime<Utc>> {
        let live = self.live.as_ref()?;
        if live.session.state != SessionState::Active {
            return None;
        }
        let by_mono = now.wall + live.mono_deadline?.saturating_duration_since(now.mono);
        Some(live.session.ends_at?.min(by_mono))
    }

    fn active_mut(&mut self) -> Result<&mut Live, SessionError> {
        let live = self.live.as_mut().ok_or(SessionError::NoSession)?;
        match live.session.state {
            SessionState::Active => Ok(live),
            SessionState::NeedsAttention => Err(SessionError::NeedsAttention),
            SessionState::Ending | SessionState::Ended => Err(SessionError::Ending),
        }
    }
}

fn check_duration(duration: Duration) -> Result<(), SessionError> {
    if duration.is_zero() {
        Err(SessionError::ZeroDuration)
    } else if duration > MAX_SESSION {
        Err(SessionError::TooLong)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::{Clock, ManualClock};
    use chrono::TimeDelta;

    const MIN: Duration = Duration::from_secs(60);

    fn clock() -> ManualClock {
        ManualClock::new("2026-10-12T00:00:00Z".parse().unwrap())
    }

    #[test]
    #[expect(
        clippy::cognitive_complexity,
        reason = "each assert macro expands to a branch; the test itself is linear"
    )]
    fn start_extend_and_end() {
        let clock = clock();
        let mut m = SessionMachine::new();
        let s = m.start(clock.now(), Some(120 * MIN), false).unwrap();
        assert_eq!(s.state, SessionState::Active);
        assert_eq!(s.ends_at, Some("2026-10-12T02:00:00Z".parse().unwrap()));
        assert_eq!(
            m.start(clock.now(), Some(MIN), false),
            Err(SessionError::SessionActive)
        );

        let s = m.extend(clock.now(), 30 * MIN).unwrap();
        assert_eq!(s.ends_at, Some("2026-10-12T02:30:00Z".parse().unwrap()));

        clock.advance(149 * MIN);
        assert!(!m.is_due(clock.now()));
        clock.advance(MIN);
        assert!(m.is_due(clock.now()));

        assert_eq!(m.begin_end().unwrap().state, SessionState::Ending);
        assert_eq!(m.begin_end(), Err(SessionError::Ending));
        assert_eq!(m.finish_end(true).unwrap().state, SessionState::Ended);
        assert!(m.current().is_none());
        assert_eq!(m.begin_end(), Err(SessionError::NoSession));
    }

    #[test]
    fn rejects_bad_durations() {
        let clock = clock();
        let mut m = SessionMachine::new();
        assert_eq!(
            m.start(clock.now(), Some(Duration::ZERO), false),
            Err(SessionError::ZeroDuration)
        );
        assert_eq!(
            m.start(clock.now(), Some(MAX_SESSION + MIN), false),
            Err(SessionError::TooLong)
        );
        assert_eq!(m.extend(clock.now(), MIN), Err(SessionError::NoSession));
        m.start(clock.now(), Some(MAX_SESSION - MIN), false)
            .unwrap();
        assert_eq!(m.extend(clock.now(), 2 * MIN), Err(SessionError::TooLong));
        assert_eq!(
            m.extend(clock.now(), Duration::ZERO),
            Err(SessionError::ZeroDuration)
        );
    }

    #[test]
    fn failed_undo_needs_attention_until_retried() {
        let clock = clock();
        let mut m = SessionMachine::new();
        m.start(clock.now(), Some(MIN), false).unwrap();
        m.begin_end().unwrap();
        assert_eq!(
            m.finish_end(false).unwrap().state,
            SessionState::NeedsAttention
        );
        assert_eq!(
            m.start(clock.now(), Some(MIN), false),
            Err(SessionError::NeedsAttention)
        );
        assert_eq!(
            m.extend(clock.now(), MIN),
            Err(SessionError::NeedsAttention)
        );
        assert!(!m.is_due(clock.now()));
        assert_eq!(m.deadline(clock.now()), None);
        m.begin_end().unwrap();
        assert!(m.finish_end(true).is_some());
        assert!(m.current().is_none());
    }

    #[test]
    fn setting_the_clock_back_does_not_extend_the_session() {
        let clock = clock();
        let mut m = SessionMachine::new();
        let s = m.start(clock.now(), Some(60 * MIN), false).unwrap();
        clock.jump_wall(TimeDelta::hours(-5));
        clock.advance(60 * MIN);
        assert!(m.is_due(clock.now()), "monotonic deadline must win");
        // The deadline the timer is armed with also follows the monotonic clock.
        let clock = self::clock();
        let mut m = SessionMachine::new();
        m.start(clock.now(), Some(60 * MIN), false).unwrap();
        clock.jump_wall(TimeDelta::hours(-5));
        assert_eq!(
            m.deadline(clock.now()).unwrap(),
            clock.now().wall + 60 * MIN
        );
        assert!(m.deadline(clock.now()) < s.ends_at);
    }

    #[test]
    fn setting_the_clock_forward_ends_early_rather_than_late() {
        let clock = clock();
        let mut m = SessionMachine::new();
        let s = m.start(clock.now(), Some(60 * MIN), false).unwrap();
        clock.jump_wall(TimeDelta::hours(2));
        assert!(m.is_due(clock.now()));
        assert_eq!(m.deadline(clock.now()), s.ends_at);
    }

    #[test]
    fn a_session_without_an_end_runs_until_stopped() {
        let clock = clock();
        let mut m = SessionMachine::new();
        let s = m.start(clock.now(), None, false).unwrap();
        assert_eq!(s.ends_at, None);
        assert_eq!(m.deadline(clock.now()), None);
        clock.advance(MAX_SESSION + MIN);
        assert!(!m.is_due(clock.now()), "no end time, so never due");

        // Extending changes nothing, and the 30-day cap doesn't apply.
        assert_eq!(m.extend(clock.now(), MAX_SESSION).unwrap(), s);
        assert_eq!(
            m.extend(clock.now(), Duration::ZERO),
            Err(SessionError::ZeroDuration)
        );

        m.begin_end().unwrap();
        assert_eq!(m.finish_end(true).unwrap().state, SessionState::Ended);
        assert!(m.current().is_none());
    }
}
