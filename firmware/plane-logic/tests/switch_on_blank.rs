//! Switch-on blanking of a slot's OV latch (ADR 0002 decision 7c;
//! `plane_logic::switch_on_blank`).
//!
//! The property these tests are about: every time the plane switches a
//! slot's rail on, the module behind it gets VIN for the first time since
//! it last lost it and glitches OV_KILL low while its own comparator
//! supply comes up. With LCLR held across that glitch the slot ends ON
//! and unlatched; without, it latches at once, which is what the bench
//! saw on 2026-11-25. Everything else is unchanged: a glitch wider than
//! the window latches, a latch that is already set is neither blanked nor
//! cleared, a real overvoltage after the window latches and is reported
//! as before, and the commanded clear keeps its own short pulse.

#![cfg(feature = "sim")]

use granite_proto::{Command, EventKind, ResultCode, SequenceState, SlotStatus};
use plane_logic::drivers::expander::{self, Out};
use plane_logic::sim::Sim;
use plane_logic::switch_on_blank::SWITCH_ON_BLANK_MS;

/// The window as a number of fake milliseconds.
const WINDOW: u64 = SWITCH_ON_BLANK_MS as u64;

/// True while the slot's OV latch is set on its simulated unit.
fn latch_set(sim: &mut Sim, slot: u8) -> bool {
    sim.fake_slot(slot).ov_latched
}

/// True while slot `slot`'s LCLR line is asserted on its expander.
fn lclr_asserted(sim: &mut Sim, slot: u8) -> bool {
    asserted(sim, slot, Out::Lclr)
}

/// True while slot `slot`'s OFF line is asserted on its expander.
fn off_asserted(sim: &mut Sim, slot: u8) -> bool {
    asserted(sim, slot, Out::Off)
}

fn asserted(sim: &mut Sim, slot: u8, line: Out) -> bool {
    let (unit, b) = ((slot / 2) as usize, slot % 2 == 1);
    sim.unit(unit)
        .expander
        .asserted(expander::out_line(b, line))
}

/// True while the log ring holds a line with this text.
fn logged(sim: &Sim, text: &str) -> bool {
    sim.plane
        .log()
        .iter()
        .any(|line| line.text() == text.as_bytes())
}

/// A booted unit with a start-up glitch of `glitch_ms` armed on every
/// slot, the blink window shortened and the power-up clear behind it.
fn with_glitch(glitch_ms: u32) -> Sim {
    let mut sim = Sim::with_units(1);
    sim.boot();
    sim.fast_led(100, 500);
    sim.advance(2_000);
    for slot in 0..2u8 {
        sim.fake_slot(slot).startup_ov_kill_ms = glitch_ms;
        assert!(!latch_set(&mut sim, slot), "slot {slot} starts clean");
    }
    sim
}

/// The same, with slot 0 switched off for long enough that the module's
/// VAUX has decayed: the next time the slot hands it VIN its comparator
/// really does start from scratch.
fn off_with_glitch(glitch_ms: u32) -> Sim {
    let mut sim = with_glitch(glitch_ms);
    assert_eq!(
        sim.ctrl_now(1, &[Command::Enable { slot: 0, on: false }]),
        ResultCode::Ok
    );
    sim.advance(1_000);
    assert!(off_asserted(&mut sim, 0));
    assert!(!latch_set(&mut sim, 0));
    sim
}

#[test]
fn a_switch_on_rides_out_the_modules_startup_glitch() {
    let mut sim = off_with_glitch(50);

    assert_eq!(
        sim.ctrl_now(2, &[Command::Enable { slot: 0, on: true }]),
        ResultCode::Ok
    );
    // LCLR went out with the OFF release and not after it.
    assert!(!off_asserted(&mut sim, 0));
    assert!(lclr_asserted(&mut sim, 0));
    assert!(sim.plane.switch_on_blanking(0));

    sim.advance(WINDOW + 50);
    assert!(!sim.plane.switch_on_blanking(0));
    assert!(
        !lclr_asserted(&mut sim, 0),
        "the line comes down with the window"
    );
    assert!(!latch_set(&mut sim, 0), "the glitch did not stick");
    assert!(!logged(&sim, "blank ended latched 0"));

    sim.advance(1_000);
    let status = sim.plane.slot(0).unwrap().status;
    assert!(status.contains(SlotStatus::ENABLED | SlotStatus::PGD));
    assert!(!status.contains(SlotStatus::OV_LATCHED));
    assert!(sim.fake_slot(0).pgd, "the rail is up");
    assert!(!latch_set(&mut sim, 1), "the other half was not touched");

    // The blanking is the switch-on's own: no result under a sequence
    // number of its own, no event, no sequence on the slot.
    assert_eq!(sim.plane.sequence_state(0), SequenceState::Idle);
    assert!(!sim.saw(EventKind::SequenceDone));
    assert!(!sim.saw(EventKind::SequenceFailed));
}

