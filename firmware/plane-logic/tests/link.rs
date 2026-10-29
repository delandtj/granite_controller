//! The controller link: every read block answered from the model, the
//! CTRL sequence memory, the event ring with EV_ACK and its overflow,
//! CONFIG_W as a set, and the counters (ADR 0002 decision 8, ADR 0003
//! decisions 1-5).
//!
//! The frames here are built and checked with granite-proto, never by
//! hand, so a change on either side of the contract breaks a test
//! rather than a board.

#![cfg(feature = "sim")]

use granite_proto::{
    BLK_CONFIG, BLK_CTRL, BLK_EVENTS, BLK_INFO, BLK_SLOT, BLK_STATUS, BLK_SUMMARY, BLK_UNIT,
    Capabilities, Command, Config, EventKind, EventRecord, EventTarget, INDEX_NONE, Info, PlaneType,
    ResultCode, SlotStatus, Status, StatusFlags, SlotSummary, frame,
};
use plane_logic::sim::{SIM_ADDRESS, Sim};

fn settled(units: usize) -> Sim {
    let mut sim = Sim::with_units(units);
    sim.boot();
    sim.fast_led(100, 500);
    sim.advance(2_000);
    sim
}

// --- reads ------------------------------------------------------------

#[test]
fn info_says_what_the_plane_is() {
    let mut sim = settled(2);
    let payload = sim.read_block(BLK_INFO, INDEX_NONE);
    let records: Vec<Info> = Info::records(&payload).map(Result::unwrap).collect();
    assert!(records.contains(&Info::FormatVersion(granite_proto::FORMAT_VERSION)));
    assert!(records.contains(&Info::PlaneType(PlaneType::Pp48Chained)));
    assert!(records.contains(&Info::SlotCapacity(8)));
    assert!(records.contains(&Info::BootCount(1)));
    assert!(records.contains(&Info::ImageState(granite_proto::ImageState::Confirmed)));
    // Units present is a bitmap of the chain.
    assert!(records.iter().any(|r| matches!(r, Info::UnitsPresent(m) if m.bits() == 0b0011)));
    // The press deadline hardware is there, so the bit is set.
    assert!(
        records
            .iter()
            .any(|r| matches!(r, Info::Capabilities(c) if c.contains(Capabilities::PRESS_DEADLINE)))
    );
    // The hash matches what CONFIG answers with.
    let hash = records
        .iter()
        .find_map(|r| match r {
            Info::ConfigHash(h) => Some(*h),
            _ => None,
        })
        .unwrap();
    let cfg = sim.read_block(BLK_CONFIG, INDEX_NONE);
    assert!(Config::records(&cfg).map(Result::unwrap).any(|r| r == Config::Hash(hash)));
}

#[test]
fn status_carries_the_counters_and_the_flags() {
    let mut sim = settled(1);
    let payload = sim.read_block(BLK_STATUS, INDEX_NONE);
    let records: Vec<Status> = Status::records(&payload).map(Result::unwrap).collect();
    assert!(records.contains(&Status::CoreInputVoltage(4_800)));
    assert!(records.contains(&Status::LastResetCause(plane_logic::reset_cause::POWER_ON)));
    assert!(records.iter().any(|r| matches!(r, Status::Uptime(_))));
    assert!(records.iter().any(|r| matches!(r, Status::EventHead(_))));
    assert!(records.iter().any(|r| matches!(r, Status::SegmentErrors(_))));
    // The boot event is still unacknowledged, so the plane says so and
    // INT_OUT is high.
    assert!(
        records
            .iter()
            .any(|r| matches!(r, Status::Flags(f) if f.contains(StatusFlags::EVENTS_PENDING)))
    );
    assert!(sim.plane.event_pending());
}

