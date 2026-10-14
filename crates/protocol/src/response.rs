//! Responses and the values they carry.

use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::context::Scope;
use crate::event::Event;

/// A keep-awake session as clients see it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Session {
    pub id: String,
    pub started_at: DateTime<Utc>,
    /// Clients draw the countdown from this; the service never sends ticks. `null` when the
    /// session runs until `session.stop`.
    #[schemars(required, extend("type" = ["string", "null"]))]
    pub ends_at: Option<DateTime<Utc>>,
    pub state: SessionState,
    #[serde(default)]
    pub lid_closed_awake: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum SessionState {
    Active,
    /// Undo steps are running.
    Ending,
    Ended,
    /// An undo step failed. `session.stop` retries it.
    NeedsAttention,
}

/// What this OS and machine support, so clients can hide what isn't available.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "a capability list is a set of independent flags by nature"
)]
pub struct Capabilities {
    /// Prevent idle sleep while the lid is open.
    pub keep_awake_idle: bool,
    /// Keep running with the lid closed.
    pub keep_awake_lid_closed: bool,
    /// Lid open/close events are available.
    pub lid_events: bool,
    /// Battery and heat change events are available.
    pub power_events: bool,
    /// A crash-recovery backstop job can be registered during a session.
    pub backstop: bool,
    /// The service was started by the OS on the first connection (launchd / systemd).
    pub socket_activation: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct HelloResult {
    pub protocol_version: u32,
    pub server_version: String,
    pub capabilities: Capabilities,
    /// The scopes this connection holds.
    pub scopes: Vec<Scope>,
}

/// Result of every `session.*` method.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SessionResult {
    pub session: Option<Session>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Empty {}

/// The rule settings a client can read and change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RuleSettings {
    pub battery_floor_percent: u8,
    pub end_at_critical_heat: bool,
}

/// The settings returned by `config.get` and `config.set`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Settings {
    pub rules: RuleSettings,
}

/// Result of `config.get` and `config.set`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ConfigResult {
    pub config: Settings,
}

/// The possible `result` shapes. Clients know which one to expect from the method they called;
/// this type exists for the JSON Schema.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum CallResult {
    Hello(HelloResult),
    Session(SessionResult),
    Empty(Empty),
    Config(ConfigResult),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    BadRequest,
    UnsupportedVersion,
    Forbidden,
    NoSession,
    SessionActive,
    Unsupported,
    Internal,
    /// A code added by a newer service. Treat it like `internal`.
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, thiserror::Error)]
#[error("{message} ({code:?})")]
pub struct ErrorBody {
    pub code: ErrorCode,
    pub message: String,
}

/// The reply to one request. Exactly one of `result` and `error` is set, matching `ok`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Response {
    /// The request's `id`, or `null` if the request was too malformed to have one.
    #[schemars(required)]
    pub id: Option<u64>,
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "Option<CallResult>")]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorBody>,
}

impl Response {
    pub fn ok<T: Serialize>(id: u64, result: &T) -> Self {
        match serde_json::to_value(result) {
            Ok(value) => Self {
                id: Some(id),
                ok: true,
                result: Some(value),
                error: None,
            },
            Err(e) => Self::err(Some(id), ErrorCode::Internal, e.to_string()),
        }
    }

    pub fn err(id: Option<u64>, code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            id,
            ok: false,
            result: None,
            error: Some(ErrorBody {
                code,
                message: message.into(),
            }),
        }
    }

    /// Converts into the result type the caller expects for the method it called.
    pub fn into_result<T: DeserializeOwned>(self) -> Result<T, ErrorBody> {
        if !self.ok {
            return Err(self.error.unwrap_or(ErrorBody {
                code: ErrorCode::Internal,
                message: "error response without a body".into(),
            }));
        }
        serde_json::from_value(self.result.unwrap_or(Value::Null)).map_err(|e| ErrorBody {
            code: ErrorCode::Internal,
            message: format!("unexpected result shape: {e}"),
        })
    }
}

/// Anything the service writes on a connection.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum ServerMessage {
    Event(Event),
    Response(Response),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session() -> Session {
        Session {
            id: "s1".into(),
            started_at: "2026-10-12T00:00:00Z".parse().unwrap(),
            ends_at: Some("2026-10-12T02:00:00Z".parse().unwrap()),
            state: SessionState::Active,
            lid_closed_awake: false,
        }
    }

    #[test]
    fn documented_response_parses() {
        let line = r#"{"id":1,"ok":true,"result":{"session":{"id":"…","started_at":"2026-10-12T00:00:00Z","ends_at":"2026-10-12T02:00:00Z","state":"Active"}}}"#;
        let msg: ServerMessage = serde_json::from_str(line).unwrap();
        let ServerMessage::Response(resp) = msg else {
            panic!("expected a response")
        };
        let result: SessionResult = resp.into_result().unwrap();
        assert_eq!(result.session.unwrap().state, SessionState::Active);
    }

    #[test]
    fn a_session_without_an_end_sends_null() {
        let session = Session {
            ends_at: None,
            ..session()
        };
        let line = serde_json::to_string(&session).unwrap();
        assert!(line.contains(r#""ends_at":null"#), "{line}");
        assert_eq!(serde_json::from_str::<Session>(&line).unwrap(), session);
    }

    #[test]
    fn events_and_responses_are_told_apart() {
        let event = Event::SessionChanged {
            session: Some(session()),
        };
        let line = serde_json::to_string(&event).unwrap();
        assert!(line.starts_with(r#"{"event":"session_changed""#), "{line}");
        assert_eq!(
            serde_json::from_str::<ServerMessage>(&line).unwrap(),
            ServerMessage::Event(event)
        );

        let resp = Response::ok(3, &SessionResult { session: None });
        let line = serde_json::to_string(&resp).unwrap();
        assert_eq!(line, r#"{"id":3,"ok":true,"result":{"session":null}}"#);
        assert_eq!(
            serde_json::from_str::<ServerMessage>(&line).unwrap(),
            ServerMessage::Response(resp)
        );
    }

    #[test]
    fn error_responses() {
        let resp = Response::err(Some(4), ErrorCode::Forbidden, "needs session.control");
        let line = serde_json::to_string(&resp).unwrap();
        assert_eq!(
            line,
            r#"{"id":4,"ok":false,"error":{"code":"forbidden","message":"needs session.control"}}"#
        );
        let err = resp.into_result::<SessionResult>().unwrap_err();
        assert_eq!(err.code, ErrorCode::Forbidden);

        let future: ErrorBody =
            serde_json::from_str(r#"{"code":"rate_limited","message":"slow down"}"#).unwrap();
        assert_eq!(future.code, ErrorCode::Unknown);
    }

    #[test]
    fn unknown_capabilities_are_ignored_and_missing_ones_default() {
        let caps: Capabilities =
            serde_json::from_str(r#"{"keep_awake_idle":true,"mute":true}"#).unwrap();
        assert!(caps.keep_awake_idle);
        assert!(!caps.keep_awake_lid_closed);
    }
}
