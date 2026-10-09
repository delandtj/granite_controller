//! Node state model: LED debounce, the `Unknown` rules, busy markers and
//! the boot policy.

mod common;

use common::{Respond, Rig};
use granite_core::actuator::{
    ActionKind, ActionResult, ActuatorEvent, Submission, Timings, apply_to_nodes,
};
use granite_core::hal::Switch;
use granite_core::node::{BootPolicy, DEBOUNCE_MS, NodeState, Nodes, SenseMode, default_name};

#[test]
fn led_needs_a_hundred_milliseconds_to_be_believed() {
    let mut nodes = Nodes::new();
    assert_eq!(nodes.state(1), NodeState::Unknown);

    nodes.update_sense(0, Some(0b0000_0001));
    assert_eq!(nodes.state(1), NodeState::Unknown, "not yet debounced");

    nodes.update_sense(DEBOUNCE_MS - 1, Some(0b0000_0001));
    assert_eq!(nodes.state(1), NodeState::Unknown);

    nodes.update_sense(DEBOUNCE_MS, Some(0b0000_0001));
    assert_eq!(nodes.state(1), NodeState::On);
    assert_eq!(nodes.led(1), Some(true));
    assert_eq!(nodes.led_bits(), 0b0000_0001);
}

#[test]
fn a_glitch_shorter_than_the_window_is_ignored() {
    let mut nodes = Nodes::new();
    for t in [0, 100, 200] {
        nodes.update_sense(t, Some(0b0000_0010));
    }
    assert_eq!(nodes.state(2), NodeState::On);

    // 50 ms of "off" then back on: the stable value never moves.
    nodes.update_sense(250, Some(0));
    nodes.update_sense(290, Some(0));
    assert_eq!(nodes.state(2), NodeState::On);
    nodes.update_sense(300, Some(0b0000_0010));
    nodes.update_sense(450, Some(0b0000_0010));
    assert_eq!(nodes.state(2), NodeState::On);

    // A real change does land.
    nodes.update_sense(500, Some(0));
    nodes.update_sense(600, Some(0));
    assert_eq!(nodes.state(2), NodeState::Off);
}

#[test]
fn an_unreadable_expander_makes_every_node_unknown_at_once() {
    let mut nodes = Nodes::new();
    nodes.update_sense(0, Some(0xff));
    nodes.update_sense(200, Some(0xff));
    assert_eq!(nodes.state(5), NodeState::On);

    nodes.update_sense(300, None);
    for n in 1..=8 {
        assert_eq!(nodes.state(n), NodeState::Unknown, "node {n}");
    }
    assert_eq!(nodes.led_bits(), 0);
}

#[test]
fn an_ignored_sense_never_reports_on_or_off() {
    let mut nodes = Nodes::new();
    let mut modes = [SenseMode::Enabled; 8];
    modes[2] = SenseMode::Ignore;
    nodes.set_sense(modes);

    nodes.update_sense(0, Some(0xff));
    nodes.update_sense(500, Some(0xff));
    assert_eq!(nodes.state(3), NodeState::Unknown);
    assert_eq!(nodes.state(4), NodeState::On);
}

#[test]
fn busy_hides_the_led_but_not_from_the_actuator() {
    let mut nodes = Nodes::new();
    nodes.update_sense(0, Some(0));
    nodes.update_sense(200, Some(0));
    assert_eq!(nodes.state(1), NodeState::Off);

    nodes.mark_busy(1, ActionKind::On, 250);
    assert_eq!(
        nodes.state(1),
        NodeState::Busy {
            action: ActionKind::On,
            started_ms: 250
        }
    );
    assert_eq!(nodes.led(1), Some(false), "the LED is still readable");

    nodes.clear_busy(1);
    assert_eq!(nodes.state(1), NodeState::Off);
}

