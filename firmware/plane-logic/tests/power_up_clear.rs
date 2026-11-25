//! The power-up clear of the slot OV latches (ADR 0002 decision 7, note
//! of 2026-11-25; `plane_logic::power_up_clear`).
//!
//! The property these tests are about: a plane that comes up finds both
//! slots latched by the module's own OV comparator glitching while its
//! supply started, clears them once, and then never clears a latch
//! again. A real overvoltage after that stays latched and is reported
//! exactly as before, and the clear changes no intended state.

#![cfg(feature = "sim")]

use granite_proto::{Command, EventKind, SlotStatus};
use plane_logic::backup::{BackupSet, Intended};
use plane_logic::drivers::expander::{self, Out};
use plane_logic::power_up_clear::SETTLE_MS;
use plane_logic::sim::Sim;

/// A plane with `units` populated units whose slot OV latches are all
/// set, as the bench finds them at every power-up.
fn latched(units: usize) -> Sim {
    let mut sim = Sim::with_units(units);
    for slot in 0..(2 * units) as u8 {
        sim.fake_slot(slot).ov_latched = true;
    }
    sim
}

/// True while the slot's OV latch is set on its simulated unit.
fn latch_set(sim: &mut Sim, slot: u8) -> bool {
    sim.fake_slot(slot).ov_latched
}

/// True while slot `slot`'s OFF line is asserted on its expander.
fn off_line_asserted(sim: &mut Sim, slot: u8) -> bool {
    let (unit, b) = ((slot / 2) as usize, slot % 2 == 1);
    sim.unit(unit)
        .expander
        .asserted(expander::out_line(b, Out::Off))
}

/// True while the log ring holds a line with this text.
fn logged(sim: &Sim, text: &str) -> bool {
    sim.plane
        .log()
        .iter()
        .any(|line| line.text() == text.as_bytes())
}

#[test]
fn a_cold_boot_clears_both_slot_latches_once() {
    let mut sim = latched(1);
    sim.boot();

    // The first poll reads U_PGD high, which is the rising edge: the
    // clear is armed and not fired yet.
    sim.tick();
    assert!(sim.plane.power_up_clear_armed(0));
    assert!(latch_set(&mut sim, 0) && latch_set(&mut sim, 1));
    assert!(
        sim.saw(EventKind::OvLatched),
        "the latch is reported as it always was"
    );

    sim.advance(SETTLE_MS + 200);
    assert!(!sim.plane.power_up_clear_armed(0), "fired once, for both halves");
    assert!(!latch_set(&mut sim, 0) && !latch_set(&mut sim, 1));
    assert!(logged(&sim, "power-up latch clear 0"));
    assert!(logged(&sim, "power-up latch clear 1"));

    // The rails come up and the plane says so.
    sim.advance(1_000);
    for slot in 0..2u8 {
        let status = sim.plane.slot(slot).unwrap().status;
        assert!(!status.contains(SlotStatus::OV_LATCHED), "slot {slot}");
        assert!(status.contains(SlotStatus::PGD), "slot {slot}");
    }

    // Nothing of the controller's was touched: the clear is the plane's
    // own, so it leaves no command result (sequence number 0 would make
    // the plane deduplicate a real write with that number) and no
    // sequence event.
    assert!(sim.result(0).is_none());
    assert!(!sim.saw(EventKind::SequenceDone));
    assert!(!sim.saw(EventKind::SequenceFailed));
    assert!(sim.plane.intended().is_default());
}

#[test]
fn a_latch_that_sets_again_after_the_clear_stays_set() {
    let mut sim = latched(1);
    sim.boot();
    sim.advance(SETTLE_MS + 500);
    assert!(!latch_set(&mut sim, 0));

    // A real overvoltage, right after the one clear this plane does.
    sim.fake_slot(0).ov_latched = true;
    sim.advance(5_000);
    assert!(
        latch_set(&mut sim, 0),
        "the power-up clear is not a fault reset"
    );
    assert!(
        sim.plane
            .slot(0)
            .unwrap()
            .status
            .contains(SlotStatus::OV_LATCHED)
    );
    assert!(!latch_set(&mut sim, 1), "the other half is unaffected");
}

