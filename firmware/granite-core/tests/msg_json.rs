//! JSON fixtures for every command and every event, in both directions.
//! These are the wire contract; changing one means changing clients.

use granite_core::Target;
use granite_core::config::{NodesCfg, Section};
use granite_core::hal::{BootReason, Switch};
use granite_core::jobs::{JobLog, SubmittedJob};
use granite_core::msg::{
    Args, Command, CommandKind, Event, EventKind, LWT_PAYLOAD, PROTOCOL_VERSION, Reply, State,
    Status, Topics, event_from_client, event_from_rule,
};
use granite_core::node::NodeSettings;
use granite_core::observed::{NodeObs, Observed, PlaneObs, ProbeObs, Stamped, UnitObs};
use granite_core::plane::event::ClientEvent;
use granite_core::plane::model::{HostState, PowerState};
use granite_core::plane::{Command as PlaneJob, FinishedJob, JobOutcome, NodeCommand, Plane};
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
        CommandKind::PowerOn,
        CommandKind::PowerOff,
        CommandKind::ClearLatch,
        CommandKind::UnitReset,
        CommandKind::PlaneReboot,
        CommandKind::RuleAck,
        CommandKind::ProbeScan,
        CommandKind::ConfigGet,
        CommandKind::ConfigSet,
        CommandKind::Ota,
        CommandKind::Reboot,
        CommandKind::FactoryReset,
    ];
    assert_eq!(kinds.len(), 19);
    for k in kinds {
        let json = serde_json::to_string(&k).unwrap();
        assert_eq!(json, format!("\"{}\"", k.as_str()));
        let back: CommandKind = serde_json::from_str(&json).unwrap();
        assert_eq!(back, k);
        assert_eq!(CommandKind::from_str_opt(k.as_str()), Some(k));
    }
    // The twelve verbs that become plane jobs (ADR 0004 decision 4).
    assert_eq!(CommandKind::PLANE_VERBS.len(), 12);
    assert!(CommandKind::On.is_plane_verb() && CommandKind::On.is_node_verb());
    assert!(CommandKind::UnitReset.is_plane_verb());
    assert!(!CommandKind::UnitReset.is_node_verb());
    assert!(!CommandKind::Reboot.is_plane_verb());
    assert_eq!(CommandKind::from_str_opt("nonsense"), None);
}

