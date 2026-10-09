//! Configuration: the defaults are a full valid config, a round trip
//! changes nothing, a corrupt section loses only itself, and an export
//! carries no secrets.

use std::collections::BTreeMap;

use granite_core::actuator::Timings;
use granite_core::config::{
    Config, ConfigError, IpMode, LogLevel, SCHEMA_VERSION, SecCfg, Secrets, Section,
};
use granite_core::node::{BootPolicy, SenseMode};
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
    assert_eq!(c.nodes.timings, Timings::default());
    assert_eq!(c.nodes.order, [1, 2, 3, 4, 5, 6, 7, 8]);
    for (i, n) in c.nodes.nodes.iter().enumerate() {
        assert_eq!(n.boot_policy, BootPolicy::Leave, "node {}", i + 1);
        assert_eq!(n.sense, SenseMode::Enabled);
        assert_eq!(n.name, format!("node{}", i + 1));
    }
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
    c.nodes.nodes[2].name = "compute-3".into();
    c.nodes.nodes[2].boot_policy = BootPolicy::On;
    c.nodes.nodes[4].sense = SenseMode::Ignore;
    c.nodes.order = [8, 7, 6, 5, 4, 3, 2, 1];
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
    good.nodes.nodes[0].name = "head".into();
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
    assert_eq!(cfg.nodes.nodes[0].name, "node1");
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
    c.nodes.order = [3, 3, 0, 9, 1, 1, 2, 2];
    c.nodes.nodes[3].name = "   ".into();
    c.nodes.timings.t_short_ms = 5;
    c.nodes.t_probe_s = 0;
    c.nodes.vin_trim = 0.0;
    c.mqtt.site = String::new();
    c.mqtt.qos = 9;
    c.normalise();

    assert_eq!(c.nodes.order, [3, 1, 2, 4, 5, 6, 7, 8]);
    assert_eq!(c.nodes.nodes[3].name, "node4");
    assert_eq!(c.nodes.timings.t_short_ms, 100);
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
