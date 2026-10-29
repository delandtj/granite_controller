//! Golden frames: hand-written byte arrays both ends of the link can
//! reuse (plane-logic and granite-core can copy this file's constants
//! into their own tests, or test against them through this crate).
//!
//! Every frame is spelled out byte by byte, including the CRC-8 over
//! (address, block, index, L, records), and then checked twice: the
//! crate must produce exactly these bytes, and parse exactly these
//! fields back out of them.

use granite_proto::*;

/// Plane 0x30, the one the goldens below read from.
const A0: u8 = 0x30;
/// Plane 0x33, the one the goldens below write to.
const A3: u8 = 0x33;

// --- Read: INFO ------------------------------------------------------

/// `W[01 FF]`: read INFO.
const INFO_REQUEST: [u8; 2] = [0x01, 0xFF];

/// `R[L INFO crc]` from plane 0x30: format version 1, PP-48 chained, 8
/// slots, firmware 0.2.1, units 0 and 1 present.
const INFO_ANSWER: [u8; 20] = [
    0x12, // L = 18
    0x01, 0x01, 0x01, // 0x01 format version 1
    0x02, 0x02, 0x02, 0x00, // 0x02 plane type 0x0002
    0x03, 0x01, 0x08, // 0x03 slot capacity 8
    0x04, 0x03, 0x00, 0x02, 0x01, // 0x04 firmware 0.2.1
    0x07, 0x01, 0x03, // 0x07 units present 0b0011
    0x17, // crc
];

// --- Read: STATUS ----------------------------------------------------

/// `R[L STATUS crc]` from plane 0x30: a sequence is running, command
/// 0x07 was accepted, the next event sequence number is 0x0142.
const STATUS_ANSWER: [u8; 14] = [
    0x0C, // L = 12
    0x13, 0x02, 0x08, 0x00, // 0x13 flags: sequence running
    0x16, 0x02, 0x07, 0x01, // 0x16 result: seq 7, accepted
    0x17, 0x02, 0x42, 0x01, // 0x17 event head 0x0142
    0x78, // crc
];

// --- Read: SUMMARY ---------------------------------------------------

/// `R[L SUMMARY crc]` from plane 0x30: slot 0 on and drawing 15 A at
/// 48.12 V, slot 1 present but off.
#[rustfmt::skip]
const SUMMARY_ANSWER: [u8; 26] = [
    0x18, // L = 24
    // 0x1C slot 0: present, enabled, PGD; 48.12 V, 15.00 A, 72.1 W, LED on
    0x1C, 0x0A, 0x00, 0x07, 0x00, 0xCC, 0x12, 0xDC, 0x05, 0xD1, 0x02, 0x01,
    // 0x1C slot 1: present only; 48.12 V, no current, LED off
    0x1C, 0x0A, 0x01, 0x01, 0x00, 0xCC, 0x12, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x21, // crc
];

// --- Read: SLOT 6, not present ---------------------------------------

/// `R[L SLOT crc]` for index 6: one status record with no bits set.
/// Never an I2C NAK (a NAK means the plane is gone).
const SLOT6_ABSENT_ANSWER: [u8; 6] = [
    0x04, // L = 4
    0x20, 0x02, 0x00, 0x00, // 0x20 status: nothing, not even present
    0x92, // crc
];

// --- Read: EVENTS ----------------------------------------------------

/// `R[L EVENTS crc]`: event 0x0141, unit 2 lost, at uptime 86400 s.
#[rustfmt::skip]
const EVENTS_ANSWER: [u8; 12] = [
    0x0A, // L = 10
    // 0x60 event: seq 0x0141, target unit 2 (0x12), kind 17 (unit lost),
    // uptime 86400 s
    0x60, 0x08, 0x41, 0x01, 0x12, 0x11, 0x80, 0x51, 0x01, 0x00,
    0x3D, // crc
];

// --- Write: CTRL -----------------------------------------------------

