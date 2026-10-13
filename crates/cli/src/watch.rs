//! `status --watch`: subscribe once, then draw the countdown locally from `ends_at`, or the time
//! so far from `started_at` for a session without an end. The service only sends a message when
//! something changes.

use std::io::Write;

use chrono::Utc;
use vigilia_protocol::{Call, Empty, Event, ServerMessage, Session, SubscribeParams};

use crate::client::Client;
use crate::error::CliError;
use crate::output;

/// How the watch prints.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// A terminal: one line, redrawn in place when the shown countdown changes.
    Live,
    /// A pipe or file: one human line per change.
    Lines,
    /// One `{"session":…}` line per change.
    Json,
}

/// Runs until the connection closes (or Ctrl-C ends the process).
pub fn watch(client: &mut Client, mode: Mode, out: &mut dyn Write) -> Result<(), CliError> {
    // The first event is the current session, so nothing between here and there is missed.
    client.call::<Empty>(Call::Subscribe(SubscribeParams::default()))?;
    let mut session: Option<Session> = None;
    loop {
        let wait = match mode {
            Mode::Live => output::redraw_in(session.as_ref(), Utc::now()),
            Mode::Lines | Mode::Json => None,
        };
        let changed = match client.next_message(wait)? {
            Some(ServerMessage::Event(Event::SessionChanged { session: s })) => {
                session = s;
                true
            }
            Some(ServerMessage::Event(event)) => {
                notice(&event, mode, out);
                false
            }
            Some(ServerMessage::Response(_)) | None => false,
        };
        draw(session.as_ref(), changed, mode, out);
    }
}

fn draw(session: Option<&Session>, changed: bool, mode: Mode, out: &mut dyn Write) {
    let _ = match mode {
        Mode::Live => write!(out, "\r\x1b[2K{}", output::human_line(session, Utc::now())),
        Mode::Lines if changed => writeln!(out, "{}", output::human_line(session, Utc::now())),
        Mode::Json if changed => writeln!(out, "{}", output::json_line(session)),
        Mode::Lines | Mode::Json => Ok(()),
    };
    let _ = out.flush();
}

/// Warnings and needs-attention notices. JSON watchers see needs-attention through the
/// session's state instead.
fn notice(event: &Event, mode: Mode, out: &mut dyn Write) {
    let text = match event {
        Event::Warning { kind, detail } => format!("warning ({kind}): {detail}"),
        Event::NeedsAttention { detail } => format!("needs attention: {detail}"),
        Event::LidChanged { state } => format!("lid {state:?}").to_lowercase(),
        Event::PowerChanged { power } => match power.battery_percent {
            Some(percent) => format!("battery at {percent}%"),
            None => "power changed".to_owned(),
        },
        Event::SessionChanged { .. } => return,
    };
    let _ = match mode {
        Mode::Live => writeln!(out, "\r\x1b[2K{text}"),
        Mode::Lines => writeln!(out, "{text}"),
        Mode::Json => Ok(()),
    };
}
