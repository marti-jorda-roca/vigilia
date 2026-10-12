//! Service behaviour with a fake platform and tokio's paused clock, so hours pass instantly.

mod common;

use std::convert::Infallible;
use std::time::Duration;

use common::Harness;
use vigilia_core::{ActionKind, Config, Journal, UndoRecord};
use vigilia_daemon::Exit;
use vigilia_platform::FakePlatform;
use vigilia_protocol::{
    Call, ErrorCode, Event, LidState, PowerSnapshot, SessionResult, SessionState, SubscribeParams,
    ThermalLevel,
};

const MIN: Duration = Duration::from_secs(60);

/// Drains whatever events are ready without waiting.
async fn ready_events(sub: &mut vigilia_daemon::bus::Subscription) -> Vec<Event> {
    let mut events = Vec::new();
    while let Ok(Some(e)) = tokio::time::timeout(Duration::ZERO, sub.recv()).await {
        events.push(e);
    }
    events
}

fn session_states(events: &[Event]) -> Vec<Option<SessionState>> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::SessionChanged { session } => Some(session.as_ref().map(|s| s.state)),
            _ => None,
        })
        .collect()
}

#[tokio::test(start_paused = true)]
async fn an_hour_long_session_wakes_the_timer_at_most_60_times() {
    let mut h = Harness::spawn();
    let mut sub = h
        .call(Call::Subscribe(SubscribeParams::default()))
        .await
        .subscription
        .unwrap();

    let started: SessionResult = h.start("1h").await.response.into_result().unwrap();
    assert!(h.platform.awake());

    // Halfway through: nothing but the start was published. No ticks.
    tokio::time::sleep(30 * MIN).await;
    let events = ready_events(&mut sub).await;
    assert_eq!(
        session_states(&events),
        vec![None, Some(SessionState::Active)]
    );
    assert_eq!(h.fires(), 0);

    tokio::time::sleep(30 * MIN + Duration::from_secs(1)).await;
    let events = ready_events(&mut sub).await;
    assert_eq!(session_states(&events), vec![Some(SessionState::Ended)]);
    assert!(!h.platform.awake());

    let fires = h.fires();
    println!("timer wake-ups over a simulated 1-hour session: {fires}");
    assert!(fires <= 60, "{fires} wake-ups");
    assert_eq!(fires, 1, "only the session end should wake the service");

    let status: SessionResult = h
        .call(Call::SessionStatus {})
        .await
        .response
        .into_result()
        .unwrap();
    assert_eq!(status.session, None);
    assert_eq!(started.session.unwrap().state, SessionState::Active);
}

#[tokio::test(start_paused = true)]
async fn extend_moves_the_deadline() {
    let mut h = Harness::spawn();
    h.start("10m").await;
    tokio::time::sleep(9 * MIN).await;
    let extended: SessionResult = h
        .call(Call::SessionExtend(vigilia_protocol::SessionExtendParams {
            duration: "5m".parse().unwrap(),
        }))
        .await
        .response
        .into_result()
        .unwrap();
    assert!(extended.session.is_some());
    tokio::time::sleep(2 * MIN).await;
    assert!(h.platform.awake(), "extended session must still run");
    tokio::time::sleep(4 * MIN + Duration::from_secs(1)).await;
    assert!(!h.platform.awake());
}

#[tokio::test(start_paused = true)]
async fn a_session_without_an_end_runs_until_stopped() {
    let mut h = Harness::spawn();
    let started: SessionResult = h
        .call(Call::SessionStart(vigilia_protocol::SessionStartParams {
            duration: None,
            lid_closed_awake: false,
        }))
        .await
        .response
        .into_result()
        .unwrap();
    assert_eq!(started.session.unwrap().ends_at, None);

    // Longer than any timed session may last, and long past the idle exit.
    tokio::time::sleep(31 * 24 * 60 * MIN).await;
    assert!(h.platform.awake());
    assert!(!h.task.is_finished());
    assert_eq!(h.fires(), 0, "nothing to wake up for");

    let stopped: SessionResult = h
        .call(Call::SessionStop {})
        .await
        .response
        .into_result()
        .unwrap();
    assert_eq!(stopped.session.unwrap().state, SessionState::Ended);
    assert!(!h.platform.awake());
}

