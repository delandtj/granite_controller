//! Whole transactions over the two sides of the link, and a round trip
//! for every typed record of ADR 0003's tag table.
//!
//! The "controller" and the "plane" here are just two halves of each
//! test: one encodes, the other parses, through the framing both ends
//! use on the wire.

use granite_proto::*;

/// Encode `records` as the plane, check the frame as the controller,
/// and give back the payload the controller got.
fn read_transaction<'a>(
    addr: u8,
    block: u8,
    index: u8,
    records: &[u8],
    answer: &'a mut [u8],
) -> &'a [u8] {
    // Controller asks.
    let request = frame::read_request(block, index);
    // Plane parses the request and answers it.
    let parsed = frame::parse_frame(addr, &request).unwrap();
    assert_eq!(parsed, Frame::Read { block, index });
    assert!(Block::try_from(parsed.block()).unwrap().is_read());
    assert!(
        Block::try_from(parsed.block())
            .unwrap()
            .accepts_index(index)
    );
    let n = frame::answer_frame(answer, addr, block, index, records)
        .unwrap()
        .len();
    // Controller checks the answer against the address and request,
    // and gets the records back.
    frame::check_answer(addr, &request, &answer[..n]).unwrap()
}

/// Round trip one block's records through a frame: encode them all,
/// frame them, check the frame, decode them back in order.
fn block_round_trip<T, E, D>(addr: u8, block: u8, index: u8, records: &[T], encode: E, decode: D)
where
    T: PartialEq + core::fmt::Debug + Copy,
    E: Fn(&T, &mut RecordWriter) -> Result<(), RecordError>,
    D: for<'a> Fn(&'a [u8]) -> Vec<Result<T, RecordError>>,
{
    let mut payload = [0u8; MAX_PAYLOAD];
    let mut w = RecordWriter::new(&mut payload);
    for r in records {
        encode(r, &mut w).unwrap();
    }
    let mut buf = [0u8; frame::MAX_ANSWER];
    let got = read_transaction(addr, block, index, w.payload(), &mut buf);
    let decoded = decode(got);
    assert_eq!(decoded.len(), records.len(), "record count differs");
    for (got, want) in decoded.iter().zip(records.iter()) {
        assert_eq!(got.as_ref(), Ok(want));
    }
}

#[test]
fn info_records_round_trip() {
    let records = [
        Info::FormatVersion(FORMAT_VERSION),
        Info::PlaneType(PlaneType::Pp48Rev1),
        Info::SlotCapacity(8),
        Info::FirmwareVersion(Version::new(1, 2, 3)),
        Info::Serial(0xDEAD_BEEF),
        Info::Capabilities(
            Capabilities::NODE_CONTROL
                | Capabilities::POWER_LIMIT
                | Capabilities::UNIT_HOT_SWAP
                | Capabilities::PRESS_DEADLINE
                | Capabilities::FIRMWARE_UPDATE,
        ),
        Info::UnitsPresent(IndexBitmap::from_bits(0x0F)),
        Info::BootCount(42),
        Info::ConfigHash(0x1234_5678),
        Info::ImageState(ImageState::Probation),
        Info::BootloaderVersion(Version::new(0, 1, 0)),
    ];
    block_round_trip(0x30, BLK_INFO, INDEX_NONE, &records, Info::encode, |p| {
        Info::records(p).collect()
    });
}

#[test]
fn status_records_round_trip() {
    let records = [
        Status::CoreInputVoltage(4812),
        Status::CoreTemperature(-123),
        Status::Uptime(1_000_000),
        Status::Flags(StatusFlags::KNOWN),
        Status::CommandResult(CommandResult::new(3, ResultCode::Ok)),
        Status::CommandResult(CommandResult::new(4, ResultCode::Failed)),
        Status::EventHead(0xFFFF),
        Status::UpdateProgress(UpdateProgress::new(UpdateState::Receiving, 0x4000)),
        Status::LinkErrors(LinkErrors {
            bad_crc: 7,
            bad_length: 1,
        }),
        Status::SegmentErrors(SegmentErrors {
            per_channel: [1, 2, 3, 4],
        }),
        Status::LastResetCause(0x24),
    ];
    block_round_trip(
        0x31,
        BLK_STATUS,
        INDEX_NONE,
        &records,
        Status::encode,
        |p| Status::records(p).collect(),
    );
}