/// `W[10 FF L {4F 07}{45 03 200ms} crc]` to plane 0x33: command
/// sequence 7, press PWR on slot 3 for 200 ms.
const CTRL_WRITE: [u8; 12] = [
    0x10, 0xFF, // CTRL, no index
    0x08, // L = 8
    0x4F, 0x01, 0x07, // 0x4F seq 7
    0x45, 0x03, 0x03, 0xC8, 0x00, // 0x45 press PWR slot 3, 200 ms
    0x7C, // crc
];

// --- Write: EV_ACK ---------------------------------------------------

/// `W[11 FF L {62 0141} crc]` to plane 0x33: events acknowledged
/// through 0x0141, which releases EXP_INT.
const EV_ACK_WRITE: [u8; 8] = [
    0x11, 0xFF, // EV_ACK, no index
    0x04, // L = 4
    0x62, 0x02, 0x41, 0x01, // 0x62 ack through 0x0141
    0xA6, // crc
];

// --- Write: FW confirm -----------------------------------------------

/// `W[13 FF L {93} crc]` to plane 0x33: the running image is good.
const FW_CONFIRM_WRITE: [u8; 6] = [
    0x13, 0xFF, // FW, no index
    0x02, // L = 2
    0x93, 0x00, // 0x93 confirm, empty value
    0x93, // crc
];

/// Encode a read answer and compare it to a golden frame.
fn answer_is(golden: &[u8], addr: u8, block: u8, index: u8, records: &[u8]) {
    let mut out = [0u8; frame::MAX_ANSWER];
    let built = frame::answer_frame(&mut out, addr, block, index, records).unwrap();
    assert_eq!(built, golden, "built answer differs from the golden frame");
    let request = frame::read_request(block, index);
    assert_eq!(
        frame::check_answer(addr, &request, golden).unwrap(),
        records
    );
}

/// Encode a write frame and compare it to a golden frame.
fn write_is(golden: &[u8], addr: u8, block: u8, index: u8, records: &[u8]) {
    let mut out = [0u8; frame::MAX_WRITE_FRAME];
    let built = frame::write_frame(&mut out, addr, block, index, records).unwrap();
    assert_eq!(built, golden, "built write differs from the golden frame");
    assert_eq!(
        frame::parse_frame(addr, golden).unwrap(),
        Frame::Write {
            block,
            index,
            payload: records
        }
    );
}

#[test]
fn info_golden() {
    assert_eq!(
        frame::read_request(BLK_INFO, INDEX_NONE),
        INFO_REQUEST,
        "the INFO request is two bytes and carries no CRC"
    );

    let mut buf = [0u8; 64];
    let mut w = RecordWriter::new(&mut buf);
    Info::FormatVersion(FORMAT_VERSION).encode(&mut w).unwrap();
    Info::PlaneType(PlaneType::Pp48Chained)
        .encode(&mut w)
        .unwrap();
    Info::SlotCapacity(8).encode(&mut w).unwrap();
    Info::FirmwareVersion(Version::new(0, 2, 1))
        .encode(&mut w)
        .unwrap();
    Info::UnitsPresent(IndexBitmap::NONE.plus(0).plus(1))
        .encode(&mut w)
        .unwrap();
    answer_is(&INFO_ANSWER, A0, BLK_INFO, INDEX_NONE, w.payload());

    let records = frame::check_answer(A0, &INFO_REQUEST, &INFO_ANSWER).unwrap();
    let mut it = Info::records(records);
    assert_eq!(it.next(), Some(Ok(Info::FormatVersion(1))));
    assert_eq!(it.next(), Some(Ok(Info::PlaneType(PlaneType::Pp48Chained))));
    assert_eq!(it.next(), Some(Ok(Info::SlotCapacity(8))));
    assert_eq!(
        it.next(),
        Some(Ok(Info::FirmwareVersion(Version::new(0, 2, 1))))
    );
    match it.next() {
        Some(Ok(Info::UnitsPresent(m))) => {
            assert!(m.has(0) && m.has(1) && !m.has(2) && !m.has(3));
        }
        other => panic!("not units present: {other:?}"),
    }
    assert_eq!(it.next(), None);
}

