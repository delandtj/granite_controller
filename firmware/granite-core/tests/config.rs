//! Configuration: the defaults are a full valid config, a round trip
//! changes nothing, a corrupt section loses only itself, and an export
//! carries no secrets.

use std::collections::BTreeMap;

use granite_core::config::{
    Config, ConfigError, IpMode, LogLevel, SCHEMA_VERSION, SecCfg, Secrets, Section,
};
use granite_core::node::{BootPolicy, DEFAULT_SOFT_OFF_S, Escalation, NodeSettings};
use granite_core::plane::UpdatePolicy;
use granite_core::rules::MAX_RULES;

#[test]
fn the_default_config_is_full_valid_and_safe() {
    let c = Config::default();
    c.validate().expect("defaults must validate");

    assert_eq!(c.schema_version, SCHEMA_VERSION);
    assert_eq!(c.net.ip_mode, IpMode::Dhcp);
    assert!(c.net.mdns);
    assert_eq!(c.net.t_confirm_s, 300);
    assert!(!c.mqtt.enabled, "the broker is off until configured");
    assert!(c.mqtt.tls);
    assert_eq!(c.mqtt.site, "default");
    assert_eq!(c.mqtt.t_state_s, 60);
    assert!(!c.sec.modbus.enabled, "Modbus is off by default");
    assert_eq!(c.sec.modbus.port, 502);
    assert!(!c.sec.admin_password_set);
    assert_eq!(c.sys.log_level, LogLevel::Warn);
    // No per-node entries by default: a node that nobody configured
    // takes the defaults, and its settings are kept when it goes away
    // (ADR 0004 decision 2).
    assert!(c.nodes.nodes.is_empty());
    assert_eq!(c.nodes.plane_update, UpdatePolicy::Manual);
    for node in [1u8, 17, 64] {
        let n = c.nodes.settings(node);
        assert_eq!(n.boot_policy, BootPolicy::Leave, "node {node}");
        assert!(n.led_sense);
        assert_eq!(n.escalation, Escalation::HoldThenCut);
        assert_eq!(n.t_soft_off_s, DEFAULT_SOFT_OFF_S);
        assert_eq!(c.nodes.name_of(node), format!("node{node}"));
    }
    assert!(c.nodes.boot_policies().is_empty());
    assert_eq!(c.rules.rules.len(), 3);
    assert!(c.rules.rules.iter().all(|r| !r.enabled));
}

#[test]
fn export_import_round_trip() {
    let mut c = Config::default();
    c.net.ip_mode = IpMode::Static;
    c.net.address = "192.168.10.5/24".into();
    c.net.gateway = "192.168.10.1".into();
    c.mqtt.enabled = true;
    c.mqtt.host = "broker.example".into();
    c.nodes.nodes.push(NodeSettings {
        node: 3,
        name: "compute-3".into(),
        boot_policy: BootPolicy::On,
        ..NodeSettings::for_node(3)
    });
    c.nodes.nodes.push(NodeSettings {
        node: 40,
        led_sense: false,
        escalation: Escalation::Hold,
        t_soft_off_s: 45,
        ..NodeSettings::for_node(40)
    });
    c.nodes.plane_update = UpdatePolicy::Auto;
    c.rules.rules[0].enabled = true;
    c.sec.modbus.enabled = true;
    c.sec.modbus.allow = vec!["10.0.0.0/8".into()];
    c.validate().unwrap();

    let json = c.export_json().unwrap();
    let back = Config::import_json(&json).unwrap();
    assert_eq!(c, back);
}

#[test]
fn every_section_round_trips_on_its_own() {
    let c = Config::default();
    for section in Section::ALL {
        let json = c.section_json(section).unwrap();
        let mut target = Config::default();
        target.set_section_json(section, &json).unwrap();
        assert_eq!(
            target.section_json(section).unwrap(),
            json,
            "section {section}"
        );
    }
}

