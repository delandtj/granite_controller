//! The fake CAN FD bus under the plane link (firmware ADR 0005).
//!
//! The planes and the messages are the same as on the rev D I2C link:
//! what changes is the metre of wire between them. Every request, every
//! answer, every EVENT and every SYNC goes through
//! [`granite_proto::can::segment`] here and comes back out of a
//! [`Reassembler`] on the other side - one for the controller, one per
//! plane - so the simulator exercises the real segmentation and the real
//! reassembly rules (decisions 3 and 4) rather than a model of them.
//!
//! What the bus adds on top of the segmentation:
//!
//! - **frame loss** ([`FrameLoss`]): every Nth frame, or a burst of the
//!   next few, simply never arrives. A message that loses a segment is
//!   never reassembled, the transaction times out, and ADR 0003's
//!   retries are what has to recover - which is the point of having the
//!   knob.
//! - **EVENT** (decision 6): a plane with unacknowledged events sends
//!   one from its own node when the first is raised and again every
//!   [`EVENT_PERIOD_MS`] of simulated time, until EV_ACK covers them.
//!   The bits land in [`CanBus::take_event_nodes`], which is what
//!   `PlaneLink::event_nodes` answers with.
//! - **SYNC** (decision 7): the controller's keepalive is counted and
//!   its epoch kept. The planes ignore it, so it is not fed to their
//!   reassemblers: a message nobody reads must not take a slot from the
//!   transaction they are answering.
//!
//! Nothing here models bit timing, arbitration or error counters
//! (decision 8): the frames arrive in the order they were sent, or not
//! at all.

use granite_proto::can::{self, CanId, MsgType, NODE_CONTROLLER, Reassembler, ReassemblyCounters};
use granite_proto::{MAX_PLANES, frame};

use granite_core::hal::LinkError;

use crate::planes::FakePlane;

/// How often a plane with unacknowledged events repeats its EVENT
/// \[ms\] (ADR 0005 decision 6).
pub const EVENT_PERIOD_MS: u64 = 1_000;

/// Which frames the bus loses.
///
/// Deterministic on purpose: a test that needs the client to recover
/// has to lose the same frames on every machine and every run.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FrameLoss {
    /// Drop every `every`-th frame the bus carries; 0 drops nothing.
    pub every: u32,
    /// Drop the next `burst` frames, whatever `every` says.
    pub burst: u32,
}

impl FrameLoss {
    /// A bus that loses nothing.
    pub const NONE: FrameLoss = FrameLoss { every: 0, burst: 0 };

    /// A bus that loses every `n`-th frame.
    pub const fn every(n: u32) -> Self {
        FrameLoss { every: n, burst: 0 }
    }
}

/// The CAN FD transport of the fake plane link.
///
/// One of these sits between [`crate::planes::FakePlaneLink`] and its
/// planes when the scenario asks for a CAN link. It owns the
/// reassemblers of both sides, the loss injection and the counters a
/// test reads.
#[derive(Debug)]
pub struct CanBus {
    /// Which frames never arrive.
    pub loss: FrameLoss,
    /// Frames the bus was asked to carry.
    pub frames: u64,
    /// Frames it dropped.
    pub dropped: u64,
    /// Messages that arrived whole, either direction.
    pub messages: u64,
    /// Messages that never arrived, because a frame of theirs was lost.
    pub lost_messages: u64,
    /// SYNC messages the controller sent (decision 7).
    pub syncs: u64,
    /// EVENT messages that reached the controller (decision 6).
    pub events: u64,
    /// The epoch of the last SYNC that got out whole.
    pub last_epoch: Option<u32>,
    /// The controller's reassembler: answers and EVENT frames.
    controller_rx: Reassembler,
    /// One reassembler per plane node: requests and writes.
    plane_rx: [Reassembler; MAX_PLANES],
    /// Planes that sent EVENT since the last [`CanBus::take_event_nodes`].
    event_nodes: u8,
    /// When each plane may send its next EVENT \[ms\].
    next_event_ms: [u64; MAX_PLANES],
}

impl Default for CanBus {
    fn default() -> Self {
        CanBus::new(FrameLoss::NONE)
    }
}

impl CanBus {
    /// An idle bus with the given loss injection.
    pub fn new(loss: FrameLoss) -> Self {
        CanBus {
            loss,
            frames: 0,
            dropped: 0,
            messages: 0,
            lost_messages: 0,
            syncs: 0,
            events: 0,
            last_epoch: None,
            controller_rx: Reassembler::new(),
            plane_rx: core::array::from_fn(|_| Reassembler::new()),
            event_nodes: 0,
            next_event_ms: [0; MAX_PLANES],
        }
    }