#[test]
fn status_golden() {
    let mut buf = [0u8; 64];
    let mut w = RecordWriter::new(&mut buf);
    Status::Flags(StatusFlags::SEQUENCE_RUNNING)
        .encode(&mut w)
        .unwrap();
    Status::CommandResult(CommandResult::new(7, ResultCode::Accepted))
        .encode(&mut w)
        .unwrap();
    Status::EventHead(0x0142).encode(&mut w).unwrap();
    answer_is(&STATUS_ANSWER, A0, BLK_STATUS, INDEX_NONE, w.payload());

    let request = frame::read_request(BLK_STATUS, INDEX_NONE);
    let records = frame::check_answer(A0, &request, &STATUS_ANSWER).unwrap();
    let mut flags = StatusFlags::NONE;
    let mut result = None;
    let mut head = None;
    for record in Status::records(records) {
        match record.unwrap() {
            Status::Flags(f) => flags = f,
            Status::CommandResult(r) => result = Some(r),
            Status::EventHead(h) => head = Some(h),
            other => panic!("unexpected record: {other:?}"),
        }
    }
    assert!(flags.contains(StatusFlags::SEQUENCE_RUNNING));
    assert_eq!(result, Some(CommandResult::new(7, ResultCode::Accepted)));
    assert!(result.unwrap().code.is_pending());
    assert_eq!(head, Some(0x0142));
}

#[test]
fn summary_golden() {
    let slots = [
        SlotSummary {
            slot: 0,
            status: SlotStatus::PRESENT | SlotStatus::ENABLED | SlotStatus::PGD,
            volts: 4812,
            amps: 1500,
            watts: 721,
            led: LedMode::On,
        },
        SlotSummary {
            slot: 1,
            status: SlotStatus::PRESENT,
            volts: 4812,
            amps: 0,
            watts: 0,
            led: LedMode::Off,
        },
    ];
    let mut buf = [0u8; 64];
    let mut w = RecordWriter::new(&mut buf);
    for s in &slots {
        s.encode(&mut w).unwrap();
    }
    answer_is(&SUMMARY_ANSWER, A0, BLK_SUMMARY, INDEX_NONE, w.payload());

    let request = frame::read_request(BLK_SUMMARY, INDEX_NONE);
    let records = frame::check_answer(A0, &request, &SUMMARY_ANSWER).unwrap();
    let got: Vec<SlotSummary> = SlotSummary::records(records).map(|r| r.unwrap()).collect();
    assert_eq!(got, slots);
}

#[test]
fn absent_slot_golden() {
    let mut buf = [0u8; 8];
    let mut w = RecordWriter::new(&mut buf);
    Slot::Status(SlotStatus::NONE).encode(&mut w).unwrap();
    answer_is(&SLOT6_ABSENT_ANSWER, A0, BLK_SLOT, 6, w.payload());

    let request = frame::read_request(BLK_SLOT, 6);
    let records = frame::check_answer(A0, &request, &SLOT6_ABSENT_ANSWER).unwrap();
    match Slot::records(records).next().unwrap().unwrap() {
        Slot::Status(s) => assert!(!s.contains(SlotStatus::PRESENT)),
        other => panic!("not a status: {other:?}"),
    }
}

#[test]
fn events_golden() {
    let event = Event::new(0x0141, EventTarget::Unit(2), EventKind::UnitLost, 86_400);
    let mut buf = [0u8; 32];
    let mut w = RecordWriter::new(&mut buf);
    event.encode(&mut w).unwrap();
    answer_is(&EVENTS_ANSWER, A0, BLK_EVENTS, INDEX_NONE, w.payload());

    let request = frame::read_request(BLK_EVENTS, INDEX_NONE);
    let records = frame::check_answer(A0, &request, &EVENTS_ANSWER).unwrap();
    assert_eq!(
        EventRecord::records(records).next(),
        Some(Ok(EventRecord::Event(event)))
    );
}

