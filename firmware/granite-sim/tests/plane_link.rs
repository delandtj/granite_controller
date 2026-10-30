//! A hand-written plane client against the fake link.
//!
//! This is the smallest thing that speaks ADR 0003 over
//! [`granite_core::hal::PlaneLink`]: discovery, the read blocks, a CTRL
//! write with its sequence number, the result in STATUS, EVENTS and
//! EV_ACK. The real client (granite-core `plane` module, controller
//! ADR 0004 decision 5) does the poll schedule, the retries and the
//! models on top; [`Client`] here is the seed of its tests, so the
//! helpers are kept clean and the assertions go through the wire rather
//! than through the simulated hardware.
//!
//! What the properties are:
//! - a scan of 0x30-0x37 finds exactly the planes the scenario has,
//! - INFO / STATUS / SUMMARY agree with the units and slots,
//! - a command completes asynchronously: Accepted, then an event on
//!   EXP_INT, then Ok, and EV_ACK releases the line,
//! - a resent CTRL write with the same sequence number is not executed
//!   twice (ADR 0003 decision 3),
//! - a corrupted answer fails its CRC and the retry is clean,
//! - an OFF slot is OFF again after its unit is unplugged and replugged
//!   (ADR 0002 decision 4, the "OFF goes back OFF" rule),
//! - a unit whose hot-swap faults is latched off by the plane and comes
//!   back on `unit_reset` (ADR 0002 decision 7a),
//! - a plane that lost its power flags "intended state lost", and one
//!   that only reset on its watchdog does not.

use std::path::Path;

use granite_core::hal::{LinkError, PlaneLink};
use granite_proto::{
    AckThrough, BLK_CTRL, BLK_EV_ACK, BLK_EVENTS, BLK_INFO, BLK_SLOT, BLK_STATUS, BLK_SUMMARY,
    BLK_UNIT, Command, Event, EventKind, EventRecord, INDEX_NONE, Info, LedMode, MAX_PAYLOAD,
    MAX_PLANES, RecordWriter, ResultCode, Slot, SlotStatus, SlotSummary, Status, StatusFlags, Unit,
    UnitStatus, frame, plane_address,
};
use granite_sim::Scenario;
use granite_sim::planes::{FakePlaneLink, PLANE_TICK_MS};
use granite_sim::scenario::{PlaneScenario, PlaneSlotScenario};

// --- the mini client --------------------------------------------------

/// How a read can fail: the link did not carry it, or what came back
/// did not check out. The two are kept apart because the controller
/// treats them differently (absence against a retry).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReadError {
    /// The transaction itself failed.
    Link(LinkError),
    /// The answer failed its length or CRC check.
    Frame(frame::FrameError),
}

/// A plane client over a [`PlaneLink`], with the simulator's clock.
struct Client {
    /// The link and the planes behind it.
    link: FakePlaneLink,
    /// The controller's monotonic clock \[ms\].
    now_ms: u64,
}

impl Client {
    /// A client over the planes of `scenario`, with the hosts' LEDs
    /// settled: the plane only calls an LED steady after its blink
    /// window plus a second.
    fn new(scenario: &Scenario) -> Self {
        let mut client = Client {
            link: FakePlaneLink::from_scenario(scenario),
            now_ms: 0,
        };
        client.advance(2_000);
        client
    }

    /// Move the controller's clock and let the planes run up to it.
    fn advance(&mut self, ms: u64) {
        self.now_ms += ms;
        self.link.tick(self.now_ms);
    }

    /// One read: request, check the answer against the address and the
    /// request, give back the payload.
    fn read(&mut self, addr: u8, block: u8, index: u8) -> Result<Vec<u8>, ReadError> {
        let request = frame::read_request(block, index);
        let mut answer = [0u8; frame::MAX_ANSWER];
        let n = self
            .link
            .write_read(addr, &request, &mut answer)
            .map_err(ReadError::Link)?;
        frame::check_answer(addr, &request, &answer[..n])
            .map(<[u8]>::to_vec)
            .map_err(ReadError::Frame)
    }

