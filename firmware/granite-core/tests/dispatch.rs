//! The shared dispatcher: the same command vocabulary for MQTT, HTTP and
//! Modbus, including the refusal rules and the side effects the firmware
//! has to carry out.

mod common;

use common::Rig;
use granite_core::Target;
use granite_core::actuator::{ActuatorEvent, Timings};
use granite_core::config::{Config, IpMode, LogLevel, Section};
use granite_core::dispatch::{DispatchCtx, SideEffect, dispatch};
use granite_core::modbus_map::{cmd_word, command_from_holding};
use granite_core::msg::{Args, Command, CommandKind};
use granite_core::rules::{RuleEngine, default_rules};
use serde_json::json;

const DEVICE: &str = "granite-aabbcc";

struct Harness {
    rig: Rig,
    config: Config,
    rules: RuleEngine,
}

impl Harness {
    fn new() -> Self {
        let mut rig = Rig::new();
        rig.set_timings(Timings {
            t_on_ms: 1_000,
            t_soft_off_ms: 5_000,
            t_hold_ms: 4_000,
            t_cycle_ms: 1_000,
            t_stagger_ms: 500,
            ..Timings::default()
        });
        let mut rules = RuleEngine::with_rules(default_rules());
        rules.set_rules(
            default_rules()
                .into_iter()
                .map(|mut r| {
                    r.enabled = true;
                    r
                })
                .collect(),
        );
        Harness {
            rig,
            config: Config::default(),
            rules,
        }
    }

    fn run(&mut self, cmd: &Command) -> granite_core::dispatch::Dispatched {
        let now = self.rig.now;
        let obs = self.rig.obs.clone();
        let mut ctx = DispatchCtx {
            now_ms: now,
            device_id: DEVICE,
            observed: &obs,
            actuator: &mut self.rig.act,
            rules: &mut self.rules,
            config: &mut self.config,
            verify_sig: None,
        };
        let out = dispatch(cmd, &mut ctx);
        self.rig.events.extend(out.events.clone());
        out
    }
}

#[test]
fn an_action_is_queued_and_acked_as_accepted() {
    let mut h = Harness::new();
    h.rig.set_led(3, Some(false));
    h.rig.set_respond(3, common::Respond::OnAfter(100));

    let cmd = Command::new("c-1", CommandKind::On).with_target(Target::Node(3));
    let out = h.run(&cmd);
    assert!(out.reply.ok);
    assert_eq!(out.reply.result.as_deref(), Some("accepted"));
    assert!(out.effects.is_empty());
    assert!(
        out.events
            .iter()
            .any(|e| matches!(e, ActuatorEvent::Accepted { node: 3, .. }))
    );

    h.rig.run_to(5_000);
    assert_eq!(
        h.rig.result_of(3),
        Some(granite_core::actuator::ActionResult::Ok)
    );
}

#[test]
fn an_action_that_is_already_true_acks_ok_immediately() {
    let mut h = Harness::new();
    h.rig.set_led(3, Some(true));
    let out = h.run(&Command::new("c-1", CommandKind::On).with_target(Target::Node(3)));
    assert!(out.reply.ok);
    assert_eq!(out.reply.result.as_deref(), Some("ok"));
}

#[test]
fn an_unknown_node_is_refused_through_every_transport() {
    let mut h = Harness::new();
    h.rig.set_led(3, None);

    let out = h.run(&Command::new("c-1", CommandKind::On).with_target(Target::Node(3)));
    assert!(!out.reply.ok);
    assert_eq!(
        out.reply.error.as_deref(),
        Some("refused: unknown_state"),
        "MQTT and HTTP"
    );

    // The same write arriving over Modbus goes through the same path.
    let modbus = command_from_holding(2, cmd_word::ON, "modbus-7")
        .unwrap()
        .unwrap();
    let out = h.run(&modbus);
    assert!(!out.reply.ok);
    assert_eq!(out.reply.error.as_deref(), Some("refused: unknown_state"));
    assert_eq!(out.reply.id, "modbus-7");

    // With force it is accepted.
    let forced = Command::new("c-2", CommandKind::On)
        .with_target(Target::Node(3))
        .with_args(Args {
            force: true,
            ..Args::default()
        });
    assert!(h.run(&forced).reply.ok);
}