#[test]
fn events_drive_the_busy_marker_and_the_last_action() {
    let mut nodes = Nodes::new();
    nodes.update_sense(0, Some(0));
    nodes.update_sense(200, Some(0));

    apply_to_nodes(
        &mut nodes,
        &[ActuatorEvent::Accepted {
            id: "c-1".into(),
            node: 2,
            action: ActionKind::On,
        }],
        300,
    );
    assert!(nodes.state(2).is_busy());
    assert_eq!(nodes.last_action(2).unwrap().id, "c-1");
    assert!(nodes.last_action(2).unwrap().result.is_none());

    apply_to_nodes(
        &mut nodes,
        &[
            ActuatorEvent::PressEnd {
                id: "c-1".into(),
                node: 2,
                switch: Switch::Rst,
                held_ms: 250,
            },
            ActuatorEvent::Done {
                id: "c-1".into(),
                node: 2,
                action: ActionKind::On,
                result: ActionResult::Ok,
            },
        ],
        900,
    );
    assert!(!nodes.state(2).is_busy());
    let last = nodes.last_action(2).unwrap();
    assert_eq!(last.started_ms, 300);
    assert_eq!(last.finished_ms, Some(900));
    assert_eq!(last.result.as_deref(), Some("ok"));
    assert_eq!(
        nodes.last_reset_ms(2),
        Some(900),
        "an RST press is remembered so a reader can reason about a hang"
    );
}

#[test]
fn node_state_wire_names_and_modbus_codes() {
    assert_eq!(NodeState::Unknown.as_modbus(), 0);
    assert_eq!(NodeState::Off.as_modbus(), 1);
    assert_eq!(NodeState::On.as_modbus(), 2);
    assert_eq!(
        NodeState::Busy {
            action: ActionKind::Cycle,
            started_ms: 1
        }
        .as_modbus(),
        3
    );
    assert_eq!(NodeState::Off.as_str(), "off");
    assert_eq!(default_name(7), "node7");
}

#[test]
fn boot_policy_leave_touches_nothing() {
    let mut rig = Rig::new();
    rig.set_all_leds(Some(false));
    let policies = [BootPolicy::Leave; 8];
    let (sub, events) = rig
        .act
        .submit_boot_policy("boot", &policies, rig.now, &rig.obs);
    assert_eq!(sub, Submission::Done(ActionResult::Ok));
    assert!(events.is_empty());
}

#[test]
fn boot_policy_applies_on_and_off_in_the_configured_order() {
    let mut rig = Rig::new();
    rig.set_timings(Timings {
        t_stagger_ms: 500,
        t_on_ms: 1_000,
        t_soft_off_ms: 5_000,
        ..Timings::default()
    });
    rig.act.set_order([8, 7, 6, 5, 4, 3, 2, 1]);
    rig.set_all_leds(Some(false));
    rig.set_led(1, Some(true));
    rig.set_respond(2, Respond::OnAfter(100));
    rig.set_respond(3, Respond::OnAfter(100));
    rig.set_respond(1, Respond::OffAfter(100));

    let mut policies = [BootPolicy::Leave; 8];
    policies[0] = BootPolicy::Off; // node 1 is on, must go off
    policies[1] = BootPolicy::On; // node 2 is off, must come on
    policies[2] = BootPolicy::On; // node 3 as well
    policies[3] = BootPolicy::Off; // node 4 is already off: nothing to do

    let (sub, events) = rig
        .act
        .submit_boot_policy("boot", &policies, rig.now, &rig.obs);
    assert_eq!(sub, Submission::Accepted { jobs: 3 });
    rig.events.extend(events);
    rig.run_to(20_000);

    let order: Vec<u8> = rig.press_starts().into_iter().map(|(n, ..)| n).collect();
    assert_eq!(
        order,
        vec![3, 2, 1],
        "the configured order is 8..1, so node 3 goes before 2, and the \
         off policy runs after the on policies"
    );
    assert_eq!(rig.group_result("boot"), Some(ActionResult::Ok));
}

#[test]
fn boot_policy_on_a_node_with_an_unknown_led_is_refused_not_guessed() {
    let mut rig = Rig::new();
    rig.set_all_leds(None);
    let mut policies = [BootPolicy::Leave; 8];
    policies[0] = BootPolicy::On;

    let (sub, events) = rig
        .act
        .submit_boot_policy("boot", &policies, rig.now, &rig.obs);
    assert_eq!(sub, Submission::Done(ActionResult::Ok));
    let refused = events.iter().any(|e| {
        matches!(
            e,
            ActuatorEvent::Done {
                result: ActionResult::Refused { .. },
                ..
            }
        )
    });
    assert!(refused, "an unknown node is never pressed by a boot policy");
    assert_eq!(rig.act.in_flight(), 0);
}
