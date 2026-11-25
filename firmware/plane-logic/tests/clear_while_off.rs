//! LCLR held while a slot is off (ADR 0002 decision 7d).
//!
//! The property these tests are about: a switched-off slot leaves its
//! module's input at about 1.2 V, the module's OV comparator then runs
//! under its minimum supply and can pull OV_KILL at any moment, so a cut
//! slot latched about 10 ms after the cut and `off` then `on` came back
//! latched three rounds out of three on the bench (2026-11-25). With
//! LCLR held for as long as OFF is high the latch cannot be set while
//! the slot is off, and the switch-on blanking of decision 7c takes over
//! from there.
//!
//! What does *not* change: a real overvoltage on a slot that is on still
//! latches, still cuts the slot and is still reported. What does change,
//! and is accepted in the ADR: a latch set while the slot was on is
//! cleared by the next cut, because the cut holds LCLR.

#![cfg(feature = "sim")]

use granite_proto::{Command, EventKind, ResultCode, SlotStatus};
use plane_logic::drivers::expander::{self, Out};
use plane_logic::sim::Sim;
use plane_logic::switch_on_blank::SWITCH_ON_BLANK_MS;

/// The switch-on window as a number of fake milliseconds.
const WINDOW: u64 = SWITCH_ON_BLANK_MS as u64;

/// True while the slot's OV latch is set on its simulated unit.
fn latch_set(sim: &mut Sim, slot: u8) -> bool {
    sim.fake_slot(slot).ov_latched
}

fn asserted(sim: &mut Sim, slot: u8, line: Out) -> bool {
    let (unit, b) = ((slot / 2) as usize, slot % 2 == 1);
    sim.unit(unit)
        .expander
        .asserted(expander::out_line(b, line))
}

/// True while slot `slot`'s LCLR line is asserted on its expander.
fn lclr_asserted(sim: &mut Sim, slot: u8) -> bool {
    asserted(sim, slot, Out::Lclr)
}

/// True while slot `slot`'s OFF line is asserted on its expander.
fn off_asserted(sim: &mut Sim, slot: u8) -> bool {
    asserted(sim, slot, Out::Off)
}

/// True while the log ring holds a line with this text.
fn logged(sim: &Sim, text: &str) -> bool {
    sim.plane
        .log()
        .iter()
        .any(|line| line.text() == text.as_bytes())
}

/// A booted unit whose modules behave as the bench's do: they pull
/// OV_KILL for `off_ms` after the slot takes VIN away, and for
/// `startup_ms` when it hands it back.
fn bench_unit(off_ms: u32, startup_ms: u32) -> Sim {
    let mut sim = Sim::with_units(1);
    sim.boot();
    sim.fast_led(100, 500);
    sim.advance(2_000);
    for slot in 0..2u8 {
        sim.fake_slot(slot).off_ov_kill_ms = off_ms;
        sim.fake_slot(slot).startup_ov_kill_ms = startup_ms;
        assert!(!latch_set(&mut sim, slot), "slot {slot} starts clean");
        assert!(sim.fake_slot(slot).pgd, "slot {slot} carries its rail");
    }
    sim
}