    /// A read with the retry of ADR 0003: a lost or corrupted answer is
    /// asked for again, up to `tries` times.
    fn read_retry(
        &mut self,
        addr: u8,
        block: u8,
        index: u8,
        tries: u32,
    ) -> Result<Vec<u8>, ReadError> {
        let mut last = ReadError::Link(LinkError::Timeout);
        for _ in 0..tries.max(1) {
            match self.read(addr, block, index) {
                Ok(payload) => return Ok(payload),
                Err(e) => last = e,
            }
        }
        Err(last)
    }

    /// A read that must work.
    fn expect_read(&mut self, addr: u8, block: u8, index: u8) -> Vec<u8> {
        self.read(addr, block, index)
            .unwrap_or_else(|e| panic!("read of block {block:#04x} on {addr:#04x} failed: {e:?}"))
    }

    /// One write frame.
    fn write(&mut self, addr: u8, block: u8, payload: &[u8]) -> Result<(), LinkError> {
        let mut buf = [0u8; frame::MAX_WRITE_FRAME];
        let bytes = frame::write_frame(&mut buf, addr, block, INDEX_NONE, payload)
            .expect("the write frame fits");
        self.link.write(addr, bytes)
    }

    /// Scan 0x30-0x37 and keep the addresses that answer INFO.
    fn scan(&mut self) -> Vec<u8> {
        let mut found = Vec::new();
        for n in 0..MAX_PLANES as u8 {
            let addr = plane_address(n).expect("n is a plane number");
            if self.read(addr, BLK_INFO, INDEX_NONE).is_ok() {
                found.push(addr);
            }
        }
        found
    }

    /// The INFO records of one plane.
    fn info(&mut self, addr: u8) -> Vec<Info> {
        let payload = self.expect_read(addr, BLK_INFO, INDEX_NONE);
        Info::records(&payload)
            .map(|r| r.expect("an INFO record"))
            .collect()
    }

    /// The STATUS records of one plane.
    fn status(&mut self, addr: u8) -> Vec<Status> {
        let payload = self.expect_read(addr, BLK_STATUS, INDEX_NONE);
        Status::records(&payload)
            .map(|r| r.expect("a STATUS record"))
            .collect()
    }

    /// The plane flags (STATUS 0x13).
    fn flags(&mut self, addr: u8) -> StatusFlags {
        self.status(addr)
            .into_iter()
            .find_map(|r| match r {
                Status::Flags(f) => Some(f),
                _ => None,
            })
            .expect("STATUS carries its flags")
    }

    /// The plane's boot count (INFO 0x08).
    fn boot_count(&mut self, addr: u8) -> u32 {
        self.info(addr)
            .into_iter()
            .find_map(|r| match r {
                Info::BootCount(n) => Some(n),
                _ => None,
            })
            .expect("INFO carries its boot count")
    }

    /// The units present bitmap (INFO 0x07).
    fn units_present(&mut self, addr: u8) -> u8 {
        self.info(addr)
            .into_iter()
            .find_map(|r| match r {
                Info::UnitsPresent(map) => Some(map.bits()),
                _ => None,
            })
            .expect("INFO carries its unit bitmap")
    }

    /// The result the plane holds for a command sequence number
    /// (STATUS 0x16).
    fn result(&mut self, addr: u8, seq: u8) -> Option<ResultCode> {
        self.status(addr).into_iter().find_map(|r| match r {
            Status::CommandResult(held) if held.seq == seq => Some(held.code),
            _ => None,
        })
    }

    /// The SUMMARY records: one per present slot.
    fn summary(&mut self, addr: u8) -> Vec<SlotSummary> {
        let payload = self.expect_read(addr, BLK_SUMMARY, INDEX_NONE);
        SlotSummary::records(&payload)
            .map(|r| r.expect("a SUMMARY record"))
            .collect()
    }

    /// One slot's status word, read from its own block so an absent
    /// slot answers too (ADR 0003 decision 2).
    fn slot_status(&mut self, addr: u8, slot: u8) -> SlotStatus {
        let payload = self.expect_read(addr, BLK_SLOT, slot);
        Slot::records(&payload)
            .find_map(|r| match r {
                Ok(Slot::Status(s)) => Some(s),
                _ => None,
            })
            .expect("a slot answer carries its status")
    }

    /// One unit's status word.
    fn unit_status(&mut self, addr: u8, unit: u8) -> UnitStatus {
        let payload = self.expect_read(addr, BLK_UNIT, unit);
        Unit::records(&payload)
            .find_map(|r| match r {
                Ok(Unit::Status(s)) => Some(s),
                _ => None,
            })
            .expect("a unit answer carries its status")
    }