#[test]
fn summary_is_one_record_per_present_slot() {
    let mut sim = settled(2);
    let payload = sim.read_block(BLK_SUMMARY, INDEX_NONE);
    let records: Vec<SlotSummary> = SlotSummary::records(&payload).map(Result::unwrap).collect();
    assert_eq!(records.len(), 4, "two units, four slots");
    assert_eq!(payload.len(), 4 * (SlotSummary::LEN + 2));
    assert_eq!(records[0].slot, 0);
    assert_eq!(records[0].volts, 4_800);
    assert_eq!(records[0].led, granite_proto::LedMode::On);
    assert!(records[0].status.contains(SlotStatus::PRESENT | SlotStatus::PGD));

    // The whole plane fits a block with room to spare.
    let mut full = Sim::with_units(4);
    full.boot();
    full.advance(1_000);
    let payload = full.read_block(BLK_SUMMARY, INDEX_NONE);
    assert_eq!(payload.len(), 96);
    assert!(payload.len() <= granite_proto::MAX_PAYLOAD);
}

#[test]
fn every_block_fits_one_frame() {
    let mut sim = Sim::with_units(4);
    sim.boot();
    sim.advance(2_000);
    for _ in 0..40 {
        sim.ctrl(0, &[Command::Enable { slot: 0, on: true }]);
        sim.tick();
    }
    for (block, index) in [
        (BLK_INFO, INDEX_NONE),
        (BLK_STATUS, INDEX_NONE),
        (BLK_SUMMARY, INDEX_NONE),
        (BLK_CONFIG, INDEX_NONE),
        (BLK_EVENTS, INDEX_NONE),
        (granite_proto::BLK_LOG, INDEX_NONE),
        (BLK_UNIT, 0),
        (BLK_SLOT, 7),
    ] {
        let payload = sim.read_block(block, index);
        assert!(
            payload.len() <= granite_proto::MAX_PAYLOAD,
            "block {block:#04x} is {} bytes",
            payload.len()
        );
        // Nothing in a block is a truncated record.
        for record in granite_proto::Records::new(&payload) {
            record.expect("a well-formed record");
        }
    }
}

#[test]
fn a_slot_block_carries_the_whole_model() {
    let mut sim = settled(1);
    sim.advance(1_000);
    let payload = sim.read_block(BLK_SLOT, 0);
    let tags: Vec<u8> = granite_proto::Records::new(&payload)
        .map(|r| r.unwrap().0)
        .collect();
    for tag in [
        granite_proto::TAG_SLOT_STATUS,
        granite_proto::TAG_RETRY_COUNT,
        granite_proto::TAG_MODULE_CLASS,
        granite_proto::TAG_MODULE_ID_RAW,
        granite_proto::TAG_SLOT_V,
        granite_proto::TAG_SLOT_I,
        granite_proto::TAG_SLOT_P,
        granite_proto::TAG_SLOT_P_MIN_MAX,
        granite_proto::TAG_SLOT_TEMPERATURE,
        granite_proto::TAG_SLOT_ENERGY,
        granite_proto::TAG_NODE_LED,
        granite_proto::TAG_CROSS_CHECK,
        granite_proto::TAG_SEQUENCE_STATE,
        granite_proto::TAG_POWER_LIMIT_IN_FORCE,
    ] {
        assert!(tags.contains(&tag), "tag {tag:#04x} is missing");
    }
}

#[test]
fn an_unknown_block_is_answered_with_nothing_rather_than_a_nak() {
    let mut sim = settled(1);
    let payload = sim.read_block(0x7E, INDEX_NONE);
    assert!(payload.is_empty());
    // A slot index out of range likewise.
    let payload = sim.read_block(BLK_SLOT, 9);
    assert!(payload.is_empty());
}

// --- writes and the sequence memory -----------------------------------