#[test]
fn rule_ack_rearms_a_latched_rule() {
    let mut h = Harness::new();
    assert_eq!(h.rules.is_armed(2), Some(true));

    // Latch rule 2 by firing it.
    let mut wet = granite_core::observed::Observed::new();
    wet.dry_in = granite_core::observed::Stamped::new(Some(0b0001), 1);
    h.rules.tick(0, &wet);
    assert_eq!(h.rules.tick(2_000, &wet).len(), 1);
    assert_eq!(h.rules.is_armed(2), Some(false));

    let cmd = Command::new("c-1", CommandKind::RuleAck).with_args(Args {
        rule: Some(2),
        ..Args::default()
    });
    let out = h.run(&cmd);
    assert!(out.reply.ok);
    assert_eq!(h.rules.is_armed(2), Some(true));

    let bad = Command::new("c-2", CommandKind::RuleAck).with_args(Args {
        rule: Some(99),
        ..Args::default()
    });
    assert!(!h.run(&bad).reply.ok);

    let missing = Command::new("c-3", CommandKind::RuleAck);
    assert_eq!(
        h.run(&missing).reply.error.as_deref(),
        Some("args.rule is required")
    );
}

#[test]
fn config_get_returns_one_section_or_the_whole_document() {
    let mut h = Harness::new();
    let whole = h.run(&Command::new("c-1", CommandKind::ConfigGet));
    let data = whole.reply.data.unwrap();
    assert_eq!(data["schema_version"], json!(1));
    assert_eq!(data["net"]["ip_mode"], json!("dhcp"));

    let one = h.run(
        &Command::new("c-2", CommandKind::ConfigGet).with_args(Args {
            section: Some(Section::Sys),
            ..Args::default()
        }),
    );
    let data = one.reply.data.unwrap();
    assert_eq!(data["log_level"], json!("warn"));
    assert!(data.get("net").is_none());
}

#[test]
fn config_set_on_a_safe_section_is_saved_directly() {
    let mut h = Harness::new();
    let cmd = Command::new("c-1", CommandKind::ConfigSet).with_args(Args {
        section: Some(Section::Sys),
        config: Some(json!({"log_level":"info","device_id":"granite-aabbcc"})),
        ..Args::default()
    });
    let out = h.run(&cmd);
    assert!(out.reply.ok);
    assert_eq!(out.reply.result.as_deref(), Some("saved"));
    assert_eq!(out.effects, vec![SideEffect::SaveSection(Section::Sys)]);
    assert_eq!(h.config.sys.log_level, LogLevel::Info);
}

#[test]
fn config_set_on_a_reachability_section_is_staged() {
    let mut h = Harness::new();
    let cmd = Command::new("c-1", CommandKind::ConfigSet).with_args(Args {
        section: Some(Section::Net),
        config: Some(json!({"ip_mode":"static","address":"10.0.0.9/24"})),
        ..Args::default()
    });
    let out = h.run(&cmd);
    assert_eq!(out.reply.result.as_deref(), Some("staged"));
    assert_eq!(out.effects, vec![SideEffect::StageSection(Section::Net)]);
    assert_eq!(h.config.net.ip_mode, IpMode::Static);
}

#[test]
fn an_invalid_config_set_changes_nothing() {
    let mut h = Harness::new();
    let before = h.config.clone();
    let cmd = Command::new("c-1", CommandKind::ConfigSet).with_args(Args {
        section: Some(Section::Net),
        // static without an address
        config: Some(json!({"ip_mode":"static"})),
        ..Args::default()
    });
    let out = h.run(&cmd);
    assert!(!out.reply.ok);
    assert!(out.reply.error.unwrap().contains("net.address"));
    assert!(out.effects.is_empty());
    assert_eq!(h.config, before);

    let cmd = Command::new("c-2", CommandKind::ConfigSet).with_args(Args {
        section: Some(Section::Net),
        config: Some(json!({"ip_mode":"carrier-pigeon"})),
        ..Args::default()
    });
    assert!(!h.run(&cmd).reply.ok);
    assert_eq!(h.config, before);

    let cmd = Command::new("c-3", CommandKind::ConfigSet);
    assert_eq!(
        h.run(&cmd).reply.error.as_deref(),
        Some("args.config is required")
    );
}

