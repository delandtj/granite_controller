//! The slot actions of ADR 0002 decision 6 and the CTRL commands of
//! ADR 0003 0x40-0x51, with the press deadline of the open-questions
//! entry and the retry policy of decision 7.

#![cfg(feature = "sim")]

use granite_proto::{
    Command, Escalate, EventKind, LedMode, ResultCode, SequenceState, SlotStatus, UnitStatus,
};
use plane_logic::drivers::expander::{self, Out};
use plane_logic::sim::Sim;

/// Two populated units, booted and settled so every host LED is
/// recognised as steady on.
///
/// The blink window is shortened to 500 ms first: every wait on an LED
/// is at least the window long (ADR 0002: "the LED timeout of the
/// power-cycle sequence must be longer than that"), so the 4 s default
/// would make each sequence here some fifteen seconds of fake time.
fn settled() -> Sim {
    let mut sim = Sim::with_units(2);
    sim.boot();
    sim.fast_led(100, 500);
    sim.advance(2_000);
    for slot in 0..4u8 {
        assert_eq!(
            sim.plane.slot(slot).unwrap().led.mode,
            LedMode::On,
            "slot {slot} should read as on"
        );
    }
    sim
}

fn asserted(sim: &mut Sim, slot: u8, line: Out) -> bool {
    let (unit, b) = ((slot / 2) as usize, slot % 2 == 1);
    sim.unit(unit)
        .expander
        .asserted(expander::out_line(b, line))
}

// --- presses ----------------------------------------------------------

#[test]
fn a_press_is_held_for_its_time_and_released_on_the_deadline() {
    let mut sim = settled();
    sim.ctrl(1, &[Command::PressPwr { slot: 0, ms: 1_000 }]);
    sim.tick();
    assert_eq!(sim.result(1), Some(ResultCode::Accepted));
    assert!(asserted(&mut sim, 0, Out::Pwrd));
    assert_eq!(sim.plane.sequence_state(0), SequenceState::Pressing);

    sim.advance(900);
    assert!(asserted(&mut sim, 0, Out::Pwrd), "still inside the press");
    sim.advance(200);
    assert!(!asserted(&mut sim, 0, Out::Pwrd), "released on the deadline");
    assert_eq!(sim.result(1), Some(ResultCode::Ok));
    assert_eq!(sim.plane.sequence_state(0), SequenceState::Done);
    assert!(sim.saw(EventKind::SequenceDone));
}

#[test]
fn press_trig_toggles_only_while_a_press_runs() {
    let mut sim = settled();
    let quiet_from = sim.now_ms;
    sim.advance(2_000);
    assert_eq!(
        sim.dev.pins.toggles_between(quiet_from, sim.now_ms),
        0,
        "no press, no edges on PC6"
    );

    // A press long enough to need the retriggering throughout.
    let press_from = sim.now_ms;
    sim.ctrl(1, &[Command::PressPwr { slot: 1, ms: 2_000 }]);
    sim.advance(2_100);
    let press_to = sim.now_ms;
    let during = sim.dev.pins.toggles_between(press_from, press_to);
    assert!(
        (8..=14).contains(&during),
        "about one edge every 200 ms, got {during}"
    );
    assert!(!asserted(&mut sim, 1, Out::Pwrd));

    // And it stops again.
    sim.advance(2_000);
    assert_eq!(sim.dev.pins.toggles_between(press_to + 1, sim.now_ms), 0);
}

#[test]
fn a_press_dies_with_the_firmware_because_press_en_does() {
    let mut sim = settled();
    // The host is off, so a press is what would wake it.
    sim.fake_slot(0).node.on = false;
    sim.advance(5_000);
    sim.ctrl(1, &[Command::PressPwr { slot: 0, ms: 10_000 }]);
    sim.tick();
    assert!(asserted(&mut sim, 0, Out::Pwrd));

    // The firmware stops ticking: the expander still drives PWRD, but
    // PRESS_EN falls about a second after the last PC6 edge and the
    // photoMOS goes dark. The host therefore sees a ~1 s press, not a
    // ten second one.
    for _ in 0..40 {
        let press_en = sim.dev.pins.press_en(sim.now_ms);
        sim.dev.bus.advance(50, press_en);
        sim.now_ms += 50;
    }
    let held = sim.fake_slot(0).node.last_press_ms();
    assert!(
        (900..=1_200).contains(&held),
        "the press ended with the deadline, got {held} ms"
    );
}