#[tokio::test(start_paused = true)]
async fn exits_when_idle_and_not_while_subscribed() {
    let mut h = Harness::spawn();
    let sub = h
        .call(Call::Subscribe(SubscribeParams::default()))
        .await
        .subscription
        .unwrap();
    tokio::time::sleep(10 * MIN).await;
    assert!(
        !h.task.is_finished(),
        "a subscriber keeps the service alive"
    );

    drop(sub);
    tokio::time::sleep(5 * MIN - Duration::from_secs(1)).await;
    assert!(!h.task.is_finished());
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!((&mut h.task).await.unwrap(), Exit::Idle);
}

#[tokio::test(start_paused = true)]
async fn session_end_then_idle_exit() {
    let mut h = Harness::spawn();
    h.start("2m").await;
    tokio::time::sleep(2 * MIN + 5 * MIN + Duration::from_secs(1)).await;
    assert_eq!((&mut h.task).await.unwrap(), Exit::Idle);
    assert!(!h.platform.awake());
    assert!(h.fires() <= 2, "{} wake-ups", h.fires());
}

#[tokio::test(start_paused = true)]
async fn idle_countdown_starts_when_the_session_ends() {
    let mut h = Harness::spawn();
    h.start("1h").await;
    tokio::time::sleep(60 * MIN + Duration::from_secs(1)).await;
    assert!(!h.platform.awake());
    assert!(!h.task.is_finished(), "exited right after the session");
    tokio::time::sleep(5 * MIN).await;
    assert_eq!((&mut h.task).await.unwrap(), Exit::Idle);
}

#[tokio::test(start_paused = true)]
async fn dropped_subscription_without_notice_still_allows_idle_exit() {
    let mut h = Harness::spawn();
    // The response never reaches anyone: the subscription is dropped on the floor, as when a
    // connection dies before its subscribe reply is written.
    drop(h.call(Call::Subscribe(SubscribeParams::default())).await);
    tokio::time::sleep(5 * MIN + Duration::from_secs(1)).await;
    assert_eq!((&mut h.task).await.unwrap(), Exit::Idle);
}

#[tokio::test(start_paused = true)]
async fn huge_idle_exit_config_does_not_crash() {
    let config = Config {
        energy: vigilia_core::config::EnergyConfig {
            idle_exit_minutes: u64::MAX / 60,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut h = Harness::spawn_with(tempfile::tempdir().unwrap(), FakePlatform::new(), config);
    h.call(Call::SessionStatus {}).await;
    tokio::time::sleep(60 * MIN).await;
    assert!(!h.task.is_finished());
}

#[tokio::test(start_paused = true)]
async fn idle_exit_follows_config() {
    let config = Config {
        energy: vigilia_core::config::EnergyConfig {
            idle_exit_minutes: 1,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut h = Harness::spawn_with(tempfile::tempdir().unwrap(), FakePlatform::new(), config);
    tokio::time::sleep(MIN + Duration::from_secs(1)).await;
    assert_eq!((&mut h.task).await.unwrap(), Exit::Idle);
}

#[tokio::test(start_paused = true)]
async fn startup_replays_actions_left_by_a_crash() {
    let dir = tempfile::tempdir().unwrap();
    {
        let mut journal = Journal::open(common::journal_path(&dir)).unwrap();
        journal
            .apply(ActionKind::KeepAwake, None, || {
                Ok::<_, Infallible>(UndoRecord::IdleSleepAssertion {
                    pid: 999_999,
                    assertion_id: 42,
                })
            })
            .unwrap();
        // Dropped without undo: the previous service was killed mid-session.
    }
    let platform = FakePlatform::new();
    let h = Harness::spawn_with(dir, platform.clone(), Config::default());
    tokio::task::yield_now().await;

    let restored = platform.restored();
    assert_eq!(restored.len(), 1);
    assert_eq!(restored[0].kind, ActionKind::KeepAwake);
    assert!(
        Journal::open(h.journal_path())
            .unwrap()
            .pending()
            .is_empty()
    );
    assert_eq!(std::fs::metadata(h.journal_path()).unwrap().len(), 0);
}

#[tokio::test(start_paused = true)]
async fn failed_release_needs_attention_and_stop_retries() {
    let mut h = Harness::spawn();
    let mut sub = h
        .call(Call::Subscribe(SubscribeParams::default()))
        .await
        .subscription
        .unwrap();
    h.start("5m").await;
    h.platform.fail_release(true);
    tokio::time::sleep(5 * MIN + Duration::from_secs(1)).await;

    let events = ready_events(&mut sub).await;
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::NeedsAttention { .. }))
    );
    let status: SessionResult = h
        .call(Call::SessionStatus {})
        .await
        .response
        .into_result()
        .unwrap();
    assert_eq!(status.session.unwrap().state, SessionState::NeedsAttention);
    assert!(h.platform.awake());
    let err = h
        .start("5m")
        .await
        .response
        .into_result::<SessionResult>()
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::SessionActive);

    assert!(
        h.platform.backstop_registered(),
        "the backstop stays while an undo has failed"
    );
    h.platform.fail_release(false);
    let stopped: SessionResult = h
        .call(Call::SessionStop {})
        .await
        .response
        .into_result()
        .unwrap();
    assert_eq!(stopped.session.unwrap().state, SessionState::Ended);
    assert!(!h.platform.awake());
    assert!(
        Journal::open(h.journal_path())
            .unwrap()
            .pending()
            .is_empty()
    );
}