#[test]
fn a_corrupt_section_falls_back_alone() {
    let mut good = Config::default();
    good.mqtt.enabled = true;
    good.mqtt.host = "broker.example".into();
    good.nodes.nodes.push(NodeSettings {
        node: 1,
        name: "head".into(),
        ..NodeSettings::for_node(1)
    });
    good.sys.log_level = LogLevel::Debug;

    let mut store: BTreeMap<Section, String> = BTreeMap::new();
    for section in Section::ALL {
        store.insert(section, good.section_json(section).unwrap());
    }
    // The nodes blob gets truncated by a bad write.
    let nodes = store.get_mut(&Section::Nodes).unwrap();
    nodes.truncate(nodes.len() / 2);

    let (cfg, fallbacks) = Config::from_sections(|s| store.get(&s).cloned());
    assert_eq!(fallbacks.len(), 1);
    assert_eq!(fallbacks[0].section, Section::Nodes);
    assert!(matches!(fallbacks[0].error, ConfigError::Parse(_)));
    assert!(fallbacks[0].to_string().contains("nodes fell back"));

    // The nodes section is back to defaults...
    assert_eq!(cfg.nodes.name_of(1), "node1");
    // ... and nothing else was lost.
    assert!(cfg.mqtt.enabled);
    assert_eq!(cfg.mqtt.host, "broker.example");
    assert_eq!(cfg.sys.log_level, LogLevel::Debug);
    cfg.validate().unwrap();
}

#[test]
fn a_missing_section_is_simply_the_default_and_not_an_error() {
    let mut store: BTreeMap<Section, String> = BTreeMap::new();
    let mut c = Config::default();
    c.sys.device_id = "granite-aabbcc".into();
    store.insert(Section::Sys, c.section_json(Section::Sys).unwrap());

    let (cfg, fallbacks) = Config::from_sections(|s| store.get(&s).cloned());
    assert!(fallbacks.is_empty(), "absent is not corrupt");
    assert_eq!(cfg.sys.device_id, "granite-aabbcc");
    assert_eq!(cfg.net.ip_mode, IpMode::Dhcp);
}

#[test]
fn an_invalid_section_also_falls_back_alone() {
    let mut store: BTreeMap<Section, String> = BTreeMap::new();
    // A sec section that enables Modbus without an allow-list parses but
    // does not validate.
    let bad = SecCfg {
        modbus: granite_core::config::ModbusCfg {
            enabled: true,
            allow: vec![],
            ..Default::default()
        },
        ..Default::default()
    };
    store.insert(Section::Sec, serde_json::to_string(&bad).unwrap());

    let (cfg, fallbacks) = Config::from_sections(|s| store.get(&s).cloned());
    assert_eq!(fallbacks.len(), 1);
    assert_eq!(fallbacks[0].section, Section::Sec);
    assert!(matches!(fallbacks[0].error, ConfigError::Invalid(_)));
    assert!(!cfg.sec.modbus.enabled);
    cfg.validate().unwrap();
}

#[test]
fn unknown_fields_are_tolerated_and_missing_ones_default() {
    let json = r#"{"ip_mode":"static","address":"10.1.2.3/24","future_knob":42}"#;
    let mut c = Config::default();
    c.set_section_json(Section::Net, json).unwrap();
    assert_eq!(c.net.ip_mode, IpMode::Static);
    assert_eq!(c.net.address, "10.1.2.3/24");
    assert!(c.net.mdns, "an absent field keeps its default");
    assert_eq!(c.net.t_deadman_s, 3_600);
}

#[test]
fn normalise_repairs_what_it_can() {
    let mut c = Config::default();
    c.nodes.nodes.push(NodeSettings {
        node: 9,
        name: "   ".into(),
        t_soft_off_s: 0,
        ..NodeSettings::for_node(9)
    });
    // A duplicate entry and one for a node that cannot exist.
    c.nodes.nodes.push(NodeSettings {
        node: 9,
        name: "second".into(),
        ..NodeSettings::for_node(9)
    });
    c.nodes.nodes.push(NodeSettings::for_node(200));
    c.nodes.t_probe_s = 0;
    c.nodes.vin_trim = 0.0;
    c.mqtt.site = String::new();
    c.mqtt.qos = 9;
    c.normalise();

    assert_eq!(c.nodes.nodes.len(), 1, "one entry per node id, in range");
    assert_eq!(c.nodes.nodes[0].node, 9);
    assert_eq!(c.nodes.name_of(9), "node9", "a blank name is the default");
    assert_eq!(c.nodes.nodes[0].t_soft_off_s, DEFAULT_SOFT_OFF_S);
    assert_eq!(c.nodes.t_probe_s, 10);
    assert_eq!(c.nodes.vin_trim, 1.0);
    assert_eq!(c.mqtt.site, "default");
    assert_eq!(c.mqtt.qos, 1);
    c.validate().unwrap();
}