#[test]
fn a_press_out_of_range_is_a_bad_argument() {
    let mut sim = settled();
    // granite-proto refuses to encode it, so the record is built by
    // hand to prove the plane checks too.
    let mut payload = [0u8; 16];
    let mut w = granite_proto::RecordWriter::new(&mut payload);
    granite_proto::Command::Seq(5).encode(&mut w).unwrap();
    w.push(granite_proto::TAG_PRESS_PWR, &[0, 0x0A, 0x00]).unwrap();
    let payload = w.payload().to_vec();
    sim.write_block(granite_proto::BLK_CTRL, &payload);
    sim.tick();
    assert_eq!(sim.result(5), Some(ResultCode::BadArgument));
}

// --- enable, latches, module run --------------------------------------

#[test]
fn enable_drives_off_and_records_it() {
    let mut sim = settled();
    assert_eq!(
        sim.ctrl_now(1, &[Command::Enable { slot: 3, on: false }]),
        ResultCode::Ok
    );
    assert!(asserted(&mut sim, 3, Out::Off));
    assert!(sim.plane.intended().off(3));
    assert!(sim.saw(EventKind::SlotOff));
    sim.advance(200);
    assert!(
        !sim.plane
            .slot(3)
            .unwrap()
            .status
            .contains(SlotStatus::ENABLED | SlotStatus::PGD)
    );

    assert_eq!(
        sim.ctrl_now(2, &[Command::Enable { slot: 3, on: true }]),
        ResultCode::Ok
    );
    assert!(!asserted(&mut sim, 3, Out::Off));
    sim.advance(200);
    assert!(sim.plane.slot(3).unwrap().status.contains(SlotStatus::PGD));
}

#[test]
fn an_ov_latch_is_cleared_only_by_its_command() {
    let mut sim = settled();
    sim.fake_slot(0).ov_latched = true;
    sim.advance(200);
    assert!(
        sim.plane.slot(0).unwrap().status.contains(SlotStatus::OV_LATCHED)
    );
    assert!(sim.saw(EventKind::OvLatched));

    // Nothing clears it on its own.
    sim.advance(5_000);
    assert!(sim.fake_slot(0).ov_latched);

    assert_eq!(
        sim.ctrl_now(1, &[Command::ClearOvLatch { slot: 0 }]),
        ResultCode::Accepted
    );
    assert!(asserted(&mut sim, 0, Out::Lclr), "the pulse is out");
    sim.advance(100);
    assert!(!asserted(&mut sim, 0, Out::Lclr), "and released again");
    assert_eq!(sim.result(1), Some(ResultCode::Ok));
    assert!(!sim.fake_slot(0).ov_latched);
}

#[test]
fn a_power_limit_trip_sets_the_latch_and_is_told_apart_from_an_ov() {
    let mut sim = settled();
    // 48 W drawn against a 20 W limit.
    sim.config_write(&[
        granite_proto::Config::PowerLimit(granite_proto::PowerLimit {
            slot: 0,
            watts: 200,
        }),
        granite_proto::Config::Commit,
    ]);
    sim.advance(500);
    assert!(
        sim.plane
            .slot(0)
            .unwrap()
            .status
            .contains(SlotStatus::POWER_LIMIT_TRIPPED),
        "DIAG_ALRT said it was the power limit"
    );
    assert!(sim.plane.slot(0).unwrap().status.contains(SlotStatus::OV_LATCHED));
    assert!(sim.saw(EventKind::PowerLimitTrip));
    assert_eq!(sim.plane.slot(0).unwrap().power_limit, 200);
}