    /// The unacknowledged events of one plane.
    fn events(&mut self, addr: u8) -> Vec<Event> {
        let payload = self.expect_read(addr, BLK_EVENTS, INDEX_NONE);
        EventRecord::records(&payload)
            .filter_map(|r| match r {
                Ok(EventRecord::Event(e)) => Some(e),
                _ => None,
            })
            .collect()
    }

    /// Write a CTRL set under `seq`. The plane executes it on its next
    /// cycle, so a caller reads the result after advancing.
    fn ctrl(&mut self, addr: u8, seq: u8, commands: &[Command]) {
        let mut buf = [0u8; MAX_PAYLOAD];
        let mut w = RecordWriter::new(&mut buf);
        Command::write_ctrl(&mut w, seq, commands).expect("the commands fit");
        let payload = w.payload().to_vec();
        self.write(addr, BLK_CTRL, &payload)
            .expect("the plane took the write");
    }

    /// Acknowledge every event up to and including `seq`.
    fn ev_ack(&mut self, addr: u8, seq: u16) {
        let mut buf = [0u8; 8];
        let mut w = RecordWriter::new(&mut buf);
        AckThrough::new(seq)
            .encode(&mut w)
            .expect("the record fits");
        let payload = w.payload().to_vec();
        self.write(addr, BLK_EV_ACK, &payload)
            .expect("the plane took the acknowledgement");
        self.advance(PLANE_TICK_MS);
    }

    /// Read and acknowledge everything the plane holds, the way the
    /// client does on EXP_INT, until it holds nothing.
    fn drain_events(&mut self, addr: u8) {
        for _ in 0..8 {
            let events = self.events(addr);
            let Some(last) = events.last() else { return };
            self.ev_ack(addr, last.seq);
        }
        panic!("the plane at {addr:#04x} keeps raising events");
    }
}

// --- scenarios --------------------------------------------------------

/// A plane section with the blink window shortened, so a wait on a host
/// LED is about a second rather than the five a shipped plane takes.
fn plane(address: u8, units: Vec<u8>, slots: Vec<PlaneSlotScenario>) -> PlaneScenario {
    PlaneScenario {
        address,
        units,
        blink_window_ms: Some([100, 500]),
        slots,
        faults: Vec::new(),
    }
}

/// One slot with a module and a host.
fn slot(slot: u8, host_on: bool, load_ma: u32) -> PlaneSlotScenario {
    PlaneSlotScenario {
        slot,
        host_on,
        load_ma,
        ..Default::default()
    }
}

/// The scenario the behaviour tests run on: a plane at 0x30 with two
/// units and four running hosts, and a plane at 0x31 with one unit whose
/// B half is dark. No scheduled faults; every test injects its own.
fn scenario() -> Scenario {
    let mut scenario = Scenario {
        planes: vec![
            plane(
                0x30,
                vec![0, 1],
                (0..4).map(|s| slot(s, true, 1_000)).collect(),
            ),
            plane(
                0x31,
                vec![0],
                vec![slot(0, true, 1_300), slot(1, false, 700)],
            ),
        ],
        ..Default::default()
    };
    scenario.normalise();
    scenario
}

/// The sample scenario shipped next to the other ones.
fn sample() -> Scenario {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("scenarios/two-planes.toml");
    Scenario::load(&path).expect("the sample scenario parses")
}

// --- discovery --------------------------------------------------------

#[test]
fn a_scan_finds_exactly_the_scenario_planes() {
    let mut client = Client::new(&scenario());
    assert_eq!(client.scan(), vec![0x30, 0x31]);
    // An address with no plane is a NAK, which is how the client reads
    // absence; it is not a frame error.
    assert_eq!(
        client.read(0x33, BLK_INFO, INDEX_NONE),
        Err(ReadError::Link(LinkError::Nak))
    );
    assert_eq!(client.link.addresses(), vec![0x30, 0x31]);
}