#[test]
fn validation_catches_the_things_normalise_cannot() {
    let mut c = Config::default();
    c.net.ip_mode = IpMode::Static;
    let errs = c.validate().unwrap_err();
    assert!(errs.iter().any(|e| e.contains("net.address")), "{errs:?}");

    let mut c = Config::default();
    c.mqtt.enabled = true;
    assert!(
        c.validate()
            .unwrap_err()
            .iter()
            .any(|e| e.contains("mqtt.host"))
    );

    let mut c = Config::default();
    c.rules.rules[1].id = c.rules.rules[0].id;
    assert!(
        c.validate()
            .unwrap_err()
            .iter()
            .any(|e| e.contains("duplicate rule id"))
    );

    let mut c = Config::default();
    c.nodes.probes.push(granite_core::config::ProbeCfg {
        rom: "not-a-rom".into(),
        name: "x".into(),
    });
    assert!(
        c.validate()
            .unwrap_err()
            .iter()
            .any(|e| e.contains("is not a ROM id"))
    );
}

#[test]
fn the_vin_window_defaults_to_the_12_v_feed_and_zero_means_unchecked() {
    // Rev D takes 12 V from the planes (ADR 0004 decision 9).
    let c = Config::default();
    assert_eq!(c.nodes.vin_min_mv, 10_000);
    assert_eq!(c.nodes.vin_max_mv, 14_000);
    assert!(c.nodes.vin_out_of_window(9_999));
    assert!(!c.nodes.vin_out_of_window(10_000));
    assert!(!c.nodes.vin_out_of_window(12_000));
    assert!(!c.nodes.vin_out_of_window(14_000));
    assert!(c.nodes.vin_out_of_window(14_001));

    // Either bound at 0 is "do not check that side".
    let mut c = Config::default();
    c.nodes.vin_min_mv = 0;
    assert!(!c.nodes.vin_out_of_window(1), "no lower bound");
    assert!(c.nodes.vin_out_of_window(30_000));
    c.nodes.vin_max_mv = 0;
    assert!(!c.nodes.vin_out_of_window(0), "an unchecked window");
    assert!(!c.nodes.vin_out_of_window(100_000));
    c.validate().expect("an unchecked window is valid");

    // A window the wrong way round is refused, and one the divider
    // cannot reach is clamped.
    let mut c = Config::default();
    c.nodes.vin_min_mv = 14_000;
    c.nodes.vin_max_mv = 10_000;
    assert!(
        c.validate()
            .unwrap_err()
            .iter()
            .any(|e| e.contains("vin_min_mv")),
        "{:?}",
        c.validate()
    );
    let mut c = Config::default();
    c.nodes.vin_max_mv = 99_000;
    assert!(c.validate().is_err());
    c.normalise();
    assert_eq!(c.nodes.vin_max_mv, granite_core::config::VIN_WINDOW_MAX_MV);
    c.validate().unwrap();
}

#[test]
fn too_many_rules_are_truncated_by_normalise() {
    let mut c = Config::default();
    let base = c.rules.rules[0].clone();
    c.rules.rules = (1..=20)
        .map(|id| granite_core::rules::Rule { id, ..base.clone() })
        .collect();
    c.normalise();
    assert_eq!(c.rules.rules.len(), MAX_RULES);
    c.validate().unwrap();
}

#[test]
fn a_newer_schema_version_is_refused_not_guessed() {
    let c = Config {
        schema_version: SCHEMA_VERSION + 1,
        ..Config::default()
    };
    let json = c.export_json().unwrap();
    assert_eq!(
        Config::import_json(&json),
        Err(ConfigError::Schema {
            found: SCHEMA_VERSION + 1,
            expected: SCHEMA_VERSION
        })
    );
}

