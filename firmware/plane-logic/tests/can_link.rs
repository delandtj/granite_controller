//! The CAN FD transport of the plane (ADR 0005 decisions 2-8), against
//! a fake controller: a read answered as RSP, a write that answers
//! nothing, a frame for another node ignored, a lost segment, the EVENT
//! schedule and its stop on EV_ACK, and FW / LOG travelling as BULK.
//!
//! Every frame here is built with granite-proto's own segmenter and
//! every answer is checked with `frame::check_answer`, so a change on
//! either side of the contract breaks a test and not a board.

#![cfg(feature = "sim")]

use granite_proto::can::{self, CanId, MsgType};
use granite_proto::{
    AckThrough, BLK_CTRL, BLK_FW, BLK_LOG, BLK_STATUS, Command, Fw, FwBegin, INDEX_NONE,
    MAX_PAYLOAD, RecordWriter, ResultCode, Status, Version, frame,
};
use plane_logic::can_link::{
    CanFrames, CanLink, EVENT_PAYLOAD, EVENT_PERIOD_MS, RawFrame, TxError,
};
use plane_logic::sim::Sim;

/// The node the harness straps its plane to: `Sim`'s address is 0x33.
const NODE: u8 = 3;

/// A CAN FD controller that keeps what was sent and hands back what a
/// test queued, with a transmit FIFO that can be made full.
#[derive(Debug, Default)]
struct FakeCan {
    /// Frames the transport queued, in order.
    sent: Vec<(u16, Vec<u8>)>,
    /// Frames waiting to be received, oldest first.
    inbox: std::collections::VecDeque<RawFrame>,
    /// How many more sends succeed; `None` means "always".
    room: Option<usize>,
    tec: u8,
    rec: u8,
    bus_off: bool,
}

impl FakeCan {
    fn deliver(&mut self, id: CanId, msg: &[u8]) {
        for fd in can::segment(msg).expect("the message segments") {
            self.inbox.push_back(RawFrame::new(id.raw(), fd.bytes()));
        }
    }

    /// Frames sent under one identifier, with the segment header.
    fn sent_under(&self, id: CanId) -> Vec<&Vec<u8>> {
        self.sent
            .iter()
            .filter(|(raw, _)| *raw == id.raw())
            .map(|(_, data)| data)
            .collect()
    }

    /// The message reassembled out of everything sent under one id (the
    /// controller's side of ADR 0005 decision 4).
    fn message_under(&self, id: CanId) -> Vec<u8> {
        let mut rx: can::Reassembler = can::Reassembler::new();
        let mut out = Vec::new();
        for (raw, data) in &self.sent {
            if *raw != id.raw() {
                continue;
            }
            if let Some(message) = rx.push(id, data, 0).expect("a frame fits") {
                out.extend_from_slice(message.bytes());
            }
        }
        out
    }
}

impl CanFrames for FakeCan {
    type Error = ();

    fn send(&mut self, id: u16, data: &[u8]) -> Result<(), TxError<()>> {
        match self.room {
            Some(0) => return Err(TxError::Full),
            Some(ref mut left) => *left -= 1,
            None => {}
        }
        self.sent.push((id, data.to_vec()));
        Ok(())
    }

    fn receive(&mut self) -> Result<Option<RawFrame>, ()> {
        Ok(self.inbox.pop_front())
    }

    fn error_counters(&mut self) -> Result<(u8, u8), ()> {
        Ok((self.tec, self.rec))
    }

    fn is_bus_off(&mut self) -> Result<bool, ()> {
        Ok(self.bus_off)
    }
}

/// A booted plane with `units` units, its transport and a fake
/// controller, with the boot event already acknowledged so an EVENT
/// frame in a test is the test's own.
fn harness(units: usize) -> (Sim, CanLink, FakeCan) {
    let mut sim = Sim::with_units(units);
    sim.boot();
    sim.advance(2_000);
    sim.ev_ack(sim.plane.events().next_seq().wrapping_sub(1));
    assert!(!sim.plane.event_pending(), "the ring starts acknowledged");
    (sim, CanLink::new(NODE), FakeCan::default())
}

