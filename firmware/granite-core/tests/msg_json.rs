//! JSON fixtures for every command and every event, in both directions.
//! These are the wire contract; changing one means changing clients.

use granite_core::Target;
use granite_core::actuator::{ActionKind, ActionResult, ActuatorEvent, FailReason, FaultKind};
use granite_core::config::{NodesCfg, Section};
use granite_core::hal::{BootReason, Switch};
use granite_core::msg::{
    Args, Command, CommandKind, Event, EventKind, LWT_PAYLOAD, PROTOCOL_VERSION, Reply, State,
    Status, Topics, event_from_actuator, event_from_rule,
};
use granite_core::observed::{Observed, ProbeObs, Stamped};
use granite_core::rules::{Fired, Rearm, RuleAction};
use serde_json::{Value, json};

fn roundtrip_command(cmd: &Command, fixture: Value) {
    let encoded: Value = serde_json::from_str(&cmd.to_json().unwrap()).unwrap();
    assert_eq!(encoded, fixture, "serialising {cmd:?}");
    let decoded = Command::from_json(&fixture.to_string()).unwrap();
    assert_eq!(&decoded, cmd, "parsing {fixture}");
}

fn roundtrip_event(ev: &Event, fixture: Value) {
    let encoded: Value = serde_json::from_str(&ev.to_json().unwrap()).unwrap();
    assert_eq!(encoded, fixture, "serialising {ev:?}");
    let decoded: Event = serde_json::from_value(fixture.clone()).unwrap();
    assert_eq!(&decoded, ev, "parsing {fixture}");
}

#[test]
fn command_fixtures() {
    roundtrip_command(
        &Command::new("c-1", CommandKind::On).with_target(Target::Node(3)),
        json!({"v":1,"id":"c-1","action":"on","target":3}),
    );
    roundtrip_command(
        &Command::new("c-2", CommandKind::Off)
            .with_target(Target::Node(3))
            .with_args(Args {
                no_escalate: true,
                ..Args::default()
            }),
        json!({"v":1,"id":"c-2","action":"off","target":3,"args":{"no_escalate":true}}),
    );
    roundtrip_command(
        &Command::new("c-3", CommandKind::ForceOff).with_target(Target::Node(8)),
        json!({"v":1,"id":"c-3","action":"force_off","target":8}),
    );
    roundtrip_command(
        &Command::new("c-4", CommandKind::Reset)
            .with_target(Target::Node(1))
            .with_args(Args {
                force: true,
                ..Args::default()
            }),
        json!({"v":1,"id":"c-4","action":"reset","target":1,"args":{"force":true}}),
    );
    roundtrip_command(
        &Command::new("c-5", CommandKind::Cycle)
            .with_target(Target::Node(2))
            .with_args(Args {
                hard: true,
                ..Args::default()
            }),
        json!({"v":1,"id":"c-5","action":"cycle","target":2,"args":{"hard":true}}),
    );
    roundtrip_command(
        &Command::new("c-6", CommandKind::Press)
            .with_target(Target::Node(5))
            .with_args(Args {
                switch: Some(Switch::Rst),
                duration_ms: Some(1_500),
                ..Args::default()
            }),
        json!({"v":1,"id":"c-6","action":"press","target":5,
               "args":{"switch":"rst","duration_ms":1500}}),
    );
    roundtrip_command(
        &Command::new("c-7", CommandKind::OnAll),
        json!({"v":1,"id":"c-7","action":"on_all","target":"all"}),
    );
    roundtrip_command(
        &Command::new("c-8", CommandKind::RuleAck).with_args(Args {
            rule: Some(2),
            ..Args::default()
        }),
        json!({"v":1,"id":"c-8","action":"rule_ack","target":"all","args":{"rule":2}}),
    );
    roundtrip_command(
        &Command::new("c-9", CommandKind::ProbeScan),
        json!({"v":1,"id":"c-9","action":"probe_scan","target":"all"}),
    );
    roundtrip_command(
        &Command::new("c-10", CommandKind::ConfigGet).with_args(Args {
            section: Some(Section::Net),
            ..Args::default()
        }),
        json!({"v":1,"id":"c-10","action":"config_get","target":"all","args":{"section":"net"}}),
    );
    roundtrip_command(
        &Command::new("c-11", CommandKind::ConfigSet).with_args(Args {
            section: Some(Section::Sys),
            config: Some(json!({"log_level":"info"})),
            ..Args::default()
        }),
        json!({"v":1,"id":"c-11","action":"config_set","target":"all",
               "args":{"section":"sys","config":{"log_level":"info"}}}),
    );
    roundtrip_command(
        &Command::new("c-12", CommandKind::Ota).with_args(Args {
            url: Some("https://fw.example/granite-0.2.0.bin".into()),
            sha256: Some("a".repeat(64)),
            ..Args::default()
        }),
        json!({"v":1,"id":"c-12","action":"ota","target":"all",
               "args":{"url":"https://fw.example/granite-0.2.0.bin","sha256":"a".repeat(64)}}),
    );
    roundtrip_command(
        &Command::new("c-13", CommandKind::Reboot),
        json!({"v":1,"id":"c-13","action":"reboot","target":"all"}),
    );
    roundtrip_command(
        &Command::new("c-14", CommandKind::FactoryReset).with_args(Args {
            confirm: Some("granite-aabbcc".into()),
            ..Args::default()
        }),
        json!({"v":1,"id":"c-14","action":"factory_reset","target":"all",
               "args":{"confirm":"granite-aabbcc"}}),
    );
}