#[test]
fn a_glitch_wider_than_the_window_latches_and_says_so() {
    let mut sim = off_with_glitch(4 * SWITCH_ON_BLANK_MS);

    assert_eq!(
        sim.ctrl_now(2, &[Command::Enable { slot: 0, on: true }]),
        ResultCode::Ok
    );
    assert!(lclr_asserted(&mut sim, 0));

    sim.advance(WINDOW + 50);
    assert!(!sim.plane.switch_on_blanking(0));
    assert!(!lclr_asserted(&mut sim, 0));
    assert!(
        latch_set(&mut sim, 0),
        "OV_KILL was still low when the line came down"
    );
    assert!(
        logged(&sim, "blank ended latched 0"),
        "and the ring says so"
    );

    // From here it is an ordinary latched slot: nothing clears it.
    sim.advance(5_000);
    assert!(latch_set(&mut sim, 0));
    assert!(
        sim.plane
            .slot(0)
            .unwrap()
            .status
            .contains(SlotStatus::OV_LATCHED)
    );
    assert!(sim.saw(EventKind::OvLatched));
    assert!(!sim.fake_slot(0).pgd, "the latch keeps the slot cut");
}

#[test]
fn a_slot_that_is_already_latched_is_not_blanked_and_not_cleared() {
    // A latch on a slot that is *not* cut: since decision 7d a cut
    // clears the latch, so the slot this is about is one whose rail was
    // never taken away (see `a_latch_set_while_on_is_cleared_by_the_cut`
    // in tests/clear_while_off.rs for the other half).
    let mut sim = with_glitch(50);
    sim.fake_slot(0).ov_latched = true;
    sim.advance(200);
    assert!(
        sim.plane
            .slot(0)
            .unwrap()
            .status
            .contains(SlotStatus::OV_LATCHED)
    );

    // The `on` goes through and reports exactly as it always did.
    assert_eq!(
        sim.ctrl_now(3, &[Command::Enable { slot: 0, on: true }]),
        ResultCode::Ok
    );
    assert!(!off_asserted(&mut sim, 0));
    assert!(
        !lclr_asserted(&mut sim, 0),
        "a latch is cleared by clear_latch, by a cut and by nothing else"
    );
    assert!(!sim.plane.switch_on_blanking(0));

    sim.advance(1_000);
    assert!(latch_set(&mut sim, 0));
    assert!(!lclr_asserted(&mut sim, 0));
    assert!(!sim.fake_slot(0).pgd, "the latch still cuts the slot");
    let status = sim.plane.slot(0).unwrap().status;
    assert!(status.contains(SlotStatus::ENABLED | SlotStatus::OV_LATCHED));
}

#[test]
fn node_on_blanks_the_rail_it_brings_up_and_its_sequence_runs_on() {
    let mut sim = off_with_glitch(50);

    assert_eq!(
        sim.ctrl_now(
            4,
            &[Command::NodeOn {
                slot: 0,
                led_timeout_s: 1
            }]
        ),
        ResultCode::Accepted
    );
    assert!(lclr_asserted(&mut sim, 0));
    assert!(sim.plane.switch_on_blanking(0));
    assert_eq!(
        sim.plane.sequence_state(0),
        SequenceState::OnWait,
        "the LED wait runs while LCLR is held"
    );

    sim.advance(WINDOW + 50);
    assert!(!lclr_asserted(&mut sim, 0));
    assert!(!latch_set(&mut sim, 0));

    // And the wake press still comes, after the window rather than
    // instead of it.
    sim.advance(6_000);
    assert_eq!(sim.result(4), Some(ResultCode::Ok));
    assert!(sim.fake_slot(0).node.on, "the host was woken");
}

#[test]
fn the_power_cycles_rail_on_is_blanked_too() {
    let mut sim = with_glitch(50);

    assert_eq!(
        sim.ctrl_now(
            5,
            &[Command::PowerCycle {
                slot: 0,
                // Longer than the module's VAUX hold, so its comparator
                // really starts again.
                off_delay_s: 1,
                press_if_dark: true,
                led_timeout_s: 1
            }]
        ),
        ResultCode::Accepted
    );
    assert!(off_asserted(&mut sim, 0));
    assert!(
        lclr_asserted(&mut sim, 0),
        "the cut holds LCLR for as long as it lasts (decision 7d)"
    );

    // Find the tick the rail comes back on.
    let mut back = None;
    for _ in 0..40 {
        sim.tick();
        if !off_asserted(&mut sim, 0) {
            back = Some(sim.now_ms);
            break;
        }
    }
    assert!(back.is_some(), "the power-cycle brings the rail back");
    assert!(
        lclr_asserted(&mut sim, 0),
        "LCLR is up with the OFF release"
    );
    assert!(sim.plane.switch_on_blanking(0));

    sim.advance(WINDOW + 50);
    assert!(!lclr_asserted(&mut sim, 0));
    assert!(!latch_set(&mut sim, 0), "the glitch did not stick");
    assert!(!logged(&sim, "blank ended latched 0"));

    sim.advance(8_000);
    assert_eq!(sim.result(5), Some(ResultCode::Ok));
    assert!(sim.fake_slot(0).pgd);
}

