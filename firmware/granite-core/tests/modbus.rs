//! The Modbus map, version 2 (ADR 0004 decision 7): snapshot to
//! registers, register writes back to commands, and the generated
//! documentation.

use granite_core::Target;
use granite_core::hal::TEMP_MISSING;
use granite_core::modbus_map::{
    BLOCKS, BOARD_TEMP_ADDR, DRY_BASE, FAULTS_ADDR, FW_ADDR, FwVersion, Image, LAST_RESULT_ADDR,
    LINK_ADDR, MAP, MAP_VERSION, MAP_VERSION_ADDR, MQTT_ADDR, MapError, NODE_BASE, NODE_CMD_BASE,
    NODE_COIL_BASE, NODE_COUNT_ADDR, NODE_HOST_BIT_BASE, NODE_POWER_BIT_BASE, NODE_STRIDE,
    ON_ALL_ADDR, PLANE_BASE, PLANE_CMD_BASE, PLANE_FAULT_MASK, PLANE_STRIDE, PLANES_PRESENT_ADDR,
    PROBE_BASE, RegKind, ResultCode, UPTIME_ADDR, VIN_ADDR, check_coil, check_holding, cmd_word,
    command_from_coil, command_from_holding, entry, fault_bits, name_of, node_fault_bits,
    plane_cmd_word, plane_faults, plane_flag_bits, render_markdown,
};
use granite_core::msg::{CommandKind, Reply};
use granite_core::observed::{NodeObs, Observed, PlaneObs, ProbeObs, Stamped, UnitObs};
use granite_core::plane::Plane;
use granite_core::plane::model::{HostState, PowerState};
use granite_core::{MAX_NODES, MAX_PLANES, NodeId};

/// A snapshot with one plane, nodes 1 (on) and 2 (off), node 3 absent,
/// probes and the controller's own sensors.
fn snapshot() -> Observed {
    let mut o = Observed::new();
    let mut plane = PlaneObs::of_plane(&Plane::new(0x30));
    plane.firmware = String::from("1.2.3");
    plane.bus_volts = 1_205;
    plane.core_temperature = 355;
    plane.boot_count = 0x0001_0002;
    plane.uptime_s = 0x0003_0004;
    plane.units_present = 0b0001;
    plane.nodes_present = 0b0011;
    plane.sequence_running = true;
    plane.intended_state_lost = true;
    plane.units = vec![UnitObs {
        unit: 0,
        present: true,
        latched_off: true,
        volts: 1_200,
        amps: 4_100,
        watts: 4_920,
        temperature: 300,
        ..UnitObs::default()
    }];
    plane.info_ms = 5;
    plane.ts_ms = 10;
    o.planes.push(plane);

    o.nodes.push(node(1, PowerState::On, HostState::On));
    o.nodes.push(node(2, PowerState::Off, HostState::Unknown));

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
    o.vin_mv = Stamped::new(Some(12_120), 10);
    o.dry_in = Stamped::new(Some(0b1001), 10);
    o.link_up = Stamped::new(true, 10);
    o.mqtt_connected = Stamped::new(false, 10);
    o.uptime_s = Stamped::new(0x0001_2345, 10);
    o
}

fn node(id: NodeId, power: PowerState, host: HostState) -> NodeObs {
    let (address, slot) = granite_core::plane::node_location(id).expect("a node id");
    let mut obs = NodeObs::unknown(id, address, slot);
    obs.present = true;
    obs.power = power;
    obs.host = host;
    obs.telemetry.volts = 4_803;
    obs.telemetry.amps = 1_250;
    obs.telemetry.watts = 600;
    obs.telemetry.temperature = 412;
    obs.telemetry.energy_joules = 0x0001_0002;
    obs.telemetry.energy_boot_count = 7;
    obs.telemetry.module_class = 3;
    obs.faults.ov_latched = power == PowerState::Off;
    obs.busy = true;
    obs.ts_ms = 10;
    obs
}

fn image(o: &Observed) -> Image<'_> {
    Image::new(
        o,
        FwVersion { major: 1, minor: 4 },
        fault_bits::NODE_FAULT,
        ResultCode::Accepted,
    )
}

