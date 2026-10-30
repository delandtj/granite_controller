//! Rule engine scenarios: hold time, hysteresis, auto and manual rearm,
//! missing values, and the three disabled defaults.

use granite_core::Target;
use granite_core::msg::CommandKind;
use granite_core::observed::NodeObs;
use granite_core::observed::{Observed, ProbeObs, Stamped};
use granite_core::plane::model::PowerState;
use granite_core::rules::{
    MAX_RULES, Missing, Op, Rearm, Rule, RuleAction, RuleEngine, Source, default_rules,
};

fn probe_obs(centi: Option<i16>, ts: u64) -> Observed {
    let mut o = Observed::new();
    o.probes.push(ProbeObs {
        rom: 0x28_0000_0000_0001,
        name: "inlet".into(),
        centi_c: centi,
        ts_ms: ts,
    });
    o
}

fn over_temp_rule() -> Rule {
    Rule {
        id: 1,
        enabled: true,
        name: "over temp".into(),
        source: Source::ProbeMax,
        op: Op::Gt,
        threshold: 7_000,
        hysteresis: 500,
        hold_s: 30,
        action: RuleAction::Act {
            kind: CommandKind::ForceOff,
        },
        target: Target::All,
        rearm: Rearm::Auto,
        missing: Missing::Ignore,
    }
}

#[test]
fn the_condition_must_hold_for_hold_s() {
    let mut e = RuleEngine::with_rules(vec![over_temp_rule()]);
    let hot = probe_obs(Some(7_100), 0);

    assert!(e.tick(0, &hot).is_empty(), "t=0 starts the hold timer");
    assert!(e.tick(29_000, &hot).is_empty(), "29 s is not 30 s");
    let fired = e.tick(30_000, &hot);
    assert_eq!(fired.len(), 1);
    assert_eq!(fired[0].rule_id, 1);
    assert_eq!(fired[0].value, Some(7_100));
    assert_eq!(
        fired[0].action,
        RuleAction::Act {
            kind: CommandKind::ForceOff
        }
    );
    assert!(
        e.tick(31_000, &hot).is_empty(),
        "a fired rule does not fire again while the condition holds"
    );
}

#[test]
fn a_condition_that_lapses_restarts_the_hold_timer() {
    let mut e = RuleEngine::with_rules(vec![over_temp_rule()]);
    let hot = probe_obs(Some(7_100), 0);
    let cool = probe_obs(Some(6_000), 0);

    e.tick(0, &hot);
    e.tick(20_000, &hot);
    e.tick(21_000, &cool);
    assert!(
        e.tick(31_000, &hot).is_empty(),
        "the timer restarted at 31 s"
    );
    assert_eq!(e.tick(61_000, &hot).len(), 1);
}

#[test]
fn auto_rearm_waits_for_the_hysteresis() {
    let mut e = RuleEngine::with_rules(vec![over_temp_rule()]);
    let hot = probe_obs(Some(7_100), 0);
    e.tick(0, &hot);
    assert_eq!(e.tick(30_000, &hot).len(), 1);
    assert_eq!(e.is_armed(1), Some(false));

    // 6600 is below the threshold but not below threshold - hysteresis.
    e.tick(40_000, &probe_obs(Some(6_600), 0));
    assert_eq!(
        e.is_armed(1),
        Some(false),
        "still inside the hysteresis band"
    );

    e.tick(41_000, &probe_obs(Some(6_400), 0));
    assert_eq!(e.is_armed(1), Some(true), "cleared past the hysteresis");

    e.tick(42_000, &hot);
    assert_eq!(e.tick(72_000, &hot).len(), 1, "and it fires again");
}

#[test]
fn manual_rearm_needs_an_ack() {
    let mut rule = Rule {
        id: 2,
        enabled: true,
        name: "leak".into(),
        source: Source::DryIn { n: 1 },
        op: Op::Eq,
        threshold: 1,
        hold_s: 2,
        action: RuleAction::Act {
            kind: CommandKind::ForceOff,
        },
        rearm: Rearm::Manual,
        ..Rule::default()
    };
    rule.enabled = true;
    let mut e = RuleEngine::with_rules(vec![rule]);

    let mut wet = Observed::new();
    wet.dry_in = Stamped::new(Some(0b0001), 1);
    let dry = {
        let mut o = Observed::new();
        o.dry_in = Stamped::new(Some(0b0000), 1);
        o
    };

    e.tick(0, &wet);
    assert_eq!(e.tick(2_000, &wet).len(), 1);
    assert_eq!(e.is_armed(2), Some(false));

    // Clearing the condition does not rearm a manual rule.
    e.tick(3_000, &dry);
    assert_eq!(e.is_armed(2), Some(false));
    e.tick(4_000, &wet);
    assert!(e.tick(7_000, &wet).is_empty());

    assert!(e.ack(2));
    assert_eq!(e.is_armed(2), Some(true));
    e.tick(8_000, &wet);
    assert_eq!(e.tick(10_000, &wet).len(), 1, "and it can fire again");
    assert!(!e.ack(99), "there is no rule 99");
}

