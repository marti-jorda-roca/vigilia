//! How results look: one human line per session, or the protocol's JSON.
//!
//! Output goes through `writeln!` rather than `println!`, so a closed pipe (`vigilia status |
//! head -0`) is ignored instead of panicking.

use std::io::Write;
use std::time::Duration;

use chrono::{DateTime, Local, Utc};
use vigilia_protocol::{Session, SessionResult, SessionState};

use crate::error::{CliError, exit};

const MINUTE_MS: i64 = 60_000;
const HOUR_SECS: i64 = 3_600;

/// Prints a command's result and returns the exit code it stands for.
pub fn print_session(session: Option<&Session>, json: bool) -> u8 {
    let line = if json {
        json_line(session)
    } else {
        human_line(session, Utc::now())
    };
    let _ = writeln!(std::io::stdout().lock(), "{line}");
    exit_code(session)
}

pub fn print_error(err: &CliError, json: bool, hint: Option<&str>) -> u8 {
    let mut stderr = std::io::stderr().lock();
    if json {
        let body =
            serde_json::json!({ "error": { "code": err.code(), "message": err.to_string() } });
        let _ = writeln!(stderr, "{body}");
    } else {
        let _ = writeln!(stderr, "vigilia: {err} ({})", err.code());
        if let Some(hint) = hint {
            let _ = writeln!(stderr, "hint: {hint}");
        }
    }
    err.exit_code()
}

/// A session that needs attention is a failure: the machine may not sleep normally.
pub fn exit_code(session: Option<&Session>) -> u8 {
    match session.map(|s| s.state) {
        Some(SessionState::NeedsAttention) => exit::NEEDS_ATTENTION,
        _ => 0,
    }
}

