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
use granite_core::switch::model::{DesiredPort, VlanMode};

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
fn a_node_port_in_vlan_1_costs_the_switch_section_and_nothing_else() {
    // What a controller boots into when the stored switch section breaks
    // a rule the image learned later (here the management VLAN
    // invariant, ADR 0006 component 25): the section parses, does not
    // validate, and falls back on its own. Boot carries on.
    let mut stored = Config::default();
    stored.sys.device_id = "granite-aabbcc".into();
    stored.mqtt.enabled = true;
    stored.mqtt.host = "broker.example".into();
    stored.switch.enabled = true;
    stored.switch.host = "192.168.64.241".into();
    stored.switch.desired.ports = vec![DesiredPort {
        port: 1,
        mode: Some(VlanMode::Access),
        vlan: Some(1),
        node: Some(1),
        ..DesiredPort::default()
    }];

    let mut store: BTreeMap<Section, String> = BTreeMap::new();
    for section in [Section::Mqtt, Section::Switch, Section::Sys] {
        store.insert(section, stored.section_json(section).unwrap());
    }
    let (cfg, fallbacks) = Config::from_sections(|s| store.get(&s).cloned());
    assert_eq!(fallbacks.len(), 1);
    assert_eq!(fallbacks[0].section, Section::Switch);
    assert!(matches!(fallbacks[0].error, ConfigError::Invalid(_)));

    // The whole switch section is gone, not just the offending port: no
    // switch is polled until the section is written again.
    assert!(!cfg.switch.enabled);
    assert!(cfg.switch.host.is_empty());
    assert!(cfg.switch.desired.is_empty());
    // Everything else loaded.
    assert!(cfg.mqtt.enabled);
    assert_eq!(cfg.sys.device_id, "granite-aabbcc");
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
fn the_switch_section_is_off_and_read_only_by_default() {
    let c = Config::default();
    assert!(!c.switch.enabled, "no switch until one is configured");
    assert!(!c.switch.manage, "reading is the default (ADR 0006)");
    assert!(!c.switch.enforce, "drift is reported, not fixed");
    assert_eq!(c.switch.user, "admin");
    assert_eq!(
        c.switch.uplink_port,
        granite_core::config::DEFAULT_UPLINK_PORT
    );
    assert_eq!(c.switch.uplink(), Some(9), "the SFP+ of the bench unit");
    assert!(c.switch.desired.is_empty(), "nothing is managed");
    assert!(!c.switch.should_poll() && !c.switch.may_write());
    assert_eq!(c.switch.poll.stats_s, 5);
    assert_eq!(c.switch.poll.counters_s, 60);

    // An empty document is the default, and a round trip changes
    // nothing.
    let mut bare = Config::default();
    bare.set_section_json(Section::Switch, "{}").unwrap();
    assert_eq!(bare.switch, granite_core::config::SwitchCfg::default());
    let json = bare.section_json(Section::Switch).expect("serialises");
    let mut back = Config::default();
    back.set_section_json(Section::Switch, &json).unwrap();
    assert_eq!(back.switch, bare.switch);
}

#[test]
fn an_enabled_switch_needs_a_host_and_a_sane_desired_state() {
    let mut c = Config::default();
    c.switch.enabled = true;
    let errs = c.validate().expect_err("no host");
    assert!(errs.iter().any(|e| e.contains("switch.host")), "{errs:?}");

    c.switch.host = String::from("10.0.202.5");
    c.validate().expect("a host is all it needs to be polled");
    assert!(c.switch.should_poll());
    assert!(!c.switch.may_write(), "manage is still false");

    c.switch.uplink_port = 12;
    let errs = c.validate().expect_err("port 12");
    assert!(
        errs.iter().any(|e| e.contains("switch.uplink_port")),
        "{errs:?}"
    );
    c.switch.uplink_port = 0;
    c.validate().expect("0 means no uplink");
    assert_eq!(c.switch.uplink(), None);

    // The desired document is validated with the section.
    c.switch.desired.jumbo = Some(4_711);
    let errs = c.validate().expect_err("not a frame size the switch has");
    assert!(
        errs.iter().any(|e| e.contains("switch.desired.jumbo")),
        "{errs:?}"
    );
}

#[test]
fn the_switch_password_is_a_secret_and_not_part_of_an_export() {
    let mut s = Secrets::default();
    assert_eq!(
        s.switch_password(),
        "admin",
        "a switch nobody provisioned still has its factory password"
    );
    s.switch_password = String::from("a-generated-one");
    assert_eq!(s.switch_password(), "a-generated-one");

    let mut c = Config::default();
    c.switch.enabled = true;
    c.switch.host = String::from("10.0.202.5");
    let export = c.export_json().expect("exports");
    assert!(
        !export.contains("a-generated-one") && !export.contains("switch_password"),
        "a Config cannot carry a secret"
    );
}

#[test]
fn section_keys_and_reachability_flags() {
    assert_eq!(Section::from_key("mqtt"), Some(Section::Mqtt));
    assert_eq!(Section::from_key("switch"), Some(Section::Switch));
    assert_eq!(Section::from_key("switch2"), Some(Section::Switch2));
    assert_eq!(Section::from_key("nope"), None);
    // A VLAN or address change on the block switch can cut the
    // controller off, so it is commit-confirmed too (ADR 0006).
    assert!(Section::Switch.affects_reachability());
    assert!(
        Section::Switch2.affects_reachability(),
        "the second switch is staged and confirmed like the first (component 26)"
    );
    assert!(Section::Net.affects_reachability());
    assert!(Section::Mqtt.affects_reachability());
    assert!(Section::Sec.affects_reachability());
    assert!(!Section::Nodes.affects_reachability());
    assert!(!Section::Rules.affects_reachability());
    assert!(!Section::Sys.affects_reachability());
}

// ---------------------------------------------------------------------
// The second switch (ADR 0006 component 26)
// ---------------------------------------------------------------------

/// A desired port entry that labels a node.
fn node_port(port: u8, node: u8) -> DesiredPort {
    DesiredPort {
        port,
        mode: Some(VlanMode::Access),
        vlan: Some(100),
        node: Some(node),
        ..DesiredPort::default()
    }
}

#[test]
fn the_second_switch_section_is_off_by_default_and_round_trips() {
    let c = Config::default();
    assert_eq!(
        c.switch2,
        granite_core::config::SwitchCfg::default(),
        "the same shape and the same defaults as `switch`"
    );
    assert!(!c.switch2.enabled, "a controller has one switch unless told");
    assert!(!c.switch2.should_poll() && !c.switch2.may_write());
    assert_eq!(c.switch2.user, "admin");
    assert_eq!(c.switch2.poll.counters_s, 60);
    c.validate().expect("defaults still validate");

    // Written, normalised and read back unchanged.
    let mut set = Config::default();
    set.set_section_json(
        Section::Switch2,
        r#"{"enabled":true,"manage":true,"host":"10.0.202.6",
            "mac":"1c:2a:a3:24:26:7f","user":"","uplink_port":9,
            "poll":{"stats_s":0},
            "desired":{"ports":[{"port":2,"mode":"access","vlan":100,"node":10},
                                {"port":1,"mode":"access","vlan":100,"node":9}]}}"#,
    )
    .expect("the second section parses");
    set.validate().expect("and validates");
    assert_eq!(set.switch2.mac, "1C:2A:A3:24:26:7F", "normalised like switch");
    assert_eq!(set.switch2.user, "admin", "the login user has a default");
    assert_eq!(set.switch2.poll.stats_s, 1, "the poll floor applies");
    assert_eq!(
        set.switch2
            .desired
            .ports
            .iter()
            .map(|p| p.port)
            .collect::<Vec<u8>>(),
        vec![1, 2],
        "the port list is sorted like switch's"
    );
    assert_eq!(set.switch2.port_of_node(10), Some(2));
    assert!(
        !set.switch.enabled,
        "writing switch2 leaves the first switch alone"
    );

    let json = set.section_json(Section::Switch2).expect("serialises");
    let mut back = Config::default();
    back.set_section_json(Section::Switch2, &json).unwrap();
    assert_eq!(back.switch2, set.switch2);

    // And a whole-document round trip.
    let imported = Config::import_json(&set.export_json().unwrap()).expect("imports");
    assert_eq!(imported.switch2, set.switch2);
}