#[test]
fn summary_records_round_trip() {
    let mut records = Vec::new();
    for slot in 0..SLOT_COUNT as u8 {
        records.push(SlotSummary {
            slot,
            status: SlotStatus::PRESENT | SlotStatus::ENABLED,
            volts: 4800 + slot as u16,
            amps: 100 * slot as u16,
            watts: 50 * slot as u16,
            led: LedMode::Blinking,
        });
    }
    block_round_trip(
        0x32,
        BLK_SUMMARY,
        INDEX_NONE,
        &records,
        SlotSummary::encode,
        |p| SlotSummary::records(p).collect(),
    );
}

#[test]
fn slot_records_round_trip() {
    let records = [
        Slot::Status(SlotStatus::KNOWN),
        Slot::RetryCount(3),
        Slot::ModuleClass(2),
        Slot::ModuleIdRaw(0x0A5A),
        Slot::Volts(4812),
        Slot::Amps(15_000),
        Slot::Watts(7_200),
        Slot::PowerMinMax(PowerMinMax { min: 10, max: 7250 }),
        Slot::Temperature(385),
        Slot::Energy(Energy {
            joules: 1_234_567_890_123,
            boot_count: 9,
        }),
        Slot::NodeLed(NodeLed {
            mode: LedMode::Blinking,
            period_ms: 1000,
        }),
        Slot::CrossCheck(CrossCheck::LED_AT_ZERO_CURRENT | CrossCheck::NO_LED_AFTER_CYCLE),
        Slot::SequenceState(SequenceState::OffWait),
        Slot::PowerLimit(7500),
    ];
    block_round_trip(0x33, BLK_SLOT, 5, &records, Slot::encode, |p| {
        Slot::records(p).collect()
    });
}

#[test]
fn unit_records_round_trip() {
    let records = [
        Unit::Status(UnitStatus::KNOWN),
        Unit::InputVolts(4815),
        Unit::InputAmps(18_300),
        Unit::InputWatts(8_800),
        Unit::InputTemperature(-10),
        Unit::SegmentErrors(4),
        Unit::TimesLost(2),
    ];
    block_round_trip(0x34, BLK_UNIT, 3, &records, Unit::encode, |p| {
        Unit::records(p).collect()
    });
}

#[test]
fn event_records_round_trip() {
    let kinds = [
        EventKind::SlotOn,
        EventKind::SlotOff,
        EventKind::PgdLost,
        EventKind::OvLatched,
        EventKind::PowerLimitTrip,
        EventKind::RetryLimitReached,
        EventKind::LedChanged,
        EventKind::SequenceDone,
        EventKind::SequenceFailed,
        EventKind::UnitPresent,
        EventKind::UnitLost,
        EventKind::UnitResetSeen,
        EventKind::UnitHotSwapLatchedOff,
        EventKind::Boot,
        EventKind::ConfigStored,
        EventKind::UpdateStateChanged,
        EventKind::ImageConfirmed,
        EventKind::RolledBack,
    ];
    let targets = [
        EventTarget::Slot(0),
        EventTarget::Slot(7),
        EventTarget::Unit(0),
        EventTarget::Unit(3),
        EventTarget::Plane,
    ];
    let mut records = Vec::new();
    for (i, kind) in kinds.iter().enumerate() {
        records.push(EventRecord::Event(Event::new(
            i as u16,
            targets[i % targets.len()],
            *kind,
            i as u32 * 1000,
        )));
    }
    records.push(EventRecord::Lost(EventsLost { count: 5 }));
    // 19 records of 10 bytes: two frames' worth, so split them.
    for chunk in records.chunks(EventRecord::CAPACITY) {
        block_round_trip(
            0x35,
            BLK_EVENTS,
            INDEX_NONE,
            chunk,
            EventRecord::encode,
            |p| EventRecord::records(p).collect(),
        );
    }
}

#[test]
fn config_records_round_trip() {
    let records = [
        Config::Hash(0xCAFE_BABE),
        Config::RetryPolicy(RetryPolicy {
            slot: 2,
            max: 5,
            window_s: 600,
            action: 1,
        }),
        Config::PowerLimit(PowerLimit {
            slot: 2,
            watts: 7500,
        }),
        Config::BlinkDetection(BlinkDetection {
            min_period_ms: 200,
            max_period_ms: 4000,
        }),
        Config::Commit,
    ];
    block_round_trip(
        0x36,
        BLK_CONFIG,
        INDEX_NONE,
        &records,
        Config::encode,
        |p| Config::records(p).collect(),
    );
}

