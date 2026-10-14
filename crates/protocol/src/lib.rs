//! The Vigilia wire protocol.
//!
//! One JSON object per line, in both directions. Clients send [`Request`]s; the service answers
//! each with a [`Response`] carrying the same `id`, and pushes [`Event`]s to subscribers. The same
//! messages travel unchanged over every transport.
//!
//! Compatibility rule: within a protocol version, changes only add things (methods, events,
//! optional fields). `schema/protocol.schema.json` is generated from these types and CI fails
//! when it is out of date.

mod context;
mod duration;
mod event;
mod request;
mod response;

pub use context::{ClientContext, Origin, Scope, ScopeParseError, ScopeSet};
pub use duration::{DurationParseError, HumanDuration};
pub use event::{Event, EventKind, LidState, PowerSnapshot, ThermalLevel};
pub use request::{
    Call, ConfigSetParams, Request, RequestParseError, RulesPatch, SessionExtendParams,
    SessionStartParams, SimulateLidParams, SubscribeParams,
};
pub use response::{
    CallResult, Capabilities, ConfigResult, Empty, ErrorBody, ErrorCode, HelloResult, Response,
    RuleSettings, ServerMessage, Session, SessionResult, SessionState, Settings,
};

/// The protocol version this build speaks. Bumped only for incompatible changes.
pub const PROTOCOL_VERSION: u32 = 1;

/// Everything that can appear on a connection, in either direction. Only used to generate the
/// schema.
#[derive(schemars::JsonSchema)]
#[serde(untagged)]
#[expect(dead_code, reason = "never constructed, only fed to schemars")]
enum WireMessage {
    Request(Request),
    ServerMessage(ServerMessage),
}

/// The JSON Schema for the protocol, as checked in at `schema/protocol.schema.json`.
pub fn schema() -> serde_json::Value {
    let mut schema = schemars::schema_for!(WireMessage).to_value();
    schema["title"] = "Vigilia protocol".into();
    schema["x-protocol-version"] = PROTOCOL_VERSION.into();
    schema
}