#[test]
fn a_stagger_blanks_every_slot_it_releases() {
    let mut sim = with_glitch(50);
    let slots = granite_proto::IndexBitmap::NONE.plus(0).plus(1);

    assert_eq!(
        sim.ctrl_now(
            6,
            &[Command::StaggerOn {
                slots,
                // Longer than the module's VAUX hold, so both slots'
                // comparators start again.
                delay_ms: 600
            }]
        ),
        ResultCode::Accepted
    );
    assert!(off_asserted(&mut sim, 0) && off_asserted(&mut sim, 1));

    sim.advance(600);
    assert!(!off_asserted(&mut sim, 0));
    assert!(lclr_asserted(&mut sim, 0));
    assert!(off_asserted(&mut sim, 1), "one at a time");

    sim.advance(600);
    assert!(!off_asserted(&mut sim, 1));
    assert!(lclr_asserted(&mut sim, 1));

    sim.advance(1_000);
    for slot in 0..2u8 {
        assert!(!latch_set(&mut sim, slot), "slot {slot} rode out its glitch");
        assert!(!lclr_asserted(&mut sim, slot), "slot {slot}");
        assert!(sim.fake_slot(slot).pgd, "slot {slot} carries its rail");
    }
    assert_eq!(sim.result(6), Some(ResultCode::Ok));
}

#[test]
fn a_real_overvoltage_after_the_window_latches_as_it_always_did() {
    let mut sim = off_with_glitch(50);
    sim.ctrl_now(7, &[Command::Enable { slot: 0, on: true }]);
    sim.advance(WINDOW + 500);
    assert!(!latch_set(&mut sim, 0) && sim.fake_slot(0).pgd);

    sim.fake_slot(0).ov_latched = true;
    sim.advance(2_000);
    assert!(latch_set(&mut sim, 0), "the blanking is not a fault reset");
    assert!(
        sim.plane
            .slot(0)
            .unwrap()
            .status
            .contains(SlotStatus::OV_LATCHED)
    );
    assert!(sim.saw(EventKind::OvLatched));
    assert!(!sim.fake_slot(0).pgd);
}

#[test]
fn the_commanded_clear_keeps_its_own_pulse_and_neither_cuts_the_other() {
    // No glitch: the only things holding LCLR here are the window and
    // the command, which is what this test is about.
    let mut sim = off_with_glitch(0);
    sim.ctrl_now(8, &[Command::Enable { slot: 0, on: true }]);
    assert!(lclr_asserted(&mut sim, 0));

    // A commanded clear 10 ms into the window: its own pulse
    // (sequencer::LCLR_PULSE_MS) is over long before the window is, and
    // the line stays up for the window's sake. The steps are 10 ms so
    // the two deadlines do not land on the same tick.
    sim.ctrl(9, &[Command::ClearOvLatch { slot: 0 }]);
    sim.step(10);
    assert_eq!(sim.result(9), Some(ResultCode::Accepted));
    sim.step(10);
    sim.step(10);
    assert_eq!(sim.result(9), Some(ResultCode::Ok), "the pulse is over");
    assert_eq!(sim.plane.sequence_state(0), SequenceState::Done);
    assert!(
        lclr_asserted(&mut sim, 0),
        "but the window still holds the line"
    );
    assert!(sim.plane.switch_on_blanking(0));

    sim.advance(WINDOW);
    assert!(!sim.plane.switch_on_blanking(0));
    assert!(!lclr_asserted(&mut sim, 0), "and lets it go at its deadline");

    // A clear on a settled slot is the ADR's short pulse again, with no
    // window anywhere near it.
    sim.advance(1_000);
    sim.fake_slot(0).ov_latched = true;
    sim.advance(200);
    assert_eq!(
        sim.ctrl_now(10, &[Command::ClearOvLatch { slot: 0 }]),
        ResultCode::Accepted
    );
    assert!(lclr_asserted(&mut sim, 0));
    sim.tick();
    assert!(
        !lclr_asserted(&mut sim, 0),
        "the commanded pulse is one cycle"
    );
    assert!(!latch_set(&mut sim, 0));
}
