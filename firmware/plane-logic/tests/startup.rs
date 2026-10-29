//! The start-up rule of ADR 0002 decision 3, and what a unit glitch
//! does to it (decision 4).
//!
//! The property these tests are really about: an operator who set a slot
//! OFF finds it OFF after a watchdog reset, a commanded reboot, a
//! firmware swap and a unit that lost its 3V3 and came back. Only a
//! *core* power loss puts it back ON, and then the plane says so.

#![cfg(feature = "sim")]

use granite_proto::{Command, EventKind, StatusFlags, UnitStatus};
use plane_logic::drivers::expander::{self, Out};
use plane_logic::reset_cause;
use plane_logic::sim::Sim;

/// A plane with two populated units, booted and settled so every host
/// LED is recognised as steady.
fn settled(units: usize) -> Sim {
    let mut sim = Sim::with_units(units);
    sim.boot();
    sim.advance(5_000);
    sim
}

/// True while slot `slot`'s OFF line is asserted on its expander.
fn off_line_asserted(sim: &mut Sim, slot: u8) -> bool {
    let (unit, b) = ((slot / 2) as usize, slot % 2 == 1);
    sim.unit(unit)
        .expander
        .asserted(expander::out_line(b, Out::Off))
}

#[test]
fn a_slot_set_off_survives_a_watchdog_reset() {
    let mut sim = settled(1);
    assert_eq!(sim.ctrl_now(1, &[Command::Enable { slot: 0, on: false }]), granite_proto::ResultCode::Ok);
    sim.advance(1_000);
    assert!(off_line_asserted(&mut sim, 0));
    assert!(!sim.fake_slot(0).node.on, "the host went down with the rail");

    // The core resets with VDD kept: the expanders held their outputs
    // and the backup registers are valid.
    sim.core_reset(reset_cause::WATCHDOG);
    assert!(sim.plane.intended().off(0));
    assert!(
        !sim.plane.status_flags().contains(StatusFlags::INTENDED_STATE_LOST),
        "nothing was lost"
    );

    // Nothing switched: the rail never came back, so neither did the
    // host (ADR 0002 decision 3, "Nothing switches").
    sim.advance(5_000);
    assert!(off_line_asserted(&mut sim, 0));
    assert!(!sim.fake_slot(0).node.on);
    assert!(sim.fake_slot(1).node.on, "the other slot is untouched");
    assert!(
        !sim.saw(EventKind::UnitResetSeen),
        "the expander kept its configuration, so nothing was reset"
    );
}

#[test]
fn a_core_power_loss_loses_the_intended_state_and_says_so() {
    let mut sim = settled(1);
    sim.ctrl_now(1, &[Command::Enable { slot: 0, on: false }]);
    sim.advance(1_000);
    assert!(off_line_asserted(&mut sim, 0));

    sim.core_power_loss();
    assert!(!sim.plane.intended().off(0), "hardware defaults");
    assert!(
        sim.plane.status_flags().contains(StatusFlags::INTENDED_STATE_LOST),
        "the controller is told to re-apply"
    );
    // The expander still held OFF, but with no backup set the plane has
    // no reason to keep it: the slot comes back ON, which is what the
    // ADR says and why the flag exists.
    sim.advance(5_000);
    assert!(!off_line_asserted(&mut sim, 0));
    // The rail is back; the host itself needs a press, which is the
    // controller's business (controller ADR 0004 boot policy).
    assert!(sim.fake_slot(0).pgd);
    assert!(
        sim.plane
            .slot(0)
            .unwrap()
            .status
            .contains(granite_proto::SlotStatus::ENABLED)
    );
}

#[test]
fn a_crash_in_a_press_releases_the_button_at_the_next_boot() {
    let mut sim = settled(1);
    sim.ctrl(1, &[Command::PressPwr { slot: 0, ms: 10_000 }]);
    sim.advance(500);
    let pwrd = expander::out_line(false, Out::Pwrd);
    assert!(sim.unit(0).expander.asserted(pwrd), "the button is held");

    // The core dies mid-press. The expander keeps driving PWRD, and
    // only the firmware coming back releases it.
    sim.core_reset(reset_cause::WATCHDOG);
    assert!(
        !sim.unit(0).expander.asserted(pwrd),
        "boot released the photoMOS before it read anything"
    );
    assert!(sim.plane.intended().is_default());
}

#[test]
fn a_unit_that_lost_its_supply_gets_its_off_slots_back_off() {
    let mut sim = settled(2);
    sim.ctrl_now(1, &[Command::Enable { slot: 2, on: false }]);
    sim.advance(1_000);
    assert!(off_line_asserted(&mut sim, 2));

    // Unit 1's chain cable comes out: no 3V3, the expander resets to
    // all inputs, and slot 2's rail comes up on its own.
    sim.unit(1).power_off();
    sim.advance(2_000);
    assert!(
        sim.plane.unit(1).unwrap().status.contains(UnitStatus::LOST),
        "two NAKs lose the unit"
    );
    assert!(
        !off_line_asserted(&mut sim, 2),
        "with no expander the hardware default is ON, as the ADR says"
    );

    // The cable goes back in.
    sim.unit(1).power_on();
    sim.advance(3_000);
    assert!(sim.plane.unit(1).unwrap().present());
    assert!(
        sim.plane.unit(1).unwrap().status.contains(UnitStatus::RESET_SEEN),
        "the unit came back at hardware defaults"
    );
    assert!(sim.saw(EventKind::UnitResetSeen));
    assert!(sim.saw(EventKind::UnitLost));
    assert!(sim.saw(EventKind::UnitPresent));
    assert!(
        off_line_asserted(&mut sim, 2),
        "an OFF slot goes back OFF (ADR 0002 decision 4)"
    );
    assert!(sim.plane.intended().off(2));
    // The slot's partner on the same unit is untouched.
    assert!(!off_line_asserted(&mut sim, 3));
}

#[test]
fn the_boot_count_rises_and_the_boot_event_is_raised() {
    let mut sim = settled(1);
    assert_eq!(sim.plane.boot_count(), 1);
    assert!(sim.saw(EventKind::Boot));
    sim.core_reset(reset_cause::SOFTWARE);
    assert_eq!(sim.plane.boot_count(), 2);
    sim.core_power_loss();
    assert_eq!(sim.plane.boot_count(), 1, "a power loss takes the count too");
}

#[test]
fn a_commanded_reboot_saves_the_intended_state_first() {
    let mut sim = settled(1);
    sim.ctrl_now(1, &[Command::Enable { slot: 1, on: false }]);
    assert!(!sim.plane.reset_requested());
    assert_eq!(
        sim.ctrl_now(2, &[Command::Reboot]),
        granite_proto::ResultCode::Ok
    );
    assert!(sim.plane.reset_requested(), "the firmware does the reset");
    // The firmware resets; the new image adopts what was saved.
    sim.core_reset(reset_cause::SOFTWARE);
    assert!(sim.plane.intended().off(1));
    assert!(!sim.plane.status_flags().contains(StatusFlags::INTENDED_STATE_LOST));
}