#[test]
fn every_command_round_trips() {
    let commands = [
        Command::Enable { slot: 0, on: true },
        Command::Enable { slot: 7, on: false },
        Command::ClearOvLatch { slot: 1 },
        Command::RetryPolicy(RetryPolicy {
            slot: 1,
            max: 3,
            window_s: 60,
            action: 0,
        }),
        Command::ClearMinMax { slot: SLOT_ALL },
        Command::ClearMinMax { slot: 4 },
        Command::PressPwr { slot: 2, ms: 50 },
        Command::PressRst {
            slot: 2,
            ms: 10_000,
        },
        Command::PowerCycle {
            slot: 3,
            off_delay_s: 5,
            press_if_dark: true,
            led_timeout_s: 60,
        },
        Command::AbortSequence { slot: 3 },
        Command::ModuleRun { slot: 4, on: false },
        Command::UnitHotSwapReset { unit: 2 },
        Command::StaggerOn {
            slots: IndexBitmap::from_bits(0xFF),
            delay_ms: 500,
        },
        Command::Reboot,
        Command::ResetExpanders,
        Command::NodeOn {
            slot: 5,
            led_timeout_s: 30,
        },
        Command::NodeOff {
            slot: 6,
            timeout_s: 0,
            escalate: Escalate::None,
        },
        Command::NodeOff {
            slot: 6,
            timeout_s: 30,
            escalate: Escalate::HoldPwr,
        },
        Command::NodeOff {
            slot: 6,
            timeout_s: 30,
            escalate: Escalate::CutRail,
        },
        Command::NodeOff {
            slot: 6,
            timeout_s: 30,
            escalate: Escalate::HoldPwrThenCutRail,
        },
    ];

    // One CTRL write per command, framed and parsed back.
    for command in &commands {
        let mut payload = [0u8; 64];
        let mut w = RecordWriter::new(&mut payload);
        Command::write_ctrl(&mut w, 0x5A, &[*command]).unwrap();
        let mut out = [0u8; frame::MAX_WRITE_FRAME];
        let frame = frame::write_frame(&mut out, 0x37, BLK_CTRL, INDEX_NONE, w.payload())
            .unwrap()
            .len();
        let parsed = frame::parse_frame(0x37, &out[..frame]).unwrap();
        let payload = match parsed {
            Frame::Write { payload, .. } => payload,
            other => panic!("not a write: {other:?}"),
        };
        let ctrl = CtrlWrite::parse(payload).unwrap();
        assert_eq!(ctrl.seq, 0x5A);
        let got: Vec<_> = ctrl.commands().collect();
        assert_eq!(got, [Ok(*command)]);
    }
}

#[test]
fn fw_records_round_trip() {
    let image = [0x5Au8; 128];
    let records = [
        Fw::Begin(FwBegin {
            size: 55_296,
            crc32: 0x1234_5678,
            version: Version::new(0, 3, 0),
        }),
        Fw::Data(FwData {
            offset: 0,
            data: &image,
        }),
        Fw::Commit { crc32: 0x1234_5678 },
        Fw::Confirm,
        Fw::Abort,
    ];
    for record in &records {
        let mut payload = [0u8; MAX_PAYLOAD];
        let mut w = RecordWriter::new(&mut payload);
        record.encode(&mut w).unwrap();
        let mut out = [0u8; frame::MAX_WRITE_FRAME];
        let n = frame::write_frame(&mut out, 0x30, BLK_FW, INDEX_NONE, w.payload())
            .unwrap()
            .len();
        let got = match frame::parse_frame(0x30, &out[..n]).unwrap() {
            Frame::Write { payload, .. } => payload,
            other => panic!("not a write: {other:?}"),
        };
        assert_eq!(Fw::records(got).collect::<Vec<_>>(), [Ok(*record)]);
    }
}