#[test]
fn a_resent_ctrl_write_is_not_executed_twice() {
    let mut sim = settled(1);
    assert_eq!(
        sim.ctrl_now(7, &[Command::Enable { slot: 0, on: false }]),
        ResultCode::Ok
    );
    assert!(sim.plane.intended().off(0));

    // The controller never saw the result and resends the identical
    // frame; the plane keeps the result and does nothing (ADR 0003
    // decision 3).
    sim.ctrl_now(7, &[Command::Enable { slot: 0, on: true }]);
    assert_eq!(sim.result(7), Some(ResultCode::Ok));
    assert!(sim.plane.intended().off(0), "the resend changed nothing");

    // A different number does run.
    assert_eq!(
        sim.ctrl_now(8, &[Command::Enable { slot: 0, on: true }]),
        ResultCode::Ok
    );
    assert!(!sim.plane.intended().off(0));
}

#[test]
fn status_holds_the_last_eight_results() {
    let mut sim = settled(1);
    for seq in 1..=12u8 {
        sim.ctrl_now(seq, &[Command::ClearMinMax { slot: 0 }]);
    }
    let payload = sim.read_block(BLK_STATUS, INDEX_NONE);
    let results: Vec<_> = Status::records(&payload)
        .map(Result::unwrap)
        .filter_map(|r| match r {
            Status::CommandResult(c) => Some(c),
            _ => None,
        })
        .collect();
    assert_eq!(results.len(), granite_proto::SEQ_MEMORY);
    assert_eq!(results.first().unwrap().seq, 5);
    assert_eq!(results.last().unwrap().seq, 12);
    assert!(results.iter().all(|r| r.code == ResultCode::Ok));
}

#[test]
fn an_unknown_ctrl_tag_is_an_error_for_that_write() {
    let mut sim = settled(1);
    let mut buf = [0u8; 32];
    let mut w = granite_proto::RecordWriter::new(&mut buf);
    Command::Seq(3).encode(&mut w).unwrap();
    w.push(0x5E, &[0x00]).unwrap();
    let payload = w.payload().to_vec();
    sim.write_block(BLK_CTRL, &payload);
    sim.tick();
    assert_eq!(sim.result(3), Some(ResultCode::UnknownTag));
}

#[test]
fn a_bad_crc_is_counted_and_never_acted_on() {
    let mut sim = settled(1);
    let mut buf = [0u8; 32];
    let mut w = granite_proto::RecordWriter::new(&mut buf);
    Command::write_ctrl(&mut w, 4, &[Command::Enable { slot: 0, on: false }]).unwrap();
    let mut out = [0u8; frame::MAX_WRITE_FRAME];
    let written = frame::write_frame(&mut out, SIM_ADDRESS, BLK_CTRL, INDEX_NONE, w.payload())
        .unwrap()
        .to_vec();
    let mut corrupt = written.clone();
    *corrupt.last_mut().unwrap() ^= 0xFF;

    let mut answer = [0u8; frame::MAX_ANSWER];
    assert!(sim.plane.handle_frame(&corrupt, &mut answer).is_none());
    sim.tick();
    assert_eq!(sim.result(4), None, "the write never happened");
    assert!(!sim.plane.intended().off(0));

    let payload = sim.read_block(BLK_STATUS, INDEX_NONE);
    let errors = Status::records(&payload)
        .map(Result::unwrap)
        .find_map(|r| match r {
            Status::LinkErrors(e) => Some(e),
            _ => None,
        })
        .unwrap();
    assert_eq!(errors.bad_crc, 1);

    // The same frame with its CRC intact does act.
    let mut answer = [0u8; frame::MAX_ANSWER];
    assert!(sim.plane.handle_frame(&written, &mut answer).is_none());
    sim.tick();
    assert_eq!(sim.result(4), Some(ResultCode::Ok));
}