#[tokio::test(start_paused = true)]
async fn failed_keep_awake_leaves_no_session() {
    let mut h = Harness::spawn();
    h.platform.fail_keep_awake(true);
    let err = h
        .start("5m")
        .await
        .response
        .into_result::<SessionResult>()
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Internal);
    let status: SessionResult = h
        .call(Call::SessionStatus {})
        .await
        .response
        .into_result()
        .unwrap();
    assert_eq!(status.session, None);
    assert!(
        Journal::open(h.journal_path())
            .unwrap()
            .pending()
            .is_empty()
    );
}

#[tokio::test(start_paused = true)]
async fn errors_for_bad_session_requests() {
    let mut h = Harness::spawn();
    let code = |d: vigilia_daemon::dispatch::Dispatched| d.response.error.map(|e| e.code);
    assert_eq!(
        code(h.call(Call::SessionStop {}).await),
        Some(ErrorCode::NoSession)
    );
    assert_eq!(code(h.start("0s").await), Some(ErrorCode::BadRequest));
    assert_eq!(code(h.start("31d").await), Some(ErrorCode::BadRequest));
    // Lid-closed sessions work since step 2 (the fake platform supports them).
    let lid = h
        .call(Call::SessionStart(vigilia_protocol::SessionStartParams {
            duration: Some("1h".parse().unwrap()),
            lid_closed_awake: true,
        }))
        .await;
    assert_eq!(code(lid), None);
    assert!(h.platform.sleep_disabled());
    assert!(h.platform.backstop_registered());
    h.call(Call::SessionStop {}).await;
    assert!(!h.platform.sleep_disabled());
    assert!(!h.platform.backstop_registered());
    h.start("1h").await;
    assert_eq!(code(h.start("1h").await), Some(ErrorCode::SessionActive));
}

#[tokio::test(start_paused = true)]
async fn a_signal_ends_the_session_before_exiting() {
    let mut h = Harness::spawn();
    h.start("1h").await;
    assert!(h.platform.awake());
    h.signal();
    assert_eq!((&mut h.task).await.unwrap(), Exit::Signal);
    assert!(!h.platform.awake());
    assert!(
        Journal::open(h.journal_path())
            .unwrap()
            .pending()
            .is_empty()
    );
}

#[tokio::test(start_paused = true)]
async fn lid_closed_session_disables_sleep_and_registers_the_backstop() {
    let mut h = Harness::spawn();
    let started: SessionResult = h
        .call(Call::SessionStart(vigilia_protocol::SessionStartParams {
            duration: Some("1h".parse().unwrap()),
            lid_closed_awake: true,
        }))
        .await
        .response
        .into_result()
        .unwrap();
    assert!(started.session.unwrap().lid_closed_awake);
    assert!(h.platform.awake());
    assert!(h.platform.sleep_disabled());
    assert!(h.platform.backstop_registered());

    let stopped: SessionResult = h
        .call(Call::SessionStop {})
        .await
        .response
        .into_result()
        .unwrap();
    assert_eq!(stopped.session.unwrap().state, SessionState::Ended);
    assert!(!h.platform.sleep_disabled());
    assert!(!h.platform.backstop_registered());
    assert!(
        Journal::open(h.journal_path())
            .unwrap()
            .pending()
            .is_empty()
    );
}