#[test]
fn info_status_and_summary_report_the_units_and_the_slots() {
    let mut client = Client::new(&scenario());

    let info = client.info(0x30);
    assert!(info.contains(&Info::FormatVersion(granite_proto::FORMAT_VERSION)));
    assert!(info.contains(&Info::PlaneType(granite_proto::PlaneType::Pp48Chained)));
    assert!(info.contains(&Info::SlotCapacity(granite_proto::SLOT_COUNT as u8)));
    assert_eq!(client.units_present(0x30), 0b0011, "two units on the chain");
    assert_eq!(client.units_present(0x31), 0b0001);
    assert_eq!(client.boot_count(0x30), 1);

    // Nothing is wrong, so no fault flag is up. "Intended state lost"
    // is up, and belongs: the plane has just had its power, so its
    // backup registers were empty and every slot came up ON. That is
    // the controller's cue to re-apply what it last commanded OFF.
    let flags = client.flags(0x30);
    assert!(!flags.contains(StatusFlags::ANY_SLOT_FAULT));
    assert!(!flags.contains(StatusFlags::ANY_UNIT_FAULT));
    assert!(flags.contains(StatusFlags::INTENDED_STATE_LOST));
    assert!(
        client
            .status(0x30)
            .iter()
            .any(|r| matches!(r, Status::CoreInputVoltage(v) if *v > 4_000)),
        "the plane reports its 48 V input"
    );

    // Four populated slots, every one on with a steady LED.
    let summary = client.summary(0x30);
    assert_eq!(summary.len(), 4);
    for record in &summary {
        assert!(record.status.contains(SlotStatus::PRESENT));
        assert!(record.status.contains(SlotStatus::ENABLED));
        assert!(record.status.contains(SlotStatus::PGD));
        assert_eq!(record.led, LedMode::On, "slot {} is lit", record.slot);
        assert!(record.amps > 0, "slot {} draws its load", record.slot);
    }

    // The unit blocks agree with them.
    for unit in 0..2u8 {
        let status = client.unit_status(0x30, unit);
        assert!(status.contains(UnitStatus::PRESENT));
        assert!(status.contains(UnitStatus::EXPANDER_OK));
        assert!(status.contains(UnitStatus::HOT_SWAP_PGD));
        assert!(!status.contains(UnitStatus::HOT_SWAP_LATCHED_OFF));
    }
    // A unit that is not on the chain still answers, with the present
    // bit clear.
    assert!(!client.unit_status(0x30, 3).contains(UnitStatus::PRESENT));

    // The second plane: one unit, and its B half is dark.
    let summary = client.summary(0x31);
    assert_eq!(summary.len(), 2);
    assert_eq!(summary[0].led, LedMode::On);
    assert_eq!(summary[1].led, LedMode::Off);
}

// --- commands ---------------------------------------------------------

#[test]
fn a_press_completes_through_exp_int_and_ev_ack_releases_the_line() {
    let mut client = Client::new(&scenario());
    client.drain_events(0x30);
    client.drain_events(0x31);
    assert!(
        !client.link.int_asserted(),
        "the boot events have been acknowledged"
    );

    client.ctrl(0x30, 7, &[Command::PressPwr { slot: 0, ms: 200 }]);
    client.advance(PLANE_TICK_MS);
    assert_eq!(
        client.result(0x30, 7),
        Some(ResultCode::Accepted),
        "a sequence answers Accepted first (ADR 0003 decision 4)"
    );
    assert!(client.flags(0x30).contains(StatusFlags::SEQUENCE_RUNNING));

    client.advance(400);
    assert_eq!(client.result(0x30, 7), Some(ResultCode::Ok));
    assert!(client.link.int_asserted(), "the sequence raised an event");
    assert!(
        client
            .events(0x30)
            .iter()
            .any(|e| e.kind == EventKind::SequenceDone),
        "the event says the sequence is done"
    );

    // The host took the press and shut down, so its LED change is an
    // event too; let it settle before acknowledging, or the line comes
    // straight back up.
    client.advance(3_000);
    let events = client.events(0x30);
    let last = events.last().expect("events are pending").seq;
    assert!(events.iter().any(|e| e.kind == EventKind::LedChanged));
    client.ev_ack(0x30, last);
    assert!(
        !client.link.int_asserted(),
        "EXP_INT drops when nothing is unacknowledged"
    );
    assert_eq!(
        client.summary(0x30)[0].led,
        LedMode::Off,
        "the host went down with the press"
    );
}