#[test]
fn a_stored_config_without_switch2_loads_unchanged() {
    // What a controller that was configured before the second switch
    // existed has in NVS: six sections and no `switch2` key at all.
    let mut stored = Config::default();
    stored.sys.device_id = "granite-aabbcc".into();
    stored.switch.enabled = true;
    stored.switch.manage = true;
    stored.switch.host = "192.168.64.241".into();
    stored.switch.desired.ports = vec![node_port(1, 1)];

    let mut store: BTreeMap<Section, String> = BTreeMap::new();
    for section in [Section::Switch, Section::Sys] {
        store.insert(section, stored.section_json(section).unwrap());
    }
    let (cfg, fallbacks) = Config::from_sections(|s| store.get(&s).cloned());
    assert!(fallbacks.is_empty(), "{fallbacks:?}");
    cfg.validate().expect("an old config is still valid");
    assert_eq!(cfg.switch, stored.switch, "switch 1 is what it was");
    assert_eq!(
        cfg.switch2,
        granite_core::config::SwitchCfg::default(),
        "and the missing section is simply off"
    );

    // An exported document from before the second switch imports too.
    let old = r#"{"schema_version":1,"switch":{"enabled":true,"host":"10.0.202.5"}}"#;
    let imported = Config::import_json(old).expect("imports");
    assert!(imported.switch.enabled && !imported.switch2.enabled);
}