    /// What the controller's reassembler has dropped, by reason.
    pub fn controller_counters(&self) -> ReassemblyCounters {
        self.controller_rx.counters()
    }

    /// What one plane's reassembler has dropped, by reason.
    pub fn plane_counters(&self, node: u8) -> ReassemblyCounters {
        self.plane_rx[node as usize % MAX_PLANES].counters()
    }

    /// True when this frame is one of the ones that never arrive.
    fn drop_frame(&mut self) -> bool {
        self.frames += 1;
        let lost = if self.loss.burst > 0 {
            self.loss.burst -= 1;
            true
        } else {
            self.loss.every > 0 && self.frames.is_multiple_of(u64::from(self.loss.every))
        };
        if lost {
            self.dropped += 1;
        }
        lost
    }

    /// Carry one message to a receiver, returning it whole when every
    /// frame of it arrived.
    ///
    /// The bytes come back with the last frame's zero padding on them
    /// (decision 3); the caller takes the length from the message's own
    /// content.
    fn carry(&mut self, id: CanId, msg: &[u8], now_ms: u64, to_plane: bool) -> Option<Vec<u8>> {
        let frames = can::segment(msg).ok()?;
        let mut out = None;
        for fd in frames {
            if self.drop_frame() {
                continue;
            }
            let rx = if to_plane {
                &mut self.plane_rx[id.node() as usize % MAX_PLANES]
            } else {
                &mut self.controller_rx
            };
            if let Ok(Some(message)) = rx.push(id, fd.bytes(), now_ms as u32) {
                out = Some(message.bytes().to_vec());
            }
        }
        match &out {
            Some(_) => self.messages += 1,
            None => self.lost_messages += 1,
        }
        out
    }

    /// A read transaction: REQ (or BULK) to the plane, its RSP (or
    /// BULK) back (decision 5).
    ///
    /// A message that loses a frame is a timeout, not a NAK: on CAN
    /// nothing acknowledges a frame, so a plane that never received the
    /// request looks exactly like one that did not answer in time, and
    /// that is what the client retries.
    pub fn write_read(
        &mut self,
        node: u8,
        plane: &mut FakePlane,
        req: &[u8],
        resp: &mut [u8],
        now_ms: u64,
    ) -> Result<usize, LinkError> {
        let block = *req.first().ok_or(LinkError::Bus)?;
        let req_id =
            CanId::new(can::msg_type_for_block(block, true), node).ok_or(LinkError::Bus)?;
        let got = self
            .carry(req_id, req, now_ms, true)
            .ok_or(LinkError::Timeout)?;
        let len = message_len(&got).ok_or(LinkError::Timeout)?;

        let mut answer = [0u8; frame::MAX_ANSWER];
        let n = plane.answer(&got[..len], &mut answer).map_err(on_can)?;

        let rsp_id =
            CanId::new(can::msg_type_for_block(block, false), node).ok_or(LinkError::Bus)?;
        let back = self
            .carry(rsp_id, &answer[..n], now_ms, false)
            .ok_or(LinkError::Timeout)?;
        // The padding may push the reassembled bytes past the caller's
        // buffer, which holds the longest answer and not the longest
        // padded one. Dropping it costs nothing: `check_answer` reads
        // the length from the answer itself and ignores what follows.
        let copied = back.len().min(resp.len());
        resp[..copied].copy_from_slice(&back[..copied]);
        Ok(copied)
    }

    /// A write: REQ (or BULK for FW) to the plane, no answer
    /// (decision 5).
    pub fn write(
        &mut self,
        node: u8,
        plane: &mut FakePlane,
        bytes: &[u8],
        now_ms: u64,
    ) -> Result<(), LinkError> {
        let block = *bytes.first().ok_or(LinkError::Bus)?;
        let id = CanId::new(can::msg_type_for_block(block, true), node).ok_or(LinkError::Bus)?;
        let got = self
            .carry(id, bytes, now_ms, true)
            .ok_or(LinkError::Timeout)?;
        let len = message_len(&got).ok_or(LinkError::Timeout)?;
        plane.take(&got[..len]).map_err(on_can)
    }