#[test]
fn ctrl_golden() {
    let mut buf = [0u8; 32];
    let mut w = RecordWriter::new(&mut buf);
    Command::write_ctrl(&mut w, 7, &[Command::PressPwr { slot: 3, ms: 200 }]).unwrap();
    write_is(&CTRL_WRITE, A3, BLK_CTRL, INDEX_NONE, w.payload());

    let parsed = frame::parse_frame(A3, &CTRL_WRITE).unwrap();
    let ctrl = CtrlWrite::parse(match parsed {
        Frame::Write { payload, .. } => payload,
        other => panic!("not a write: {other:?}"),
    })
    .unwrap();
    assert_eq!(ctrl.seq, 7);
    assert_eq!(
        ctrl.commands().next(),
        Some(Ok(Command::PressPwr { slot: 3, ms: 200 }))
    );
}

#[test]
fn ev_ack_golden() {
    let mut buf = [0u8; 16];
    let mut w = RecordWriter::new(&mut buf);
    AckThrough::new(0x0141).encode(&mut w).unwrap();
    write_is(&EV_ACK_WRITE, A3, BLK_EV_ACK, INDEX_NONE, w.payload());

    let payload = match frame::parse_frame(A3, &EV_ACK_WRITE).unwrap() {
        Frame::Write { payload, .. } => payload,
        other => panic!("not a write: {other:?}"),
    };
    let (tag, value) = Records::new(payload).next().unwrap().unwrap();
    assert_eq!(tag, TAG_ACK_THROUGH);
    assert_eq!(AckThrough::decode(value), Ok(AckThrough::new(0x0141)));
}

#[test]
fn fw_confirm_golden() {
    let mut buf = [0u8; 16];
    let mut w = RecordWriter::new(&mut buf);
    Fw::Confirm.encode(&mut w).unwrap();
    write_is(&FW_CONFIRM_WRITE, A3, BLK_FW, INDEX_NONE, w.payload());

    let payload = match frame::parse_frame(A3, &FW_CONFIRM_WRITE).unwrap() {
        Frame::Write { payload, .. } => payload,
        other => panic!("not a write: {other:?}"),
    };
    assert_eq!(Fw::records(payload).next(), Some(Ok(Fw::Confirm)));
}

#[test]
fn goldens_are_bound_to_their_address() {
    // Every answer golden belongs to plane 0x30 and fails for 0x31.
    for (golden, block, index) in [
        (&INFO_ANSWER[..], BLK_INFO, INDEX_NONE),
        (&STATUS_ANSWER[..], BLK_STATUS, INDEX_NONE),
        (&SUMMARY_ANSWER[..], BLK_SUMMARY, INDEX_NONE),
        (&SLOT6_ABSENT_ANSWER[..], BLK_SLOT, 6),
        (&EVENTS_ANSWER[..], BLK_EVENTS, INDEX_NONE),
    ] {
        let request = frame::read_request(block, index);
        assert!(frame::check_answer(A0, &request, golden).is_ok());
        assert_eq!(
            frame::check_answer(A0 + 1, &request, golden),
            Err(FrameError::BadCrc),
            "golden for block {block:#04X} passed on the wrong address"
        );
    }
    // Every write golden belongs to plane 0x33 and fails for 0x30.
    for golden in [&CTRL_WRITE[..], &EV_ACK_WRITE[..], &FW_CONFIRM_WRITE[..]] {
        assert!(frame::parse_frame(A3, golden).is_ok());
        assert_eq!(frame::parse_frame(A0, golden), Err(FrameError::BadCrc));
    }
}