#[test]
fn an_answer_to_another_request_fails_its_crc() {
    let mut sim = settled(1);
    let request = frame::read_request(BLK_SLOT, 0);
    let mut out = [0u8; frame::MAX_ANSWER];
    let answer = sim.plane.handle_frame(&request, &mut out).unwrap().to_vec();
    // The CRC covers the address and the request, so the same bytes
    // checked against another slot, or another plane, do not pass.
    assert!(frame::check_answer(SIM_ADDRESS, &request, &answer).is_ok());
    let wrong = frame::read_request(BLK_SLOT, 1);
    assert_eq!(
        frame::check_answer(SIM_ADDRESS, &wrong, &answer),
        Err(frame::FrameError::BadCrc)
    );
    assert_eq!(
        frame::check_answer(SIM_ADDRESS + 1, &request, &answer),
        Err(frame::FrameError::BadCrc)
    );
}

// --- events -----------------------------------------------------------

#[test]
fn events_survive_a_read_and_go_on_ev_ack() {
    let mut sim = settled(1);
    sim.ctrl_now(1, &[Command::Enable { slot: 0, on: false }]);
    let payload = sim.read_block(BLK_EVENTS, INDEX_NONE);
    let first: Vec<_> = EventRecord::records(&payload).map(Result::unwrap).collect();
    assert!(!first.is_empty());
    // Reading again gives the same events: a read clears nothing.
    let again = sim.read_block(BLK_EVENTS, INDEX_NONE);
    assert_eq!(again, payload);

    // Let the rail change settle first: a poll cycle that sees PGD or
    // an LED move raises its own events, and those are numbered after
    // the ones being acknowledged here.
    sim.advance(2_000);
    let head = sim.plane.events().next_seq().wrapping_sub(1);
    assert!(sim.plane.event_pending());
    sim.ev_ack(head);
    assert!(!sim.plane.event_pending(), "INT_OUT is released");
    assert!(sim.read_block(BLK_EVENTS, INDEX_NONE).is_empty());

    // Acknowledging the oldest keeps the rest: the ring is a queue, not
    // a clear-all.
    sim.ctrl_now(2, &[Command::Enable { slot: 0, on: true }]);
    sim.ctrl_now(3, &[Command::Enable { slot: 1, on: false }]);
    sim.advance(2_000);
    let pending = sim.events();
    assert!(pending.len() >= 2);
    sim.ev_ack(pending[0].seq);
    let left = sim.events();
    assert!(!left.iter().any(|e| e.seq == pending[0].seq));
    assert!(left.iter().any(|e| e.seq == pending[1].seq));
}

#[test]
fn an_overflowing_ring_keeps_a_lost_count() {
    let mut sim = settled(1);
    // Far more events than the ring holds, without acknowledging any.
    for seq in 0..40u8 {
        sim.ctrl_now(seq, &[Command::Enable { slot: 0, on: seq % 2 == 0 }]);
    }
    let payload = sim.read_block(BLK_EVENTS, INDEX_NONE);
    let records: Vec<_> = EventRecord::records(&payload).map(Result::unwrap).collect();
    let lost = records
        .iter()
        .find_map(|r| match r {
            EventRecord::Lost(l) => Some(l.count),
            _ => None,
        })
        .expect("the ring says what it dropped");
    assert!(lost > 0, "the ring overflowed");
    // What is left is the newest, in order, and still numbered.
    let events: Vec<_> = records
        .iter()
        .filter_map(|r| match r {
            EventRecord::Event(e) => Some(*e),
            _ => None,
        })
        .collect();
    assert!(events.windows(2).all(|w| w[1].seq > w[0].seq));
    assert!(events.len() <= granite_proto::EventRecord::CAPACITY);

    // Acknowledging everything clears the lost count too.
    sim.advance(2_000);
    let head = sim.plane.events().next_seq().wrapping_sub(1);
    sim.ev_ack(head);
    assert!(!sim.plane.event_pending());
    assert!(sim.read_block(BLK_EVENTS, INDEX_NONE).is_empty());
}