#[test]
fn a_latch_that_sets_while_the_unit_is_up_is_not_cleared() {
    let mut sim = Sim::with_units(1);
    sim.boot();
    sim.advance(10_000);
    assert!(!sim.plane.power_up_clear_armed(0));

    sim.fake_slot(1).ov_latched = true;
    sim.advance(10_000);
    assert!(latch_set(&mut sim, 1));
    assert!(
        sim.plane
            .slot(1)
            .unwrap()
            .status
            .contains(SlotStatus::OV_LATCHED)
    );
    assert!(sim.saw(EventKind::OvLatched));
}

#[test]
fn the_input_dropping_and_coming_back_earns_one_more_clear() {
    let mut sim = latched(1);
    sim.boot();
    sim.advance(SETTLE_MS + 500);
    assert!(!latch_set(&mut sim, 0));

    // The bus goes away: the core reads no volts either, so the unit
    // hot-swap latch stays out of it (`plane_logic::unit_latch`: U_PGD
    // low with the input out of range is the bus, not a fault).
    sim.dev.pins.core_volts = 0;
    sim.unit(0).input_mv = 0;
    sim.advance(2_000);
    assert!(!sim.plane.unit_held_off(0));

    // It comes back, and with it the module's start-up glitch.
    sim.fake_slot(0).ov_latched = true;
    sim.fake_slot(1).ov_latched = true;
    sim.dev.pins.core_volts = 4_800;
    sim.unit(0).input_mv = 48_000;
    sim.advance(SETTLE_MS + 500);
    assert!(!latch_set(&mut sim, 0) && !latch_set(&mut sim, 1));
    assert!(!sim.plane.power_up_clear_armed(0));
}

#[test]
fn the_clear_leaves_a_slot_that_was_commanded_off_off() {
    let mut sim = latched(1);
    sim.boot();
    sim.advance(SETTLE_MS + 500);
    sim.ctrl_now(7, &[Command::Enable { slot: 0, on: false }]);
    sim.advance(1_000);
    assert!(off_line_asserted(&mut sim, 0));

    // The unit's input restarts, so the clear runs again.
    sim.dev.pins.core_volts = 0;
    sim.unit(0).input_mv = 0;
    sim.advance(2_000);
    sim.fake_slot(0).ov_latched = true;
    sim.dev.pins.core_volts = 4_800;
    sim.unit(0).input_mv = 48_000;
    sim.advance(SETTLE_MS + 500);

    assert!(!latch_set(&mut sim, 0), "the latch was cleared");
    assert!(
        off_line_asserted(&mut sim, 0),
        "LCLR touches the latch and nothing else"
    );
    assert!(sim.plane.intended().off(0));
    assert!(!sim.fake_slot(0).pgd, "the rail stays down");
    assert!(sim.fake_slot(1).pgd, "the other half runs");
}

#[test]
fn a_unit_the_core_holds_off_gets_no_clear() {
    let mut sim = latched(1);
    // The core came back from a reset still holding unit 0 off
    // (`plane_logic::unit_latch`: U_OFF in the intended state is that
    // latch and nothing else writes it).
    let mut intended = Intended::hardware_defaults();
    intended.set_u_off(0, true);
    BackupSet {
        intended,
        boot_count: 3,
        flags: 0,
    }
    .store(&mut sim.dev.backup);
    sim.boot();
    assert!(sim.plane.unit_held_off(0));

    sim.advance(5_000);
    assert!(!sim.plane.power_up_clear_armed(0));
    assert!(
        latch_set(&mut sim, 0) && latch_set(&mut sim, 1),
        "nothing was cleared on a unit that carries no power"
    );
    assert!(!logged(&sim, "power-up latch clear 0"));
}

#[test]
fn a_busy_slot_is_skipped_rather_than_cleared_later() {
    let mut sim = latched(1);
    sim.boot();
    // The first tick arms the clear; the press starts on the next one,
    // before the settle time is out.
    sim.tick();
    sim.ctrl(1, &[Command::PressPwr { slot: 0, ms: 5_000 }]);
    sim.advance(SETTLE_MS + 200);

    assert!(logged(&sim, "clear skipped, slot busy 0"));
    assert!(latch_set(&mut sim, 0), "slot 0 keeps its latch");
    assert!(!latch_set(&mut sim, 1), "slot 1 was cleared");
    assert!(
        !sim.plane.power_up_clear_armed(0),
        "the clear is once per edge, deferred for nobody"
    );

    // The press finishes normally: the skip cost the command nothing.
    sim.advance(6_000);
    assert_eq!(sim.result(1), Some(granite_proto::ResultCode::Ok));
    assert!(latch_set(&mut sim, 0));
}
