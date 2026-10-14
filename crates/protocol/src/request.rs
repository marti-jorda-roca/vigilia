//! Requests: `{"id":1,"v":1,"method":"session.start","params":{...}}`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::context::Scope;
use crate::duration::HumanDuration;
use crate::event::{EventKind, LidState};

/// One request from a client. `id` is echoed in the response; `v` is the protocol version the
/// client speaks.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct Request {
    pub id: u64,
    pub v: u32,
    #[serde(flatten)]
    pub call: Call,
}

/// A method and its parameters. Methods without parameters accept `params` being absent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "method", content = "params")]
pub enum Call {
    /// Returns the protocol version, capabilities, and the scopes this client holds.
    #[serde(rename = "hello")]
    Hello {},
    #[serde(rename = "session.start")]
    SessionStart(SessionStartParams),
    #[serde(rename = "session.extend")]
    SessionExtend(SessionExtendParams),
    #[serde(rename = "session.stop")]
    SessionStop {},
    #[serde(rename = "session.status")]
    SessionStatus {},
    /// After a successful response, matching events are streamed on the same connection.
    #[serde(rename = "subscribe")]
    Subscribe(SubscribeParams),
    /// Pretends the lid opened or closed (`vigilia debug simulate-lid`). Admin only.
    #[serde(rename = "debug.simulate_lid")]
    DebugSimulateLid(SimulateLidParams),
    #[serde(rename = "config.get")]
    ConfigGet {},
    #[serde(rename = "config.set")]
    ConfigSet(ConfigSetParams),
}