#[test]
fn the_two_switch_sections_may_not_be_the_same_switch() {
    let mut c = Config::default();
    c.switch.enabled = true;
    c.switch.host = String::from("10.0.202.5");
    c.switch.mac = String::from("1C:2A:A3:24:26:7E");
    c.switch2.enabled = true;
    c.switch2.host = String::from("10.0.202.6");
    c.switch2.mac = String::from("1C:2A:A3:24:26:7F");
    c.validate().expect("two switches, two addresses, two MACs");

    // The same host.
    let mut same_host = c.clone();
    same_host.switch2.host = String::from("10.0.202.5");
    let errs = same_host.validate().expect_err("one address, two sections");
    assert!(
        errs.iter().any(|e| e.contains("both at 10.0.202.5")),
        "{errs:?}"
    );

    // The same MAC. An empty MAC on both is "do not check" and no
    // clash.
    let mut same_mac = c.clone();
    same_mac.switch2.mac = String::from("1c:2a:a3:24:26:7e");
    same_mac.normalise();
    let errs = same_mac.validate().expect_err("one MAC, two sections");
    assert!(
        errs.iter().any(|e| e.contains("both have MAC")),
        "{errs:?}"
    );
    let mut no_mac = c.clone();
    no_mac.switch.mac = String::new();
    no_mac.switch2.mac = String::new();
    no_mac.validate().expect("no MAC is not a clash");

    // The same node on both.
    let mut same_node = c.clone();
    same_node.switch.desired.ports = vec![node_port(1, 9)];
    same_node.switch2.desired.ports = vec![node_port(3, 9)];
    let errs = same_node.validate().expect_err("node 9 on two switches");
    assert!(
        errs.iter()
            .any(|e| e.contains("node 9 is on switch port 1 and on switch2 port 3")),
        "{errs:?}"
    );
    // Different nodes are the point of the second switch.
    let mut two_blocks = c.clone();
    two_blocks.switch.desired.ports = vec![node_port(1, 1)];
    two_blocks.switch2.desired.ports = vec![node_port(1, 9)];
    two_blocks.validate().expect("nodes 1 and 9, one each");
    assert_eq!(
        two_blocks.switch_of_node(9),
        Some(granite_core::config::SwitchId::Two)
    );
    assert_eq!(
        two_blocks.switch_of_node(1),
        Some(granite_core::config::SwitchId::One)
    );
    assert_eq!(two_blocks.switch_of_node(20), None);

    // While switch2 is off, nothing is cross-checked: the sections are
    // not two switches then.
    let mut off = same_host.clone();
    off.switch2.enabled = false;
    off.validate().expect("a disabled section clashes with nothing");
}

#[test]
fn the_second_switch_is_validated_under_its_own_name() {
    let mut c = Config::default();
    c.switch2.enabled = true;
    let errs = c.validate().expect_err("no host");
    assert!(errs.iter().any(|e| e.contains("switch2.host")), "{errs:?}");

    c.switch2.host = String::from("10.0.202.6");
    c.switch2.uplink_port = 12;
    let errs = c.validate().expect_err("port 12");
    assert!(
        errs.iter().any(|e| e.contains("switch2.uplink_port")),
        "{errs:?}"
    );

    c.switch2.uplink_port = 9;
    c.switch2.desired.jumbo = Some(4_711);
    let errs = c.validate().expect_err("not a frame size the switch has");
    assert!(
        errs.iter().any(|e| e.contains("switch2.desired.jumbo")),
        "{errs:?}"
    );

    // The management VLAN invariant holds per section (component 25).
    c.switch2.desired.jumbo = None;
    c.switch2.desired.ports = vec![DesiredPort {
        port: 1,
        mode: Some(VlanMode::Access),
        vlan: Some(1),
        node: Some(9),
        ..DesiredPort::default()
    }];
    let errs = c.validate().expect_err("a node port in VLAN 1");
    assert!(
        errs.iter()
            .any(|e| e.starts_with("switch2.desired.ports[1]") && e.contains("VLAN 1")),
        "{errs:?}"
    );
}

#[test]
fn the_second_switch_has_its_own_password_slot() {
    let mut s = Secrets::default();
    assert_eq!(s.switch2_password(), "admin", "the factory password");
    s.switch_password = String::from("first-one");
    s.switch2_password = String::from("second-one");
    assert_eq!(
        s.switch_password_of(granite_core::config::SwitchId::One),
        "first-one"
    );
    assert_eq!(
        s.switch_password_of(granite_core::config::SwitchId::Two),
        "second-one"
    );

    let mut c = Config::default();
    c.switch2.enabled = true;
    c.switch2.host = String::from("10.0.202.6");
    let export = c.export_json().expect("exports");
    assert!(
        !export.contains("second-one") && !export.contains("switch2_password"),
        "a Config cannot carry a secret"
    );
}