#[test]
fn an_older_document_is_migrated_and_stamped() {
    let c = Config {
        schema_version: 0,
        ..Config::default()
    };
    let json = c.export_json().unwrap();
    let back = Config::import_json(&json).unwrap();
    assert_eq!(back.schema_version, SCHEMA_VERSION);
}

#[test]
fn an_export_contains_no_secret_fields() {
    let c = Config::default();
    let json = c.export_json().unwrap();
    for needle in [
        "mqtt_password",
        "admin_hash",
        "admin_salt",
        "recovery_token",
        "api_tokens",
        "device_key_pem",
        "mqtt_client_key_pem",
        "private",
    ] {
        assert!(
            !json.contains(needle),
            "an exported config must not mention {needle}"
        );
    }
    // The only mention of a password is the boolean that says whether
    // one has been set, which the setup page needs.
    assert!(json.contains("\"admin_password_set\""));
    assert_eq!(json.matches("password").count(), 1, "{json}");
    // The public material is there.
    assert!(json.contains("fleet_recovery_pubkey_pem"));
}

#[test]
fn secrets_round_trip_separately() {
    let mut s = Secrets::default();
    assert!(s.admin_password_missing());
    s.admin_hash = "deadbeef".into();
    s.admin_salt = "cafe".into();
    s.recovery_token = "ABCD-EFGH-IJKL".into();
    s.api_tokens.push(granite_core::config::ApiToken {
        name: "scripts".into(),
        hash: "0011".into(),
        created_s: 42,
    });

    let json = s.to_json().unwrap();
    assert_eq!(Secrets::from_json(&json).unwrap(), s);
    assert!(!s.admin_password_missing());
    assert_eq!(s.admin_iters, 20_000);
}

#[test]
fn topic_base_follows_the_adr_tree() {
    let c = Config::default();
    assert_eq!(
        c.topic_base("granite-aabbcc"),
        "granite/default/granite-aabbcc"
    );
}

#[test]
fn an_mqtt_section_from_an_older_firmware_still_loads() {
    // `mqtt.skip_time_check` was accepted and ignored (gaps-2026-10 item
    // 8) and is gone. A board that stored it keeps its other settings, the
    // unknown field is dropped, and the schema does not move for it.
    let stored = r#"{"enabled":true,"host":"broker.example","port":8883,
        "skip_time_check":true,"qos":1}"#;
    let mut c = Config::default();
    c.set_section_json(Section::Mqtt, stored)
        .expect("an unknown field is ignored, not an error");
    assert!(c.mqtt.enabled);
    assert_eq!(c.mqtt.host, "broker.example");
    assert_eq!(c.schema_version, SCHEMA_VERSION);
    assert!(
        !c.section_json(Section::Mqtt)
            .unwrap()
            .contains("skip_time_check"),
        "the field must not come back on a write"
    );

    // The same through a whole-document import, which is the other way in.
    let mut whole = Config::default();
    whole.mqtt.enabled = true;
    whole.mqtt.host = "broker.example".into();
    let mut doc: serde_json::Value = serde_json::from_str(&whole.export_json().unwrap()).unwrap();
    doc["mqtt"]["skip_time_check"] = serde_json::Value::Bool(true);
    let imported = Config::import_json(&doc.to_string()).expect("unknown fields are ignored");
    assert_eq!(imported.mqtt.host, "broker.example");
    assert_eq!(imported.schema_version, SCHEMA_VERSION);

    // And its absence is simply the default: nothing in `mqtt` is
    // mandatory.
    let mut bare = Config::default();
    bare.set_section_json(Section::Mqtt, "{}").unwrap();
    assert_eq!(bare.mqtt, granite_core::config::MqttCfg::default());
}

#[test]
fn section_keys_and_reachability_flags() {
    assert_eq!(Section::from_key("mqtt"), Some(Section::Mqtt));
    assert_eq!(Section::from_key("nope"), None);
    assert!(Section::Net.affects_reachability());
    assert!(Section::Mqtt.affects_reachability());
    assert!(Section::Sec.affects_reachability());
    assert!(!Section::Nodes.affects_reachability());
    assert!(!Section::Rules.affects_reachability());
    assert!(!Section::Sys.affects_reachability());
}
