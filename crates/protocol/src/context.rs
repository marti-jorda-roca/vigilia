//! Who is asking, and what they are allowed to do.
//!
//! Every request reaches `dispatch` together with a [`ClientContext`]. Permissions are checked
//! there, so every transport (local socket today, remote listeners later) gets the same checks.

use std::borrow::Cow;
use std::collections::BTreeSet;
use std::fmt;
use std::str::FromStr;

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Where a request came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Origin {
    /// A process of the same user on this machine.
    Local,
    /// A paired device, identified by the id it was given at pairing.
    Remote { device_id: String },
}

/// One permission. On the wire a scope is a string such as `session.read` or `agent.send:*`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Scope {
    SessionRead,
    SessionControl,
    AgentRead,
    /// Send text to one agent, or to every agent with `*`.
    AgentSend(String),
    ConfigWrite,
    /// Covers every other scope.
    Admin,
}

impl Scope {
    /// Every scope a local client gets.
    pub fn all() -> Vec<Scope> {
        vec![
            Scope::SessionRead,
            Scope::SessionControl,
            Scope::AgentRead,
            Scope::AgentSend("*".into()),
            Scope::ConfigWrite,
            Scope::Admin,
        ]
    }

    /// Whether holding `self` allows an action that requires `required`.
    pub fn covers(&self, required: &Scope) -> bool {
        match (self, required) {
            (Scope::Admin, _) => true,
            (Scope::AgentSend(held), Scope::AgentSend(wanted)) => held == "*" || held == wanted,
            (held, wanted) => held == wanted,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unknown scope `{0}`")]
pub struct ScopeParseError(String);

impl FromStr for Scope {
    type Err = ScopeParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "session.read" => Scope::SessionRead,
            "session.control" => Scope::SessionControl,
            "agent.read" => Scope::AgentRead,
            "config.write" => Scope::ConfigWrite,
            "admin" => Scope::Admin,
            other => match other.strip_prefix("agent.send:") {
                Some(agent) if !agent.is_empty() => Scope::AgentSend(agent.into()),
                _ => return Err(ScopeParseError(other.into())),
            },
        })
    }
}

impl fmt::Display for Scope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Scope::SessionRead => f.write_str("session.read"),
            Scope::SessionControl => f.write_str("session.control"),
            Scope::AgentRead => f.write_str("agent.read"),
            Scope::AgentSend(agent) => write!(f, "agent.send:{agent}"),
            Scope::ConfigWrite => f.write_str("config.write"),
            Scope::Admin => f.write_str("admin"),
        }
    }
}

impl Serialize for Scope {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Scope {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = Cow::<str>::deserialize(d)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

impl JsonSchema for Scope {
    fn schema_name() -> Cow<'static, str> {
        "Scope".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "string",
            "pattern": "^(session\\.read|session\\.control|agent\\.read|agent\\.send:.+|config\\.write|admin)$"
        })
    }
}

/// The scopes a client holds.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScopeSet(BTreeSet<Scope>);

impl ScopeSet {
    pub fn all() -> Self {
        Scope::all().into_iter().collect()
    }

    pub fn covers(&self, required: &Scope) -> bool {
        self.0.iter().any(|held| held.covers(required))
    }

    pub fn iter(&self) -> impl Iterator<Item = &Scope> {
        self.0.iter()
    }
}

impl FromIterator<Scope> for ScopeSet {
    fn from_iter<I: IntoIterator<Item = Scope>>(iter: I) -> Self {
        Self(iter.into_iter().collect())
    }
}

/// Who sent a request and what they may do. Built by the transport, checked by `dispatch`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientContext {
    pub origin: Origin,
    pub scopes: ScopeSet,
}

impl ClientContext {
    /// A same-user process on this machine. Local clients get every permission.
    pub fn local() -> Self {
        Self {
            origin: Origin::Local,
            scopes: ScopeSet::all(),
        }
    }

    pub fn remote(device_id: impl Into<String>, scopes: impl IntoIterator<Item = Scope>) -> Self {
        Self {
            origin: Origin::Remote {
                device_id: device_id.into(),
            },
            scopes: scopes.into_iter().collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scope_strings_round_trip() {
        for scope in Scope::all()
            .into_iter()
            .chain([Scope::AgentSend("tmux:work:1.0".into())])
        {
            assert_eq!(scope.to_string().parse::<Scope>(), Ok(scope));
        }
        assert!("agent.send:".parse::<Scope>().is_err());
        assert!("session.write".parse::<Scope>().is_err());
    }

    #[test]
    fn coverage() {
        let a = Scope::AgentSend("a".into());
        assert!(Scope::Admin.covers(&Scope::SessionControl));
        assert!(Scope::AgentSend("*".into()).covers(&a));
        assert!(a.covers(&a));
        assert!(!a.covers(&Scope::AgentSend("b".into())));
        assert!(!Scope::SessionRead.covers(&Scope::SessionControl));
    }

    #[test]
    fn local_context_covers_everything() {
        let ctx = ClientContext::local();
        assert!(ctx.scopes.covers(&Scope::SessionControl));
        assert!(ctx.scopes.covers(&Scope::AgentSend("x".into())));
        let remote = ClientContext::remote("phone", [Scope::SessionRead]);
        assert!(remote.scopes.covers(&Scope::SessionRead));
        assert!(!remote.scopes.covers(&Scope::SessionControl));
    }
}
