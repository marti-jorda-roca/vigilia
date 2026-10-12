//! Events pushed to subscribers. Events are only sent when something changes: there is no
//! periodic tick, clients compute the remaining time from `ends_at`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::response::Session;

/// Whether the laptop lid is open or closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LidState {
    Open,
    Closed,
}

/// The machine's heat level, mirroring the OS thermal state.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum ThermalLevel {
    Nominal,
    Fair,
    Serious,
    Critical,
}

/// A reading of battery and heat, taken when the OS reports a change (never polled).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PowerSnapshot {
    /// Battery charge, or `None` on a machine without a battery.
    #[schemars(required, extend("type" = ["integer", "null"]))]
    pub battery_percent: Option<u8>,
    /// Whether external power is connected.
    pub on_ac: bool,
    pub thermal: ThermalLevel,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
    /// The session started, was extended, or ended. `session` is `None` when there is none.
    SessionChanged { session: Option<Session> },
    /// The lid opened or closed.
    LidChanged { state: LidState },
    /// Battery or heat changed.
    PowerChanged { power: PowerSnapshot },
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
    LidChanged,
    PowerChanged,
    Warning,
    NeedsAttention,
}

impl Event {
    pub fn kind(&self) -> EventKind {
        match self {
            Event::SessionChanged { .. } => EventKind::SessionChanged,
            Event::LidChanged { .. } => EventKind::LidChanged,
            Event::PowerChanged { .. } => EventKind::PowerChanged,
            Event::Warning { .. } => EventKind::Warning,
            Event::NeedsAttention { .. } => EventKind::NeedsAttention,
        }
    }
}
