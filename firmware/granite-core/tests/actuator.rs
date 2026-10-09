//! Table-driven actuator tests: every action against every initial LED
//! state against the plausible LED responses, plus the escalation, the
//! `Unknown` refusal, the press deadline and the stagger order.

mod common;

use common::{Respond, Rig};
use granite_core::Target;
use granite_core::actuator::{
    ActionArgs, ActionKind, ActionRequest, ActionResult, ActuatorEvent, FailReason, RefuseReason,
    Submission, Timings, deadline_for,
};
use granite_core::hal::{HalError, Switch};
use granite_core::node::SenseMode;

/// Compressed timings: the same shape as the defaults, short enough to
/// run a soft-off escalation in a few thousand ticks. Each value is still
/// inside the validation range.
fn fast() -> Timings {
    let t = Timings {
        t_short_ms: 250,
        t_hold_ms: 4_000,
        t_on_ms: 1_000,
        t_soft_off_ms: 5_000,
        t_cycle_ms: 1_000,
        t_stagger_ms: 500,
        t_settle_ms: 0,
    };
    t.validate().expect("test timings are valid");
    t
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExpectSubmit {
    Accepted,
    Immediate,
}

struct Case {
    name: &'static str,
    kind: ActionKind,
    args: ActionArgs,
    led: Option<bool>,
    sense: SenseMode,
    respond: Respond,
    /// Absolute LED changes scheduled before the request is submitted,
    /// for the holds whose end depends on the LED rather than on a press.
    pre: &'static [(u64, Option<bool>)],
    submit: ExpectSubmit,
    result: ActionResult,
    presses: &'static [(Switch, u32)],
}

const NODE: u8 = 4;

fn args() -> ActionArgs {
    ActionArgs::default()
}

fn cases() -> Vec<Case> {
    let a = args();
    alloc_cases(a)
}

fn alloc_cases(a: ActionArgs) -> Vec<Case> {
    vec![
        Case {
            name: "on: already on, nothing to do",
            kind: ActionKind::On,
            args: a,
            led: Some(true),
            sense: SenseMode::Enabled,
            respond: Respond::Never,
            pre: &[],
            submit: ExpectSubmit::Immediate,
            result: ActionResult::Ok,
            presses: &[],
        },
        Case {
            name: "on: off, motherboard comes up",
            kind: ActionKind::On,
            args: a,
            led: Some(false),
            sense: SenseMode::Enabled,
            respond: Respond::OnAfter(300),
            pre: &[],
            submit: ExpectSubmit::Accepted,
            result: ActionResult::Ok,
            presses: &[(Switch::Pwr, 250)],
        },
        Case {
            name: "on: off, LED never comes on",
            kind: ActionKind::On,
            args: a,
            led: Some(false),
            sense: SenseMode::Enabled,
            respond: Respond::Never,
            pre: &[],
            submit: ExpectSubmit::Accepted,
            result: ActionResult::failed(FailReason::PowerOnTimeout),
            presses: &[(Switch::Pwr, 250)],
        },
        Case {
            name: "on: unknown state is refused",
            kind: ActionKind::On,
            args: a,
            led: None,
            sense: SenseMode::Enabled,
            respond: Respond::Never,
            pre: &[],
            submit: ExpectSubmit::Immediate,
            result: ActionResult::refused(RefuseReason::UnknownState),
            presses: &[],
        },
        Case {
            name: "on: unknown state with force presses anyway",
            kind: ActionKind::On,
            args: ActionArgs { force: true, ..a },
            led: None,
            sense: SenseMode::Enabled,
            respond: Respond::Never,
            pre: &[],
            submit: ExpectSubmit::Accepted,
            result: ActionResult::failed(FailReason::PowerOnTimeout),
            presses: &[(Switch::Pwr, 250)],
        },
        Case {
            name: "on: sense ignored behaves like a press",
            kind: ActionKind::On,
            args: a,
            led: None,
            sense: SenseMode::Ignore,
            respond: Respond::Never,
            pre: &[],
            submit: ExpectSubmit::Accepted,
            result: ActionResult::Ok,
            presses: &[(Switch::Pwr, 250)],
        },
        Case {
            name: "off: already off, nothing to do",
            kind: ActionKind::Off,
            args: a,
            led: Some(false),
            sense: SenseMode::Enabled,
            respond: Respond::Never,
            pre: &[],
            submit: ExpectSubmit::Immediate,
            result: ActionResult::Ok,
            presses: &[],
        },
        Case {
            name: "off: on, ACPI soft-off works",
            kind: ActionKind::Off,
            args: a,
            led: Some(true),
            sense: SenseMode::Enabled,
            respond: Respond::OffAfter(800),
            pre: &[],
            submit: ExpectSubmit::Accepted,
            result: ActionResult::Ok,
            presses: &[(Switch::Pwr, 250)],
        },
        Case {
            name: "off: hung node escalates to force_off and the hold fails",
            kind: ActionKind::Off,
            args: a,
            led: Some(true),
            sense: SenseMode::Enabled,
            respond: Respond::Never,
            pre: &[],
            submit: ExpectSubmit::Accepted,
            result: ActionResult::failed(FailReason::StillOn),
            presses: &[(Switch::Pwr, 250), (Switch::Pwr, 4_000)],
        },
        Case {
            name: "off: no_escalate reports shutdown_pending",
            kind: ActionKind::Off,
            args: ActionArgs {
                no_escalate: true,
                ..a
            },
            led: Some(true),
            sense: SenseMode::Enabled,
            respond: Respond::Never,
            pre: &[],
            submit: ExpectSubmit::Accepted,
            result: ActionResult::ShutdownPending,
            presses: &[(Switch::Pwr, 250)],
        },
        Case {
            name: "off: unknown state is refused",
            kind: ActionKind::Off,
            args: a,
            led: None,
            sense: SenseMode::Enabled,
            respond: Respond::Never,
            pre: &[],
            submit: ExpectSubmit::Immediate,
            result: ActionResult::refused(RefuseReason::UnknownState),
            presses: &[],
        },
        Case {
            name: "off: sense ignored is a single short press",
            kind: ActionKind::Off,
            args: a,
            led: None,
            sense: SenseMode::Ignore,
            respond: Respond::Never,
            pre: &[],
            submit: ExpectSubmit::Accepted,
            result: ActionResult::Ok,
            presses: &[(Switch::Pwr, 250)],
        },
        Case {
            name: "force_off: already off, nothing to do",
            kind: ActionKind::ForceOff,
            args: a,
            led: Some(false),
            sense: SenseMode::Enabled,
            respond: Respond::Never,
            pre: &[],
            submit: ExpectSubmit::Immediate,
            result: ActionResult::Ok,
            presses: &[],
        },
        Case {
            name: "force_off: on, LED drops during the hold",
            kind: ActionKind::ForceOff,
            args: a,
            led: Some(true),
            sense: SenseMode::Enabled,
            respond: Respond::Never,
            pre: &[(1_000, Some(false))],
            submit: ExpectSubmit::Accepted,
            result: ActionResult::Ok,
            presses: &[(Switch::Pwr, 4_000)],
        },
        Case {
            name: "force_off: LED stays on for the whole hold",
            kind: ActionKind::ForceOff,
            args: a,
            led: Some(true),
            sense: SenseMode::Enabled,
            respond: Respond::Never,
            pre: &[],
            submit: ExpectSubmit::Accepted,
            result: ActionResult::failed(FailReason::StillOn),
            presses: &[(Switch::Pwr, 4_000)],
        },
        Case {
            name: "force_off: unknown state is refused",
            kind: ActionKind::ForceOff,
            args: a,
            led: None,
            sense: SenseMode::Enabled,
            respond: Respond::Never,
            pre: &[],
            submit: ExpectSubmit::Immediate,
            result: ActionResult::refused(RefuseReason::UnknownState),
            presses: &[],
        },
        Case {
            name: "force_off: sense ignored holds for the full t_hold",
            kind: ActionKind::ForceOff,
            args: a,
            led: None,
            sense: SenseMode::Ignore,
            respond: Respond::Never,
            pre: &[],
            submit: ExpectSubmit::Accepted,
            result: ActionResult::Ok,
            presses: &[(Switch::Pwr, 4_000)],
        },
        Case {
            name: "reset: on, short RST press",
            kind: ActionKind::Reset,
            args: a,
            led: Some(true),
            sense: SenseMode::Enabled,
            respond: Respond::Never,
            pre: &[],
            submit: ExpectSubmit::Accepted,
            result: ActionResult::Ok,
            presses: &[(Switch::Rst, 250)],
        },
        Case {
            name: "reset: off is rejected",
            kind: ActionKind::Reset,
            args: a,
            led: Some(false),
            sense: SenseMode::Enabled,
            respond: Respond::Never,
            pre: &[],
            submit: ExpectSubmit::Immediate,
            result: ActionResult::refused(RefuseReason::NodeOff),
            presses: &[],
        },
        Case {
            name: "reset: unknown state is refused",
            kind: ActionKind::Reset,
            args: a,
            led: None,
            sense: SenseMode::Enabled,
            respond: Respond::Never,
            pre: &[],
            submit: ExpectSubmit::Immediate,
            result: ActionResult::refused(RefuseReason::UnknownState),
            presses: &[],
        },
        Case {
            name: "reset: unknown state with force presses RST",
            kind: ActionKind::Reset,
            args: ActionArgs { force: true, ..a },
            led: None,
            sense: SenseMode::Enabled,
            respond: Respond::Never,
            pre: &[],
            submit: ExpectSubmit::Accepted,
            result: ActionResult::Ok,
            presses: &[(Switch::Rst, 250)],
        },
        Case {
            name: "cycle: on, cooperative off then on",
            kind: ActionKind::Cycle,
            args: a,
            led: Some(true),
            sense: SenseMode::Enabled,
            respond: Respond::OffThenOn(400, 400),
            pre: &[],
            submit: ExpectSubmit::Accepted,
            result: ActionResult::Ok,
            presses: &[(Switch::Pwr, 250), (Switch::Pwr, 250)],
        },
        Case {
            name: "cycle: already off skips the off half",
            kind: ActionKind::Cycle,
            args: a,
            led: Some(false),
            sense: SenseMode::Enabled,
            respond: Respond::OnAfter(300),
            pre: &[],
            submit: ExpectSubmit::Accepted,
            result: ActionResult::Ok,
            presses: &[(Switch::Pwr, 250)],
        },
        Case {
            name: "cycle hard: force_off then on",
            kind: ActionKind::Cycle,
            args: ActionArgs { hard: true, ..a },
            led: Some(true),
            sense: SenseMode::Enabled,
            respond: Respond::OffThenOn(0, 400),
            pre: &[(1_000, Some(false))],
            submit: ExpectSubmit::Accepted,
            result: ActionResult::Ok,
            presses: &[(Switch::Pwr, 4_000), (Switch::Pwr, 250)],
        },
        Case {
            name: "cycle: unknown state is refused",
            kind: ActionKind::Cycle,
            args: a,
            led: None,
            sense: SenseMode::Enabled,
            respond: Respond::Never,
            pre: &[],
            submit: ExpectSubmit::Immediate,
            result: ActionResult::refused(RefuseReason::UnknownState),
            presses: &[],
        },
        Case {
            name: "press: raw 500 ms press on an unknown node",
            kind: ActionKind::Press,
            args: ActionArgs {
                duration_ms: Some(500),
                ..a
            },
            led: None,
            sense: SenseMode::Enabled,
            respond: Respond::Never,
            pre: &[],
            submit: ExpectSubmit::Accepted,
            result: ActionResult::Ok,
            presses: &[(Switch::Pwr, 500)],
        },
        Case {
            name: "press: RST for 10 s is allowed",
            kind: ActionKind::Press,
            args: ActionArgs {
                duration_ms: Some(10_000),
                switch: Some(Switch::Rst),
                ..a
            },
            led: Some(true),
            sense: SenseMode::Enabled,
            respond: Respond::Never,
            pre: &[],
            submit: ExpectSubmit::Accepted,
            result: ActionResult::Ok,
            presses: &[(Switch::Rst, 10_000)],
        },
        Case {
            name: "press: 50 ms is refused",
            kind: ActionKind::Press,
            args: ActionArgs {
                duration_ms: Some(50),
                ..a
            },
            led: Some(true),
            sense: SenseMode::Enabled,
            respond: Respond::Never,
            pre: &[],
            submit: ExpectSubmit::Immediate,
            result: ActionResult::refused(RefuseReason::BadDuration),
            presses: &[],
        },
        Case {
            name: "press: 20 s is refused",
            kind: ActionKind::Press,
            args: ActionArgs {
                duration_ms: Some(20_000),
                ..a
            },
            led: Some(true),
            sense: SenseMode::Enabled,
            respond: Respond::Never,
            pre: &[],
            submit: ExpectSubmit::Immediate,
            result: ActionResult::refused(RefuseReason::BadDuration),
            presses: &[],
        },
        Case {
            name: "press: missing duration is refused",
            kind: ActionKind::Press,
            args: a,
            led: Some(true),
            sense: SenseMode::Enabled,
            respond: Respond::Never,
            pre: &[],
            submit: ExpectSubmit::Immediate,
            result: ActionResult::refused(RefuseReason::BadDuration),
            presses: &[],
        },
    ]
}

#[test]
fn action_state_response_table() {
    for case in cases() {
        let mut rig = Rig::new();
        rig.set_timings(fast());
        rig.set_sense(NODE, case.sense);
        rig.set_led(NODE, case.led);
        rig.set_respond(NODE, case.respond);
        for (at, value) in case.pre {
            rig.schedule_led(*at, NODE, *value);
        }

        let req = ActionRequest::new("c-1", case.kind, Target::Node(NODE)).with_args(case.args);
        let sub = rig.submit(&req);

        match case.submit {
            ExpectSubmit::Accepted => {
                assert!(
                    matches!(sub, Submission::Accepted { .. }),
                    "{}: expected the job to be queued, got {sub:?}",
                    case.name
                );
            }
            ExpectSubmit::Immediate => {
                assert_eq!(
                    sub,
                    Submission::Done(case.result.clone()),
                    "{}: wrong immediate outcome",
                    case.name
                );
            }
        }

        // Long enough for soft-off (5 s) plus an escalated hold (4 s)
        // plus every delay in a cycle.
        rig.run_to(30_000);

        if case.submit == ExpectSubmit::Accepted {
            assert_eq!(
                rig.result_of(NODE),
                Some(case.result.clone()),
                "{}: wrong final result",
                case.name
            );
        }

        let starts: Vec<(Switch, u32)> = rig
            .press_starts()
            .into_iter()
            .map(|(_, sw, dur, _)| (sw, dur))
            .collect();
        assert_eq!(
            starts,
            case.presses.to_vec(),
            "{}: wrong press sequence",
            case.name
        );

        for (_, _, dur, deadline) in rig.press_starts() {
            assert_eq!(
                deadline,
                deadline_for(dur),
                "{}: deadline must be duration + 500 ms capped at 12 s",
                case.name
            );
        }

        assert!(
            rig.act.press_owner().is_none(),
            "{}: the press lock must be free at the end",
            case.name
        );
        assert!(
            rig.act.switches_mut().closed.is_empty(),
            "{}: no relay may stay closed",
            case.name
        );
        assert!(
            rig.act.switches_mut().max_concurrent <= 1,
            "{}: only one relay may ever be closed at a time",
            case.name
        );
    }
}

#[test]
fn soft_off_emits_timeout_then_escalates() {
    let mut rig = Rig::new();
    rig.set_timings(fast());
    rig.set_led(3, Some(true));
    // The node ignores the soft-off press but dies 200 ms into the hold.
    rig.schedule_led(5_500, 3, Some(false));

    let req = ActionRequest::new("c-esc", ActionKind::Off, Target::Node(3));
    assert!(matches!(rig.submit(&req), Submission::Accepted { .. }));
    rig.run_to(20_000);

    assert!(
        rig.has_soft_off_timeout(3),
        "soft_off_timeout must be emitted"
    );
    assert!(rig.has_escalated(3), "the escalation must be announced");
    assert_eq!(rig.result_of(3), Some(ActionResult::Ok));

    let starts = rig.press_starts();
    assert_eq!(starts.len(), 2, "one soft press plus one hold");
    assert_eq!(starts[0].2, 250);
    assert_eq!(starts[1].2, 4_000, "the hold is bounded by t_hold");

    // The hold stops 500 ms after the LED went off, not at t_hold.
    let ends = rig.press_ends();
    assert!(
        ends[1].2 >= 500 && ends[1].2 < 4_000,
        "hold was {} ms, expected LED-off plus 500 ms",
        ends[1].2
    );
}

#[test]
fn deadline_is_enforced_in_software() {
    let mut rig = Rig::new();
    rig.set_timings(fast());
    rig.set_led(2, Some(true));

    let req = ActionRequest::new("c-dl", ActionKind::Reset, Target::Node(2));
    assert!(matches!(rig.submit(&req), Submission::Accepted { .. }));

    // One tick starts the press, then the task is starved past
    // 250 + 500 ms.
    rig.run_for(10);
    assert_eq!(rig.act.switches_mut().armed_ms, Some(750));
    rig.jump_to(2_000);

    assert_eq!(
        rig.result_of(2),
        Some(ActionResult::failed(FailReason::DeadlineExceeded))
    );
    let faults = rig.faults();
    assert_eq!(faults.len(), 1, "exactly one fault event");
    assert!(matches!(
        faults[0],
        ActuatorEvent::Fault {
            fault: granite_core::actuator::FaultKind::PressDeadline,
            ..
        }
    ));
    assert!(rig.act.switches_mut().closed.is_empty());
    assert!(
        rig.act.switches_mut().ops.contains(&common::Op::ReleaseAll),
        "every relay must be released when the deadline blows"
    );
}

#[test]
fn deadline_never_exceeds_twelve_seconds() {
    assert_eq!(deadline_for(250), 750);
    assert_eq!(deadline_for(8_000), 8_500);
    assert_eq!(deadline_for(10_000), 10_500);
    assert_eq!(deadline_for(12_000), 12_000);
    assert_eq!(deadline_for(60_000), 12_000);
}

#[test]
fn on_all_follows_the_configured_order_with_a_stagger() {
    let mut rig = Rig::new();
    rig.set_timings(fast());
    rig.act.set_order([3, 1, 2, 4, 5, 6, 7, 8]);
    rig.set_all_leds(Some(false));
    // Node 5 is already up and must be skipped.
    rig.set_led(5, Some(true));
    for n in 1..=8 {
        rig.set_respond(n, Respond::OnAfter(100));
    }

    let req = ActionRequest::new("c-all", ActionKind::OnAll, Target::All);
    let sub = rig.submit(&req);
    assert_eq!(sub, Submission::Accepted { jobs: 7 });

    rig.run_to(30_000);

    let order: Vec<u8> = rig.press_starts().into_iter().map(|(n, ..)| n).collect();
    assert_eq!(
        order,
        vec![3, 1, 2, 4, 6, 7, 8],
        "presses must follow the configured order, skipping the node already on"
    );
    assert_eq!(rig.group_result("c-all"), Some(ActionResult::Ok));
    assert!(
        rig.act.switches_mut().max_concurrent <= 1,
        "a stagger is still one press at a time"
    );
    for n in [1, 2, 3, 4, 6, 7, 8] {
        assert_eq!(rig.result_of(n), Some(ActionResult::Ok), "node {n}");
    }
}

#[test]
fn on_all_when_everything_is_already_on_does_nothing() {
    let mut rig = Rig::new();
    rig.set_timings(fast());
    rig.set_all_leds(Some(true));
    let req = ActionRequest::new("c-all", ActionKind::OnAll, Target::All);
    assert_eq!(rig.submit(&req), Submission::Done(ActionResult::Ok));
    rig.run_to(1_000);
    assert!(rig.press_starts().is_empty());
}

#[test]
fn on_all_reports_partial_when_a_node_is_unknown() {
    let mut rig = Rig::new();
    rig.set_timings(fast());
    rig.set_all_leds(Some(false));
    rig.set_led(6, None);
    for n in 1..=8 {
        rig.set_respond(n, Respond::OnAfter(100));
    }

    let req = ActionRequest::new("c-all", ActionKind::OnAll, Target::All);
    assert_eq!(rig.submit(&req), Submission::Accepted { jobs: 7 });
    rig.run_to(30_000);

    assert_eq!(
        rig.result_of(6),
        Some(ActionResult::refused(RefuseReason::UnknownState))
    );
    assert_eq!(
        rig.group_result("c-all"),
        Some(ActionResult::failed(FailReason::Partial))
    );
}

#[test]
fn second_action_on_a_busy_node_is_refused() {
    let mut rig = Rig::new();
    rig.set_timings(fast());
    rig.set_led(1, Some(false));

    let first = ActionRequest::new("c-1", ActionKind::On, Target::Node(1));
    assert!(matches!(rig.submit(&first), Submission::Accepted { .. }));
    let second = ActionRequest::new("c-2", ActionKind::On, Target::Node(1));
    assert_eq!(
        rig.submit(&second),
        Submission::Done(ActionResult::refused(RefuseReason::Busy))
    );
}

#[test]
fn a_soft_off_wait_does_not_block_another_node() {
    let mut rig = Rig::new();
    rig.set_timings(fast());
    rig.set_led(1, Some(true));
    rig.set_led(2, Some(false));
    rig.set_respond(2, Respond::OnAfter(100));

    // Node 1 goes into a long soft-off wait.
    let off = ActionRequest::new("c-off", ActionKind::Off, Target::Node(1));
    assert!(matches!(rig.submit(&off), Submission::Accepted { .. }));
    rig.run_for(400);

    // Node 2 is powered on while node 1 is still waiting.
    let on = ActionRequest::new("c-on", ActionKind::On, Target::Node(2));
    assert!(matches!(rig.submit(&on), Submission::Accepted { .. }));
    rig.run_for(1_000);

    assert_eq!(rig.result_of(2), Some(ActionResult::Ok));
    assert!(rig.result_of(1).is_none(), "node 1 is still shutting down");
}

#[test]
fn bad_targets_are_refused() {
    let mut rig = Rig::new();
    let req = ActionRequest::new("c-x", ActionKind::On, Target::Node(9));
    assert_eq!(
        rig.submit(&req),
        Submission::Done(ActionResult::refused(RefuseReason::BadTarget))
    );
    let req = ActionRequest::new("c-y", ActionKind::On, Target::All);
    assert_eq!(
        rig.submit(&req),
        Submission::Done(ActionResult::refused(RefuseReason::BadTarget))
    );
}

#[test]
fn a_hal_error_fails_the_action_and_releases_everything() {
    let mut rig = Rig::new();
    rig.set_timings(fast());
    rig.set_led(7, Some(true));
    rig.act.switches_mut().fail_next_assert = Some(HalError::ExpanderFault);

    let req = ActionRequest::new("c-hal", ActionKind::Reset, Target::Node(7));
    assert!(matches!(rig.submit(&req), Submission::Accepted { .. }));
    rig.run_for(100);

    assert_eq!(
        rig.result_of(7),
        Some(ActionResult::failed(FailReason::ExpanderFault))
    );
    assert_eq!(rig.faults().len(), 1);
    assert!(rig.act.switches_mut().closed.is_empty());
    assert!(rig.act.press_owner().is_none());
}

#[test]
fn abort_all_drops_every_job() {
    let mut rig = Rig::new();
    rig.set_timings(fast());
    rig.set_all_leds(Some(false));
    let req = ActionRequest::new("c-all", ActionKind::OnAll, Target::All);
    assert!(matches!(rig.submit(&req), Submission::Accepted { jobs: 8 }));
    rig.run_for(100);

    let evs = rig.act.abort_all(rig.now);
    assert_eq!(rig.act.in_flight(), 0);
    let dones = evs
        .iter()
        .filter(|e| matches!(e, ActuatorEvent::Done { .. }))
        .count();
    assert_eq!(dones, 8);
    assert!(
        evs.iter()
            .any(|e| matches!(e, ActuatorEvent::GroupDone { .. })),
        "the group must be closed out as well"
    );
}

#[test]
fn timing_validation_matches_the_adr_ranges() {
    let d = Timings::default();
    assert_eq!(d.t_short_ms, 250);
    assert_eq!(d.t_hold_ms, 8_000);
    assert_eq!(d.t_on_ms, 10_000);
    assert_eq!(d.t_soft_off_ms, 120_000);
    assert_eq!(d.t_cycle_ms, 10_000);
    assert_eq!(d.t_stagger_ms, 5_000);
    assert_eq!(d.t_settle_ms, 5_000);
    d.validate().unwrap();

    for (t, field) in [
        (
            Timings {
                t_short_ms: 99,
                ..d
            },
            "t_short_ms",
        ),
        (
            Timings {
                t_short_ms: 1_001,
                ..d
            },
            "t_short_ms",
        ),
        (
            Timings {
                t_hold_ms: 3_999,
                ..d
            },
            "t_hold_ms",
        ),
        (
            Timings {
                t_hold_ms: 10_001,
                ..d
            },
            "t_hold_ms",
        ),
    ] {
        let err = t.validate().expect_err("must be rejected");
        assert_eq!(err.range.field, field);
    }

    let clamped = Timings {
        t_short_ms: 5,
        t_hold_ms: 99_999,
        ..d
    }
    .clamped();
    assert_eq!(clamped.t_short_ms, 100);
    assert_eq!(clamped.t_hold_ms, 10_000);
    clamped.validate().unwrap();
}
