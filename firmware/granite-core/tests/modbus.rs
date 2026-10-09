//! The Modbus map: snapshot to registers, register writes back to
//! commands, and the generated documentation.

use granite_core::Target;
use granite_core::actuator::{ActionResult, FailReason, RefuseReason};
use granite_core::hal::TEMP_MISSING;
use granite_core::modbus_map::{
    Access, COIL_COUNT, DISCRETE_COUNT, FwVersion, HOLDING_COUNT, INPUT_COUNT, MAP, MAP_VERSION,
    MapError, RegKind, ResultCode, cmd_word, coils, command_from_coil, command_from_holding,
    discrete_inputs, entry, fault_bits, holding_registers, input_registers, render_markdown,
};
use granite_core::msg::CommandKind;
use granite_core::node::NodeState;
use granite_core::observed::{Observed, ProbeObs, Stamped};

fn snapshot() -> Observed {
    let mut o = Observed::new();
    // Nodes 1 and 3 on, node 2 off, node 4 busy, the rest unknown.
    o.nodes[0].led = Some(true);
    o.nodes[0].state = NodeState::On;
    o.nodes[1].led = Some(false);
    o.nodes[1].state = NodeState::Off;
    o.nodes[2].led = Some(true);
    o.nodes[2].state = NodeState::On;
    o.nodes[3].led = Some(false);
    o.nodes[3].state = NodeState::Busy {
        action: granite_core::actuator::ActionKind::On,
        started_ms: 5,
    };
    o.probes.push(ProbeObs {
        rom: 1,
        name: "inlet".into(),
        centi_c: Some(2_537),
        ts_ms: 10,
    });
    o.probes.push(ProbeObs {
        rom: 2,
        name: "cold".into(),
        centi_c: Some(-1_250),
        ts_ms: 10,
    });
    o.probes.push(ProbeObs {
        rom: 3,
        name: "gone".into(),
        centi_c: None,
        ts_ms: 10,
    });
    o.board_temp = Stamped::new(Some(3_100), 10);
    o.vin_mv = Stamped::new(Some(19_120), 10);
    o.dry_in = Stamped::new(Some(0b1001), 10);
    o.link_up = Stamped::new(true, 10);
    o.mqtt_connected = Stamped::new(false, 10);
    o.uptime_s = Stamped::new(0x0001_2345, 10);
    o
}

#[test]
fn discrete_inputs_follow_the_adr_layout() {
    let d = discrete_inputs(&snapshot());
    assert_eq!(d.len(), DISCRETE_COUNT);
    assert_eq!(
        &d[0..8],
        &[true, false, true, false, false, false, false, false]
    );
    assert_eq!(&d[8..12], &[true, false, false, true]);
    assert!(d[12], "link");
    assert!(!d[13], "mqtt");
}

#[test]
fn input_registers_follow_the_adr_layout() {
    let r = input_registers(
        &snapshot(),
        FwVersion { major: 1, minor: 2 },
        fault_bits::SENSE,
    );
    assert_eq!(r.len(), INPUT_COUNT);
    assert_eq!(r[0], 2_537);
    assert_eq!(
        r[1] as i16, -1_250,
        "negative temperatures are two's complement"
    );
    assert_eq!(r[2] as i16, TEMP_MISSING, "a silent probe reads 0x8000");
    assert_eq!(r[2], 0x8000);
    for (slot, reg) in r.iter().enumerate().take(8).skip(3) {
        assert_eq!(*reg, 0x8000, "unconfigured slot {slot}");
    }
    assert_eq!(r[8], 3_100);
    assert_eq!(r[9], 19_120);
    assert_eq!(r[10], 0x0001);
    assert_eq!(r[11], 0x2345);
    assert_eq!(r[12], 0x0102);
    assert_eq!(r[13], fault_bits::SENSE);
    assert_eq!(r[14], MAP_VERSION);
}

#[test]
fn a_never_read_sensor_is_distinguishable() {
    let r = input_registers(&Observed::new(), FwVersion::default(), 0);
    assert_eq!(r[8] as i16, TEMP_MISSING, "board temp never read");
    assert_eq!(r[9], 0, "vin never read");
}

#[test]
fn holding_registers_read_back_node_state() {
    let h = holding_registers(&snapshot(), ResultCode::Accepted);
    assert_eq!(h.len(), HOLDING_COUNT);
    assert_eq!(&h[0..4], &[2, 1, 2, 3], "on, off, on, busy");
    assert_eq!(h[4], 0, "unknown");
    assert_eq!(h[8], 0, "the on_all trigger always reads 0");
    assert_eq!(h[9], 2);
}

#[test]
fn result_codes_cover_every_action_result() {
    assert_eq!(ResultCode::of_result(&ActionResult::Ok), ResultCode::Ok);
    assert_eq!(
        ResultCode::of_result(&ActionResult::ShutdownPending),
        ResultCode::ShutdownPending
    );
    assert_eq!(
        ResultCode::of_result(&ActionResult::refused(RefuseReason::UnknownState)),
        ResultCode::Refused
    );
    assert_eq!(
        ResultCode::of_result(&ActionResult::failed(FailReason::StillOn)),
        ResultCode::Failed
    );
    assert_eq!(ResultCode::default().as_u16(), 0);
}