fn req() -> CanId {
    CanId::new(MsgType::Req, NODE).unwrap()
}

fn rsp() -> CanId {
    CanId::new(MsgType::Rsp, NODE).unwrap()
}

fn bulk() -> CanId {
    CanId::new(MsgType::Bulk, NODE).unwrap()
}

fn event() -> CanId {
    CanId::new(MsgType::Event, NODE).unwrap()
}

// --- reads ------------------------------------------------------------

#[test]
fn a_status_read_comes_back_as_rsp_and_checks_out() {
    let (mut sim, mut link, mut can) = harness(1);
    let request = frame::read_request(BLK_STATUS, INDEX_NONE);
    can.deliver(req(), &request);

    link.poll(100, &mut sim.plane, &mut can);

    // One answer, under RSP from this plane's node.
    assert_eq!(link.counters().messages_in, 1);
    assert_eq!(link.counters().messages_out, 1);
    let answer = can.message_under(rsp());
    assert!(!answer.is_empty());
    let payload = frame::check_answer(can::node_addr(NODE), &request, &answer)
        .expect("the answer's CRC covers 0x30 + node");
    // It is the STATUS block the I2C link answers with, records and all.
    assert!(
        Status::records(payload)
            .map(Result::unwrap)
            .any(|r| matches!(r, Status::Uptime(_)))
    );
    // Every frame but the last is full, and the last is padded to a
    // valid CAN FD length (decision 3).
    for data in can.sent_under(rsp()) {
        assert!(can::FD_LENGTHS.contains(&data.len()));
    }
}

#[test]
fn the_can_counters_ride_in_status() {
    let (mut sim, mut link, mut can) = harness(1);
    can.tec = 24;
    can.rec = 7;
    link.poll(0, &mut sim.plane, &mut can);
    sim.plane.set_can_errors(link.can_errors());

    let request = frame::read_request(BLK_STATUS, INDEX_NONE);
    can.deliver(req(), &request);
    link.poll(10, &mut sim.plane, &mut can);
    let answer = can.message_under(rsp());
    let payload = frame::check_answer(can::node_addr(NODE), &request, &answer).unwrap();
    let errors = Status::records(payload)
        .map(Result::unwrap)
        .find_map(|r| match r {
            Status::CanErrors(e) => Some(e),
            _ => None,
        })
        .expect("STATUS 0x1D is there once the transport published it");
    assert_eq!((errors.tec, errors.rec), (24, 7));
    assert_eq!((errors.bus_off, errors.dropped, errors.tx_full), (0, 0, 0));
}

#[test]
fn a_bus_off_is_counted_once_per_edge() {
    let (mut sim, mut link, mut can) = harness(1);
    link.poll(0, &mut sim.plane, &mut can);
    can.bus_off = true;
    link.poll(1_000, &mut sim.plane, &mut can);
    link.poll(2_000, &mut sim.plane, &mut can);
    assert!(link.is_bus_off());
    assert_eq!(link.counters().bus_off, 1, "the state is not an event");
    // Recovery is the controller's own; a second entry is a second
    // count.
    can.bus_off = false;
    link.poll(3_000, &mut sim.plane, &mut can);
    can.bus_off = true;
    link.poll(4_000, &mut sim.plane, &mut can);
    assert_eq!(link.counters().bus_off, 2);
    assert_eq!(link.can_errors().bus_off, 2);
}

// --- writes -----------------------------------------------------------

#[test]
fn a_ctrl_write_is_taken_and_answered_with_nothing() {
    let (mut sim, mut link, mut can) = harness(1);
    let mut buf = [0u8; MAX_PAYLOAD];
    let mut w = RecordWriter::new(&mut buf);
    Command::write_ctrl(&mut w, 7, &[Command::Enable { slot: 0, on: false }]).unwrap();
    let mut out = [0u8; frame::MAX_WRITE_FRAME];
    let written = frame::write_frame(
        &mut out,
        can::node_addr(NODE),
        BLK_CTRL,
        INDEX_NONE,
        w.payload(),
    )
    .unwrap()
    .to_vec();

    can.deliver(req(), &written);
    link.poll(100, &mut sim.plane, &mut can);

    assert_eq!(link.counters().messages_in, 1);
    assert_eq!(link.counters().messages_out, 0, "a write has no answer");
    assert!(can.sent.is_empty());
    // It really was taken: the sequencer runs it on the next tick and
    // the result is read back from STATUS, as on rev D.
    sim.tick();
    assert_eq!(sim.result(7), Some(ResultCode::Ok));
    assert!(sim.plane.intended().off(0));
}