#[test]
fn module_run_off_disarms_first_and_is_refused_without_it() {
    let mut sim = settled();
    assert_eq!(
        sim.ctrl_now(1, &[Command::ModuleRun { slot: 0, on: false }]),
        ResultCode::Ok
    );
    assert!(asserted(&mut sim, 0, Out::Disarm));
    assert!(asserted(&mut sim, 0, Out::Runoff));
    assert!(sim.plane.intended().disarm(0) && sim.plane.intended().runoff(0));
    sim.advance(200);
    assert!(
        sim.plane
            .slot(0)
            .unwrap()
            .status
            .contains(SlotStatus::MODULE_RUN_OFF | SlotStatus::DISARMED)
    );

    // On again: RUNOFF goes first, DISARM after.
    assert_eq!(
        sim.ctrl_now(2, &[Command::ModuleRun { slot: 0, on: true }]),
        ResultCode::Ok
    );
    assert!(!asserted(&mut sim, 0, Out::Runoff));
    assert!(!asserted(&mut sim, 0, Out::Disarm));

    // A DISARM line that does not hold refuses the command, and RUNOFF
    // never goes out: the PGOOD-loss trip would fire on the next
    // module stop (ADR 0002 decision 6).
    let disarm = expander::out_line(true, Out::Disarm);
    sim.unit(0).expander.wedged |= 1 << disarm;
    assert_eq!(
        sim.ctrl_now(3, &[Command::ModuleRun { slot: 1, on: false }]),
        ResultCode::Refused
    );
    assert!(!asserted(&mut sim, 1, Out::Runoff), "never RUNOFF alone");
    assert!(!sim.plane.intended().runoff(1));
    assert!(!sim.plane.intended().disarm(1));
}

// --- node on / off ----------------------------------------------------

#[test]
fn node_on_presses_a_dark_host_and_waits_for_its_led() {
    let mut sim = settled();
    sim.fake_slot(0).node.on = false;
    sim.advance(2_000);
    assert_eq!(sim.plane.slot(0).unwrap().led.mode, LedMode::Off);

    assert_eq!(
        sim.ctrl_now(
            1,
            &[Command::NodeOn {
                slot: 0,
                led_timeout_s: 2
            }]
        ),
        ResultCode::Accepted
    );
    // Dark for the timeout, then a press, then the host comes up.
    sim.advance(3_000);
    assert!(sim.fake_slot(0).node.on, "the press woke it");
    sim.advance(3_000);
    assert_eq!(sim.result(1), Some(ResultCode::Ok));
    assert_eq!(sim.plane.slot(0).unwrap().led.mode, LedMode::On);
    assert_eq!(sim.plane.sequence_state(0), SequenceState::Done);
}

#[test]
fn node_on_is_a_no_op_on_a_host_that_is_already_up() {
    let mut sim = settled();
    assert_eq!(
        sim.ctrl_now(
            1,
            &[Command::NodeOn {
                slot: 0,
                led_timeout_s: 30
            }]
        ),
        ResultCode::Ok
    );
    assert_eq!(sim.plane.sequence_state(0), SequenceState::Idle);
}

#[test]
fn node_off_is_a_short_press_when_the_host_listens() {
    let mut sim = settled();
    assert_eq!(
        sim.ctrl_now(
            1,
            &[Command::NodeOff {
                slot: 0,
                timeout_s: 10,
                escalate: Escalate::HoldPwr
            }]
        ),
        ResultCode::Accepted
    );
    sim.advance(1_000);
    assert!(!sim.fake_slot(0).node.on, "the short press was enough");
    sim.advance(5_000);
    assert_eq!(sim.result(1), Some(ResultCode::Ok));
    assert!(sim.saw(EventKind::SequenceDone));
    // The rail is untouched: only the host went down.
    assert!(!asserted(&mut sim, 0, Out::Off));
}

