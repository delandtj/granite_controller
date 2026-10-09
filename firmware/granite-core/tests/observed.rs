//! The snapshot: timestamps, staleness, aggregation and the node view.

use granite_core::actuator::ActionKind;
use granite_core::node::{NodeState, Nodes, SenseMode};
use granite_core::observed::{Observed, ProbeObs, Stamped};

#[test]
fn a_never_read_field_is_distinguishable_from_a_zero() {
    let o = Observed::new();
    assert_eq!(o.board_temp.ts_ms, 0);
    assert_eq!(o.board_temp.value, None);
    assert!(o.board_temp.is_stale(10_000, 5_000));

    let fresh = Stamped::new(Some(2_500i16), 9_000);
    assert!(!fresh.is_stale(10_000, 5_000));
    assert!(fresh.is_stale(20_000, 5_000));
}

#[test]
fn probe_max_ignores_silent_probes() {
    let mut o = Observed::new();
    assert_eq!(o.probe_max(), None);
    o.probes.push(ProbeObs {
        rom: 1,
        name: "a".into(),
        centi_c: None,
        ts_ms: 1,
    });
    assert_eq!(o.probe_max(), None, "a silent probe is not 0 C");
    o.probes.push(ProbeObs {
        rom: 2,
        name: "b".into(),
        centi_c: Some(2_000),
        ts_ms: 1,
    });
    o.probes.push(ProbeObs {
        rom: 3,
        name: "c".into(),
        centi_c: Some(7_100),
        ts_ms: 1,
    });
    assert_eq!(o.probe_max(), Some(7_100));
    assert_eq!(o.probe(1).unwrap().name, "b");
    assert!(o.probe(9).is_none());
}

#[test]
fn dry_inputs_are_one_based_and_bounded() {
    let mut o = Observed::new();
    assert_eq!(o.dry(1), None, "never read");
    o.dry_in = Stamped::new(Some(0b1010), 5);
    assert_eq!(o.dry(1), Some(false));
    assert_eq!(o.dry(2), Some(true));
    assert_eq!(o.dry(4), Some(true));
    assert_eq!(o.dry(0), None);
    assert_eq!(o.dry(5), None);
}

#[test]
fn refresh_nodes_copies_the_tracker_and_keeps_the_sense_flags() {
    let mut nodes = Nodes::new();
    nodes.update_sense(0, Some(0b0000_0001));
    nodes.update_sense(200, Some(0b0000_0001));
    nodes.mark_busy(2, ActionKind::Cycle, 150);
    nodes.note_reset(3, 100);

    let mut o = Observed::new();
    o.nodes[4].sense = SenseMode::Ignore;
    o.refresh_nodes(&nodes);

    assert_eq!(o.node_state(1), NodeState::On);
    assert_eq!(o.node_led(1), Some(true));
    assert_eq!(
        o.node_state(2),
        NodeState::Busy {
            action: ActionKind::Cycle,
            started_ms: 150
        }
    );
    assert_eq!(o.node_led(2), Some(false), "the LED is still visible");
    assert_eq!(o.nodes[2].last_reset_ms, Some(100));
    assert_eq!(
        o.node_sense(5),
        SenseMode::Ignore,
        "config-driven flags survive a refresh"
    );
    assert_eq!(o.node_state(9), NodeState::Unknown, "out of range");
    assert_eq!(o.nodes[0].ts_ms, 200);
}

#[test]
fn the_snapshot_serialises_for_the_simulator_and_the_log() {
    let mut o = Observed::new();
    o.vin_mv = Stamped::new(Some(19_000), 7);
    let json = serde_json::to_string(&o).unwrap();
    let back: Observed = serde_json::from_str(&json).unwrap();
    assert_eq!(o, back);
    assert!(
        json.contains("\"vin_mv\":{\"value\":19000,\"ts_ms\":7}"),
        "{json}"
    );
}