#[test]
fn a_missing_value_never_fires() {
    let mut e = RuleEngine::with_rules(vec![over_temp_rule()]);
    let missing = probe_obs(None, 0);
    e.tick(0, &missing);
    assert!(e.tick(60_000, &missing).is_empty());
    assert_eq!(e.is_armed(1), Some(true));
}

#[test]
fn an_unconfigured_boolean_source_never_fires() {
    // link_up has never been written: ts_ms == 0 means "no reading".
    let rule = Rule {
        id: 4,
        enabled: true,
        source: Source::LinkUp,
        op: Op::Eq,
        threshold: 0,
        hold_s: 1,
        action: RuleAction::Event,
        ..Rule::default()
    };
    let mut e = RuleEngine::with_rules(vec![rule]);
    let o = Observed::new();
    e.tick(0, &o);
    assert!(e.tick(5_000, &o).is_empty());

    let mut down = Observed::new();
    down.link_up = Stamped::new(false, 10);
    e.tick(6_000, &down);
    assert_eq!(e.tick(8_000, &down).len(), 1);
}

#[test]
fn changed_fires_once_per_change() {
    let rule = Rule {
        id: 5,
        enabled: true,
        source: Source::NodeOn { n: 1 },
        op: Op::Changed,
        hold_s: 10,
        action: RuleAction::Event,
        ..Rule::default()
    };
    let mut e = RuleEngine::with_rules(vec![rule]);

    let on = snapshot_with_node(1, PowerState::On);
    let off = snapshot_with_node(1, PowerState::Off);

    assert!(
        e.tick(0, &on).is_empty(),
        "the first reading is not a change"
    );
    assert_eq!(
        e.tick(1_000, &off).len(),
        1,
        "hold_s does not apply to `changed`"
    );
    assert!(e.tick(2_000, &off).is_empty());
    assert_eq!(e.tick(3_000, &on).len(), 1);
}

#[test]
fn a_disabled_rule_is_never_evaluated() {
    let mut rule = over_temp_rule();
    rule.enabled = false;
    let mut e = RuleEngine::with_rules(vec![rule]);
    let hot = probe_obs(Some(9_000), 0);
    e.tick(0, &hot);
    assert!(e.tick(60_000, &hot).is_empty());
}

#[test]
fn the_shipped_defaults_are_the_adr_examples_and_all_disabled() {
    let rules = default_rules();
    assert_eq!(rules.len(), 3);
    assert!(rules.iter().all(|r| !r.enabled));

    assert_eq!(rules[0].source, Source::ProbeMax);
    assert_eq!(rules[0].op, Op::Gt);
    assert_eq!(rules[0].threshold, 7_000, "70.00 C in centi-degrees");
    assert_eq!(rules[0].hold_s, 30);
    assert_eq!(
        rules[0].action,
        RuleAction::Act {
            kind: CommandKind::ForceOff
        }
    );
    assert_eq!(rules[0].target, Target::All);

    assert_eq!(rules[1].source, Source::DryIn { n: 1 });
    assert_eq!(rules[1].op, Op::Eq);
    assert_eq!(rules[1].threshold, 1);
    assert_eq!(rules[1].hold_s, 2);
    assert_eq!(rules[1].rearm, Rearm::Manual);

    assert_eq!(rules[2].source, Source::Vin);
    assert_eq!(rules[2].op, Op::Lt);
    assert_eq!(
        rules[2].threshold, 10_000,
        "the bottom of the 12 V window, in millivolts"
    );
    assert_eq!(rules[2].hold_s, 5);
    assert_eq!(rules[2].action, RuleAction::Event);
}