#[test]
fn the_controller_registers_did_not_move() {
    let o = snapshot();
    let img = image(&o);
    assert_eq!(
        img.word(RegKind::InputRegister, MAP_VERSION_ADDR),
        Some(MAP_VERSION),
        "the map version stays at its address"
    );
    assert_eq!(MAP_VERSION, 2);
    assert_eq!(
        img.word(RegKind::InputRegister, PROBE_BASE),
        Some(2_537),
        "probe 1"
    );
    assert_eq!(
        img.word(RegKind::InputRegister, PROBE_BASE + 1),
        Some((-1_250i16) as u16)
    );
    assert_eq!(
        img.word(RegKind::InputRegister, PROBE_BASE + 2),
        Some(TEMP_MISSING as u16),
        "a probe that did not answer"
    );
    assert_eq!(
        img.word(RegKind::InputRegister, BOARD_TEMP_ADDR),
        Some(3_100)
    );
    assert_eq!(img.word(RegKind::InputRegister, VIN_ADDR), Some(12_120));
    assert_eq!(img.word(RegKind::InputRegister, UPTIME_ADDR), Some(1));
    assert_eq!(
        img.word(RegKind::InputRegister, UPTIME_ADDR + 1),
        Some(0x2345)
    );
    assert_eq!(img.word(RegKind::InputRegister, FW_ADDR), Some(0x0104));
    assert_eq!(
        img.word(RegKind::InputRegister, FAULTS_ADDR),
        Some(fault_bits::NODE_FAULT)
    );
    assert_eq!(
        img.word(RegKind::InputRegister, PLANES_PRESENT_ADDR),
        Some(0b1)
    );
    assert_eq!(img.word(RegKind::InputRegister, NODE_COUNT_ADDR), Some(2));
    assert_eq!(img.bit(RegKind::DiscreteInput, DRY_BASE), Some(true));
    assert_eq!(img.bit(RegKind::DiscreteInput, DRY_BASE + 1), Some(false));
    assert_eq!(img.bit(RegKind::DiscreteInput, DRY_BASE + 3), Some(true));
    assert_eq!(img.bit(RegKind::DiscreteInput, LINK_ADDR), Some(true));
    assert_eq!(img.bit(RegKind::DiscreteInput, MQTT_ADDR), Some(false));
    assert_eq!(
        img.word(RegKind::HoldingRegister, LAST_RESULT_ADDR),
        Some(ResultCode::Accepted.as_u16())
    );
    assert_eq!(img.word(RegKind::HoldingRegister, ON_ALL_ADDR), Some(0));
}

/// Bug: fault bit 0 (`PLANE_LOST`, "a plane the controller had found
/// stopped answering") was defined but never set; only `NO_PLANE` and
/// `NODE_FAULT` were. The decision is `plane_faults` here, and the
/// firmware only applies it.
#[test]
fn the_plane_fault_bits_report_a_lost_plane() {
    // One plane answers, one node in fault: bit 1 only.
    let mut o = snapshot();
    assert_eq!(plane_faults(&o), fault_bits::NODE_FAULT);

    // A plane that had been found stops answering: bit 0 joins it, and
    // bit 7 stays clear because another plane still answers.
    o.planes_lost = vec![0x31];
    assert!(o.any_plane_lost());
    let both = fault_bits::PLANE_LOST | fault_bits::NODE_FAULT;
    assert_eq!(plane_faults(&o), both);
    let img = Image::new(&o, FwVersion::default(), both, ResultCode::Idle);
    assert_eq!(
        img.word(RegKind::InputRegister, FAULTS_ADDR),
        Some(both),
        "and that is what input register 13 reads"
    );

    // Every plane gone: bit 0 and bit 7, and no node left to fault.
    o.planes.clear();
    o.nodes.clear();
    assert_eq!(
        plane_faults(&o),
        fault_bits::PLANE_LOST | fault_bits::NO_PLANE
    );

    // Answering again, or forgotten: nothing left to report.
    o.planes_lost.clear();
    assert_eq!(plane_faults(&o), fault_bits::NO_PLANE);
    // The mask covers exactly the bits the function decides, so a
    // caller can clear what it did not set.
    assert_eq!(
        PLANE_FAULT_MASK,
        fault_bits::PLANE_LOST | fault_bits::NODE_FAULT | fault_bits::NO_PLANE
    );
}