#[test]
fn coils_mirror_the_node_state() {
    let c = coils(&snapshot());
    assert_eq!(c.len(), COIL_COUNT);
    assert_eq!(&c[0..4], &[true, false, true, false]);
}

#[test]
fn holding_writes_become_commands() {
    for (value, kind) in [
        (cmd_word::ON, CommandKind::On),
        (cmd_word::OFF, CommandKind::Off),
        (cmd_word::FORCE_OFF, CommandKind::ForceOff),
        (cmd_word::RESET, CommandKind::Reset),
        (cmd_word::CYCLE, CommandKind::Cycle),
    ] {
        for addr in 0u16..8 {
            let cmd = command_from_holding(addr, value, "modbus-1")
                .unwrap()
                .expect("a command word produces a command");
            assert_eq!(cmd.action, kind);
            assert_eq!(cmd.target, Target::Node((addr + 1) as u8));
            assert_eq!(cmd.id, "modbus-1");
            assert!(cmd.args.is_empty(), "Modbus cannot carry force");
        }
    }

    assert_eq!(
        command_from_holding(0, cmd_word::NONE, "m").unwrap(),
        None,
        "writing 0 clears the word"
    );
    assert_eq!(
        command_from_holding(8, 1, "m").unwrap().unwrap().action,
        CommandKind::OnAll
    );
    assert_eq!(command_from_holding(8, 0, "m").unwrap(), None);
}

#[test]
fn bad_holding_writes_are_errors_not_guesses() {
    assert_eq!(
        command_from_holding(3, 42, "m"),
        Err(MapError::BadValue { addr: 3, value: 42 })
    );
    assert_eq!(
        command_from_holding(8, 7, "m"),
        Err(MapError::BadValue { addr: 8, value: 7 })
    );
    assert!(matches!(
        command_from_holding(9, 1, "m"),
        Err(MapError::BadValue { .. })
    ));
    assert_eq!(
        command_from_holding(20, 1, "m"),
        Err(MapError::UnknownRegister {
            kind: RegKind::HoldingRegister,
            addr: 20
        })
    );
    assert!(
        command_from_holding(20, 1, "m")
            .unwrap_err()
            .to_string()
            .contains("Holding registers")
    );
}

#[test]
fn coil_writes_become_on_and_off() {
    let on = command_from_coil(2, true, "m").unwrap().unwrap();
    assert_eq!(on.action, CommandKind::On);
    assert_eq!(on.target, Target::Node(3));

    let off = command_from_coil(7, false, "m").unwrap().unwrap();
    assert_eq!(off.action, CommandKind::Off);
    assert_eq!(off.target, Target::Node(8));

    assert_eq!(
        command_from_coil(8, true, "m"),
        Err(MapError::UnknownRegister {
            kind: RegKind::Coil,
            addr: 8
        })
    );
}

#[test]
fn the_table_is_complete_and_unique() {
    for kind in [
        RegKind::DiscreteInput,
        RegKind::InputRegister,
        RegKind::HoldingRegister,
        RegKind::Coil,
    ] {
        let addrs: Vec<u16> = MAP
            .iter()
            .filter(|e| e.kind == kind)
            .map(|e| e.addr)
            .collect();
        let expected: Vec<u16> = (0..addrs.len() as u16).collect();
        assert_eq!(addrs, expected, "{} must be dense from 0", kind.as_str());
    }
    assert_eq!(
        MAP.iter()
            .filter(|e| e.kind == RegKind::DiscreteInput)
            .count(),
        DISCRETE_COUNT
    );
    assert_eq!(
        MAP.iter()
            .filter(|e| e.kind == RegKind::InputRegister)
            .count(),
        INPUT_COUNT
    );
    assert_eq!(
        MAP.iter()
            .filter(|e| e.kind == RegKind::HoldingRegister)
            .count(),
        HOLDING_COUNT
    );
    assert_eq!(
        MAP.iter().filter(|e| e.kind == RegKind::Coil).count(),
        COIL_COUNT
    );

    assert_eq!(
        entry(RegKind::InputRegister, 14).unwrap().name,
        "map_version"
    );
    assert_eq!(entry(RegKind::InputRegister, 14).unwrap().access, Access::R);
    assert_eq!(
        entry(RegKind::HoldingRegister, 0).unwrap().access,
        Access::Rw
    );
    assert!(entry(RegKind::Coil, 99).is_none());
}

#[test]
fn the_markdown_documents_every_register() {
    let md = render_markdown();
    assert!(md.starts_with("# Granite controller Modbus map"));
    assert!(md.contains(&format!("Map version: {MAP_VERSION}")));
    for kind in [
        RegKind::DiscreteInput,
        RegKind::InputRegister,
        RegKind::HoldingRegister,
        RegKind::Coil,
    ] {
        assert!(
            md.contains(&format!("## {}", kind.as_str())),
            "{}",
            kind.as_str()
        );
    }
    for e in MAP {
        assert!(md.contains(e.name), "{} is undocumented", e.name);
    }
    assert!(md.contains("## Fault flags (input register 13)"));
    assert!(md.contains("expander readback mismatch"));
}