#[test]
fn a_whole_document_set_reports_only_the_changed_sections() {
    let mut h = Harness::new();
    let mut next = Config::default();
    next.sys.log_level = LogLevel::Debug;
    next.nodes.nodes[0].name = "head".into();
    let doc: serde_json::Value = serde_json::from_str(&next.export_json().unwrap()).unwrap();

    let cmd = Command::new("c-1", CommandKind::ConfigSet).with_args(Args {
        config: Some(doc),
        ..Args::default()
    });
    let out = h.run(&cmd);
    assert!(out.reply.ok);
    assert_eq!(
        out.effects,
        vec![
            SideEffect::SaveSection(Section::Nodes),
            SideEffect::SaveSection(Section::Sys)
        ]
    );
}

#[test]
fn probe_scan_ota_reboot_and_factory_reset_come_back_as_effects() {
    let mut h = Harness::new();

    let out = h.run(&Command::new("c-1", CommandKind::ProbeScan));
    assert_eq!(out.effects, vec![SideEffect::ProbeScan]);
    assert!(out.reply.ok);

    let sha = "b".repeat(64);
    let out = h.run(&Command::new("c-2", CommandKind::Ota).with_args(Args {
        url: Some("https://fw.example/img.bin".into()),
        sha256: Some(sha.to_uppercase()),
        ..Args::default()
    }));
    assert_eq!(
        out.effects,
        vec![SideEffect::Ota {
            url: "https://fw.example/img.bin".into(),
            sha256: sha,
        }]
    );

    let out = h.run(&Command::new("c-3", CommandKind::Ota).with_args(Args {
        url: Some("https://fw.example/img.bin".into()),
        sha256: Some("short".into()),
        ..Args::default()
    }));
    assert!(!out.reply.ok);
    assert!(out.effects.is_empty());

    let out = h.run(&Command::new("c-4", CommandKind::Reboot));
    assert_eq!(out.effects, vec![SideEffect::Reboot]);

    let out = h.run(
        &Command::new("c-5", CommandKind::FactoryReset).with_args(Args {
            confirm: Some(DEVICE.into()),
            ..Args::default()
        }),
    );
    assert_eq!(out.effects, vec![SideEffect::FactoryReset]);

    let out = h.run(
        &Command::new("c-6", CommandKind::FactoryReset).with_args(Args {
            confirm: Some("granite-ffffff".into()),
            ..Args::default()
        }),
    );
    assert!(!out.reply.ok);
    assert!(out.effects.is_empty());

    let out = h.run(&Command::new("c-7", CommandKind::FactoryReset));
    assert!(!out.reply.ok);
}

#[test]
fn an_unsupported_payload_version_is_refused() {
    let mut h = Harness::new();
    let mut cmd = Command::new("c-1", CommandKind::Reboot);
    cmd.v = 99;
    let out = h.run(&cmd);
    assert!(!out.reply.ok);
    assert!(out.reply.error.unwrap().contains("version 99"));
    assert!(out.effects.is_empty());
}

#[test]
fn the_signature_hook_can_reject_a_command() {
    let mut rig = Rig::new();
    rig.set_led(1, Some(false));
    let mut rules = RuleEngine::new();
    let mut config = Config::default();
    let obs = rig.obs.clone();
    let reject = |_: &Command| false;
    let mut ctx = DispatchCtx {
        now_ms: 0,
        device_id: DEVICE,
        observed: &obs,
        actuator: &mut rig.act,
        rules: &mut rules,
        config: &mut config,
        verify_sig: Some(&reject),
    };
    let cmd = Command::new("c-1", CommandKind::On).with_target(Target::Node(1));
    let out = dispatch(&cmd, &mut ctx);
    assert!(!out.reply.ok);
    assert_eq!(out.reply.error.as_deref(), Some("signature rejected"));
    assert_eq!(ctx.actuator.in_flight(), 0);
}

#[test]
fn a_modbus_coil_write_and_an_mqtt_command_take_the_same_path() {
    let mut h = Harness::new();
    h.rig.set_all_leds(Some(false));
    h.rig.set_respond(1, common::Respond::OnAfter(100));

    let coil = granite_core::modbus_map::command_from_coil(0, true, "modbus-1")
        .unwrap()
        .unwrap();
    let out = h.run(&coil);
    assert!(out.reply.ok);
    assert_eq!(out.reply.result.as_deref(), Some("accepted"));

    // The second, identical command is refused while the first runs,
    // whichever transport it came from.
    let mqtt = Command::new("c-1", CommandKind::On).with_target(Target::Node(1));
    let out = h.run(&mqtt);
    assert_eq!(out.reply.error.as_deref(), Some("refused: busy"));
}
