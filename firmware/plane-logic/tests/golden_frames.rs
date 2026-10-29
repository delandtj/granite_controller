//! The plane answering and taking granite-proto's own golden frames,
//! byte for byte.
//!
//! granite-proto's `tests/golden.rs` holds the frames of ADR 0003 as
//! literal bytes, all addressed to plane 0x33, which is the address the
//! simulated plane answers on. So the frames can go straight into
//! [`plane_logic::Plane::handle_frame`]: if either end of the contract
//! moves, these fail rather than a board.

#![cfg(feature = "sim")]

use granite_proto::{
    BLK_SLOT, Command, EventKind, INDEX_NONE, ResultCode, SlotStatus, UpdateState, frame,
};
use plane_logic::sim::{SIM_ADDRESS, Sim};

/// The golden read request for INFO (`W[01 FF]`).
const INFO_REQUEST: [u8; 2] = [0x01, 0xFF];

/// The golden answer to `W[05 06]`, a slot that is not present: one
/// status record with no bits set, L = 4, never a NAK.
const SLOT6_ABSENT_ANSWER: [u8; 6] = [0x04, 0x20, 0x02, 0x00, 0x00, 0x92];

/// The golden CTRL write to plane 0x33: sequence 7, press PWR on slot 3
/// for 200 ms.
const CTRL_WRITE: [u8; 12] = [
    0x10, 0xFF, 0x08, 0x4F, 0x01, 0x07, 0x45, 0x03, 0x03, 0xC8, 0x00, 0x7C,
];

/// The golden EV_ACK write to plane 0x33: acknowledged through 0x0141.
const EV_ACK_WRITE: [u8; 8] = [0x11, 0xFF, 0x04, 0x62, 0x02, 0x41, 0x01, 0xA6];

/// The golden FW confirm write to plane 0x33.
const FW_CONFIRM_WRITE: [u8; 6] = [0x13, 0xFF, 0x02, 0x93, 0x00, 0x93];

/// The address the CTRL, EV_ACK and FW goldens are bound to.
const GOLDEN_ADDRESS: u8 = 0x33;

/// The address the absent-slot golden is bound to (granite-proto's
/// `A0`): a frame's CRC covers the plane's address, so the same records
/// from another plane are different bytes.
const GOLDEN_ADDRESS_A0: u8 = 0x30;

fn settled(units: usize) -> Sim {
    assert_eq!(SIM_ADDRESS, GOLDEN_ADDRESS, "the goldens are for 0x33");
    let mut sim = Sim::with_units(units);
    sim.boot();
    sim.fast_led(100, 500);
    sim.advance(2_000);
    sim
}

#[test]
fn the_plane_answers_the_golden_info_request() {
    let mut sim = settled(2);
    let mut out = [0u8; frame::MAX_ANSWER];
    let answer = sim
        .plane
        .handle_frame(&INFO_REQUEST, &mut out)
        .expect("a read is answered")
        .to_vec();
    // The payload differs from the golden INFO answer (that one is a
    // plane with four units and another serial), but the framing is the
    // contract: L, records, and a CRC over the address and the request.
    let payload = frame::check_answer(GOLDEN_ADDRESS, &INFO_REQUEST, &answer).unwrap();
    assert_eq!(answer[0] as usize, payload.len());
    assert!(
        granite_proto::Info::records(payload).all(|r| r.is_ok()),
        "every record is one the controller knows"
    );
}

#[test]
fn an_absent_slot_answers_the_golden_bytes_exactly() {
    // One unit, so slot 6 is on a channel with nothing on it, and the
    // plane is strapped to the address the golden was built for.
    let mut sim = Sim::with_address(
        GOLDEN_ADDRESS_A0,
        plane_logic::sim::Fakes::with_units(1),
    );
    sim.boot();
    sim.advance(500);
    let request = frame::read_request(BLK_SLOT, 6);
    let mut out = [0u8; frame::MAX_ANSWER];
    let answer = sim.plane.handle_frame(&request, &mut out).unwrap();
    assert_eq!(answer, &SLOT6_ABSENT_ANSWER);

    // And the controller reads it as "not present" rather than as a
    // gone plane.
    let payload = frame::check_answer(GOLDEN_ADDRESS_A0, &request, answer).unwrap();
    match granite_proto::Slot::records(payload).next().unwrap().unwrap() {
        granite_proto::Slot::Status(s) => assert!(!s.contains(SlotStatus::PRESENT)),
        other => panic!("not a status: {other:?}"),
    }
}

