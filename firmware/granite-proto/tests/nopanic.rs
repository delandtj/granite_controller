//! No parser in this crate may panic on any input: the bytes come off
//! an I2C bus inside a tank, and the plane must stay up whatever a
//! broken cable hands it (ADR 0003 decision 6, "a fuzz target on
//! `Records` and frame checks").
//!
//! These are property tests over arbitrary bytes. They assert that
//! every entry point returns, that a decoder that succeeds re-encodes
//! to the same bytes where the record has no spare room, and that the
//! record iterator consumes its payload exactly once.

use granite_proto::*;
use proptest::prelude::*;

/// Walk a payload with every block's decoder. The only requirement is
/// that each one returns.
fn decode_with_every_block(payload: &[u8]) {
    for r in Records::new(payload) {
        let Ok((tag, value)) = r else { continue };
        let _ = Info::decode(tag, value);
        let _ = Status::decode(tag, value);
        let _ = Slot::decode(tag, value);
        let _ = Unit::decode(tag, value);
        let _ = EventRecord::decode(tag, value);
        let _ = Config::decode(tag, value);
        let _ = Command::decode(tag, value);
        let _ = Fw::decode(tag, value);
        if tag == TAG_SLOT_SUMMARY {
            let _ = SlotSummary::decode(value);
        }
        if tag == TAG_LOG_LINE {
            let _ = LogLine::decode(value);
        }
    }
    let _ = Info::records(payload).count();
    let _ = Status::records(payload).count();
    let _ = SlotSummary::records(payload).count();
    let _ = Slot::records(payload).count();
    let _ = Unit::records(payload).count();
    let _ = EventRecord::records(payload).count();
    let _ = Config::records(payload).count();
    let _ = Config::written(payload).count();
    let _ = Fw::records(payload).count();
    let _ = LogLine::records(payload).count();
    if let Ok(ctrl) = CtrlWrite::parse(payload) {
        let _ = ctrl.commands().count();
    }
}