#[tokio::test(start_paused = true)]
async fn battery_below_the_floor_at_start_ends_straight_away() {
    let platform = FakePlatform::new();
    platform.set_power(PowerSnapshot {
        battery_percent: Some(5),
        on_ac: false,
        thermal: ThermalLevel::Nominal,
    });
    let mut h = Harness::spawn_with(tempfile::tempdir().unwrap(), platform, Config::default());
    let started: SessionResult = h.start("1h").await.response.into_result().unwrap();
    assert_eq!(started.session.unwrap().state, SessionState::Ended);
    assert!(!h.platform.awake());
    let status: SessionResult = h
        .call(Call::SessionStatus {})
        .await
        .response
        .into_result()
        .unwrap();
    assert_eq!(status.session, None);
}

#[tokio::test(start_paused = true)]
async fn a_power_event_below_the_floor_ends_the_session() {
    let mut h = Harness::spawn();
    h.start("1h").await;
    assert!(h.platform.awake());
    h.platform.emit_power(PowerSnapshot {
        battery_percent: Some(5),
        on_ac: false,
        thermal: ThermalLevel::Nominal,
    });
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert!(!h.platform.awake());
    assert!(!h.platform.backstop_registered());
}

#[tokio::test(start_paused = true)]
async fn critical_heat_ends_the_session_but_serious_heat_does_not() {
    let mut h = Harness::spawn();
    h.start("1h").await;
    h.platform.emit_power(PowerSnapshot {
        battery_percent: Some(90),
        on_ac: false,
        thermal: ThermalLevel::Serious,
    });
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert!(h.platform.awake());
    h.platform.emit_power(PowerSnapshot {
        battery_percent: Some(90),
        on_ac: false,
        thermal: ThermalLevel::Critical,
    });
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert!(!h.platform.awake());
}

#[tokio::test(start_paused = true)]
async fn simulate_lid_publishes_a_lid_event() {
    let mut h = Harness::spawn();
    let mut sub = h
        .call(Call::Subscribe(SubscribeParams::default()))
        .await
        .subscription
        .unwrap();
    let out = h
        .call(Call::DebugSimulateLid(
            vigilia_protocol::SimulateLidParams {
                state: LidState::Closed,
            },
        ))
        .await;
    assert!(out.response.ok);
    let events = ready_events(&mut sub).await;
    assert!(
        events.iter().any(|e| matches!(
            e,
            Event::LidChanged {
                state: LidState::Closed
            }
        )),
        "{events:?}"
    );
    // And a real platform lid event takes the same path.
    h.platform.simulate_lid(LidState::Open);
    tokio::time::sleep(Duration::from_secs(1)).await;
    let events = ready_events(&mut sub).await;
    assert!(
        events.iter().any(|e| matches!(
            e,
            Event::LidChanged {
                state: LidState::Open
            }
        )),
        "{events:?}"
    );
}

#[tokio::test(start_paused = true)]
async fn crash_mid_lid_session_replays_sleep_and_backstop() {
    let dir = tempfile::tempdir().unwrap();
    let platform = FakePlatform::new();
    {
        let mut journal = Journal::open(common::journal_path(&dir)).unwrap();
        journal
            .apply(ActionKind::KeepAwake, None, || {
                Ok::<_, Infallible>(UndoRecord::LidClosedAwake {
                    pid: 999_999,
                    assertion_id: 42,
                    sleep_was_disabled: false,
                })
            })
            .unwrap();
        journal
            .apply(ActionKind::Backstop, None, || {
                Ok::<_, Infallible>(UndoRecord::Backstop {
                    label: "com.vigilia.backstop".into(),
                    plist: String::new(),
                })
            })
            .unwrap();
        // Dropped without undo: killed mid-session. The fake's sleep flag is set by hand
        // to stand in for the persisted pmset value.
    }
    let h = Harness::spawn_with(dir, platform.clone(), Config::default());
    tokio::task::yield_now().await;
    assert_eq!(platform.restored().len(), 2);
    assert!(
        Journal::open(h.journal_path())
            .unwrap()
            .pending()
            .is_empty()
    );
}