#[test]
fn node_off_escalate_zero_gives_up_on_a_stubborn_host() {
    let mut sim = settled();
    sim.fake_slot(0).node.ignores_short_press = true;
    assert_eq!(
        sim.ctrl_now(
            1,
            &[Command::NodeOff {
                slot: 0,
                timeout_s: 2,
                escalate: Escalate::None
            }]
        ),
        ResultCode::Accepted
    );
    sim.advance(3_000);
    assert_eq!(sim.result(1), Some(ResultCode::Failed));
    assert_eq!(sim.plane.sequence_state(0), SequenceState::Failed);
    assert!(sim.saw(EventKind::SequenceFailed));
    assert!(sim.fake_slot(0).node.on, "the host is still up");
    assert!(!asserted(&mut sim, 0, Out::Off), "the rail was not cut");
}

#[test]
fn node_off_escalate_one_holds_the_button_for_five_seconds() {
    let mut sim = settled();
    sim.fake_slot(0).node.ignores_short_press = true;
    sim.fake_slot(0).node.force_hold_ms = 4_000;
    sim.ctrl(
        1,
        &[Command::NodeOff {
            slot: 0,
            timeout_s: 1,
            escalate: Escalate::HoldPwr,
        }],
    );
    // Short press, the wait, then the five second hold.
    sim.advance(2_000);
    assert!(asserted(&mut sim, 0, Out::Pwrd), "holding PWR");
    sim.advance(5_000);
    assert!(!sim.fake_slot(0).node.on, "four seconds of PWR killed it");
    assert!(!asserted(&mut sim, 0, Out::Pwrd));
    // The LED then gets the blink window to settle before the plane
    // calls it dark.
    sim.advance(2_000);
    assert_eq!(sim.result(1), Some(ResultCode::Ok));
}

#[test]
fn node_off_escalate_three_cuts_the_rail_when_the_hold_fails() {
    let mut sim = settled();
    // A host that answers nothing: not the short press, not the hold.
    sim.fake_slot(0).node.ignores_short_press = true;
    sim.fake_slot(0).node.ignores_hold = true;
    sim.ctrl(
        1,
        &[Command::NodeOff {
            slot: 0,
            timeout_s: 1,
            escalate: Escalate::HoldPwrThenCutRail,
        }],
    );
    sim.advance(2_000);
    assert!(asserted(&mut sim, 0, Out::Pwrd));
    assert!(sim.fake_slot(0).node.on);

    // Five seconds of PWR, then the blink window to be sure, and the
    // host is still lit: the rail goes (ADR 0003 0x51 escalate 3 and
    // ADR 0004's force_off).
    sim.advance(5_500);
    assert!(!asserted(&mut sim, 0, Out::Off), "still confirming");
    sim.advance(2_000);
    assert!(asserted(&mut sim, 0, Out::Off), "the rail was cut");
    assert!(!sim.fake_slot(0).node.on);
    assert!(sim.plane.intended().off(0), "and the cut is remembered");
    sim.advance(1_500);
    assert_eq!(sim.result(1), Some(ResultCode::Ok));
    assert_eq!(sim.plane.sequence_state(0), SequenceState::Done);
    // The OFF survives a reset, like any intended state.
    sim.core_reset(plane_logic::reset_cause::WATCHDOG);
    assert!(sim.plane.intended().off(0));
}

#[test]
fn a_timeout_of_zero_escalates_at_once() {
    let mut sim = settled();
    sim.fake_slot(0).node.ignores_short_press = true;
    sim.fake_slot(0).node.ignores_hold = true;
    sim.ctrl(
        1,
        &[Command::NodeOff {
            slot: 0,
            timeout_s: 0,
            escalate: Escalate::HoldPwrThenCutRail,
        }],
    );
    sim.tick();
    // No short press and no waiting: straight to the five second hold.
    assert!(asserted(&mut sim, 0, Out::Pwrd));
    sim.advance(7_500);
    assert!(asserted(&mut sim, 0, Out::Off));
}

#[test]
fn a_sleeping_host_is_blinking_and_not_off() {
    let mut sim = settled();
    sim.fast_led(200, 2_000);
    sim.fake_slot(0).node.blink_period_ms = Some(1_000);
    sim.advance(3_000);
    assert_eq!(sim.plane.slot(0).unwrap().led.mode, LedMode::Blinking);
    let period = sim.plane.slot(0).unwrap().led.period_ms;
    assert!((800..=1_200).contains(&period), "got {period} ms");
    assert!(sim.saw(EventKind::LedChanged));
    // A blinking host still counts as showing light, so `node off`
    // presses it rather than calling it already down.
    assert_eq!(
        sim.ctrl_now(
            1,
            &[Command::NodeOff {
                slot: 0,
                timeout_s: 5,
                escalate: Escalate::None
            }]
        ),
        ResultCode::Accepted
    );
}