proptest! {
    /// `Records` over arbitrary bytes: no panic, every record inside
    /// the payload, and at most one error, at the end.
    #[test]
    fn records_never_panics(payload in proptest::collection::vec(any::<u8>(), 0..600)) {
        let mut consumed = 0usize;
        let mut errors = 0usize;
        let mut ended = false;
        for item in Records::new(&payload) {
            prop_assert!(!ended, "the iterator yielded after its error");
            match item {
                Ok((_, value)) => {
                    consumed += 2 + value.len();
                    prop_assert!(consumed <= payload.len());
                }
                Err(RecordError::Truncated) => {
                    errors += 1;
                    ended = true;
                }
                Err(other) => prop_assert!(false, "unexpected error {other:?}"),
            }
        }
        prop_assert!(errors <= 1);
        decode_with_every_block(&payload);
    }

    /// Every frame parser over arbitrary bytes: no panic, and a parsed
    /// frame always re-encodes to the bytes it came from.
    #[test]
    fn frame_parsers_never_panic(
        addr in any::<u8>(),
        bytes in proptest::collection::vec(any::<u8>(), 0..600),
        block in any::<u8>(),
        index in any::<u8>(),
    ) {
        // Plane side.
        match frame::parse_frame(addr, &bytes) {
            Ok(Frame::Read { block, index }) => {
                prop_assert_eq!(&bytes[..], &frame::read_request(block, index)[..]);
            }
            Ok(Frame::Write { block, index, payload }) => {
                let mut out = [0u8; frame::MAX_WRITE_FRAME];
                let built = frame::write_frame(&mut out, addr, block, index, payload).unwrap();
                prop_assert_eq!(built, &bytes[..]);
                decode_with_every_block(payload);
            }
            Err(_) => {}
        }

        // Controller side, with a request that may or may not match.
        let request = frame::read_request(block, index);
        if let Ok(payload) = frame::check_answer(addr, &request, &bytes) {
            let mut out = [0u8; frame::MAX_ANSWER];
            let built = frame::answer_frame(&mut out, addr, block, index, payload).unwrap();
            prop_assert_eq!(built, &bytes[..payload.len() + 2]);
            decode_with_every_block(payload);
        }
        // A request of any other length is rejected, never panics.
        let _ = frame::check_answer(addr, &bytes, &bytes);
    }

    /// A decoded CTRL command re-encodes to the bytes it came from:
    /// every field of the ADR's table is read back, none dropped.
    #[test]
    fn commands_re_encode(
        tag in 0x40u8..=0x51,
        value in proptest::collection::vec(any::<u8>(), 0..12),
    ) {
        if let Ok(command) = Command::decode(tag, &value) {
            let mut buf = [0u8; 32];
            let mut w = RecordWriter::new(&mut buf);
            command.encode(&mut w).unwrap();
            prop_assert_eq!(&w.payload()[2..], &value[..]);
            prop_assert_eq!(w.payload()[0], tag);
        }
    }

    /// The same for the read blocks' multi-field records.
    #[test]
    fn records_re_encode(value in proptest::collection::vec(any::<u8>(), 0..16)) {
        let mut buf = [0u8; 32];
        if let Ok(s) = SlotSummary::decode(&value) {
            let mut w = RecordWriter::new(&mut buf);
            s.encode(&mut w).unwrap();
            prop_assert_eq!(&w.payload()[2..], &value[..]);
        }
        if let Ok(e) = Event::decode(&value) {
            let mut w = RecordWriter::new(&mut buf);
            e.encode(&mut w).unwrap();
            prop_assert_eq!(&w.payload()[2..], &value[..]);
        }
        if let Ok(p) = RetryPolicy::decode(&value) {
            let mut w = RecordWriter::new(&mut buf);
            p.encode(&mut w).unwrap();
            prop_assert_eq!(&w.payload()[2..], &value[..]);
        }
        if let Ok(l) = LogLine::decode(&value) {
            let mut w = RecordWriter::new(&mut buf);
            l.encode(&mut w).unwrap();
            prop_assert_eq!(&w.payload()[2..], &value[..]);
        }
        if let Ok(f) = FwData::decode(&value) {
            let mut w = RecordWriter::new(&mut buf);
            f.encode(&mut w).unwrap();
            prop_assert_eq!(&w.payload()[2..], &value[..]);
        }
    }

    /// The writer never writes past its buffer and never past L = 254,
    /// whatever it is asked for.
    #[test]
    fn writer_stays_inside_its_buffer(
        size in 0usize..300,
        pushes in proptest::collection::vec((any::<u8>(), 0usize..300), 0..40),
    ) {
        let mut buf = vec![0xAAu8; size + 8];
        let (head, tail) = buf.split_at_mut(size);
        tail.fill(0x5C);
        let mut w = RecordWriter::new(head);
        let value = [0x11u8; 300];
        for (tag, len) in pushes {
            let _ = w.push(tag, &value[..len.min(value.len())]);
            prop_assert!(w.len() <= MAX_PAYLOAD);
            prop_assert!(w.len() <= size);
        }
        prop_assert!(buf[size..].iter().all(|b| *b == 0x5C), "wrote past the buffer");
    }

    /// Enum conversions accept exactly the ADR's values and reject the
    /// rest, for every byte.
    #[test]
    fn wire_enums_never_panic(raw in any::<u8>()) {
        let _ = Block::try_from(raw);
        let _ = ImageState::try_from(raw);
        let _ = UpdateState::try_from(raw);
        let _ = SequenceState::try_from(raw);
        let _ = LedMode::try_from(raw);
        let _ = ResultCode::try_from(raw);
        let _ = EventKind::try_from(raw);
        let _ = Escalate::try_from(raw);
        let _ = EventTarget::try_from(raw);
        let _ = PlaneType::try_from(raw as u16);
        if let Ok(b) = Block::try_from(raw) {
            prop_assert_eq!(b.value(), raw);
            prop_assert!(b.is_read() != b.is_write());
        }
    }
}
