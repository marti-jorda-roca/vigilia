//! Fails when `schema/protocol.schema.json` or `schema/examples.jsonl` no longer matches
//! the Rust types.
//! Regenerate with `UPDATE_SCHEMA=1 cargo test -p vigilia-protocol --test schema`.

use std::path::PathBuf;

use vigilia_protocol::{
    Call, Capabilities, ConfigResult, ConfigSetParams, Empty, ErrorCode, Event, HelloResult,
    HumanDuration, LidState, PowerSnapshot, Request, Response, RuleSettings, RulesPatch, Scope,
    Session, SessionResult, SessionState, Settings, SimulateLidParams, SubscribeParams,
    ThermalLevel,
};

fn settings() -> Settings {
    Settings {
        rules: RuleSettings {
            battery_floor_percent: 10,
            end_at_critical_heat: true,
        },
    }
}

fn session() -> Session {
    Session {
        id: "00000000-0000-4000-8000-000000000001".into(),
        started_at: "2026-10-12T00:00:00Z".parse().unwrap(),
        ends_at: Some("2026-10-12T02:00:00Z".parse().unwrap()),
        state: SessionState::Active,
        lid_closed_awake: false,
    }
}

fn request_examples() -> Vec<String> {
    let req = |id: u64, call: Call| serde_json::to_value(Request::new(id, call)).unwrap();
    let calls = [
        Call::Hello {},
        Call::SessionStart(vigilia_protocol::SessionStartParams {
            duration: Some(HumanDuration::from_secs(7_200)),
            lid_closed_awake: true,
        }),
        Call::SessionExtend(vigilia_protocol::SessionExtendParams {
            duration: HumanDuration::from_secs(1_800),
        }),
        Call::SessionStop {},
        Call::SessionStatus {},
        Call::Subscribe(SubscribeParams {
            filter: vec![vigilia_protocol::EventKind::SessionChanged],
        }),
        Call::DebugSimulateLid(SimulateLidParams {
            state: LidState::Closed,
        }),
        Call::ConfigGet {},
        Call::ConfigSet(ConfigSetParams {
            rules: Some(RulesPatch {
                battery_floor_percent: Some(20),
                end_at_critical_heat: Some(false),
            }),
        }),
    ];
    calls
        .into_iter()
        .enumerate()
        .map(|(i, call)| req(i as u64 + 1, call).to_string())
        .collect()
}

fn response_examples() -> Vec<String> {
    let responses = [
        Response::ok(
            1,
            &HelloResult {
                protocol_version: 1,
                server_version: "0.2.0".into(),
                capabilities: Capabilities {
                    keep_awake_idle: true,
                    keep_awake_lid_closed: true,
                    lid_events: true,
                    power_events: true,
                    backstop: true,
                    socket_activation: true,
                },
                scopes: vec![Scope::SessionRead],
            },
        ),
        Response::ok(
            2,
            &SessionResult {
                session: Some(session()),
            },
        ),
        Response::ok(3, &SessionResult { session: None }),
        Response::ok(4, &Empty {}),
        Response::ok(5, &ConfigResult { config: settings() }),
        Response::err(Some(6), ErrorCode::BadRequest, "bad request"),
    ];
    responses
        .iter()
        .map(|r| serde_json::to_value(r).unwrap().to_string())
        .collect()
}

fn event_examples() -> Vec<String> {
    let events = [
        Event::SessionChanged {
            session: Some(session()),
        },
        Event::SessionChanged { session: None },
        Event::LidChanged {
            state: LidState::Closed,
        },
        Event::PowerChanged {
            power: PowerSnapshot {
                battery_percent: Some(80),
                on_ac: false,
                thermal: ThermalLevel::Nominal,
            },
        },
        Event::Warning {
            kind: "rule".into(),
            detail: "battery at 9% is below the 10% floor".into(),
        },
        Event::NeedsAttention {
            detail: "could not undo the session's changes: injected failure".into(),
        },
        Event::ConfigChanged { config: settings() },
    ];
    events
        .iter()
        .map(|e| serde_json::to_value(e).unwrap().to_string())
        .collect()
}

fn examples() -> String {
    let mut lines = request_examples();
    lines.extend(response_examples());
    lines.extend(event_examples());
    lines.join("\n") + "\n"
}

#[test]
fn checked_in_schema_matches_types() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("schema/protocol.schema.json");
    let generated = serde_json::to_string_pretty(&vigilia_protocol::schema()).unwrap() + "\n";

    if std::env::var_os("UPDATE_SCHEMA").is_some() {
        std::fs::write(&path, &generated).unwrap();
        return;
    }

    let on_disk = std::fs::read_to_string(&path)
        .unwrap_or_default()
        .replace("\r\n", "\n");
    assert!(
        on_disk == generated,
        "{} is out of date. Run `UPDATE_SCHEMA=1 cargo test -p vigilia-protocol --test schema` \
         and commit the result. Remember: protocol changes may only add things.",
        path.display()
    );
}

#[test]
fn checked_in_examples_match_types() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("schema/examples.jsonl");
    let generated = examples();

    if std::env::var_os("UPDATE_SCHEMA").is_some() {
        std::fs::write(&path, &generated).unwrap();
        return;
    }

    let on_disk = std::fs::read_to_string(&path)
        .unwrap_or_default()
        .replace("\r\n", "\n");
    assert!(
        on_disk == generated,
        "{} is out of date. Run `UPDATE_SCHEMA=1 cargo test -p vigilia-protocol --test schema` \
         and commit the result.",
        path.display()
    );
}
