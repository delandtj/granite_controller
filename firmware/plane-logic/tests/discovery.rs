//! Discovery, the 2 s re-scan, losing a unit and the stuck-bus recovery
//! (ADR 0002 decision 4), plus the poll schedule of decision 5.

#![cfg(feature = "sim")]

use granite_proto::{EventKind, SlotStatus, StatusFlags, UnitStatus};
use plane_logic::sim::{FakeUnit, Sim};

#[test]
fn only_the_channels_with_a_unit_are_found() {
    // Units on channels 0 and 2, nothing on 1 and 3.
    let mut sim = Sim::with_units(0);
    sim.dev.bus.units[0] = Some(FakeUnit::populated());
    sim.dev.bus.units[2] = Some(FakeUnit::populated());
    sim.boot();
    assert_eq!(sim.plane.units_present().bits(), 0b0101);
    assert!(sim.plane.unit(0).unwrap().present());
    assert!(!sim.plane.unit(1).unwrap().present());

    // An absent channel's slots read as not present, and the block is
    // one status record rather than a NAK (ADR 0003 decision 2).
    let payload = sim.read_block(granite_proto::BLK_SLOT, 2);
    assert_eq!(payload.len(), 4);
    let payload = sim.read_block(granite_proto::BLK_UNIT, 1);
    assert_eq!(payload.len(), 4);
    match granite_proto::Unit::records(&payload).next().unwrap().unwrap() {
        granite_proto::Unit::Status(s) => assert!(!s.contains(UnitStatus::PRESENT)),
        other => panic!("not a status: {other:?}"),
    }
}

#[test]
fn a_unit_plugged_in_later_is_found_by_the_rescan() {
    let mut sim = Sim::with_units(1);
    sim.boot();
    sim.advance(1_000);
    assert_eq!(sim.plane.units_present().bits(), 0b0001);

    sim.dev.bus.units[1] = Some(FakeUnit::populated());
    // Nothing yet: the re-scan is every 2 s.
    sim.advance(500);
    assert_eq!(sim.plane.units_present().bits(), 0b0001);
    sim.advance(2_500);
    assert_eq!(sim.plane.units_present().bits(), 0b0011);
    assert!(sim.saw(EventKind::UnitPresent));
    // It is configured, so its slots report telemetry straight away.
    sim.advance(200);
    assert_eq!(sim.plane.slot(2).unwrap().volts, 4_800);
    assert!(
        sim.plane
            .slot(2)
            .unwrap()
            .status
            .contains(SlotStatus::PRESENT | SlotStatus::ENABLED | SlotStatus::PGD)
    );
}

#[test]
fn two_naks_lose_a_unit_and_one_does_not() {
    let mut sim = Sim::with_units(1);
    sim.boot();
    sim.advance(1_000);

    // One cycle's worth of NAKs: not lost yet.
    sim.unit(0).power_off();
    sim.tick();
    assert!(sim.plane.unit(0).unwrap().present(), "one NAK is not enough");
    sim.unit(0).power_on();
    sim.advance(500);
    assert!(sim.plane.unit(0).unwrap().present());
    assert!(!sim.saw(EventKind::UnitLost));

    // Two in a row.
    sim.unit(0).power_off();
    sim.tick();
    sim.tick();
    assert!(!sim.plane.unit(0).unwrap().present());
    assert!(sim.plane.unit(0).unwrap().status.contains(UnitStatus::LOST));
    assert_eq!(sim.plane.unit(0).unwrap().times_lost, 1);
    assert!(sim.saw(EventKind::UnitLost));
    assert!(
        sim.plane.status_flags().contains(StatusFlags::ANY_UNIT_FAULT),
        "a lost unit is a fault"
    );
    // Its slots claim nothing while it is away.
    assert_eq!(sim.plane.slot(0).unwrap().volts, 0);
    assert!(sim.plane.slot(0).unwrap().status.is_empty());
}

#[test]
fn a_stuck_segment_is_clocked_out_and_the_mux_reset() {
    let mut sim = Sim::with_units(1);
    sim.boot();
    sim.advance(1_000);
    let resets_before = sim.dev.bus.mux_resets;

    sim.dev.bus.stuck = true;
    sim.tick();
    assert_eq!(sim.dev.bus.unsticks, 1, "nine clocks");
    assert!(sim.dev.bus.mux_resets > resets_before, "then MUX_RST");
    assert!(!sim.plane.unit(0).unwrap().present(), "the channel is skipped");

    // unstick() freed the bus, so the next re-scan finds the unit again.
    sim.advance(2_500);
    assert!(sim.plane.unit(0).unwrap().present());
}

#[test]
fn the_poll_schedule_is_twenty_two_and_one_hertz() {
    let mut sim = Sim::with_units(1);
    sim.boot();
    // The unit input monitor reads at 2 Hz: its current is the two
    // slots', so a change shows up within 500 ms and not sooner than
    // the slots' own 50 ms.
    sim.advance(1_000);
    assert_eq!(sim.plane.unit(0).unwrap().amps, 2_000);
    sim.fake_slot(0).ina.current_ma = 3_000;
    sim.tick();
    assert_eq!(sim.plane.slot(0).unwrap().amps, 3_000, "slots are 20 Hz");
    sim.advance(600);
    assert_eq!(sim.plane.unit(0).unwrap().amps, 4_000, "the unit followed");

    // The ID ADC is 1 Hz, and a PG change pulls it forward.
    sim.fake_slot(0).adc.counts = 0x0800;
    sim.tick();
    sim.advance(1_100);
    assert_eq!(sim.plane.slot(0).unwrap().module_id_raw, 0x0800);
    assert_eq!(sim.plane.slot(0).unwrap().module_class, 9);
}

#[test]
fn the_watchdog_is_fed_once_per_cycle_and_not_more() {
    let mut sim = Sim::with_units(1);
    sim.boot();
    // Ten 50 ms ticks is ten cycles, so ten feeds; a tick that polls
    // nothing feeds nothing.
    let before = sim.feeds;
    sim.advance(500);
    assert_eq!(sim.feeds - before, 10);

    // Half-period steps: only every other one gets a cycle.
    let before = sim.feeds;
    for _ in 0..10 {
        sim.step(25);
    }
    assert_eq!(sim.feeds - before, 5);
}

#[test]
fn energy_is_integrated_and_carries_the_boot_count() {
    let mut sim = Sim::with_units(1);
    sim.boot();
    // 48 V at 1 A is 48 W; ten seconds is 480 J.
    sim.advance(10_000);
    let joules = sim.plane.slot(0).unwrap().energy_joules();
    assert!((470..=490).contains(&joules), "got {joules} J");

    let payload = sim.read_block(granite_proto::BLK_SLOT, 0);
    let energy = granite_proto::Slot::records(&payload)
        .filter_map(|r| match r {
            Ok(granite_proto::Slot::Energy(e)) => Some(e),
            _ => None,
        })
        .next()
        .expect("the slot block carries energy");
    assert_eq!(energy.boot_count, 1);
    assert_eq!(energy.joules, joules);
}