#[test]
fn the_new_targets_round_trip() {
    for (text, target) in [
        ("all", Target::All),
        ("plane2", Target::Plane(2)),
        ("plane2.unit3", Target::Unit { plane: 2, unit: 3 }),
    ] {
        let cmd = Command::new("c-t", CommandKind::On).with_target(target);
        let v: Value = serde_json::from_str(&cmd.to_json().unwrap()).unwrap();
        assert_eq!(v["target"], json!(text));
        assert_eq!(Command::from_json(&v.to_string()).unwrap().target, target);
    }
    let node = Command::new("c-t", CommandKind::On).with_target(Target::Node(40));
    let v: Value = serde_json::from_str(&node.to_json().unwrap()).unwrap();
    assert_eq!(v["target"], json!(40), "a node id stays a number");
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
    assert!(Command::from_json(r#"{"id":"c","action":"on","target":65}"#).is_err());
    assert!(Command::from_json(r#"{"id":"c","action":"on","target":0}"#).is_err());
    assert!(Command::from_json(r#"{"id":"c","action":"on","target":"plane8"}"#).is_err());
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

    let queued = Reply::accepted("c-9").with_job(12);
    assert_eq!(
        serde_json::from_str::<Value>(&queued.to_json().unwrap()).unwrap(),
        json!({"v":1,"id":"c-9","ok":true,"result":"accepted","error":null,"job":12})
    );

    let failed = Reply::of_outcome("c-2", JobOutcome::Failed, Some("not_present"));
    assert_eq!(
        serde_json::from_str::<Value>(&failed.to_json().unwrap()).unwrap(),
        json!({"v":1,"id":"c-2","ok":false,"result":null,
               "error":"failed: not_present"})
    );
    let absent = Reply::of_outcome("c-3", JobOutcome::Absent, None);
    assert_eq!(absent.error.as_deref(), Some("absent"));
    let done = Reply::of_outcome("c-4", JobOutcome::Done, None);
    assert_eq!(done.result.as_deref(), Some("done"));

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
                power: "on".into(),
                host: "sleep".into(),
            },
        ),
        json!({"v":1,"ts":1000,"kind":"node_state","node":3,"power":"on","host":"sleep"}),
    );
    roundtrip_event(
        &Event::new(
            1_100,
            EventKind::Accepted {
                id: "c-1".into(),
                action: CommandKind::OnAll,
                target: Target::All,
                job: 3,
            },
        ),
        json!({"v":1,"ts":1100,"kind":"accepted","id":"c-1","action":"on_all",
               "target":"all","job":3}),
    );
    roundtrip_event(
        &Event::new(
            1_200,
            EventKind::ActionDone {
                id: "c-1".into(),
                node: Some(3),
                action: CommandKind::On,
                result: "done".into(),
            },
        ),
        json!({"v":1,"ts":1200,"kind":"action_done","id":"c-1","node":3,
               "action":"on","result":"done"}),
    );
    roundtrip_event(
        &Event::new(
            1_300,
            EventKind::ActionFailed {
                id: "c-2".into(),
                node: Some(4),
                action: CommandKind::ForceOff,
                error: "failed: failed".into(),
            },
        ),
        json!({"v":1,"ts":1300,"kind":"action_failed","id":"c-2","node":4,
               "action":"force_off","error":"failed: failed"}),
    );
    roundtrip_event(
        &Event::new(
            1_400,
            EventKind::Plane {
                what: "plane_lost".into(),
                address: Some(0x31),
                node: None,
                unit: None,
                detail: None,
            },
        ),
        json!({"v":1,"ts":1400,"kind":"plane","what":"plane_lost","address":49}),
    );
    roundtrip_event(
        &Event::new(
            1_500,
            EventKind::RuleFired {
                rule: 1,
                name: "probe over temperature".into(),
                action: "force_off".into(),
                target: Target::All,
                value: Some(7_123),
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
fn plane_client_events_become_published_events() {
    let found = ClientEvent::PlaneFound {
        address: 0x30,
        firmware: granite_proto::Version::new(0, 2, 1),
        boot_count: 4,
    };
    let ev = event_from_client(&found, 900).expect("a published event");
    assert_eq!(
        serde_json::from_str::<Value>(&ev.to_json().unwrap()).unwrap(),
        json!({"v":1,"ts":900,"kind":"plane","what":"plane_found","address":48,
               "detail":"firmware 0.2.1, boot 4"})
    );

    let slot = ClientEvent::Node {
        node: 3,
        address: 0x30,
        slot: 2,
        kind: granite_proto::EventKind::OvLatched,
        uptime_s: 12,
    };
    let ev = event_from_client(&slot, 1).expect("a published event");
    assert_eq!(
        serde_json::from_str::<Value>(&ev.to_json().unwrap()).unwrap(),
        json!({"v":1,"ts":1,"kind":"plane","what":"ov_latched","address":48,
               "node":3,"detail":"ov_latched"})
    );

    let unit = ClientEvent::Unit {
        address: 0x31,
        unit: 2,
        kind: granite_proto::EventKind::UnitHotSwapLatchedOff,
        uptime_s: 12,
    };
    let ev = event_from_client(&unit, 2).expect("a published event");
    let v: Value = serde_json::from_str(&ev.to_json().unwrap()).unwrap();
    assert_eq!(v["unit"], json!(2));
    assert_eq!(v["what"], json!("unit_hot_swap_latched_off"));

    // A finished job is answered by the ack and by action_done, not by a
    // plane event.
    assert!(
        event_from_client(
            &ClientEvent::JobFinished {
                id: 1,
                outcome: JobOutcome::Done,
                code: None,
            },
            1
        )
        .is_none()
    );
}

#[test]
fn a_finished_job_becomes_an_ack_and_an_event() {
    let mut log = JobLog::new();
    log.note(
        "c-7",
        CommandKind::ForceOff,
        Target::Node(2),
        &[SubmittedJob::new(5, vec![2])],
        100,
    );
    let finished = FinishedJob {
        id: 5,
        command: PlaneJob::Node(NodeCommand::force_off(2)),
        outcome: JobOutcome::Done,
        code: Some(granite_proto::ResultCode::Ok),
        submitted_ms: 100,
        finished_ms: 900,
    };
    let done = log.finish(&finished, 900);
    let reply = done.reply.expect("the command is finished");
    let event = done.event.expect("and it has an event");
    assert_eq!(
        serde_json::from_str::<Value>(&reply.to_json().unwrap()).unwrap(),
        json!({"v":1,"id":"c-7","ok":true,"result":"done","error":null,"job":5})
    );
    assert_eq!(
        serde_json::from_str::<Value>(&Event::new(900, event).to_json().unwrap()).unwrap(),
        json!({"v":1,"ts":900,"kind":"action_done","id":"c-7","node":2,
               "action":"force_off","result":"done"})
    );
}

#[test]
fn fired_rules_become_published_events() {
    let fired = Fired {
        rule_id: 2,
        name: "leak float closed".into(),
        action: RuleAction::Act {
            kind: CommandKind::ForceOff,
        },
        target: Target::All,
        value: Some(1),
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
    let mut plane = PlaneObs::of_plane(&Plane::new(0x30));
    plane.plane_type = "pp48_chained".into();
    plane.firmware = "0.2.1".into();
    plane.bus_volts = 1_204;
    plane.core_temperature = 352;
    plane.boot_count = 3;
    plane.uptime_s = 77;
    plane.image_state = "confirmed".into();
    plane.units = vec![UnitObs {
        unit: 0,
        present: true,
        volts: 1_200,
        amps: 4_149,
        watts: 4_980,
        temperature: 301,
        ..UnitObs::default()
    }];
    o.planes.push(plane);

    let (address, slot) = granite_core::plane::node_location(1).unwrap();
    let mut node = NodeObs::unknown(1, address, slot);
    node.present = true;
    node.power = PowerState::On;
    node.host = HostState::On;
    node.telemetry.volts = 4_804;
    node.telemetry.amps = 1_049;
    node.telemetry.watts = 504;
    node.telemetry.watts_min = 120;
    node.telemetry.watts_max = 910;
    node.telemetry.temperature = 314;
    node.telemetry.energy_joules = 7_300;
    node.telemetry.module_class = 2;
    o.nodes.push(node);

    let (address, slot) = granite_core::plane::node_location(2).unwrap();
    let mut off = NodeObs::unknown(2, address, slot);
    off.present = true;
    off.power = PowerState::Off;
    off.faults.ov_latched = true;
    o.nodes.push(off);

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
    o.vin_mv = Stamped::new(Some(12_120), 10);
    o.dry_in = Stamped::new(Some(0b0101), 10);
    o.link_up = Stamped::new(true, 10);
    o.mqtt_connected = Stamped::new(true, 10);
    o.uptime_s = Stamped::new(99, 10);

    let mut cfg = NodesCfg::default();
    cfg.nodes.push(NodeSettings {
        node: 1,
        name: "head".into(),
        ..NodeSettings::for_node(1)
    });

    let state = State::from_observed(&o, &cfg, 1_234);
    let v: Value = serde_json::from_str(&state.to_json().unwrap()).unwrap();

    assert_eq!(v["v"], json!(1));
    assert_eq!(v["ts"], json!(1234));
    // Only present nodes are listed (ADR 0004 decision 2).
    assert_eq!(v["nodes"].as_array().unwrap().len(), 2);
    assert_eq!(v["nodes"][0]["node"], json!(1));
    assert_eq!(v["nodes"][0]["name"], json!("head"));
    assert_eq!(v["nodes"][0]["plane"], json!(0));
    assert_eq!(v["nodes"][0]["slot"], json!(0));
    assert_eq!(v["nodes"][0]["unit"], json!(0));
    assert_eq!(v["nodes"][0]["power"], json!("on"));
    assert_eq!(v["nodes"][0]["host"], json!("on"));
    // Rounded telemetry: 48.0 V, 1.0 A, 50 W, 31 C, 2 Wh.
    assert_eq!(v["nodes"][0]["v"], json!(48.0));
    assert_eq!(v["nodes"][0]["a"], json!(1.0));
    assert_eq!(v["nodes"][0]["w"], json!(50));
    assert_eq!(v["nodes"][0]["w_min"], json!(12));
    assert_eq!(v["nodes"][0]["w_max"], json!(91));
    assert_eq!(v["nodes"][0]["temp_c"], json!(31));
    assert_eq!(v["nodes"][0]["energy_wh"], json!(2));
    assert_eq!(v["nodes"][0]["module_class"], json!(2));
    assert_eq!(v["nodes"][1]["name"], json!("node2"));
    assert_eq!(v["nodes"][1]["power"], json!("off"));
    assert_eq!(v["nodes"][1]["host"], json!("unknown"));
    assert_eq!(v["nodes"][1]["faults"], json!(["ov_latched"]));

    assert_eq!(v["planes"].as_array().unwrap().len(), 1);
    assert_eq!(v["planes"][0]["plane"], json!(0));
    assert_eq!(v["planes"][0]["address"], json!(48));
    assert_eq!(v["planes"][0]["type"], json!("pp48_chained"));
    assert_eq!(v["planes"][0]["firmware"], json!("0.2.1"));
    assert_eq!(v["planes"][0]["bus_v"], json!(12.0));
    assert_eq!(v["planes"][0]["temp_c"], json!(35));
    assert_eq!(v["planes"][0]["boot_count"], json!(3));
    assert_eq!(v["planes"][0]["units"][0]["present"], json!(true));
    assert_eq!(v["planes"][0]["units"][0]["v"], json!(12.0));
    assert_eq!(v["planes"][0]["units"][0]["a"], json!(4.1));
    assert_eq!(v["planes"][0]["units"][0]["w"], json!(498));

    assert_eq!(v["probes"][0]["slot"], json!(1));
    assert_eq!(v["probes"][0]["rom"], json!("0028123456789abc"));
    assert_eq!(v["probes"][0]["temp_c"], json!(25.37));
    assert_eq!(v["probes"][1]["temp_c"], Value::Null);
    assert_eq!(v["board_temp_c"], json!(31.0));
    assert_eq!(v["vin_v"], json!(12.12));
    assert_eq!(v["dry_in"], json!([true, false, true, false]));
    assert_eq!(v["link_up"], json!(true));
    assert_eq!(v["mqtt_connected"], json!(true));
    assert_eq!(v["uptime_s"], json!(99));
}

#[test]
fn rounded_telemetry_does_not_move_on_noise() {
    let mut o = Observed::new();
    let (address, slot) = granite_core::plane::node_location(1).unwrap();
    let mut node = NodeObs::unknown(1, address, slot);
    node.present = true;
    node.power = PowerState::On;
    node.telemetry.volts = 4_800;
    node.telemetry.amps = 1_000;
    node.telemetry.watts = 480;
    node.telemetry.energy_joules = 7_200;
    o.nodes.push(node);
    let before = State::from_observed(&o, &NodesCfg::default(), 0)
        .to_json()
        .unwrap();

    // A few millivolts, a few milliamps, a tenth of a watt and a minute
    // of energy at 50 W: nothing a dashboard needs to hear about.
    let node = o.node_mut(1).unwrap();
    node.telemetry.volts = 4_803;
    node.telemetry.amps = 1_004;
    node.telemetry.watts = 483;
    node.telemetry.energy_joules = 9_000;
    let after = State::from_observed(&o, &NodesCfg::default(), 0)
        .to_json()
        .unwrap();
    assert_eq!(before, after, "an on-change publisher stays quiet");

    // A real change does show up.
    let node = o.node_mut(1).unwrap();
    node.telemetry.watts = 1_480;
    let changed = State::from_observed(&o, &NodesCfg::default(), 0)
        .to_json()
        .unwrap();
    assert_ne!(before, changed);
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