#[test]
fn log_lines_round_trip() {
    let lines = [
        LogLine {
            uptime_ms: 0,
            level: 0,
            text: b"",
        },
        LogLine {
            uptime_ms: 4_294_967_295,
            level: 4,
            text: b"slot 3 PGD lost, retrying (2/3)",
        },
        LogLine {
            uptime_ms: 1,
            level: 1,
            text: &[b'x'; 48],
        },
    ];
    let mut payload = [0u8; MAX_PAYLOAD];
    let mut w = RecordWriter::new(&mut payload);
    for line in &lines {
        line.encode(&mut w).unwrap();
    }
    let mut buf = [0u8; frame::MAX_ANSWER];
    let got = {
        let request = frame::read_request(BLK_LOG, INDEX_NONE);
        let n = frame::answer_frame(&mut buf, 0x30, BLK_LOG, INDEX_NONE, w.payload())
            .unwrap()
            .len();
        frame::check_answer(0x30, &request, &buf[..n]).unwrap()
    };
    let decoded: Vec<_> = LogLine::records(got).map(|r| r.unwrap()).collect();
    assert_eq!(decoded, lines);
}

#[test]
fn a_command_runs_to_its_result() {
    // ADR 0003 decision 4, the sequence of the data-flow diagram:
    // CTRL write, STATUS says accepted, the event says it is done.
    let addr = 0x33;
    let seq = 0x21;

    let mut payload = [0u8; 64];
    let mut w = RecordWriter::new(&mut payload);
    Command::write_ctrl(
        &mut w,
        seq,
        &[Command::NodeOff {
            slot: 2,
            timeout_s: 30,
            escalate: Escalate::HoldPwrThenCutRail,
        }],
    )
    .unwrap();
    let mut out = [0u8; frame::MAX_WRITE_FRAME];
    let n = frame::write_frame(&mut out, addr, BLK_CTRL, INDEX_NONE, w.payload())
        .unwrap()
        .len();

    // Plane side: parse, take the command, remember the seq.
    let ctrl = match frame::parse_frame(addr, &out[..n]).unwrap() {
        Frame::Write { block, payload, .. } => {
            assert_eq!(block, BLK_CTRL);
            CtrlWrite::parse(payload).unwrap()
        }
        other => panic!("not a write: {other:?}"),
    };
    assert_eq!(ctrl.seq, seq);
    let command = ctrl.commands().next().unwrap().unwrap();
    assert!(command.is_sequence());
    assert_eq!(command.slot(), Some(2));

    // The same frame again: the controller retried, the plane must see
    // the same sequence number and not run it twice.
    let again = match frame::parse_frame(addr, &out[..n]).unwrap() {
        Frame::Write { payload, .. } => CtrlWrite::parse(payload).unwrap(),
        other => panic!("not a write: {other:?}"),
    };
    assert_eq!(again.seq, seq);

    // STATUS read: accepted, a sequence is running.
    let mut sp = [0u8; 64];
    let mut sw = RecordWriter::new(&mut sp);
    Status::Flags(StatusFlags::SEQUENCE_RUNNING)
        .encode(&mut sw)
        .unwrap();
    Status::CommandResult(CommandResult::new(seq, ResultCode::Accepted))
        .encode(&mut sw)
        .unwrap();
    let mut sbuf = [0u8; frame::MAX_ANSWER];
    let request = frame::read_request(BLK_STATUS, INDEX_NONE);
    let sn = frame::answer_frame(&mut sbuf, addr, BLK_STATUS, INDEX_NONE, sw.payload())
        .unwrap()
        .len();
    let records = frame::check_answer(addr, &request, &sbuf[..sn]).unwrap();
    let mut pending = false;
    for record in Status::records(records) {
        if let Ok(Status::CommandResult(r)) = record {
            assert_eq!(r.seq, seq);
            pending = r.code.is_pending();
        }
    }
    assert!(pending);

    // The sequence ends: STATUS says ok, EVENTS carries "sequence
    // done", EV_ACK clears it.
    let mut ep = [0u8; 64];
    let mut ew = RecordWriter::new(&mut ep);
    Event::new(9, EventTarget::Slot(2), EventKind::SequenceDone, 1234)
        .encode(&mut ew)
        .unwrap();
    let mut ebuf = [0u8; frame::MAX_ANSWER];
    let erequest = frame::read_request(BLK_EVENTS, INDEX_NONE);
    let en = frame::answer_frame(&mut ebuf, addr, BLK_EVENTS, INDEX_NONE, ew.payload())
        .unwrap()
        .len();
    let erecords = frame::check_answer(addr, &erequest, &ebuf[..en]).unwrap();
    let event = match EventRecord::records(erecords).next().unwrap().unwrap() {
        EventRecord::Event(e) => e,
        other => panic!("not an event: {other:?}"),
    };
    assert_eq!(event.kind, EventKind::SequenceDone);

    let mut ap = [0u8; 16];
    let mut aw = RecordWriter::new(&mut ap);
    AckThrough::new(event.seq).encode(&mut aw).unwrap();
    let mut abuf = [0u8; frame::MAX_WRITE_FRAME];
    let an = frame::write_frame(&mut abuf, addr, BLK_EV_ACK, INDEX_NONE, aw.payload())
        .unwrap()
        .len();
    let ack = match frame::parse_frame(addr, &abuf[..an]).unwrap() {
        Frame::Write { block, payload, .. } => {
            assert_eq!(block, BLK_EV_ACK);
            let (tag, value) = Records::new(payload).next().unwrap().unwrap();
            assert_eq!(tag, TAG_ACK_THROUGH);
            AckThrough::decode(value).unwrap()
        }
        other => panic!("not a write: {other:?}"),
    };
    assert_eq!(ack.seq, 9);
}