    /// One plane's EVENT, if it has unacknowledged events and its
    /// second is up (decision 6).
    ///
    /// A lost EVENT is not repeated before its second is up either: the
    /// plane does not know it was lost, and the next one is what brings
    /// the controller back in step.
    pub fn note_event(&mut self, node: u8, pending: bool, now_ms: u64) {
        let slot = node as usize % MAX_PLANES;
        if !pending {
            // The next event raised is sent at once.
            self.next_event_ms[slot] = 0;
            return;
        }
        if now_ms < self.next_event_ms[slot] {
            return;
        }
        self.next_event_ms[slot] = now_ms + EVENT_PERIOD_MS;
        let Some(id) = CanId::new(MsgType::Event, node) else {
            return;
        };
        if self.carry(id, &[0x01], now_ms, false).is_some() {
            self.events += 1;
            self.event_nodes |= 1 << slot;
        }
    }

    /// The planes that sent EVENT since the last call, and forget them.
    pub fn take_event_nodes(&mut self) -> u8 {
        core::mem::take(&mut self.event_nodes)
    }

    /// The controller's SYNC (decision 7): `epoch` little endian from
    /// node 14.
    pub fn sync(&mut self, epoch: u32) -> Result<(), LinkError> {
        // The identifier is (SYNC, node 14), and nothing on this bus
        // reads it: the planes ignore SYNC, so its frames are counted
        // and let go rather than reassembled anywhere.
        CanId::new(MsgType::Sync, NODE_CONTROLLER).ok_or(LinkError::Bus)?;
        let payload = epoch.to_le_bytes();
        let frames = can::segment(&payload).map_err(|_| LinkError::Bus)?;
        let mut lost = false;
        for _ in frames {
            lost |= self.drop_frame();
        }
        self.syncs += 1;
        if lost {
            return Err(LinkError::Bus);
        }
        self.last_epoch = Some(epoch);
        Ok(())
    }
}

/// What a plane's fault knob means on a CAN link.
///
/// The `drop_transactions` knob NAKs a transaction, which is an I2C
/// answer; there is no NAK on CAN, so a transaction the plane swallows
/// is one nobody answered - and a timeout is what ADR 0003 retries.
fn on_can(error: LinkError) -> LinkError {
    match error {
        LinkError::Nak => LinkError::Timeout,
        other => other,
    }
}

/// The bytes of a controller message, without the last frame's padding.
///
/// This is the trim decision 3 asks of a receiver: "the message length
/// comes from its own content (ADR 0003 L), so padding is ignored". An
/// answer needs none - `frame::check_answer` ignores what follows the
/// CRC - but `frame::parse_frame` on the plane's side checks that the
/// frame is exactly `3 + L + 1` bytes, so the zeros have to go. Two
/// bytes are a read request, which never pads: its frame is three bytes
/// on the wire, already a valid CAN FD length.
fn message_len(bytes: &[u8]) -> Option<usize> {
    match bytes.len() {
        frame::REQUEST_LEN => Some(frame::REQUEST_LEN),
        n if n >= frame::MIN_WRITE_FRAME => {
            let len = bytes[2] as usize + frame::MIN_WRITE_FRAME;
            (len <= n).then_some(len)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_read_request_is_not_trimmed_and_a_write_is() {
        // A 2-byte read request: one frame of three bytes, no padding.
        assert_eq!(message_len(&[0x11, 0xFF]), Some(2));
        // A write of two records: block, index, L = 2, records, crc.
        let mut buf = [0u8; frame::MAX_WRITE_FRAME];
        let bytes = frame::write_frame(&mut buf, 0x30, 0x40, 0xFF, &[1, 2]).unwrap();
        assert_eq!(bytes.len(), 6);
        let mut padded = bytes.to_vec();
        padded.extend_from_slice(&[0, 0]);
        assert_eq!(message_len(&padded), Some(6));
        // Nothing sensible comes out of a stump.
        assert_eq!(message_len(&[]), None);
        assert_eq!(message_len(&[0x11]), None);
        assert_eq!(message_len(&[0x11, 0x00, 0x00]), None);
        // A length byte that promises more than arrived.
        assert_eq!(message_len(&[0x11, 0x00, 0x40, 0x00]), None);
    }

    #[test]
    fn the_loss_knobs_pick_the_frames_they_say() {
        let mut bus = CanBus::new(FrameLoss::every(3));
        let dropped: Vec<bool> = (0..9).map(|_| bus.drop_frame()).collect();
        assert_eq!(
            dropped,
            [false, false, true, false, false, true, false, false, true]
        );
        assert_eq!((bus.frames, bus.dropped), (9, 3));

        let mut bus = CanBus::new(FrameLoss { every: 0, burst: 2 });
        assert!(bus.drop_frame() && bus.drop_frame());
        assert!(!bus.drop_frame());
        assert_eq!(bus.dropped, 2);

        let mut bus = CanBus::default();
        assert!(!bus.drop_frame());
        assert_eq!(bus.dropped, 0);
    }
}