#[test]
fn a_cut_holds_lclr_and_the_slot_comes_back_on_unlatched() {
    // 10 ms after the cut, which is what the log ring showed, and the
    // module's start-up glitch on top of it at the switch-on.
    let mut sim = bench_unit(10, 50);

    assert_eq!(
        sim.ctrl_now(1, &[Command::Enable { slot: 0, on: false }]),
        ResultCode::Ok
    );
    assert!(off_asserted(&mut sim, 0));
    assert!(
        lclr_asserted(&mut sim, 0),
        "LCLR went up with the cut, not after it"
    );
    assert!(!lclr_asserted(&mut sim, 1), "the other half is untouched");

    // The module pulls OV_KILL while it sits there without VIN. Without
    // the hold the slot would be latched from here on; with it Q is
    // high for the width of the pull and no longer, so the poll that
    // follows reads it clear.
    //
    // The pull is stepped in 10 ms slices on purpose: the poll runs at
    // 20 Hz and the simulated hardware moves one whole step at a time,
    // so a pull stepped in 50 ms slices is *sampled* as a full cycle of
    // Q high and reported, which is what the real 20 Hz poll does when
    // it happens to land inside a pull. Either way the latch does not
    // stay set, which is what the hold is for.
    for _ in 0..200 {
        sim.step(10);
    }
    assert!(
        !latch_set(&mut sim, 0),
        "the held clear kept the latch from setting"
    );
    assert!(lclr_asserted(&mut sim, 0), "and is still held");
    assert!(!logged(&sim, "ov latched 0"));
    assert!(!logged(&sim, "latched while off 0"));
    assert!(!sim.saw(EventKind::OvLatched));

    // The switch-on is then the ordinary 7c case: the window takes over
    // and the line comes down with it.
    assert_eq!(
        sim.ctrl_now(2, &[Command::Enable { slot: 0, on: true }]),
        ResultCode::Ok
    );
    assert!(!off_asserted(&mut sim, 0));
    assert!(lclr_asserted(&mut sim, 0));
    assert!(sim.plane.switch_on_blanking(0));

    sim.advance(WINDOW + 50);
    assert!(!sim.plane.switch_on_blanking(0));
    assert!(!lclr_asserted(&mut sim, 0), "the line comes down");
    assert!(!latch_set(&mut sim, 0));
    assert!(!logged(&sim, "blank ended latched 0"));

    sim.advance(1_000);
    let status = sim.plane.slot(0).unwrap().status;
    assert!(status.contains(SlotStatus::ENABLED | SlotStatus::PGD));
    assert!(!status.contains(SlotStatus::OV_LATCHED));
    assert!(sim.fake_slot(0).pgd, "the rail is up and stayed up");
}

#[test]
fn off_then_on_three_rounds_running_comes_back_on_every_time() {
    // The bench's own sequence, which latched three rounds out of three
    // before the hold.
    let mut sim = bench_unit(10, 50);
    for round in 1..=3u8 {
        assert_eq!(
            sim.ctrl_now(2 * round, &[Command::Enable { slot: 0, on: false }]),
            ResultCode::Ok
        );
        sim.advance(1_000);
        assert!(!latch_set(&mut sim, 0), "round {round}: off and unlatched");

        assert_eq!(
            sim.ctrl_now(2 * round + 1, &[Command::Enable { slot: 0, on: true }]),
            ResultCode::Ok
        );
        sim.advance(WINDOW + 1_000);
        assert!(!latch_set(&mut sim, 0), "round {round}: on and unlatched");
        assert!(sim.fake_slot(0).pgd, "round {round}: the rail is up");
        assert!(!lclr_asserted(&mut sim, 0), "round {round}: line released");
        assert!(
            !sim.plane
                .slot(0)
                .unwrap()
                .status
                .contains(SlotStatus::OV_LATCHED),
            "round {round}: and the status agrees"
        );
    }
}

#[test]
fn a_real_overvoltage_while_the_slot_is_on_still_latches_and_is_reported() {
    let mut sim = bench_unit(10, 50);
    assert!(!lclr_asserted(&mut sim, 0), "nothing holds the line");

    sim.fake_slot(0).ov_latched = true;
    sim.advance(2_000);
    assert!(latch_set(&mut sim, 0), "the hold is not a fault reset");
    assert!(
        sim.plane
            .slot(0)
            .unwrap()
            .status
            .contains(SlotStatus::OV_LATCHED)
    );
    assert!(sim.saw(EventKind::OvLatched));
    assert!(logged(&sim, "ov latched 0"));
    assert!(!sim.fake_slot(0).pgd, "the latch cuts the slot");
    assert!(
        !lclr_asserted(&mut sim, 0),
        "a slot that is latched and not cut keeps its latch"
    );
    assert!(!logged(&sim, "latched while off 0"), "OFF is low");
}

#[test]
fn a_latch_set_while_on_is_cleared_by_the_cut() {
    // The documented behaviour change of decision 7d: the cut holds
    // LCLR, so the latch it finds set goes with it. The latch was
    // reported when it set, which is what the controller acts on.
    let mut sim = bench_unit(0, 50);
    sim.fake_slot(0).ov_latched = true;
    sim.advance(500);
    assert!(sim.saw(EventKind::OvLatched));
    assert!(latch_set(&mut sim, 0));

    assert_eq!(
        sim.ctrl_now(3, &[Command::Enable { slot: 0, on: false }]),
        ResultCode::Ok
    );
    sim.advance(200);
    assert!(off_asserted(&mut sim, 0) && lclr_asserted(&mut sim, 0));
    assert!(!latch_set(&mut sim, 0), "the cut cleared it");
    assert!(
        !sim.plane
            .slot(0)
            .unwrap()
            .status
            .contains(SlotStatus::OV_LATCHED)
    );

    // And the switch-on after it is an ordinary blanked one.
    assert_eq!(
        sim.ctrl_now(4, &[Command::Enable { slot: 0, on: true }]),
        ResultCode::Ok
    );
    sim.advance(WINDOW + 1_000);
    assert!(!latch_set(&mut sim, 0));
    assert!(sim.fake_slot(0).pgd);
}