// --- power-cycle, stagger, unit reset ---------------------------------

#[test]
fn a_power_cycle_brings_the_rail_back_and_presses_if_dark() {
    let mut sim = settled();
    assert_eq!(
        sim.ctrl_now(
            1,
            &[Command::PowerCycle {
                slot: 0,
                off_delay_s: 2,
                press_if_dark: true,
                led_timeout_s: 3,
            }]
        ),
        ResultCode::Accepted
    );
    assert!(asserted(&mut sim, 0, Out::Off));
    sim.advance(100);
    assert!(!sim.fake_slot(0).node.on, "the host went with the rail");

    sim.advance(2_200);
    assert!(!asserted(&mut sim, 0, Out::Off), "the rail is back");
    assert!(!sim.plane.intended().off(0));

    // The host does not start on its own, so after the LED timeout the
    // plane presses PWR and the host comes up.
    sim.advance(4_000);
    assert!(sim.fake_slot(0).node.on, "the plane pressed PWR");
    sim.advance(3_000);
    assert_eq!(sim.result(1), Some(ResultCode::Ok));
    assert!(
        !sim.plane
            .slot(0)
            .unwrap()
            .cross
            .contains(granite_proto::CrossCheck::NO_LED_AFTER_CYCLE)
    );
}

#[test]
fn a_power_cycle_that_never_lights_up_fails_and_says_why() {
    let mut sim = settled();
    // No module: the rail comes back but nothing behind it.
    sim.fake_slot(0).module = false;
    sim.advance(1_000);
    sim.ctrl(
        1,
        &[Command::PowerCycle {
            slot: 0,
            off_delay_s: 1,
            press_if_dark: true,
            led_timeout_s: 1,
        }],
    );
    sim.advance(8_000);
    assert_eq!(sim.result(1), Some(ResultCode::Failed));
    assert!(
        sim.plane
            .slot(0)
            .unwrap()
            .cross
            .contains(granite_proto::CrossCheck::NO_LED_AFTER_CYCLE)
    );
    assert!(sim.saw(EventKind::SequenceFailed));
}

#[test]
fn a_stagger_releases_the_slots_one_at_a_time() {
    let mut sim = settled();
    let slots = granite_proto::IndexBitmap::NONE
        .plus(0)
        .plus(1)
        .plus(2)
        .plus(3);
    assert_eq!(
        sim.ctrl_now(
            1,
            &[Command::StaggerOn {
                slots,
                delay_ms: 500
            }]
        ),
        ResultCode::Accepted
    );
    // Everything named went off first.
    for slot in 0..4u8 {
        assert!(asserted(&mut sim, slot, Out::Off), "slot {slot}");
    }
    sim.advance(600);
    assert!(!asserted(&mut sim, 0, Out::Off));
    assert!(asserted(&mut sim, 1, Out::Off), "one at a time");
    sim.advance(600);
    assert!(!asserted(&mut sim, 1, Out::Off));
    sim.advance(1_500);
    for slot in 0..4u8 {
        assert!(!asserted(&mut sim, slot, Out::Off), "slot {slot}");
    }
    assert_eq!(sim.result(1), Some(ResultCode::Ok));
}

