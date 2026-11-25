//! What the plane reads off the host power-LED sense, and which
//! cross-check a slot with nothing plugged into its front-panel header
//! can and cannot raise (ADR 0003 SLOT 0x2C).
//!
//! The sense is an opto, TLP291 U8 / U58 on the unit board
//! (power/pp-48-fd/tools/sch/blocks/unit.py): the host's LED+ feeds its
//! LED through R21 220R, its collector is the expander's A_LED / B_LED
//! with a 47k pull-up to 3V3 and its emitter is GND, so the line is
//! **low while the host's LED is lit** and high otherwise. An open
//! front-panel cable passes no current, so the opto is dark and the
//! line reads high: the same reading as a host whose LED is off, which
//! is the right answer - neither shows light.
//!
//! These tests pin that, because the first unit-fd on the bench
//! (2026-11-25) reported `cross_check` on both slots with no node and no
//! front-panel cable, and the two bits that come off this line were the
//! first suspects.

#![cfg(feature = "sim")]

use granite_proto::{Command, CrossCheck, LedMode, ResultCode, SlotStatus};
use plane_logic::sim::Sim;

/// A plane with one unit whose slots carry a module but no host and no
/// front-panel cable: the LED line reads high throughout.
fn no_hosts() -> Sim {
    let mut sim = Sim::with_units(1);
    for slot in 0..2u8 {
        let node = &mut sim.fake_slot(slot).node;
        node.on = false;
        // Nothing is plugged into the front-panel header, so closing
        // PWRD reaches no host: a press of any length does nothing and
        // the LED line never goes low.
        node.min_press_ms = u32::MAX;
    }
    sim.boot();
    // A short blink window, so an LED is called steady in a test's worth
    // of time rather than the default four seconds.
    sim.fast_led(200, 500);
    sim.advance(3_000);
    sim
}

#[test]
fn an_open_front_panel_cable_reads_as_no_light() {
    let sim = no_hosts();
    for slot in 0..2u8 {
        let model = sim.plane.slot(slot).unwrap();
        assert_eq!(model.led.mode, LedMode::Off, "slot {slot}");
        assert!(!model.led.lit, "slot {slot}");
        assert_eq!(model.led.period_ms, 0, "slot {slot}");
    }
}

#[test]
fn an_open_cable_raises_neither_led_cross_check() {
    let mut sim = no_hosts();

    // The rail is on and the module draws current: the "LED at zero
    // current" check needs light, and there is none.
    sim.fake_slot(0).ina.current_ma = 1_000;
    sim.advance(1_000);
    assert!(sim.plane.slot(0).unwrap().status.contains(SlotStatus::ENABLED));
    assert!(
        sim.plane.slot(0).unwrap().cross.is_empty(),
        "nothing to cross-check: {:?}",
        sim.plane.slot(0).unwrap().cross
    );

    // And with the slot commanded off, which is the state the bench
    // reported `cross_check` in.
    assert_eq!(
        sim.ctrl_now(1, &[Command::Enable { slot: 0, on: false }]),
        ResultCode::Ok
    );
    sim.advance(3_000);
    let cross = sim.plane.slot(0).unwrap().cross;
    assert!(
        !cross.contains(CrossCheck::LED_WHILE_OFF),
        "an open cable is not a lit LED"
    );
    assert!(!cross.contains(CrossCheck::LED_AT_ZERO_CURRENT));
    assert!(cross.is_empty());
}

/// The one cross-check a slot with no host *can* carry, and the fact
/// that it is sticky: this is what the bench saw.
///
/// `node on` with nothing behind the slot waits out its LED timeout,
/// presses PWR, waits again and then sets NO_LED_AFTER_CYCLE. Nothing
/// clears that bit except a later wait that does see light, or the unit
/// going away, so it survives the slot being switched off again - and a
/// slot that will never have a host keeps it for ever.
#[test]
fn a_node_on_with_no_host_leaves_a_sticky_no_led_cross_check() {
    let mut sim = no_hosts();
    assert!(sim.plane.slot(0).unwrap().cross.is_empty());

    sim.ctrl(
        1,
        &[Command::NodeOn {
            slot: 0,
            led_timeout_s: 1,
        }],
    );
    sim.advance(10_000);
    assert_eq!(sim.result(1), Some(ResultCode::Failed));
    let cross = sim.plane.slot(0).unwrap().cross;
    assert_eq!(
        cross,
        CrossCheck::NO_LED_AFTER_CYCLE,
        "the only bit an empty slot can raise"
    );

    // Switch the slot off: the bit stays, so the controller still
    // reports `cross_check` on a slot that is off with no host - which
    // is exactly the bench reading of 2026-11-25.
    sim.ctrl_now(2, &[Command::Enable { slot: 0, on: false }]);
    sim.advance(5_000);
    assert!(
        sim.plane
            .slot(0)
            .unwrap()
            .cross
            .contains(CrossCheck::NO_LED_AFTER_CYCLE),
        "sticky until a cycle sees light"
    );
    assert!(
        sim.plane.slot(1).unwrap().cross.is_empty(),
        "the other half was never asked to come on"
    );
}