/// Bug: default rule 3 carried the 15 V threshold of rev C's 19 V
/// supply, so on a rev D controller (12 V from the planes, window 10 V
/// to 14 V) it fired the moment it was enabled. It now watches the
/// bottom of that window.
#[test]
fn the_bus_voltage_rule_fits_the_twelve_volt_input() {
    let nodes = granite_core::config::NodesCfg::default();
    let rule = &default_rules()[2];
    assert_eq!(
        rule.threshold as u32, nodes.vin_min_mv,
        "the rule and the fault bit watch the same number"
    );

    let mut e = RuleEngine::with_rules(vec![Rule {
        enabled: true,
        ..rule.clone()
    }]);
    // A healthy rev D controller: 12 V in, and a loaded bus at 10.5 V.
    for (mv, when) in [(12_000u32, 0u64), (11_000, 10_000), (10_500, 20_000)] {
        let o = vin_obs(mv);
        assert!(
            e.tick(when, &o).is_empty(),
            "{mv} mV is inside the window and must not fire"
        );
        assert!(!nodes.vin_out_of_window(mv), "{mv} mV is not a vin fault");
    }

    // Below the window it fires, after its hold.
    let sagging = vin_obs(9_800);
    assert!(e.tick(30_000, &sagging).is_empty(), "the hold starts");
    let fired = e.tick(35_000, &sagging);
    assert_eq!(fired.len(), 1, "5 s below 10 V is the warning");
    assert_eq!(fired[0].value, Some(9_800));
    assert!(nodes.vin_out_of_window(9_800), "and the vin fault bit too");
}

/// A snapshot with nothing but a VIN reading.
fn vin_obs(mv: u32) -> Observed {
    let mut o = Observed::new();
    o.vin_mv = Stamped::new(Some(mv), 1);
    o
}

#[test]
fn the_engine_holds_at_most_sixteen_rules() {
    let many: Vec<Rule> = (1..=30)
        .map(|id| Rule {
            id,
            ..Rule::default()
        })
        .collect();
    let e = RuleEngine::with_rules(many);
    assert_eq!(e.rules().len(), MAX_RULES);
}

#[test]
fn replacing_the_rule_set_keeps_the_latch_of_unchanged_rules() {
    let mut e = RuleEngine::with_rules(vec![over_temp_rule()]);
    let hot = probe_obs(Some(7_100), 0);
    e.tick(0, &hot);
    assert_eq!(e.tick(30_000, &hot).len(), 1);
    assert_eq!(e.is_armed(1), Some(false));

    e.set_rules(vec![over_temp_rule()]);
    assert_eq!(e.is_armed(1), Some(false), "same rule, same latch");

    let mut edited = over_temp_rule();
    edited.threshold = 8_000;
    e.set_rules(vec![edited]);
    assert_eq!(e.is_armed(1), Some(true), "an edited rule starts armed");
}

#[test]
fn rules_serialise_round_trip() {
    for rule in default_rules() {
        let json = serde_json::to_string(&rule).unwrap();
        let back: Rule = serde_json::from_str(&json).unwrap();
        assert_eq!(rule, back, "{json}");
    }
    // The wire form is flat, with the ADR's operator spelling.
    let json = serde_json::to_string(&default_rules()[0]).unwrap();
    assert!(json.contains("\"source\":\"probe_max\""), "{json}");
    assert!(json.contains("\"op\":\">\""), "{json}");
    assert!(json.contains("\"action\":\"act\""), "{json}");
    assert!(json.contains("\"kind\":\"force_off\""), "{json}");
    assert!(json.contains("\"target\":\"all\""), "{json}");
}

#[test]
fn a_missing_value_trips_only_when_asked() {
    let no_probe = Observed::new();
    let mut ignore = RuleEngine::with_rules(vec![over_temp_rule()]);
    ignore.tick(0, &no_probe);
    assert!(
        ignore.tick(60_000, &no_probe).is_empty(),
        "ignore waits for a value"
    );

    let mut rule = over_temp_rule();
    rule.missing = Missing::Trip;
    let mut trip = RuleEngine::with_rules(vec![rule]);
    assert!(trip.tick(0, &no_probe).is_empty(), "the hold applies");
    let fired = trip.tick(30_000, &no_probe);
    assert_eq!(fired.len(), 1);
    assert_eq!(fired[0].value, None);

    // It stays fired while the value is missing, and an auto rule only
    // re-arms on a real value below the hysteresis.
    assert!(trip.tick(60_000, &no_probe).is_empty());
    assert_eq!(trip.is_armed(1), Some(false));
    trip.tick(61_000, &probe_obs(Some(6_000), 61_000));
    assert_eq!(trip.is_armed(1), Some(true));
}