#[test]
fn a_node_block_carries_both_axes_and_the_telemetry() {
    let o = snapshot();
    let img = image(&o);
    let base = NODE_BASE;
    assert_eq!(img.word(RegKind::InputRegister, base), Some(2), "on");
    assert_eq!(
        img.word(RegKind::InputRegister, base + 1),
        Some(2),
        "host on"
    );
    assert_eq!(img.word(RegKind::InputRegister, base + 2), Some(4_803));
    assert_eq!(img.word(RegKind::InputRegister, base + 3), Some(1_250));
    assert_eq!(img.word(RegKind::InputRegister, base + 4), Some(600));
    assert_eq!(img.word(RegKind::InputRegister, base + 5), Some(412));
    assert_eq!(img.word(RegKind::InputRegister, base + 6), Some(1));
    assert_eq!(img.word(RegKind::InputRegister, base + 7), Some(2));
    assert_eq!(img.word(RegKind::InputRegister, base + 8), Some(0));
    assert_eq!(img.word(RegKind::InputRegister, base + 9), Some(7));
    assert_eq!(
        img.word(RegKind::InputRegister, base + 10),
        Some(node_fault_bits::BUSY)
    );
    assert_eq!(
        img.word(RegKind::InputRegister, base + 11),
        Some(3),
        "slot 0, module class 3"
    );

    // Node 2 is off with its OV latch set, on slot 1.
    let two = NODE_BASE + NODE_STRIDE;
    assert_eq!(img.word(RegKind::InputRegister, two), Some(1), "off");
    assert_eq!(
        img.word(RegKind::InputRegister, two + 1),
        Some(0),
        "unknown"
    );
    assert_eq!(
        img.word(RegKind::InputRegister, two + 10),
        Some(node_fault_bits::OV_LATCHED | node_fault_bits::BUSY)
    );
    assert_eq!(
        img.word(RegKind::InputRegister, two + 11),
        Some(0x0103),
        "slot 1, module class 3"
    );

    // Node 3 is not there: zeros, and the absent bit.
    let three = NODE_BASE + 2 * NODE_STRIDE;
    assert_eq!(img.word(RegKind::InputRegister, three), Some(0));
    assert_eq!(img.word(RegKind::InputRegister, three + 2), Some(0));
    assert_eq!(
        img.word(RegKind::InputRegister, three + 10),
        Some(node_fault_bits::ABSENT)
    );

    // And the bit tables.
    assert_eq!(
        img.bit(RegKind::DiscreteInput, NODE_POWER_BIT_BASE),
        Some(true)
    );
    assert_eq!(
        img.bit(RegKind::DiscreteInput, NODE_POWER_BIT_BASE + 1),
        Some(false)
    );
    assert_eq!(
        img.bit(RegKind::DiscreteInput, NODE_HOST_BIT_BASE),
        Some(true)
    );
    assert_eq!(
        img.bit(RegKind::DiscreteInput, NODE_HOST_BIT_BASE + 1),
        Some(false)
    );
    assert_eq!(img.bit(RegKind::Coil, NODE_COIL_BASE), Some(true));
    assert_eq!(img.bit(RegKind::Coil, NODE_COIL_BASE + 1), Some(false));
    assert_eq!(
        img.word(RegKind::HoldingRegister, NODE_CMD_BASE),
        Some(2),
        "a command word reads the rail state"
    );
}

#[test]
fn a_plane_block_carries_the_board() {
    let o = snapshot();
    let img = image(&o);
    assert_eq!(
        img.word(RegKind::InputRegister, PLANE_BASE),
        Some(
            plane_flag_bits::PRESENT
                | plane_flag_bits::SEQUENCE_RUNNING
                | plane_flag_bits::INTENDED_STATE_LOST
        )
    );
    assert_eq!(
        img.word(RegKind::InputRegister, PLANE_BASE + 1),
        Some(0x0102)
    );
    assert_eq!(
        img.word(RegKind::InputRegister, PLANE_BASE + 2),
        Some(1_205)
    );
    assert_eq!(img.word(RegKind::InputRegister, PLANE_BASE + 3), Some(355));
    assert_eq!(
        img.word(RegKind::InputRegister, PLANE_BASE + 4),
        Some(0b0001)
    );
    assert_eq!(
        img.word(RegKind::InputRegister, PLANE_BASE + 5),
        Some(0b0011)
    );
    assert_eq!(img.word(RegKind::InputRegister, PLANE_BASE + 6), Some(1));
    assert_eq!(img.word(RegKind::InputRegister, PLANE_BASE + 7), Some(2));
    assert_eq!(img.word(RegKind::InputRegister, PLANE_BASE + 8), Some(3));
    assert_eq!(img.word(RegKind::InputRegister, PLANE_BASE + 9), Some(4));
    assert_eq!(
        img.word(RegKind::InputRegister, PLANE_BASE + 10),
        Some(4_920)
    );
    assert_eq!(img.word(RegKind::InputRegister, PLANE_BASE + 11), Some(0));
    assert_eq!(
        img.word(RegKind::InputRegister, PLANE_BASE + 14),
        Some(0b0001)
    );

    // A plane that does not answer reads zero, flags included.
    let one = PLANE_BASE + PLANE_STRIDE;
    assert_eq!(img.word(RegKind::InputRegister, one), Some(0));
    assert_eq!(
        img.word(RegKind::HoldingRegister, PLANE_CMD_BASE + 1),
        Some(0)
    );
}