// --- what a plane does not answer -------------------------------------

#[test]
fn a_request_for_another_node_is_ignored() {
    let (mut sim, mut link, mut can) = harness(1);
    let request = frame::read_request(BLK_STATUS, INDEX_NONE);
    can.deliver(CanId::new(MsgType::Req, NODE + 1).unwrap(), &request);
    link.poll(100, &mut sim.plane, &mut can);
    assert!(can.sent.is_empty());
    assert_eq!(link.counters().messages_in, 0);
    assert_eq!(link.counters().ignored, 1);
    assert_eq!(link.counters().bad_frames, 0);
}

#[test]
fn sync_and_another_planes_answer_are_ignored() {
    let (mut sim, mut link, mut can) = harness(1);
    // SYNC from the controller, payload epoch (decision 7): planes
    // ignore it for now.
    can.inbox.push_back(RawFrame::new(
        CanId::new(MsgType::Sync, can::NODE_CONTROLLER)
            .unwrap()
            .raw(),
        &7u32.to_le_bytes(),
    ));
    // A RSP on this plane's own node (a board strapped twice) is not a
    // request either.
    can.inbox
        .push_back(RawFrame::new(rsp().raw(), &[0x80, BLK_STATUS]));
    // A reserved type (5-15) is not one of ours at all.
    can.inbox.push_back(RawFrame::new(0x05F, &[0x80, 0x00]));

    link.poll(100, &mut sim.plane, &mut can);
    assert!(can.sent.is_empty());
    assert_eq!(link.counters().messages_in, 0);
    assert_eq!(link.counters().ignored, 2);
    assert_eq!(link.counters().bad_frames, 1);
}

#[test]
fn a_lost_segment_drops_the_message_and_is_counted() {
    let (mut sim, mut link, mut can) = harness(1);
    // An FW write is two segments; dropping the second leaves the
    // first in its slot and nothing is handed to the plane.
    let mut buf = [0u8; MAX_PAYLOAD];
    let mut w = RecordWriter::new(&mut buf);
    Fw::Begin(FwBegin {
        size: 1024,
        crc32: 0x1234_5678,
        version: Version::new(0, 2, 0),
    })
    .encode(&mut w)
    .unwrap();
    Fw::Data(granite_proto::FwData {
        offset: 0,
        data: &[0xA5; 128],
    })
    .encode(&mut w)
    .unwrap();
    let mut out = [0u8; frame::MAX_WRITE_FRAME];
    let written = frame::write_frame(
        &mut out,
        can::node_addr(NODE),
        BLK_FW,
        INDEX_NONE,
        w.payload(),
    )
    .unwrap()
    .to_vec();
    // FW travels as BULK in both directions (decision 2).
    let frames: Vec<RawFrame> = can::segment(&written)
        .unwrap()
        .map(|fd| RawFrame::new(bulk().raw(), fd.bytes()))
        .collect();
    assert!(frames.len() >= 3, "a ~190-byte write is several segments");

    // Segment 1 lost: the one after it does not follow the index the
    // slot waits for, so the partial message goes and is counted.
    can.inbox.push_back(frames[0]);
    can.inbox.push_back(frames[2]);
    link.poll(100, &mut sim.plane, &mut can);
    assert_eq!(link.counters().messages_in, 0, "nothing was complete");
    assert_eq!(link.reassembly_counters().out_of_order, 1);
    assert_eq!(link.can_errors().dropped, 1);

    // The controller's retry is a whole message again, and it lands.
    for fd in &frames {
        can.inbox.push_back(*fd);
    }
    link.poll(120, &mut sim.plane, &mut can);
    assert_eq!(link.counters().messages_in, 1);
    assert_eq!(link.can_errors().dropped, 1, "a clean retry drops nothing");

    // A segment 0 on top of a partial message of the same id is the
    // sender restarting: also a drop, also counted.
    can.inbox.push_back(frames[0]);
    can.inbox.push_back(frames[0]);
    link.poll(140, &mut sim.plane, &mut can);
    assert_eq!(link.reassembly_counters().restarted, 1);
    assert_eq!(link.can_errors().dropped, 2);
    assert_eq!(link.counters().messages_in, 1);
}