#[test]
fn a_unit_hot_swap_reset_pulses_u_off() {
    let mut sim = settled();
    sim.unit(0).hotswap_latched = true;
    sim.advance(200);
    assert!(
        sim.plane
            .unit(0)
            .unwrap()
            .status
            .contains(UnitStatus::HOT_SWAP_LATCHED_OFF)
    );
    assert!(sim.saw(EventKind::UnitHotSwapLatchedOff));

    assert_eq!(
        sim.ctrl_now(1, &[Command::UnitHotSwapReset { unit: 0 }]),
        ResultCode::Accepted
    );
    assert!(
        sim.unit(0).expander.asserted(expander::LINE_U_OFF),
        "U_OFF is out"
    );
    sim.advance(200);
    assert!(
        !sim.unit(0).expander.asserted(expander::LINE_U_OFF),
        "and released after the pulse"
    );
    assert!(!sim.unit(0).hotswap_latched, "the LM5069-1 is re-armed");
    assert_eq!(sim.result(1), Some(ResultCode::Ok));
    assert!(!sim.plane.intended().u_off(0));
}

#[test]
fn resetting_the_expanders_restores_the_intended_state() {
    let mut sim = settled();
    sim.ctrl_now(1, &[Command::Enable { slot: 2, on: false }]);
    sim.advance(500);
    assert!(asserted(&mut sim, 2, Out::Off));

    assert_eq!(
        sim.ctrl_now(2, &[Command::ResetExpanders]),
        ResultCode::Ok
    );
    assert_eq!(sim.dev.pins.expander_resets, 1);
    // Every expander was reconfigured straight away and the OFF slot is
    // off again (ADR 0002 open questions, CTRL 0x4D).
    assert!(asserted(&mut sim, 2, Out::Off));
    assert!(sim.saw(EventKind::UnitResetSeen));
    sim.advance(500);
    assert!(sim.plane.unit(0).unwrap().status.contains(UnitStatus::EXPANDER_OK));
}

// --- conflicts and the retry policy -----------------------------------

#[test]
fn a_second_action_on_a_busy_slot_is_a_conflict() {
    let mut sim = settled();
    sim.ctrl(1, &[Command::PressPwr { slot: 0, ms: 2_000 }]);
    sim.tick();
    assert_eq!(
        sim.ctrl_now(2, &[Command::PressRst { slot: 0, ms: 200 }]),
        ResultCode::Conflict
    );
    // Another slot is free.
    assert_eq!(
        sim.ctrl_now(3, &[Command::PressRst { slot: 1, ms: 200 }]),
        ResultCode::Accepted
    );
    // Aborting frees it.
    assert_eq!(
        sim.ctrl_now(4, &[Command::AbortSequence { slot: 0 }]),
        ResultCode::Ok
    );
    assert_eq!(sim.result(1), Some(ResultCode::Failed));
    assert!(!asserted(&mut sim, 0, Out::Pwrd));
}

#[test]
fn a_command_for_an_absent_slot_is_not_present() {
    let mut sim = Sim::with_units(1);
    sim.boot();
    sim.advance(500);
    assert_eq!(
        sim.ctrl_now(1, &[Command::Enable { slot: 6, on: false }]),
        ResultCode::NotPresent
    );
    assert_eq!(
        sim.ctrl_now(2, &[Command::UnitHotSwapReset { unit: 3 }]),
        ResultCode::NotPresent
    );
}

#[test]
fn the_retry_limit_sets_the_slot_off_and_stays_off() {
    let mut sim = settled();
    // Three drops in the window are allowed, the fourth is not.
    sim.config_write(&[
        granite_proto::Config::RetryPolicy(granite_proto::RetryPolicy {
            slot: 0,
            max: 3,
            window_s: 60,
            action: 0,
        }),
        granite_proto::Config::Commit,
    ]);
    for _ in 0..3 {
        sim.fake_slot(0).glitch_pgd = true;
        sim.advance(200);
        assert!(!sim.plane.intended().off(0), "still allowed to retry");
    }
    assert_eq!(sim.plane.slot(0).unwrap().retry_count, 3);
    assert!(sim.saw(EventKind::PgdLost));

    sim.fake_slot(0).glitch_pgd = true;
    sim.advance(200);
    assert!(sim.saw(EventKind::RetryLimitReached));
    assert!(sim.plane.intended().off(0), "past the maximum, set OFF");
    assert!(asserted(&mut sim, 0, Out::Off));
    // And it stays off: nothing re-enables it on its own.
    sim.advance(10_000);
    assert!(asserted(&mut sim, 0, Out::Off));
}

