//! Events pushed to subscribers. Events are only sent when something changes: there is no
//! periodic tick, clients compute the remaining time from `ends_at`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::response::Session;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
    /// The session started, was extended, or ended. `session` is `None` when there is none.
    SessionChanged { session: Option<Session> },
    /// Something clients should show, but that doesn't stop the session.
    Warning { kind: String, detail: String },
    /// An undo step failed; the machine may not sleep normally until this is resolved.
    NeedsAttention { detail: String },
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    SessionChanged,
    Warning,
    NeedsAttention,
}

impl Event {
    pub fn kind(&self) -> EventKind {
        match self {
            Event::SessionChanged { .. } => EventKind::SessionChanged,
            Event::Warning { .. } => EventKind::Warning,
            Event::NeedsAttention { .. } => EventKind::NeedsAttention,
        }
    }
}