#[test]
fn everything_outside_the_map_is_an_exception() {
    let o = snapshot();
    let img = image(&o);
    // The rev C node registers are gone on purpose: a version 1 client
    // fails loudly rather than reading a plausible wrong node.
    assert_eq!(img.bit(RegKind::DiscreteInput, 0), None);
    assert_eq!(img.word(RegKind::HoldingRegister, 0), None);
    assert_eq!(img.bit(RegKind::Coil, 0), None);
    assert_eq!(img.word(RegKind::InputRegister, 17), None);
    assert_eq!(img.word(RegKind::InputRegister, 99), None);
    assert_eq!(
        img.word(
            RegKind::InputRegister,
            NODE_BASE + NODE_STRIDE * MAX_NODES as u16
        ),
        None
    );
    assert_eq!(
        img.word(
            RegKind::InputRegister,
            PLANE_BASE + PLANE_STRIDE * MAX_PLANES as u16
        ),
        None
    );
    assert_eq!(
        img.bit(RegKind::Coil, NODE_COIL_BASE + MAX_NODES as u16),
        None
    );
}

#[test]
fn holding_writes_become_commands() {
    for (word, action) in [
        (cmd_word::ON, CommandKind::On),
        (cmd_word::OFF, CommandKind::Off),
        (cmd_word::FORCE_OFF, CommandKind::ForceOff),
        (cmd_word::RESET, CommandKind::Reset),
        (cmd_word::CYCLE, CommandKind::Cycle),
        (cmd_word::POWER_ON, CommandKind::PowerOn),
        (cmd_word::POWER_OFF, CommandKind::PowerOff),
        (cmd_word::CLEAR_LATCH, CommandKind::ClearLatch),
    ] {
        let cmd = command_from_holding(NODE_CMD_BASE + 4, word, "modbus-7")
            .expect("node 5's word")
            .expect("a command");
        assert_eq!(cmd.action, action);
        assert_eq!(cmd.target, Target::Node(5));
        assert_eq!(cmd.id, "modbus-7");
        assert!(cmd.args.is_empty(), "Modbus carries no args");
    }

    let all = command_from_holding(ON_ALL_ADDR, 1, "modbus-8")
        .unwrap()
        .unwrap();
    assert_eq!(all.action, CommandKind::OnAll);
    assert_eq!(all.target, Target::All);
    assert!(command_from_holding(ON_ALL_ADDR, 0, "m").unwrap().is_none());

    let reboot = command_from_holding(PLANE_CMD_BASE + 7, plane_cmd_word::REBOOT, "m")
        .unwrap()
        .unwrap();
    assert_eq!(reboot.action, CommandKind::PlaneReboot);
    assert_eq!(reboot.target, Target::Plane(7));

    for unit in 0..4u8 {
        let cmd = command_from_holding(
            PLANE_CMD_BASE,
            plane_cmd_word::UNIT_RESET + u16::from(unit),
            "m",
        )
        .unwrap()
        .unwrap();
        assert_eq!(cmd.action, CommandKind::UnitReset);
        assert_eq!(cmd.target, Target::Unit { plane: 0, unit });
    }
}