#[test]
fn the_shipped_over_temperature_rule_trips_on_no_probe() {
    assert_eq!(default_rules()[0].missing, Missing::Trip);
    assert_eq!(default_rules()[1].missing, Missing::Ignore);
    // An older stored rule set without the field reads as `ignore`.
    let old = r#"{"id":9,"enabled":true,"source":"vin","op":"<","threshold":1,"action":"event"}"#;
    let rule: Rule = serde_json::from_str(old).unwrap();
    assert_eq!(rule.missing, Missing::Ignore);
}

/// A snapshot with one node of plane 0 in a given rail state.
fn snapshot_with_node(node: granite_core::NodeId, power: PowerState) -> Observed {
    let (address, slot) = granite_core::plane::node_location(node).expect("a node id");
    let mut obs = NodeObs::unknown(node, address, slot);
    obs.present = true;
    obs.power = power;
    obs.ts_ms = 10;
    let mut observed = Observed::new();
    observed.nodes.push(obs);
    observed
}

#[test]
fn the_new_sources_read_the_plane_model() {
    use granite_core::observed::{PlaneObs, UnitObs};
    use granite_core::plane::Plane;

    let mut observed = snapshot_with_node(3, PowerState::On);
    let node = observed.node_mut(3).expect("node 3");
    node.host = granite_core::plane::model::HostState::Sleep;
    node.telemetry.volts = 4_810;
    node.telemetry.amps = 1_250;
    node.telemetry.watts = 601;
    node.telemetry.temperature = 412;

    let mut plane = PlaneObs::of_plane(&Plane::new(0x30));
    plane.bus_volts = 1_205;
    plane.units = vec![UnitObs {
        unit: 0,
        present: true,
        amps: 3_300,
        watts: 1_580,
        ..UnitObs::default()
    }];
    observed.planes.push(plane);

    for (source, expected) in [
        (Source::NodeOn { n: 3 }, 1),
        (Source::NodeHostOn { n: 3 }, 0),
        (Source::NodePower { n: 3 }, 2),
        (Source::NodeHost { n: 3 }, 3),
        (Source::NodeVolts { n: 3 }, 48_100),
        (Source::NodeAmps { n: 3 }, 1_250),
        (Source::NodeWatts { n: 3 }, 601),
        (Source::NodeTemp { n: 3 }, 4_120),
        (Source::PlaneBusVolts { p: 0 }, 12_050),
        (Source::UnitAmps { p: 0, u: 0 }, 3_300),
        (Source::UnitWatts { p: 0, u: 0 }, 1_580),
    ] {
        assert_eq!(source.read(&observed), Some(expected), "{source:?}");
        assert!(source.is_valid());
    }

    // A node, plane or unit that is not there has no value, which makes
    // every comparison false.
    assert_eq!(Source::NodeWatts { n: 4 }.read(&observed), None);
    assert_eq!(Source::PlaneBusVolts { p: 2 }.read(&observed), None);
    assert_eq!(Source::UnitAmps { p: 0, u: 2 }.read(&observed), None);
    assert!(!Source::NodeOn { n: 65 }.is_valid());
    assert!(!Source::UnitWatts { p: 8, u: 0 }.is_valid());
}

#[test]
fn a_firing_rule_builds_the_command_it_asks_for() {
    use granite_core::rules::Fired;

    let fired = Fired {
        rule_id: 4,
        name: String::from("leak"),
        action: RuleAction::Act {
            kind: CommandKind::ForceOff,
        },
        target: granite_core::Target::Node(9),
        value: Some(1),
        rearm: granite_core::rules::Rearm::Manual,
    };
    let cmd = fired.command().expect("an acting rule");
    assert_eq!(cmd.id, "rule-4");
    assert_eq!(cmd.action, CommandKind::ForceOff);
    assert_eq!(cmd.target, granite_core::Target::Node(9));
    assert!(cmd.args.force, "the safety verb acts on a doubtful node");

    let plane_verb = Fired {
        action: RuleAction::Act {
            kind: CommandKind::UnitReset,
        },
        target: granite_core::Target::Unit { plane: 1, unit: 2 },
        ..fired.clone()
    };
    let cmd = plane_verb.command().expect("a plane verb acts too");
    assert_eq!(cmd.action, CommandKind::UnitReset);
    assert!(!cmd.args.force);

    let event_only = Fired {
        action: RuleAction::Event,
        ..fired
    };
    assert!(event_only.command().is_none());
}