#[test]
fn a_gap_over_fifty_milliseconds_drops_the_partial_message() {
    let (mut sim, mut link, mut can) = harness(1);
    let mut buf = [0u8; MAX_PAYLOAD];
    let mut w = RecordWriter::new(&mut buf);
    Fw::Data(granite_proto::FwData {
        offset: 0,
        data: &[0x5A; 128],
    })
    .encode(&mut w)
    .unwrap();
    let mut out = [0u8; frame::MAX_WRITE_FRAME];
    let written = frame::write_frame(
        &mut out,
        can::node_addr(NODE),
        BLK_FW,
        INDEX_NONE,
        w.payload(),
    )
    .unwrap()
    .to_vec();
    let frames: Vec<RawFrame> = can::segment(&written)
        .unwrap()
        .map(|fd| RawFrame::new(bulk().raw(), fd.bytes()))
        .collect();

    can.inbox.push_back(frames[0]);
    link.poll(1_000, &mut sim.plane, &mut can);
    can.inbox.push_back(frames[1]);
    link.poll(1_000 + can::SEGMENT_GAP_MS + 1, &mut sim.plane, &mut can);
    assert_eq!(link.counters().messages_in, 0);
    assert_eq!(link.reassembly_counters().timeout, 1);
    assert_eq!(link.can_errors().dropped, 1);
}

// --- BULK -------------------------------------------------------------

#[test]
fn a_log_read_is_answered_as_bulk() {
    let (mut sim, mut link, mut can) = harness(1);
    let request = frame::read_request(BLK_LOG, INDEX_NONE);
    // LOG is read with a BULK request and answered with BULK frames, so
    // a long transfer never blocks a poll (decision 2).
    can.deliver(bulk(), &request);
    link.poll(100, &mut sim.plane, &mut can);

    assert!(can.sent_under(rsp()).is_empty(), "LOG is never a RSP");
    let answer = can.message_under(bulk());
    let payload = frame::check_answer(can::node_addr(NODE), &request, &answer).unwrap();
    assert!(
        granite_proto::LogLine::records(payload)
            .map(Result::unwrap)
            .any(|line| String::from_utf8_lossy(line.text).contains("boot"))
    );
}

#[test]
fn an_fw_write_arrives_as_bulk_and_is_executed() {
    let (mut sim, mut link, mut can) = harness(1);
    let mut buf = [0u8; MAX_PAYLOAD];
    let mut w = RecordWriter::new(&mut buf);
    Fw::Begin(FwBegin {
        size: 2048,
        crc32: 0,
        version: Version::new(0, 2, 0),
    })
    .encode(&mut w)
    .unwrap();
    let mut out = [0u8; frame::MAX_WRITE_FRAME];
    let written = frame::write_frame(
        &mut out,
        can::node_addr(NODE),
        BLK_FW,
        INDEX_NONE,
        w.payload(),
    )
    .unwrap()
    .to_vec();
    can.deliver(bulk(), &written);
    link.poll(100, &mut sim.plane, &mut can);
    assert_eq!(link.counters().messages_in, 1);
    assert!(can.sent.is_empty(), "a write answers nothing, BULK or not");
    sim.tick();
    // The sim's DFU slot takes it, so the plane is receiving an image.
    let payload = sim.read_block(BLK_STATUS, INDEX_NONE);
    let progress = Status::records(&payload)
        .map(Result::unwrap)
        .find_map(|r| match r {
            Status::UpdateProgress(p) => Some(p),
            _ => None,
        })
        .unwrap();
    assert_eq!(
        progress.state,
        granite_proto::UpdateState::Receiving,
        "events: {:?}",
        sim.events()
    );
}

