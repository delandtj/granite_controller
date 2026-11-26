//! The INA228 path: an FD batch 2 unit, whose three current monitors
//! are INA228s instead of INA238s, has to look exactly like a batch 1
//! unit on the link (ADR 0002, as built 2026-11-26).
//!
//! Everything here runs the same scenario twice, once per kind, and
//! compares. The physical quantities are set on the fake parts and each
//! one answers its own registers in its own scaling, so what is being
//! checked is the whole path: DEVICE_ID at configure time, SHUNT_CAL,
//! the 24-bit reads, the conversions and the power-limit register.

#![cfg(feature = "sim")]

use granite_proto::{Config, EventKind, PowerLimit, SlotStatus};
use plane_logic::drivers::ina;
use plane_logic::sim::{FakeUnit, Sim};

/// The same board, at 48.00 V with 2.300 A in each slot and a die at
/// 31.0 C, with monitors of kind `kind`.
fn plane_with(kind: ina::Kind) -> Sim {
    let mut sim = Sim::with_units(0);
    let mut unit = FakeUnit::populated_with(kind);
    for slot in unit.slots.iter_mut() {
        slot.ina.bus_mv = 48_000;
        slot.ina.current_ma = 2_300;
        slot.ina.temperature = 310;
    }
    unit.input_mv = 48_000;
    unit.input.temperature = 310;
    sim.dev.bus.units[0] = Some(unit);
    sim.boot();
    sim.advance(2_000);
    sim
}

#[test]
fn the_kind_comes_from_device_id_and_picks_the_shunt_cal() {
    let mut sim = plane_with(ina::Kind::Ina228);
    assert_eq!(
        sim.unit(0).slots[0].ina.shunt_cal,
        ina::SLOT_SHUNT_CAL_228,
        "the slot's 3.5 mOhm at CURRENT_LSB = 0.5 mA"
    );
    assert_eq!(sim.unit(0).input.shunt_cal, ina::UNIT_SHUNT_CAL_228);
    // Everything else configure writes is the same word on both kinds.
    assert_eq!(sim.unit(0).slots[0].ina.config, ina::CONFIG_VALUE);
    assert_eq!(sim.unit(0).slots[0].ina.adc_config, ina::ADC_CONFIG_VALUE);

    let mut sim = plane_with(ina::Kind::Ina238);
    assert_eq!(sim.unit(0).slots[0].ina.shunt_cal, ina::SLOT_SHUNT_CAL);
    assert_eq!(sim.unit(0).input.shunt_cal, ina::UNIT_SHUNT_CAL);
}

#[test]
fn both_kinds_report_the_same_telemetry() {
    let a = plane_with(ina::Kind::Ina238);
    let b = plane_with(ina::Kind::Ina228);
    for slot in 0..2u8 {
        let x = a.plane.slot(slot).unwrap();
        let y = b.plane.slot(slot).unwrap();
        // 48.00 V, 2.300 A, 110.4 W, 31.0 C, in the link's units.
        assert_eq!((x.volts, x.amps, x.watts, x.temperature), (4800, 2300, 1104, 310));
        assert_eq!(
            (x.volts, x.amps, x.watts, x.temperature),
            (y.volts, y.amps, y.watts, y.temperature),
            "slot {slot} reads differently on an INA228"
        );
    }
    // The unit input carries both slots: 4.600 A and 220.8 W.
    let x = a.plane.unit(0).unwrap();
    let y = b.plane.unit(0).unwrap();
    assert_eq!((x.volts, x.amps, x.watts, x.temperature), (4800, 4600, 2208, 310));
    assert_eq!(
        (x.volts, x.amps, x.watts, x.temperature),
        (y.volts, y.amps, y.watts, y.temperature),
        "the unit input reads differently on an INA228"
    );
}

/// Run a plane of `kind` with a 75.0 W slot limit and `current_ma` in
/// slot 0, and say whether the limit tripped.
fn trips_at(kind: ina::Kind, current_ma: u32) -> bool {
    let mut sim = plane_with(kind);
    sim.config_write(&[
        Config::PowerLimit(PowerLimit { slot: 0, watts: 750 }),
        Config::Commit,
    ]);
    sim.fake_slot(0).ina.current_ma = current_ma;
    sim.advance(500);
    let tripped = sim
        .plane
        .slot(0)
        .unwrap()
        .status
        .contains(SlotStatus::POWER_LIMIT_TRIPPED);
    assert_eq!(tripped, sim.saw(EventKind::PowerLimitTrip));
    assert_eq!(sim.plane.slot(0).unwrap().power_limit, 750);
    tripped
}

#[test]
fn the_power_limit_trips_at_the_same_power_on_both_kinds() {
    // A 75.0 W limit lands on 74.9568 W in both parts' PWR_LIMIT
    // (1464 x 51.2 mW, 183 x 409.6 mW), so 48 V x 1.562 A = 74.976 W
    // is over it and 48 V x 1.560 A = 74.880 W is not, on either.
    for kind in [ina::Kind::Ina238, ina::Kind::Ina228] {
        assert!(trips_at(kind, 1_562), "74.976 W is over the limit: {kind:?}");
        assert!(
            !trips_at(kind, 1_560),
            "74.880 W is under the limit: {kind:?}"
        );
    }
}

#[test]
fn an_unknown_device_id_is_logged_once_and_read_as_an_ina238() {
    let mut sim = Sim::with_units(0);
    let mut unit = FakeUnit::populated();
    // The part answers, but with an id this driver has never heard of:
    // a third monitor dropped into the same footprint. It is otherwise
    // an INA238, which is the assumption the fallback makes - a fake
    // that answered INA228-width registers under an unknown id would
    // simply fail its reads, which is the sim being strict about
    // widths and says nothing about the fallback.
    unit.slots[0].ina.device_id = Some(0x1234);
    sim.dev.bus.units[0] = Some(unit);
    sim.boot();
    sim.advance(2_000);

    let lines: Vec<String> = sim
        .plane
        .log()
        .iter()
        .map(|l| String::from_utf8_lossy(l.text()).into_owned())
        .collect();
    assert_eq!(
        lines
            .iter()
            .filter(|t| t.starts_with("ina device id unknown"))
            .count(),
        1,
        "one line, with the id: {lines:?}"
    );
    assert!(lines.contains(&"ina device id unknown 4660".to_string()));

    // It is configured as an INA238 all the same: the monitor counts as
    // there, and the slot's SHUNT_CAL is the INA238 one.
    assert_eq!(sim.unit(0).slots[0].ina.shunt_cal, ina::SLOT_SHUNT_CAL);
    assert!(
        sim.plane
            .slot(0)
            .unwrap()
            .status
            .contains(SlotStatus::PRESENT)
    );
}