#[test]
fn every_command_kind_has_a_fixture() {
    // Keeps the fixture list above honest when a kind is added.
    let kinds = [
        CommandKind::On,
        CommandKind::Off,
        CommandKind::ForceOff,
        CommandKind::Reset,
        CommandKind::Cycle,
        CommandKind::Press,
        CommandKind::OnAll,
        CommandKind::RuleAck,
        CommandKind::ProbeScan,
        CommandKind::ConfigGet,
        CommandKind::ConfigSet,
        CommandKind::Ota,
        CommandKind::Reboot,
        CommandKind::FactoryReset,
    ];
    assert_eq!(kinds.len(), 14);
    for k in kinds {
        let json = serde_json::to_string(&k).unwrap();
        assert_eq!(json, format!("\"{}\"", k.as_str()));
        let back: CommandKind = serde_json::from_str(&json).unwrap();
        assert_eq!(back, k);
    }
    assert!(CommandKind::On.is_actuator());
    assert!(!CommandKind::Reboot.is_actuator());
    assert_eq!(
        CommandKind::of_action(ActionKind::Cycle),
        CommandKind::Cycle
    );
}

#[test]
fn a_command_with_a_signature_keeps_it() {
    let json = r#"{"v":1,"id":"c-1","action":"on","target":3,"sig":"MEUCIQ.."}"#;
    let cmd = Command::from_json(json).unwrap();
    assert_eq!(cmd.sig.as_deref(), Some("MEUCIQ.."));
    assert!(cmd.to_json().unwrap().contains("\"sig\":\"MEUCIQ..\""));
}