#[test]
fn a_resent_ctrl_write_with_the_same_seq_is_not_executed_twice() {
    let mut client = Client::new(&scenario());
    // Plane 0x31's B half is dark, so one press turns it on and a
    // second would shut it down again.
    assert_eq!(client.summary(0x31)[1].led, LedMode::Off);

    let press = [Command::PressPwr { slot: 1, ms: 200 }];
    client.ctrl(0x31, 42, &press);
    client.advance(2_000);
    assert_eq!(client.result(0x31, 42), Some(ResultCode::Ok));
    assert_eq!(client.summary(0x31)[1].led, LedMode::On, "the host came up");

    // The controller did not see the result and resends the identical
    // frame (ADR 0003 decision 3: the seq memory makes that safe).
    client.ctrl(0x31, 42, &press);
    client.advance(2_000);
    assert_eq!(client.result(0x31, 42), Some(ResultCode::Ok));
    assert!(
        !client.flags(0x31).contains(StatusFlags::SEQUENCE_RUNNING),
        "no second sequence ran"
    );
    assert_eq!(
        client.summary(0x31)[1].led,
        LedMode::On,
        "a second press would have shut it down"
    );
}

// --- the link's own failures ------------------------------------------

#[test]
fn a_corrupted_answer_fails_its_crc_and_the_retry_is_clean() {
    let mut client = Client::new(&scenario());
    client
        .link
        .plane_mut(0x30)
        .expect("plane 0x30")
        .corrupt_answers(1);

    assert_eq!(
        client.read(0x30, BLK_STATUS, INDEX_NONE),
        Err(ReadError::Frame(frame::FrameError::BadCrc)),
        "the flipped byte is caught by the CRC, not by the length"
    );
    assert!(
        client.read(0x30, BLK_STATUS, INDEX_NONE).is_ok(),
        "the next answer is good"
    );

    // And the retry loop a client runs papers over a single corruption.
    client
        .link
        .plane_mut(0x30)
        .expect("plane 0x30")
        .corrupt_answers(1);
    assert!(client.read_retry(0x30, BLK_SUMMARY, INDEX_NONE, 3).is_ok());
}

#[test]
fn dropped_transactions_look_like_a_plane_that_is_not_there() {
    let mut client = Client::new(&scenario());
    client
        .link
        .plane_mut(0x30)
        .expect("plane 0x30")
        .drop_transactions(2);

    for _ in 0..2 {
        assert_eq!(
            client.read(0x30, BLK_STATUS, INDEX_NONE),
            Err(ReadError::Link(LinkError::Nak))
        );
    }
    assert!(client.read(0x30, BLK_STATUS, INDEX_NONE).is_ok());
    assert_eq!(client.link.plane(0x30).expect("plane 0x30").dropped, 2);

    // A write is dropped the same way, and the retry with the same seq
    // is what the controller does about it.
    client
        .link
        .plane_mut(0x30)
        .expect("plane 0x30")
        .drop_transactions(1);
    let mut buf = [0u8; MAX_PAYLOAD];
    let mut w = RecordWriter::new(&mut buf);
    Command::write_ctrl(&mut w, 3, &[Command::ClearMinMax { slot: 0 }]).expect("it fits");
    let payload = w.payload().to_vec();
    assert_eq!(client.write(0x30, BLK_CTRL, &payload), Err(LinkError::Nak));
    assert!(client.write(0x30, BLK_CTRL, &payload).is_ok());
    client.advance(PLANE_TICK_MS);
    assert_eq!(client.result(0x30, 3), Some(ResultCode::Ok));
}

// --- the state the plane keeps ----------------------------------------