#[test]
fn the_plane_executes_the_golden_ctrl_write() {
    let mut sim = settled(2);
    let mut out = [0u8; frame::MAX_ANSWER];
    assert!(
        sim.plane.handle_frame(&CTRL_WRITE, &mut out).is_none(),
        "a write has no answer"
    );
    sim.tick();
    // Sequence 7 pressed PWR on slot 3 for 200 ms.
    assert_eq!(sim.result(7), Some(ResultCode::Accepted));
    assert!(sim.plane.sequence_state(3) == granite_proto::SequenceState::Pressing);
    sim.advance(300);
    assert_eq!(sim.result(7), Some(ResultCode::Ok));
    assert!(sim.saw(EventKind::SequenceDone));

    // The same bytes again are the controller's retry: the seq memory
    // makes it safe (ADR 0003 decision 4).
    let presses_before = sim.fake_slot(3).node.last_press_ms();
    assert!(
        sim.plane.handle_frame(&CTRL_WRITE, &mut out).is_none(),
        "a write has no answer"
    );
    sim.advance(500);
    assert_eq!(sim.result(7), Some(ResultCode::Ok));
    assert_eq!(
        sim.fake_slot(3).node.last_press_ms(),
        presses_before,
        "the retry pressed nothing"
    );
}

#[test]
fn the_plane_takes_the_golden_ev_ack() {
    let mut sim = settled(1);
    // The golden acknowledges through 0x0141, well past anything this
    // plane has raised, so every event goes and INT_OUT is released.
    assert!(sim.plane.event_pending());
    let mut out = [0u8; frame::MAX_ANSWER];
    assert!(sim.plane.handle_frame(&EV_ACK_WRITE, &mut out).is_none());
    sim.tick();
    assert!(!sim.plane.event_pending());
}

#[test]
fn the_plane_takes_the_golden_fw_confirm() {
    let mut sim = Sim::new(plane_logic::sim::Fakes {
        update: plane_logic::sim::FakeUpdateStore::on_probation(),
        ..plane_logic::sim::Fakes::with_units(1)
    });
    sim.boot();
    assert_eq!(sim.plane.image_state(), granite_proto::ImageState::Probation);
    let mut out = [0u8; frame::MAX_ANSWER];
    assert!(sim.plane.handle_frame(&FW_CONFIRM_WRITE, &mut out).is_none());
    sim.tick();
    assert_eq!(sim.plane.image_state(), granite_proto::ImageState::Confirmed);
    assert!(sim.dev.update.booted);
}

#[test]
fn a_golden_frame_for_another_plane_is_dropped() {
    // A frame whose CRC was computed for 0x30 arriving at 0x33.
    let mut sim = settled(1);
    let mut buf = [0u8; 32];
    let mut w = granite_proto::RecordWriter::new(&mut buf);
    Command::write_ctrl(&mut w, 9, &[Command::Enable { slot: 0, on: false }]).unwrap();
    let mut out = [0u8; frame::MAX_WRITE_FRAME];
    let for_another = frame::write_frame(
        &mut out,
        SIM_ADDRESS - 3,
        granite_proto::BLK_CTRL,
        INDEX_NONE,
        w.payload(),
    )
    .unwrap()
    .to_vec();
    let mut answer = [0u8; frame::MAX_ANSWER];
    assert!(sim.plane.handle_frame(&for_another, &mut answer).is_none());
    sim.tick();
    assert_eq!(sim.result(9), None, "not this plane's write");
    assert!(!sim.plane.intended().off(0));
}

#[test]
fn a_whole_poll_round_trips_through_frames() {
    // What the controller does every second: STATUS and SUMMARY, with
    // every record decoded back into its typed form.
    let mut sim = settled(2);
    for block in [granite_proto::BLK_STATUS, granite_proto::BLK_SUMMARY] {
        let payload = sim.read_block(block, INDEX_NONE);
        if block == granite_proto::BLK_STATUS {
            let records: Vec<_> = granite_proto::Status::records(&payload)
                .map(Result::unwrap)
                .collect();
            assert!(records.iter().any(|r| matches!(
                r,
                granite_proto::Status::UpdateProgress(p) if p.state == UpdateState::Idle
            )));
        } else {
            assert_eq!(
                granite_proto::SlotSummary::records(&payload)
                    .map(Result::unwrap)
                    .count(),
                4
            );
        }
    }
    // And the per-slot and per-unit blocks the controller reads every
    // ten seconds.
    for slot in 0..8u8 {
        let payload = sim.read_block(BLK_SLOT, slot);
        assert!(granite_proto::Slot::records(&payload).all(|r| r.is_ok()));
    }
    for unit in 0..4u8 {
        let payload = sim.read_block(granite_proto::BLK_UNIT, unit);
        assert!(granite_proto::Unit::records(&payload).all(|r| r.is_ok()));
    }
}

#[test]
fn the_plane_fits_the_g070() {
    // ADR 0002: 36 KB of RAM for the whole application, which also has
    // to hold the embassy executor, the I2C buffers and the stack.
    let size = core::mem::size_of::<plane_logic::Plane>();
    assert!(size < 8 * 1024, "size_of::<Plane>() is {size} bytes");
}