#[test]
fn a_command_without_v_defaults_to_the_current_version() {
    let cmd = Command::from_json(r#"{"id":"c","action":"reboot"}"#).unwrap();
    assert_eq!(cmd.v, PROTOCOL_VERSION);
    assert_eq!(cmd.target, Target::All);
    assert!(cmd.args.is_empty());
}

#[test]
fn a_bad_target_is_rejected_at_parse_time() {
    assert!(Command::from_json(r#"{"id":"c","action":"on","target":9}"#).is_err());
    assert!(Command::from_json(r#"{"id":"c","action":"on","target":0}"#).is_err());
    assert!(Command::from_json(r#"{"id":"c","action":"on","target":"both"}"#).is_err());
    // The ADR's two spellings both work, and "3" as a string too.
    assert_eq!(
        Command::from_json(r#"{"id":"c","action":"on","target":"all"}"#)
            .unwrap()
            .target,
        Target::All
    );
    assert_eq!(
        Command::from_json(r#"{"id":"c","action":"on","target":"3"}"#)
            .unwrap()
            .target,
        Target::Node(3)
    );
}

#[test]
fn an_unknown_action_is_rejected() {
    assert!(Command::from_json(r#"{"id":"c","action":"self_destruct"}"#).is_err());
}

#[test]
fn reply_fixtures() {
    let accepted = Reply::accepted("c-1");
    assert_eq!(
        serde_json::from_str::<Value>(&accepted.to_json().unwrap()).unwrap(),
        json!({"v":1,"id":"c-1","ok":true,"result":"accepted","error":null})
    );

    let failed = Reply::of_result("c-2", &ActionResult::failed(FailReason::PowerOnTimeout));
    assert_eq!(
        serde_json::from_str::<Value>(&failed.to_json().unwrap()).unwrap(),
        json!({"v":1,"id":"c-2","ok":false,"result":null,
               "error":"failed: power_on_timeout"})
    );

    let data = Reply::ok("c-3", "ok").with_data(json!({"log_level":"warn"}));
    assert_eq!(
        serde_json::from_str::<Value>(&data.to_json().unwrap()).unwrap(),
        json!({"v":1,"id":"c-3","ok":true,"result":"ok","error":null,
               "data":{"log_level":"warn"}})
    );
}

#[test]
fn event_fixtures() {
    roundtrip_event(
        &Event::new(
            1_000,
            EventKind::NodeState {
                node: 3,
                state: "on".into(),
                led: Some(true),
            },
        ),
        json!({"v":1,"ts":1000,"kind":"node_state","node":3,"state":"on","led":true}),
    );
    roundtrip_event(
        &Event::new(
            1_100,
            EventKind::Accepted {
                id: "c-1".into(),
                action: ActionKind::OnAll,
                target: Target::All,
            },
        ),
        json!({"v":1,"ts":1100,"kind":"accepted","id":"c-1","action":"on_all","target":"all"}),
    );
    roundtrip_event(
        &Event::new(
            1_200,
            EventKind::ActionDone {
                id: "c-1".into(),
                node: Some(3),
                action: ActionKind::On,
                result: "ok".into(),
            },
        ),
        json!({"v":1,"ts":1200,"kind":"action_done","id":"c-1","node":3,
               "action":"on","result":"ok"}),
    );
    roundtrip_event(
        &Event::new(
            1_300,
            EventKind::ActionFailed {
                id: "c-2".into(),
                node: Some(4),
                action: ActionKind::ForceOff,
                error: "failed: still_on".into(),
            },
        ),
        json!({"v":1,"ts":1300,"kind":"action_failed","id":"c-2","node":4,
               "action":"force_off","error":"failed: still_on"}),
    );
    roundtrip_event(
        &Event::new(
            1_400,
            EventKind::SoftOffTimeout {
                id: "c-3".into(),
                node: 5,
            },
        ),
        json!({"v":1,"ts":1400,"kind":"soft_off_timeout","id":"c-3","node":5}),
    );
    roundtrip_event(
        &Event::new(
            1_500,
            EventKind::RuleFired {
                rule: 1,
                name: "probe over temperature".into(),
                action: "force_off".into(),
                target: Target::All,
                value: 7_123,
                needs_ack: false,
            },
        ),
        json!({"v":1,"ts":1500,"kind":"rule_fired","rule":1,
               "name":"probe over temperature","action":"force_off","target":"all",
               "value":7123,"needs_ack":false}),
    );
    roundtrip_event(
        &Event::new(
            1_600,
            EventKind::Fault {
                fault: "press_deadline".into(),
                node: Some(2),
                id: Some("c-4".into()),
            },
        ),
        json!({"v":1,"ts":1600,"kind":"fault","fault":"press_deadline",
               "node":2,"id":"c-4"}),
    );
    roundtrip_event(
        &Event::new(
            1_700,
            EventKind::Ota {
                phase: "downloading".into(),
                progress: Some(42),
                detail: None,
            },
        ),
        json!({"v":1,"ts":1700,"kind":"ota","phase":"downloading",
               "progress":42,"detail":null}),
    );
    roundtrip_event(
        &Event::new(
            1_800,
            EventKind::Config {
                section: Some(Section::Nodes),
                change: "fallback".into(),
                detail: Some("parse error".into()),
            },
        ),
        json!({"v":1,"ts":1800,"kind":"config","section":"nodes",
               "change":"fallback","detail":"parse error"}),
    );
}

#[test]
fn actuator_events_become_published_events() {
    let done = ActuatorEvent::Done {
        id: "c-1".into(),
        node: 2,
        action: ActionKind::Off,
        result: ActionResult::ShutdownPending,
    };
    let ev = event_from_actuator(&done, 900).unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&ev.to_json().unwrap()).unwrap(),
        json!({"v":1,"ts":900,"kind":"action_done","id":"c-1","node":2,
               "action":"off","result":"shutdown_pending"})
    );

    let failed = ActuatorEvent::Done {
        id: "c-2".into(),
        node: 2,
        action: ActionKind::On,
        result: ActionResult::failed(FailReason::DeadlineExceeded),
    };
    assert!(matches!(
        event_from_actuator(&failed, 1).unwrap().kind,
        EventKind::ActionFailed { .. }
    ));

    let fault = ActuatorEvent::Fault {
        id: None,
        node: None,
        fault: FaultKind::Expander,
    };
    assert!(matches!(
        event_from_actuator(&fault, 1).unwrap().kind,
        EventKind::Fault { .. }
    ));

    // Press bookkeeping is log-only.
    assert!(
        event_from_actuator(
            &ActuatorEvent::PressStart {
                id: "c".into(),
                node: 1,
                switch: Switch::Pwr,
                duration_ms: 250,
                deadline_ms: 750,
            },
            1
        )
        .is_none()
    );
}

#[test]
fn fired_rules_become_published_events() {
    let fired = Fired {
        rule_id: 2,
        name: "leak float closed".into(),
        action: RuleAction::Act {
            kind: ActionKind::ForceOff,
        },
        target: Target::All,
        value: 1,
        rearm: Rearm::Manual,
    };
    let ev = event_from_rule(&fired, 2_000);
    assert_eq!(
        serde_json::from_str::<Value>(&ev.to_json().unwrap()).unwrap(),
        json!({"v":1,"ts":2000,"kind":"rule_fired","rule":2,
               "name":"leak float closed","action":"force_off","target":"all",
               "value":1,"needs_ack":true})
    );
}

#[test]
fn status_and_lwt_fixtures() {
    let s = Status::online("0.1.0", "192.168.1.5", 4_242, BootReason::PowerOn);
    assert_eq!(
        serde_json::from_str::<Value>(&s.to_json().unwrap()).unwrap(),
        json!({"v":1,"online":true,"fw":"0.1.0","ip":"192.168.1.5",
               "uptime_s":4242,"boot_reason":"power_on"})
    );
    assert_eq!(
        serde_json::from_str::<Value>(LWT_PAYLOAD).unwrap(),
        json!({"v":1,"online":false})
    );
}

#[test]
fn state_payload_shape() {
    let mut o = Observed::new();
    o.nodes[0].led = Some(true);
    o.nodes[0].state = granite_core::node::NodeState::On;
    o.nodes[1].led = Some(false);
    o.nodes[1].state = granite_core::node::NodeState::Off;
    o.probes.push(ProbeObs {
        rom: 0x28_1234_5678_9abc,
        name: "inlet".into(),
        centi_c: Some(2_537),
        ts_ms: 10,
    });
    o.probes.push(ProbeObs {
        rom: 0x28_0000_0000_0002,
        name: "outlet".into(),
        centi_c: None,
        ts_ms: 10,
    });
    o.board_temp = Stamped::new(Some(3_100), 10);
    o.vin_mv = Stamped::new(Some(19_120), 10);
    o.dry_in = Stamped::new(Some(0b0101), 10);
    o.link_up = Stamped::new(true, 10);
    o.mqtt_connected = Stamped::new(true, 10);
    o.uptime_s = Stamped::new(99, 10);

    let state = State::from_observed(&o, &NodesCfg::default(), 1_234);
    let v: Value = serde_json::from_str(&state.to_json().unwrap()).unwrap();

    assert_eq!(v["v"], json!(1));
    assert_eq!(v["ts"], json!(1234));
    assert_eq!(v["nodes"].as_array().unwrap().len(), 8);
    assert_eq!(v["nodes"][0]["node"], json!(1));
    assert_eq!(v["nodes"][0]["name"], json!("node1"));
    assert_eq!(v["nodes"][0]["state"], json!("on"));
    assert_eq!(v["nodes"][0]["led"], json!(true));
    assert_eq!(v["nodes"][1]["state"], json!("off"));
    assert_eq!(v["nodes"][7]["state"], json!("unknown"));
    assert_eq!(v["probes"][0]["slot"], json!(1));
    assert_eq!(v["probes"][0]["rom"], json!("0028123456789abc"));
    assert_eq!(v["probes"][0]["temp_c"], json!(25.37));
    assert_eq!(v["probes"][1]["temp_c"], Value::Null);
    assert_eq!(v["board_temp_c"], json!(31.0));
    assert_eq!(v["vin_v"], json!(19.12));
    assert_eq!(v["dry_in"], json!([true, false, true, false]));
    assert_eq!(v["link_up"], json!(true));
    assert_eq!(v["mqtt_connected"], json!(true));
    assert_eq!(v["uptime_s"], json!(99));
}

#[test]
fn topic_tree() {
    let t = Topics::new("granite/default/granite-aabbcc");
    assert_eq!(t.status(), "granite/default/granite-aabbcc/status");
    assert_eq!(t.state(), "granite/default/granite-aabbcc/state");
    assert_eq!(t.event(), "granite/default/granite-aabbcc/event");
    assert_eq!(t.log(), "granite/default/granite-aabbcc/log");
    assert_eq!(t.cmd(), "granite/default/granite-aabbcc/cmd");
    assert_eq!(t.ack("c-1"), "granite/default/granite-aabbcc/ack/c-1");
}