/// A patch to the `[rules]` settings: only the fields sent change.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RulesPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub battery_floor_percent: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_at_critical_heat: Option<bool>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ConfigSetParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rules: Option<RulesPatch>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SessionStartParams {
    /// How long the session lasts. Absent or `null`: until `session.stop`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration: Option<HumanDuration>,
    /// Keep the machine awake with the lid closed too. Not supported before v0.2.
    #[serde(default)]
    pub lid_closed_awake: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SessionExtendParams {
    pub duration: HumanDuration,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SimulateLidParams {
    pub state: LidState,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SubscribeParams {
    /// Event kinds to receive. Empty means every kind.
    #[serde(default)]
    pub filter: Vec<EventKind>,
}

impl Call {
    /// The method name as it appears on the wire.
    pub fn method(&self) -> &'static str {
        match self {
            Call::Hello {} => "hello",
            Call::SessionStart(_) => "session.start",
            Call::SessionExtend(_) => "session.extend",
            Call::SessionStop {} => "session.stop",
            Call::SessionStatus {} => "session.status",
            Call::Subscribe(_) => "subscribe",
            Call::DebugSimulateLid(_) => "debug.simulate_lid",
            Call::ConfigGet {} => "config.get",
            Call::ConfigSet(_) => "config.set",
        }
    }

    /// The scope a client must hold to make this call. `None` means anyone may call it.
    pub fn required_scope(&self) -> Option<Scope> {
        match self {
            Call::Hello {} => None,
            Call::SessionStatus {} | Call::Subscribe(_) | Call::ConfigGet {} => {
                Some(Scope::SessionRead)
            }
            Call::SessionStart(_) | Call::SessionExtend(_) | Call::SessionStop {} => {
                Some(Scope::SessionControl)
            }
            Call::ConfigSet(_) => Some(Scope::ConfigWrite),
            Call::DebugSimulateLid(_) => Some(Scope::Admin),
        }
    }
}

/// A line that could not be turned into a [`Request`]. `id` is set when the line had one, so the
/// error response can still be matched by the client.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[error("{message}")]
pub struct RequestParseError {
    pub id: Option<u64>,
    pub message: String,
}

impl Request {
    pub fn new(id: u64, call: Call) -> Self {
        Self {
            id,
            v: crate::PROTOCOL_VERSION,
            call,
        }
    }

    /// Parses one line of the wire format.
    pub fn parse(line: &str) -> Result<Self, RequestParseError> {
        let value: Value = serde_json::from_str(line).map_err(|e| RequestParseError {
            id: None,
            message: format!("invalid JSON: {e}"),
        })?;
        Self::from_value(value)
    }

    fn from_value(value: Value) -> Result<Self, RequestParseError> {
        let Value::Object(mut obj) = value else {
            return Err(RequestParseError {
                id: None,
                message: "request must be a JSON object".into(),
            });
        };
        let id = obj.get("id").and_then(Value::as_u64);
        let fail = |message: String| RequestParseError { id, message };
        let id = id.ok_or_else(|| fail("missing or invalid `id`".into()))?;
        let v = obj
            .get("v")
            .and_then(Value::as_u64)
            .and_then(|v| u32::try_from(v).ok())
            .ok_or_else(|| fail("missing or invalid `v`".into()))?;
        let method = obj
            .remove("method")
            .filter(Value::is_string)
            .ok_or_else(|| fail("missing or invalid `method`".into()))?;
        let params = match obj.remove("params") {
            None | Some(Value::Null) => Value::Object(Map::new()),
            Some(p) => p,
        };
        let mut tagged = Map::new();
        tagged.insert("method".into(), method);
        tagged.insert("params".into(), params);
        let call =
            serde_json::from_value(Value::Object(tagged)).map_err(|e| fail(e.to_string()))?;
        Ok(Self { id, v, call })
    }
}

impl<'de> Deserialize<'de> for Request {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(d)?;
        Self::from_value(value).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_documented_example() {
        let req = Request::parse(
            r#"{"id":1,"v":1,"method":"session.start","params":{"duration":"2h","lid_closed_awake":true}}"#,
        )
        .unwrap();
        assert_eq!(req.id, 1);
        assert_eq!(
            req.call,
            Call::SessionStart(SessionStartParams {
                duration: Some(HumanDuration::from_secs(7_200)),
                lid_closed_awake: true,
            })
        );
    }

    #[test]
    fn start_without_a_duration_runs_until_stopped() {
        for line in [
            r#"{"id":1,"v":1,"method":"session.start"}"#,
            r#"{"id":1,"v":1,"method":"session.start","params":{"duration":null}}"#,
        ] {
            assert_eq!(
                Request::parse(line).unwrap().call,
                Call::SessionStart(SessionStartParams {
                    duration: None,
                    lid_closed_awake: false,
                }),
                "{line}"
            );
        }
    }

    #[test]
    fn params_are_optional_for_parameterless_methods() {
        for line in [
            r#"{"id":2,"v":1,"method":"session.stop"}"#,
            r#"{"id":2,"v":1,"method":"session.stop","params":null}"#,
            r#"{"id":2,"v":1,"method":"session.stop","params":{}}"#,
        ] {
            assert_eq!(Request::parse(line).unwrap().call, Call::SessionStop {});
        }
        let sub = Request::parse(r#"{"id":3,"v":1,"method":"subscribe"}"#).unwrap();
        assert_eq!(sub.call, Call::Subscribe(SubscribeParams::default()));
    }

    #[test]
    fn unknown_fields_are_ignored() {
        let req = Request::parse(
            r#"{"id":4,"v":1,"method":"session.extend","params":{"duration":"30m","later":1},"trace":"x"}"#,
        )
        .unwrap();
        assert_eq!(req.call.method(), "session.extend");
    }

    #[test]
    fn errors_keep_the_id_when_present() {
        let err = Request::parse(r#"{"id":7,"v":1,"method":"session.explode"}"#).unwrap_err();
        assert_eq!(err.id, Some(7));
        let err = Request::parse(r#"{"v":1,"method":"hello"}"#).unwrap_err();
        assert_eq!(err.id, None);
        assert!(Request::parse("not json").is_err());
        let err =
            Request::parse(r#"{"id":8,"v":1,"method":"session.start","params":{"duration":"2x"}}"#)
                .unwrap_err();
        assert_eq!(err.id, Some(8));
    }

    #[test]
    fn serializes_then_parses_back() {
        let calls = [
            Call::Hello {},
            Call::SessionStart(SessionStartParams {
                duration: Some(HumanDuration::from_secs(60)),
                lid_closed_awake: false,
            }),
            Call::SessionStart(SessionStartParams {
                duration: None,
                lid_closed_awake: false,
            }),
            Call::SessionExtend(SessionExtendParams {
                duration: HumanDuration::from_secs(1_800),
            }),
            Call::SessionStop {},
            Call::SessionStatus {},
            Call::Subscribe(SubscribeParams {
                filter: vec![EventKind::SessionChanged],
            }),
            Call::DebugSimulateLid(SimulateLidParams {
                state: crate::LidState::Closed,
            }),
            Call::ConfigGet {},
            Call::ConfigSet(ConfigSetParams {
                rules: Some(RulesPatch {
                    battery_floor_percent: Some(20),
                    end_at_critical_heat: Some(false),
                }),
            }),
            Call::ConfigSet(ConfigSetParams { rules: None }),
        ];
        for (i, call) in calls.into_iter().enumerate() {
            let req = Request::new(i as u64, call);
            let line = serde_json::to_string(&req).unwrap();
            assert_eq!(Request::parse(&line).unwrap(), req, "{line}");
        }
    }
}
