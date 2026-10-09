//! Rule engine scenarios: hold time, hysteresis, auto and manual rearm,
//! missing values, and the three disabled defaults.

use granite_core::Target;
use granite_core::actuator::ActionKind;
use granite_core::observed::{Observed, ProbeObs, Stamped};
use granite_core::rules::{
    MAX_RULES, Op, Rearm, Rule, RuleAction, RuleEngine, Source, default_rules,
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
            kind: ActionKind::ForceOff,
        },
        target: Target::All,
        rearm: Rearm::Auto,
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
    assert_eq!(fired[0].value, 7_100);
    assert_eq!(
        fired[0].action,
        RuleAction::Act {
            kind: ActionKind::ForceOff
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
            kind: ActionKind::ForceOff,
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

    let mut on = Observed::new();
    on.nodes[0].led = Some(true);
    let mut off = Observed::new();
    off.nodes[0].led = Some(false);

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
            kind: ActionKind::ForceOff
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
    assert_eq!(rules[2].threshold, 15_000, "15 V in millivolts");
    assert_eq!(rules[2].hold_s, 5);
    assert_eq!(rules[2].action, RuleAction::Event);
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