#[test]
fn an_off_slot_is_off_again_after_its_unit_is_replugged() {
    let mut client = Client::new(&scenario());
    client.ctrl(0x30, 1, &[Command::Enable { slot: 2, on: false }]);
    client.advance(500);
    assert_eq!(client.result(0x30, 1), Some(ResultCode::Ok));
    assert!(!client.slot_status(0x30, 2).contains(SlotStatus::ENABLED));

    // Unit 1's chain cable comes out: no 3V3, so its expander forgets
    // its outputs and slot 2's rail comes back on its own.
    client
        .link
        .plane_mut(0x30)
        .expect("plane 0x30")
        .set_unit_power(1, false);
    client.advance(2_000);
    assert!(!client.unit_status(0x30, 1).contains(UnitStatus::PRESENT));
    assert!(client.unit_status(0x30, 1).contains(UnitStatus::LOST));
    assert!(
        client.flags(0x30).contains(StatusFlags::ANY_UNIT_FAULT),
        "a lost unit is a fault the controller should see"
    );

    // And goes back in.
    client
        .link
        .plane_mut(0x30)
        .expect("plane 0x30")
        .set_unit_power(1, true);
    client.advance(3_000);
    let unit = client.unit_status(0x30, 1);
    assert!(unit.contains(UnitStatus::PRESENT));
    assert!(
        unit.contains(UnitStatus::RESET_SEEN),
        "the unit came back at hardware defaults"
    );
    assert!(
        !client.slot_status(0x30, 2).contains(SlotStatus::ENABLED),
        "OFF goes back OFF (ADR 0002 decision 4)"
    );
    assert!(
        client.slot_status(0x30, 3).contains(SlotStatus::ENABLED),
        "its partner on the same unit is untouched"
    );
}

#[test]
fn a_faulted_unit_is_latched_off_and_unit_reset_brings_it_back() {
    let mut client = Client::new(&scenario());
    client.drain_events(0x30);

    // An overload on unit 1's input: its LM5069-2 faults and retries
    // for ever, so the plane is what holds it off (ADR 0005 decision 3).
    client
        .link
        .plane_mut(0x30)
        .expect("plane 0x30")
        .fault_unit_hot_swap(1, true);
    client.advance(1_000);

    let unit = client.unit_status(0x30, 1);
    assert!(unit.contains(UnitStatus::PRESENT), "the unit still answers");
    assert!(unit.contains(UnitStatus::HOT_SWAP_LATCHED_OFF));
    assert!(unit.contains(UnitStatus::U_OFF_ASSERTED));
    assert!(!unit.contains(UnitStatus::HOT_SWAP_PGD));
    assert!(client.flags(0x30).contains(StatusFlags::ANY_UNIT_FAULT));
    assert!(
        client
            .events(0x30)
            .iter()
            .any(|e| e.kind == EventKind::UnitHotSwapLatchedOff),
        "the plane said so once"
    );
    client.drain_events(0x30);

    // A reset while the overload is still there is answered `failed`
    // and the unit is held off again.
    client.ctrl(0x30, 11, &[Command::UnitHotSwapReset { unit: 1 }]);
    client.advance(PLANE_TICK_MS);
    assert_eq!(client.result(0x30, 11), Some(ResultCode::Accepted));
    client.advance(1_500);
    assert_eq!(client.result(0x30, 11), Some(ResultCode::Failed));
    assert!(
        client
            .unit_status(0x30, 1)
            .contains(UnitStatus::HOT_SWAP_LATCHED_OFF)
    );

    // The shorted module comes out, and the reset takes.
    client
        .link
        .plane_mut(0x30)
        .expect("plane 0x30")
        .fault_unit_hot_swap(1, false);
    client.ctrl(0x30, 12, &[Command::UnitHotSwapReset { unit: 1 }]);
    client.advance(1_500);
    assert_eq!(client.result(0x30, 12), Some(ResultCode::Ok));
    let unit = client.unit_status(0x30, 1);
    assert!(unit.contains(UnitStatus::HOT_SWAP_PGD));
    assert!(!unit.contains(UnitStatus::HOT_SWAP_LATCHED_OFF));
    assert!(!unit.contains(UnitStatus::U_OFF_ASSERTED));
    assert!(!client.flags(0x30).contains(StatusFlags::ANY_UNIT_FAULT));
    for slot in 2..4u8 {
        assert!(client.slot_status(0x30, slot).contains(SlotStatus::PGD));
    }
}