#[test]
fn bad_writes_are_errors_not_guesses() {
    assert_eq!(
        command_from_holding(NODE_CMD_BASE, 42, "m"),
        Err(MapError::BadValue {
            addr: NODE_CMD_BASE,
            value: 42
        })
    );
    assert_eq!(
        command_from_holding(PLANE_CMD_BASE, 6, "m"),
        Err(MapError::BadValue {
            addr: PLANE_CMD_BASE,
            value: 6
        })
    );
    assert_eq!(
        command_from_holding(LAST_RESULT_ADDR, 1, "m"),
        Err(MapError::BadValue {
            addr: LAST_RESULT_ADDR,
            value: 1
        }),
        "last_result is read only"
    );
    assert!(matches!(
        command_from_holding(NODE_CMD_BASE + MAX_NODES as u16, 1, "m"),
        Err(MapError::UnknownRegister {
            kind: RegKind::HoldingRegister,
            ..
        })
    ));
    assert!(matches!(
        command_from_coil(NODE_COIL_BASE + MAX_NODES as u16, true, "m"),
        Err(MapError::UnknownRegister {
            kind: RegKind::Coil,
            ..
        })
    ));
    // The cheap checks say the same thing as the builders.
    assert_eq!(
        check_holding(NODE_CMD_BASE, 42).is_err(),
        command_from_holding(NODE_CMD_BASE, 42, "").is_err()
    );
    assert_eq!(
        check_coil(NODE_COIL_BASE).is_ok(),
        command_from_coil(NODE_COIL_BASE, false, "").is_ok()
    );
}

#[test]
fn coil_writes_become_on_and_off() {
    let on = command_from_coil(NODE_COIL_BASE + 63, true, "m")
        .unwrap()
        .unwrap();
    assert_eq!(on.action, CommandKind::On);
    assert_eq!(on.target, Target::Node(64));
    let off = command_from_coil(NODE_COIL_BASE, false, "m")
        .unwrap()
        .unwrap();
    assert_eq!(off.action, CommandKind::Off);
    assert_eq!(off.target, Target::Node(1));
}

#[test]
fn the_table_is_unique_and_the_blocks_do_not_overlap() {
    for kind in [
        RegKind::DiscreteInput,
        RegKind::InputRegister,
        RegKind::HoldingRegister,
        RegKind::Coil,
    ] {
        let mut seen: Vec<u16> = MAP
            .iter()
            .filter(|e| e.kind == kind)
            .map(|e| e.addr)
            .collect();
        let before = seen.len();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(before, seen.len(), "{kind:?} has a duplicate address");
        for block in BLOCKS.iter().filter(|b| b.kind == kind) {
            let end = block.base + block.stride * block.count;
            for addr in seen.iter() {
                assert!(
                    *addr < block.base || *addr >= end,
                    "{kind:?} {addr} is both fixed and in a block"
                );
            }
            for other in BLOCKS.iter().filter(|b| b.kind == kind) {
                if other.base == block.base {
                    continue;
                }
                let other_end = other.base + other.stride * other.count;
                assert!(
                    other_end <= block.base || other.base >= end,
                    "{kind:?} blocks at {} and {} overlap",
                    block.base,
                    other.base
                );
            }
        }
        // Every fixed entry answers through the map.
        for e in MAP.iter().filter(|e| e.kind == kind) {
            assert_eq!(
                name_of(kind, e.addr).as_deref(),
                Some(e.name),
                "{kind:?} {} is not in the map",
                e.addr
            );
            assert!(entry(kind, e.addr).is_some());
        }
    }
}

#[test]
fn the_markdown_documents_the_blocks() {
    let md = render_markdown();
    assert!(md.contains("Map version: 2"));
    assert!(md.contains("ADR 0004 decision 2"));
    for e in MAP {
        assert!(md.contains(e.name), "{} is undocumented", e.name);
    }
    for block in BLOCKS {
        for field in block.fields {
            assert!(md.contains(field.name), "{} is undocumented", field.name);
        }
    }
    assert!(md.contains("exception 2"));
    assert!(md.contains("absent"));
}

#[test]
fn the_result_code_follows_the_ack() {
    assert_eq!(
        ResultCode::of_reply(&Reply::accepted("c")),
        ResultCode::Accepted
    );
    assert_eq!(ResultCode::of_reply(&Reply::ok("c", "ok")), ResultCode::Ok);
    assert_eq!(
        ResultCode::of_reply(&Reply::err("c", "failed: sequence")),
        ResultCode::Failed
    );
    assert_eq!(
        ResultCode::of_reply(&Reply::err("c", "node 5 is not present")),
        ResultCode::Absent
    );
    assert_eq!(
        ResultCode::of_reply(&Reply::err("c", "bad argument")),
        ResultCode::Refused
    );
    assert_eq!(ResultCode::Idle.as_u16(), 0);
}