/// `{"session":…}`, the shape every `session.*` method returns.
pub fn json_line(session: Option<&Session>) -> String {
    let result = SessionResult {
        session: session.cloned(),
    };
    serde_json::to_string(&result).unwrap_or_else(|_| r#"{"session":null}"#.to_owned())
}

pub fn human_line(session: Option<&Session>, now: DateTime<Utc>) -> String {
    let Some(session) = session else {
        return "No session running".to_owned();
    };
    match (session.state, session.ends_at) {
        (SessionState::Active, Some(ends_at)) => format!(
            "Awake until {} · {} left",
            clock_time(ends_at, now),
            countdown(remaining_ms(ends_at, now))
        ),
        (SessionState::Active, None) => format!(
            "Awake until stopped · {} so far",
            elapsed(elapsed_ms(session, now))
        ),
        (SessionState::Ending, _) => "Ending the session".to_owned(),
        (SessionState::Ended, _) => format!(
            "Session ended · started {} · ran {}",
            clock_time(session.started_at, now),
            elapsed(elapsed_ms(session, now))
        ),
        (SessionState::NeedsAttention, _) => {
            "Needs attention: undoing the session's changes failed, \
             so this machine may not sleep normally. Run `vigilia stop` to retry."
                .to_owned()
        }
    }
}

/// How long until the human line for `session` changes, or `None` if it only changes when the
/// service sends something.
pub fn redraw_in(session: Option<&Session>, now: DateTime<Utc>) -> Option<Duration> {
    let session = session.filter(|s| s.state == SessionState::Active)?;
    let wait = match session.ends_at {
        Some(ends_at) => until_countdown_changes(remaining_ms(ends_at, now))?,
        None => until_elapsed_changes(elapsed_ms(session, now)),
    };
    Some(Duration::from_millis(u64::try_from(wait).unwrap_or(1)) + Duration::from_millis(2))
}

/// The shown countdown is rounded up, so it drops just after `ms` reaches a multiple of its unit.
/// `None` once it reaches zero.
fn until_countdown_changes(ms: i64) -> Option<i64> {
    if ms == 0 {
        return None;
    }
    let unit = if ceil_div(ms, 1_000) > HOUR_SECS {
        MINUTE_MS
    } else {
        1_000
    };
    Some((ms - 1) % unit + 1)
}

/// The shown time so far is rounded down, so it rises when `ms` reaches the next multiple of its
/// unit.
fn until_elapsed_changes(ms: i64) -> i64 {
    let unit = if ms / 1_000 >= HOUR_SECS {
        MINUTE_MS
    } else {
        1_000
    };
    unit - ms % unit
}

fn remaining_ms(ends_at: DateTime<Utc>, now: DateTime<Utc>) -> i64 {
    (ends_at - now).num_milliseconds().max(0)
}

fn elapsed_ms(session: &Session, now: DateTime<Utc>) -> i64 {
    (now - session.started_at).num_milliseconds().max(0)
}

/// Remaining time, rounded up: seconds under an hour, minutes above.
fn countdown(ms: i64) -> String {
    span(ms, ceil_div)
}

/// Time since the start, rounded down, in the same units as [`countdown`].
fn elapsed(ms: i64) -> String {
    span(ms, i64::div_euclid)
}

fn span(ms: i64, round: fn(i64, i64) -> i64) -> String {
    let secs = round(ms, 1_000);
    if secs < HOUR_SECS {
        return match secs {
            0..60 => format!("{secs}s"),
            _ => format!("{}m {:02}s", secs / 60, secs % 60),
        };
    }
    let mins = round(ms, MINUTE_MS);
    let (days, hours, mins) = (mins / 1_440, mins % 1_440 / 60, mins % 60);
    if days > 0 {
        format!("{days}d {hours}h {mins:02}m")
    } else {
        format!("{hours}h {mins:02}m")
    }
}

fn ceil_div(n: i64, d: i64) -> i64 {
    n.div_euclid(d) + i64::from(n.rem_euclid(d) != 0)
}

/// `11:00` today, `Tue 13 Oct 11:00` on another day, in local time.
fn clock_time(at: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let at = at.with_timezone(&Local);
    if at.date_naive() == now.with_timezone(&Local).date_naive() {
        at.format("%H:%M").to_string()
    } else {
        at.format("%a %-d %b %H:%M").to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeDelta;

    fn session(state: SessionState, ends_in_ms: i64, now: DateTime<Utc>) -> Session {
        Session {
            id: "s".into(),
            started_at: now,
            ends_at: Some(now + TimeDelta::milliseconds(ends_in_ms)),
            state,
            lid_closed_awake: false,
        }
    }

    #[test]
    fn countdown_rounds_up_to_its_unit() {
        assert_eq!(countdown(0), "0s");
        assert_eq!(countdown(1), "1s");
        assert_eq!(countdown(59_000), "59s");
        assert_eq!(countdown(59_001), "1m 00s");
        assert_eq!(countdown(3_599_000), "59m 59s");
        assert_eq!(countdown(3_600_000), "1h 00m");
        assert_eq!(countdown(3_600_001), "1h 01m");
        assert_eq!(countdown(7_200_000), "2h 00m");
        assert_eq!(countdown((86_400 + 7_200 + 300) * 1_000), "1d 2h 05m");
    }

    #[test]
    fn elapsed_rounds_down_to_its_unit() {
        assert_eq!(elapsed(999), "0s");
        assert_eq!(elapsed(59_999), "59s");
        assert_eq!(elapsed(60_000), "1m 00s");
        assert_eq!(elapsed(3_599_999), "59m 59s");
        assert_eq!(elapsed(3_600_000), "1h 00m");
        assert_eq!(elapsed(3_659_999), "1h 00m");
        assert_eq!(
            elapsed((86_400 + 7_200 + 300) * 1_000 + 59_999),
            "1d 2h 05m"
        );
    }

    fn until_stopped(started_ms_ago: i64, now: DateTime<Utc>) -> Session {
        Session {
            started_at: now - TimeDelta::milliseconds(started_ms_ago),
            ends_at: None,
            ..session(SessionState::Active, 0, now)
        }
    }

    #[test]
    fn redraws_a_session_without_an_end_as_time_passes() {
        let now = Utc::now();
        let wait = |ms| redraw_in(Some(&until_stopped(ms, now)), now);
        let ms = |n: u64| Some(Duration::from_millis(n + 2));
        assert_eq!(wait(0), ms(1_000));
        assert_eq!(wait(10_250), ms(750));
        assert_eq!(wait(3_599_500), ms(500));
        assert_eq!(wait(3_600_000), ms(60_000));
        assert_eq!(wait(3_630_000), ms(30_000));
    }

    #[test]
    fn redraws_when_the_shown_value_changes() {
        let now = Utc::now();
        let wait = |ms| redraw_in(Some(&session(SessionState::Active, ms, now)), now);
        let ms = |n: u64| Some(Duration::from_millis(n + 2));
        assert_eq!(wait(10_000), ms(1_000));
        assert_eq!(wait(10_250), ms(250));
        assert_eq!(wait(7_200_000), ms(60_000));
        assert_eq!(wait(7_230_000), ms(30_000));
        // Just over an hour: the minute display drops to `1h 00m` first.
        assert_eq!(wait(3_600_400), ms(400));
        assert_eq!(wait(0), None);
        assert_eq!(redraw_in(None, now), None);
        assert_eq!(
            redraw_in(Some(&session(SessionState::Ended, 5_000, now)), now),
            None
        );
    }

    #[test]
    fn human_lines() {
        let now = Utc::now();
        assert_eq!(human_line(None, now), "No session running");
        let active = human_line(Some(&session(SessionState::Active, 90_000, now)), now);
        assert!(active.starts_with("Awake until "), "{active}");
        assert!(active.ends_with(" · 1m 30s left"), "{active}");
        let open = human_line(Some(&until_stopped(90_000, now)), now);
        assert_eq!(open, "Awake until stopped · 1m 30s so far");
        let ended = Session {
            started_at: now - TimeDelta::milliseconds(90_000),
            ..session(SessionState::Ended, 0, now)
        };
        let ended = human_line(Some(&ended), now);
        assert!(ended.starts_with("Session ended · started "), "{ended}");
        assert!(ended.ends_with(" · ran 1m 30s"), "{ended}");
        let attention = human_line(Some(&session(SessionState::NeedsAttention, 0, now)), now);
        assert!(attention.contains("vigilia stop"), "{attention}");
    }

    #[test]
    fn json_has_the_protocol_shape() {
        assert_eq!(json_line(None), r#"{"session":null}"#);
        let now = Utc::now();
        let line = json_line(Some(&session(SessionState::Active, 1_000, now)));
        let back: SessionResult = serde_json::from_str(&line).unwrap();
        assert_eq!(back.session.unwrap().state, SessionState::Active);
    }

    #[test]
    fn needs_attention_exits_non_zero() {
        let now = Utc::now();
        assert_eq!(exit_code(None), 0);
        assert_eq!(
            exit_code(Some(&session(SessionState::Active, 1_000, now))),
            0
        );
        assert_eq!(
            exit_code(Some(&session(SessionState::NeedsAttention, 0, now))),
            exit::NEEDS_ATTENTION
        );
    }
}