#[test]
fn a_watchdog_reset_keeps_off_slots_and_a_power_loss_says_it_lost_them() {
    let mut client = Client::new(&scenario());
    client.ctrl(0x31, 3, &[Command::Enable { slot: 0, on: false }]);
    client.advance(500);
    assert!(!client.slot_status(0x31, 0).contains(SlotStatus::ENABLED));
    assert_eq!(client.boot_count(0x31), 1);

    // A watchdog reset keeps VDD: the backup registers and the units'
    // expanders survive, so nothing switched.
    client
        .link
        .plane_mut(0x31)
        .expect("plane 0x31")
        .reboot(plane_logic::reset_cause::WATCHDOG);
    client.advance(2_000);
    assert_eq!(client.boot_count(0x31), 2, "the controller sees the reboot");
    assert!(
        !client
            .flags(0x31)
            .contains(StatusFlags::INTENDED_STATE_LOST)
    );
    assert!(!client.slot_status(0x31, 0).contains(SlotStatus::ENABLED));

    // A power loss takes the backup registers with it: the slot comes
    // back ON and the plane says the intended state is gone, which is
    // the controller's cue to re-apply OFF (ADR 0004).
    client
        .link
        .plane_mut(0x31)
        .expect("plane 0x31")
        .power_loss();
    client.advance(2_000);
    assert!(
        client
            .flags(0x31)
            .contains(StatusFlags::INTENDED_STATE_LOST)
    );
    assert!(
        client.slot_status(0x31, 0).contains(SlotStatus::ENABLED),
        "the slot came back on, which is why the flag exists"
    );
    assert!(
        client
            .status(0x31)
            .contains(&Status::LastResetCause(plane_logic::reset_cause::POWER_ON))
    );
}

// --- the sample scenario ----------------------------------------------

#[test]
fn the_sample_scenario_describes_two_planes_and_injects_its_faults() {
    let scenario = sample();
    assert_eq!(scenario.planes.len(), 2);
    assert_eq!(scenario.plane(0x30).expect("0x30").units, vec![0, 1, 2, 3]);
    assert_eq!(scenario.plane(0x31).expect("0x31").units, vec![0]);

    let mut client = Client::new(&scenario);
    assert_eq!(client.scan(), vec![0x30, 0x31]);
    assert_eq!(client.units_present(0x30), 0b1111, "four unit boards");
    // SUMMARY carries both slots of every present unit; an empty slot
    // travels with its present bit clear rather than being left out.
    let summary = client.summary(0x30);
    assert_eq!(summary.len(), 8, "four unit boards, two slots each");
    assert!(
        !summary
            .iter()
            .find(|s| s.slot == 5)
            .expect("slot 5 is in the summary")
            .status
            .contains(SlotStatus::PRESENT),
        "slot 5 has no module"
    );
    assert!(!client.slot_status(0x30, 5).contains(SlotStatus::PRESENT));
    assert_eq!(
        summary.iter().find(|s| s.slot == 3).expect("slot 3").led,
        LedMode::Blinking,
        "slot 3's host sleeps"
    );

    // 20 s: three dropped transactions. Nobody read in between, so they
    // are still waiting for the next three.
    client.advance(23_000);
    for _ in 0..3 {
        assert_eq!(
            client.read(0x30, BLK_STATUS, INDEX_NONE),
            Err(ReadError::Link(LinkError::Nak))
        );
    }
    assert!(client.read(0x30, BLK_STATUS, INDEX_NONE).is_ok());

    // 30 s: one corrupted answer.
    client.advance(10_000);
    assert_eq!(
        client.read(0x30, BLK_STATUS, INDEX_NONE),
        Err(ReadError::Frame(frame::FrameError::BadCrc))
    );
    assert!(client.read(0x30, BLK_STATUS, INDEX_NONE).is_ok());

    // 45 s: unit 1's cable comes out. 60 s: it goes back in.
    client.advance(17_000);
    assert!(!client.unit_status(0x30, 1).contains(UnitStatus::PRESENT));
    client.advance(20_000);
    assert!(client.unit_status(0x30, 1).contains(UnitStatus::PRESENT));

    // 75 s: slot 6 trips its power limit.
    client.advance(10_000);
    let slot6 = client.slot_status(0x30, 6);
    assert!(slot6.contains(SlotStatus::POWER_LIMIT_TRIPPED));
    assert!(slot6.contains(SlotStatus::OV_LATCHED));
    assert!(!slot6.contains(SlotStatus::PGD));

    // 90 s: the second plane loses its power.
    client.advance(15_000);
    assert!(
        client
            .flags(0x31)
            .contains(StatusFlags::INTENDED_STATE_LOST)
    );
}