#[test]
fn a_commanded_clear_during_a_cut_does_not_release_the_hold() {
    let mut sim = bench_unit(10, 50);
    assert_eq!(
        sim.ctrl_now(5, &[Command::Enable { slot: 0, on: false }]),
        ResultCode::Ok
    );
    sim.advance(500);
    assert!(lclr_asserted(&mut sim, 0));

    // The clear runs its own short pulse to its end and reports as it
    // always did; the line stays up because the cut still wants it.
    sim.ctrl(6, &[Command::ClearOvLatch { slot: 0 }]);
    sim.step(10);
    assert_eq!(sim.result(6), Some(ResultCode::Accepted));
    sim.step(10);
    sim.step(10);
    assert_eq!(sim.result(6), Some(ResultCode::Ok), "the pulse is over");
    assert!(
        lclr_asserted(&mut sim, 0),
        "but the cut holds the line (decision 7d)"
    );

    sim.advance(2_000);
    assert!(
        lclr_asserted(&mut sim, 0),
        "for as long as the slot is off"
    );
    assert!(!latch_set(&mut sim, 0));

    // It is the switch-on window, and nothing else, that lets go.
    assert_eq!(
        sim.ctrl_now(7, &[Command::Enable { slot: 0, on: true }]),
        ResultCode::Ok
    );
    sim.advance(WINDOW + 50);
    assert!(!lclr_asserted(&mut sim, 0));
    assert!(!latch_set(&mut sim, 0));
}

#[test]
fn a_latch_that_reads_set_while_off_is_said_once() {
    // OV_KILL stuck low while the slot is off: the hold cannot take, so
    // this is a stuck latch or a broken LCLR path and the ring says so -
    // once, not at 20 Hz.
    let mut sim = bench_unit(60_000, 50);
    assert_eq!(
        sim.ctrl_now(8, &[Command::Enable { slot: 0, on: false }]),
        ResultCode::Ok
    );
    sim.advance(2_000);

    assert!(off_asserted(&mut sim, 0) && lclr_asserted(&mut sim, 0));
    assert!(latch_set(&mut sim, 0), "the pull wins while it lasts");
    assert!(logged(&sim, "latched while off 0"));
    let said = sim
        .plane
        .log()
        .iter()
        .filter(|line| line.text() == b"latched while off 0")
        .count();
    assert_eq!(said, 1, "and says it once");
    assert!(!logged(&sim, "latched while off 1"), "not the other half");
}

#[test]
fn the_hold_comes_back_with_the_intended_state_after_a_core_reset() {
    let mut sim = bench_unit(10, 50);
    assert_eq!(
        sim.ctrl_now(9, &[Command::Enable { slot: 0, on: false }]),
        ResultCode::Ok
    );
    sim.advance(1_000);

    // A reset that keeps VDD: the expander held its outputs, so the
    // hold was never dropped.
    sim.core_reset(plane_logic::plane::reset_cause::WATCHDOG);
    sim.advance(1_000);
    assert!(off_asserted(&mut sim, 0), "the slot is still off");
    assert!(lclr_asserted(&mut sim, 0), "and still held");
    assert!(!latch_set(&mut sim, 0));

    // A unit that lost its 3V3 comes back at its hardware defaults, and
    // the intended-state re-application of decision 3 writes the hold
    // back with the OFF line.
    sim.unit(0).power_off();
    sim.advance(2_000);
    sim.unit(0).power_on();
    sim.advance(5_000);
    assert!(off_asserted(&mut sim, 0), "the slot went back off");
    assert!(lclr_asserted(&mut sim, 0), "with its hold");
    assert!(!latch_set(&mut sim, 0));
}
