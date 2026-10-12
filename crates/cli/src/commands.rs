//! The session commands, on top of [`Client`].

use chrono::{DateTime, Utc};
use vigilia_protocol::{
    Call, ErrorBody, ErrorCode, HumanDuration, Session, SessionExtendParams, SessionStartParams,
    SessionState,
};

use crate::client::Client;
use crate::error::CliError;

/// Starts a session that ends after `duration`, or runs until stopped when it is `None`.
pub fn start(
    client: &mut Client,
    duration: Option<HumanDuration>,
) -> Result<Option<Session>, CliError> {
    client.session(Call::SessionStart(SessionStartParams {
        duration,
        lid_closed_awake: false,
    }))
}

/// `start --extend`: makes sure a session runs for at least `duration` from now. Starts one if
/// none is running; otherwise extends the running one by the shortfall, if any.
pub fn start_or_extend(
    client: &mut Client,
    duration: HumanDuration,
) -> Result<Option<Session>, CliError> {
    match start(client, Some(duration)) {
        Err(CliError::Refused(e)) if e.code == ErrorCode::SessionActive => {}
        other => return other,
    }
    // A session ending between two calls is settled by one more start.
    let Some(session) = status(client)? else {
        return start(client, Some(duration));
    };
    if session.state != SessionState::Active {
        return Err(not_active(&session));
    }
    let Some(shortfall) = shortfall(&session, duration, Utc::now()) else {
        return Ok(Some(session));
    };
    match extend(client, shortfall) {
        Err(CliError::Refused(e)) if e.code == ErrorCode::NoSession => {
            start(client, Some(duration))
        }
        other => other,
    }
}

pub fn extend(client: &mut Client, duration: HumanDuration) -> Result<Option<Session>, CliError> {
    client.session(Call::SessionExtend(SessionExtendParams { duration }))
}

/// Ends the session. No session running counts as success, so cleanup steps don't fail when the
/// session already ran out.
pub fn stop(client: &mut Client) -> Result<Option<Session>, CliError> {
    match client.session(Call::SessionStop {}) {
        Err(CliError::Refused(e)) if e.code == ErrorCode::NoSession => Ok(None),
        other => other,
    }
}

pub fn status(client: &mut Client) -> Result<Option<Session>, CliError> {
    client.session(Call::SessionStatus {})
}

/// How much to extend `session` so it lasts `wanted` from `now`, to the second: the shortfall
/// rounded down, so the countdown then shows `wanted` rather than one second more. `None` when
/// the session already lasts that long (including when it runs until stopped), or misses by less
/// than a second.
fn shortfall(
    session: &Session,
    wanted: HumanDuration,
    now: DateTime<Utc>,
) -> Option<HumanDuration> {
    let remaining_ms = (session.ends_at? - now).num_milliseconds().max(0);
    let wanted_ms = i64::try_from(wanted.as_duration().as_millis()).unwrap_or(i64::MAX);
    let missing_ms = wanted_ms.saturating_sub(remaining_ms);
    if missing_ms < 1_000 {
        return None;
    }
    let secs = u64::try_from(missing_ms / 1_000).ok()?;
    Some(HumanDuration::from_secs(secs))
}

fn not_active(session: &Session) -> CliError {
    let message = match session.state {
        SessionState::NeedsAttention => {
            "a session needs attention; run `vigilia stop` to retry the undo"
        }
        _ => "the session is ending; try again in a moment",
    };
    CliError::Refused(ErrorBody {
        code: ErrorCode::SessionActive,
        message: message.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session_ending_in(secs: i64, now: DateTime<Utc>) -> Session {
        Session {
            id: "s".into(),
            started_at: now,
            ends_at: Some(now + chrono::TimeDelta::seconds(secs)),
            state: SessionState::Active,
            lid_closed_awake: false,
        }
    }

    #[test]
    fn shortfall_only_when_the_session_ends_too_soon() {
        let now = Utc::now();
        let hour = HumanDuration::from_secs(3_600);
        assert_eq!(shortfall(&session_ending_in(7_200, now), hour, now), None);
        assert_eq!(shortfall(&session_ending_in(3_600, now), hour, now), None);
        assert_eq!(
            shortfall(&session_ending_in(600, now), hour, now),
            Some(HumanDuration::from_secs(3_000))
        );
        // Already past its end (the service hasn't ended it yet): the full duration.
        assert_eq!(
            shortfall(&session_ending_in(-5, now), hour, now),
            Some(hour)
        );
        let until_stopped = Session {
            ends_at: None,
            ..session_ending_in(0, now)
        };
        assert_eq!(shortfall(&until_stopped, hour, now), None);
    }

    #[test]
    fn shortfall_rounds_down_and_ignores_less_than_a_second() {
        let now = Utc::now();
        let hour = HumanDuration::from_secs(3_600);
        let mut session = session_ending_in(3_600, now);
        let ends_at = session.ends_at.as_mut().unwrap();
        *ends_at -= chrono::TimeDelta::milliseconds(400);
        assert_eq!(shortfall(&session, hour, now), None);
        let ends_at = session.ends_at.as_mut().unwrap();
        *ends_at -= chrono::TimeDelta::milliseconds(1_000);
        assert_eq!(
            shortfall(&session, hour, now),
            Some(HumanDuration::from_secs(1))
        );
    }
}