// --- EVENT ------------------------------------------------------------

#[test]
fn an_event_goes_out_at_once_and_once_a_second_until_acknowledged() {
    let (mut sim, mut link, mut can) = harness(1);

    // Nothing pending: no EVENT, whatever the clock says.
    link.poll(0, &mut sim.plane, &mut can);
    link.poll(5_000, &mut sim.plane, &mut can);
    assert!(can.sent.is_empty());

    // The first event is announced on the next poll, not a second later.
    sim.ctrl_now(1, &[Command::Enable { slot: 0, on: false }]);
    assert!(sim.plane.event_pending());
    link.poll(10_000, &mut sim.plane, &mut can);
    assert_eq!(can.sent_under(event()).len(), 1);
    assert_eq!(can.sent_under(event())[0], &vec![EVENT_PAYLOAD]);

    // Not again before the period is out.
    link.poll(10_000 + EVENT_PERIOD_MS - 1, &mut sim.plane, &mut can);
    assert_eq!(can.sent_under(event()).len(), 1);
    // And again on the period.
    link.poll(10_000 + EVENT_PERIOD_MS, &mut sim.plane, &mut can);
    assert_eq!(can.sent_under(event()).len(), 2);

    // EV_ACK through the head stops it. Let the rail settle first: a
    // poll cycle that sees PGD or an LED move raises events of its own,
    // and those are numbered after the head taken here.
    sim.advance(2_000);
    let head = sim.plane.events().next_seq().wrapping_sub(1);
    let mut buf = [0u8; 8];
    let mut w = RecordWriter::new(&mut buf);
    AckThrough::new(head).encode(&mut w).unwrap();
    let mut out = [0u8; frame::MAX_WRITE_FRAME];
    let written = frame::write_frame(
        &mut out,
        can::node_addr(NODE),
        granite_proto::BLK_EV_ACK,
        INDEX_NONE,
        w.payload(),
    )
    .unwrap()
    .to_vec();
    can.deliver(req(), &written);
    link.poll(10_000 + 2 * EVENT_PERIOD_MS, &mut sim.plane, &mut can);
    sim.tick();
    assert!(!sim.plane.event_pending());
    let before = can.sent_under(event()).len();
    link.poll(10_000 + 4 * EVENT_PERIOD_MS, &mut sim.plane, &mut can);
    assert_eq!(can.sent_under(event()).len(), before, "acknowledged, so quiet");

    // A new event starts the schedule over, at once again.
    sim.ctrl_now(2, &[Command::Enable { slot: 1, on: false }]);
    link.poll(20_000, &mut sim.plane, &mut can);
    assert_eq!(can.sent_under(event()).len(), before + 1);
}

#[test]
fn a_full_transmit_fifo_is_counted_and_retried() {
    let (mut sim, mut link, mut can) = harness(1);
    sim.ctrl_now(1, &[Command::Enable { slot: 0, on: false }]);
    assert!(sim.plane.event_pending());

    can.room = Some(0);
    link.poll(1_000, &mut sim.plane, &mut can);
    assert!(can.sent.is_empty());
    assert_eq!(link.counters().tx_full, 1);
    assert_eq!(link.can_errors().tx_full, 1);

    // The schedule did not advance, so the very next poll tries again.
    can.room = None;
    link.poll(1_010, &mut sim.plane, &mut can);
    assert_eq!(can.sent_under(event()).len(), 1);
}

#[test]
fn an_answer_cut_short_by_a_full_fifo_is_counted() {
    let (mut sim, mut link, mut can) = harness(4);
    // SUMMARY over four units is two segments; room for one of them.
    can.room = Some(1);
    let request = frame::read_request(granite_proto::BLK_SUMMARY, INDEX_NONE);
    can.deliver(req(), &request);
    link.poll(100, &mut sim.plane, &mut can);
    assert_eq!(can.sent_under(rsp()).len(), 1);
    assert_eq!(link.counters().tx_full, 1);
    assert_eq!(link.counters().messages_out, 0, "half a message is none");
}