#[test]
fn an_event_names_its_target() {
    let mut sim = settled(1);
    sim.ctrl_now(1, &[Command::Enable { slot: 1, on: false }]);
    let slot_off = sim
        .events()
        .into_iter()
        .find(|e| e.kind == EventKind::SlotOff)
        .expect("the slot off event");
    assert_eq!(slot_off.target, EventTarget::Slot(1));
    let boot = sim
        .events()
        .into_iter()
        .find(|e| e.kind == EventKind::Boot)
        .expect("the boot event");
    assert_eq!(boot.target, EventTarget::Plane);
    let present = sim
        .events()
        .into_iter()
        .find(|e| e.kind == EventKind::UnitPresent)
        .expect("the unit present event");
    assert_eq!(present.target, EventTarget::Unit(0));
}

// --- config -----------------------------------------------------------

#[test]
fn a_config_set_takes_effect_only_on_its_commit() {
    let mut sim = settled(1);
    let before = sim.plane.config_hash();
    let records = [Config::PowerLimit(granite_proto::PowerLimit {
        slot: 1,
        watts: 5_000,
    })];

    // No commit record: the set waits, and nothing changed yet.
    let mut buf = [0u8; 64];
    let mut w = granite_proto::RecordWriter::new(&mut buf);
    for r in &records {
        r.encode(&mut w).unwrap();
    }
    let payload = w.payload().to_vec();
    sim.write_block(granite_proto::BLK_CONFIG_W, &payload);
    sim.tick();
    assert_eq!(sim.plane.config_hash(), before);

    // The commit applies the set that was collected.
    sim.config_write(&[Config::Commit]);
    assert_eq!(sim.plane.config().power_limit[1], 5_000);
    assert_ne!(sim.plane.config_hash(), before);
    assert!(sim.saw(EventKind::ConfigStored));
    assert!(!sim.plane.config_dirty(), "stored, so not dirty");
    // It really went to flash: a reset finds it again.
    sim.core_reset(plane_logic::reset_cause::WATCHDOG);
    assert_eq!(sim.plane.config().power_limit[1], 5_000);
}

#[test]
fn a_set_with_a_bad_record_is_discarded_whole() {
    let mut sim = settled(1);
    let before = sim.plane.config_hash();
    sim.config_write(&[
        Config::PowerLimit(granite_proto::PowerLimit {
            slot: 0,
            watts: 1_000,
        }),
        Config::BlinkDetection(granite_proto::BlinkDetection {
            min_period_ms: 900,
            max_period_ms: 100,
        }),
        Config::Commit,
    ]);
    assert_eq!(
        sim.plane.config_hash(),
        before,
        "half a config is worse than none"
    );
    assert_eq!(sim.plane.config().power_limit[0], 0);
}

#[test]
fn a_ctrl_retry_policy_applies_without_being_stored() {
    let mut sim = settled(1);
    assert_eq!(
        sim.ctrl_now(
            1,
            &[Command::RetryPolicy(granite_proto::RetryPolicy {
                slot: 2,
                max: 9,
                window_s: 30,
                action: 0,
            })]
        ),
        ResultCode::Ok
    );
    assert_eq!(sim.plane.config().retry[2].max, 9);
    assert!(
        sim.plane.config_dirty(),
        "applied in RAM, so the stored set differs"
    );
    assert!(
        sim.plane
            .status_flags()
            .contains(StatusFlags::CONFIG_DIRTY)
    );
}

#[test]
fn the_log_ring_is_readable_over_the_link() {
    let mut sim = settled(1);
    sim.ctrl_now(1, &[Command::Enable { slot: 0, on: false }]);
    let payload = sim.read_block(granite_proto::BLK_LOG, INDEX_NONE);
    let lines: Vec<_> = granite_proto::LogLine::records(&payload)
        .map(Result::unwrap)
        .collect();
    assert!(!lines.is_empty());
    assert!(
        lines.iter().any(|l| l.as_str() == Some("slot off 0")),
        "got {:?}",
        lines.iter().map(|l| l.as_str()).collect::<Vec<_>>()
    );
}