#[test]
fn a_payload_over_254_bytes_is_refused() {
    // The writer stops at 254 ...
    let mut payload = [0u8; 512];
    let mut w = RecordWriter::new(&mut payload);
    let mut pushed = 0;
    while w.push(TAG_SLOT_V, &[0x00, 0x10]).is_ok() {
        pushed += 1;
    }
    assert_eq!(pushed, 63);
    assert_eq!(w.len(), 252);
    assert_eq!(
        w.push(TAG_SLOT_V, &[0, 0]),
        Err(RecordError::TooLong),
        "the 64th record would make L = 256"
    );

    // ... and the framing refuses a longer payload outright.
    let long = [0u8; MAX_PAYLOAD + 1];
    let mut out = [0u8; 512];
    assert_eq!(
        frame::write_frame(&mut out, 0x30, BLK_CTRL, INDEX_NONE, &long),
        Err(FrameError::BadLength)
    );
    assert_eq!(
        frame::answer_frame(&mut out, 0x30, BLK_STATUS, INDEX_NONE, &long),
        Err(FrameError::BadLength)
    );
}

#[test]
fn truncated_records_do_not_hide_the_good_ones() {
    // A payload whose last record is cut short: the records before it
    // decode, the iterator ends with one error.
    let mut payload = [0u8; 32];
    let mut w = RecordWriter::new(&mut payload);
    Status::Uptime(7).encode(&mut w).unwrap();
    let good = w.len();
    let mut cut = [0u8; 32];
    cut[..good].copy_from_slice(w.payload());
    cut[good] = TAG_FLAGS;
    cut[good + 1] = 2;
    cut[good + 2] = 0x01; // one byte short

    let got: Vec<_> = Status::records(&cut[..good + 3]).collect();
    assert_eq!(got, [Ok(Status::Uptime(7)), Err(RecordError::Truncated)]);

    // A lone tag byte at the end is the same error.
    let got: Vec<_> = Status::records(&cut[..good + 1]).collect();
    assert_eq!(got, [Ok(Status::Uptime(7)), Err(RecordError::Truncated)]);
}

#[test]
fn an_answer_checked_against_the_wrong_request_fails() {
    let addr = 0x32;
    let mut payload = [0u8; 16];
    let mut w = RecordWriter::new(&mut payload);
    Slot::Volts(4812).encode(&mut w).unwrap();
    let mut buf = [0u8; frame::MAX_ANSWER];
    let n = frame::answer_frame(&mut buf, addr, BLK_SLOT, 3, w.payload())
        .unwrap()
        .len();
    let answer = &buf[..n];

    assert!(frame::check_answer(addr, &frame::read_request(BLK_SLOT, 3), answer).is_ok());
    // Another slot, another block, another plane: all three fail.
    assert_eq!(
        frame::check_answer(addr, &frame::read_request(BLK_SLOT, 4), answer),
        Err(FrameError::BadCrc)
    );
    assert_eq!(
        frame::check_answer(addr, &frame::read_request(BLK_UNIT, 3), answer),
        Err(FrameError::BadCrc)
    );
    assert_eq!(
        frame::check_answer(addr + 1, &frame::read_request(BLK_SLOT, 3), answer),
        Err(FrameError::BadCrc)
    );
    // A request of the wrong size is not a request.
    assert_eq!(
        frame::check_answer(addr, &[BLK_SLOT], answer),
        Err(FrameError::BadLength)
    );
}
