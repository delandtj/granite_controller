//! The controller's side of the CAN FD plane link (ADR 0005), over a
//! frame driver it does not own.
//!
//! [`CanPlaneLink`] is a [`PlaneLink`]: the plane client above it
//! (`crate::plane::client`) hands it whole ADR 0003 messages and never
//! learns that they travel in 64-byte frames. What this module adds
//! between the two is exactly the transport of ADR 0005:
//!
//! - **identifiers** from [`granite_proto::can::msg_type_for_block`], so
//!   FW (0x13) and LOG (0x08) go as BULK and everything else as REQ /
//!   RSP (decision 2);
//! - **segmentation** of the request and **reassembly** of the answer
//!   through `granite_proto::can` (decisions 3 and 4), so both ends run
//!   the same code;
//! - **one transaction at a time**, with [`TRANSACTION_TIMEOUT_MS`]
//!   after the last request segment (decision 5). A timeout is
//!   [`LinkError::Timeout`], which is what the client retries; nothing
//!   on CAN acknowledges a frame, so a plane that never heard the
//!   request looks the same as one that did not answer;
//! - **EVENT frames** (decision 6) are collected into the mask
//!   [`PlaneLink::event_nodes`] answers with, whenever they arrive -
//!   including in the middle of a transaction, where they must not be
//!   lost and must not be mistaken for the answer;
//! - **SYNC** from node [`granite_proto::can::NODE_CONTROLLER`] with the
//!   epoch little endian (decision 7). The client decides when, this
//!   only sends it;
//! - **counters** for the diagnostics, including the CAN error counters
//!   and the bus-off count of decision 8 (recovery is the chip's, this
//!   only counts the transitions).
//!
//! Nothing here allocates, nothing here blocks on anything but the
//! driver's own [`CanFrames::idle`], and nothing here is `async`: the
//! plane client is synchronous and so is the firmware task that drives
//! it.
//!
//! # The driver
//!
//! [`CanFrames`] is the whole hardware surface: queue one CAN FD frame
//! with an 11-bit identifier, take one received frame, say what time it
//! is, and report the error counters. The firmware implements it over
//! the MCP2518FD on the controller's SPI bus (hardware ADR 0006
//! decision 9); the tests below implement it over a pair of queues.
//!
//! ```
//! use granite_core::hal::PlaneLink;
//! use granite_core::plane::can_link::{BusState, CanFrames, CanPlaneLink, DriverError, RxFrame};
//!
//! /// A driver that drops every frame and never answers.
//! struct Deaf(u64);
//!
//! impl CanFrames for Deaf {
//!     fn send(&mut self, _id: u16, _data: &[u8]) -> Result<(), DriverError> {
//!         Ok(())
//!     }
//!     fn receive(&mut self) -> Result<Option<RxFrame>, DriverError> {
//!         Ok(None)
//!     }
//!     fn now_ms(&self) -> u64 {
//!         self.0
//!     }
//!     fn bus_state(&mut self) -> Result<BusState, DriverError> {
//!         Ok(BusState::default())
//!     }
//!     fn idle(&mut self) {
//!         // No real clock here: the idle call is what moves time.
//!         self.0 += 1;
//!     }
//! }
//!
//! let mut link = CanPlaneLink::new(Deaf(0));
//! let mut resp = [0u8; 8];
//! assert!(link.write_read(0x30, &[0x11, 0xFF], &mut resp).is_err());
//! assert_eq!(link.counters().timeouts, 1);
//! ```

use granite_proto::can::{
    self, CanId, MAX_FD_LEN, MsgType, Reassembler, ReassemblyCounters, SegmentError,
};

/// The controller's node number, re-exported so a caller that only has
/// this module (the firmware's driver) does not need `granite_proto`.
pub use granite_proto::can::{NODE_CONTROLLER, NODE_STANDBY};

use crate::hal::{LinkError, PlaneLink};

/// How long the controller waits for an answer after the last request
/// segment \[ms\] (ADR 0005 decision 5).
pub const TRANSACTION_TIMEOUT_MS: u64 = 20;

/// How long a full transmit FIFO is tolerated before a send is given up
/// \[ms\].
///
/// A frame of 64 bytes takes ~0.3 ms at the link's bit rates, so a FIFO
/// that is still full after this long is not busy but stuck (a bus with
/// no other node acknowledging, or bus-off). Giving up is a
/// [`LinkError::Timeout`], the one the client retries.
pub const SEND_TIMEOUT_MS: u64 = 20;

/// Messages the reassembler can hold at once.
///
/// The controller reassembles one message at a time: the answer to the
/// transaction it is in. Two slots is one spare, so a late segment of
/// the message before does not evict the one being collected.
pub const SLOTS: usize = 2;

/// One CAN FD frame off the bus: the 11-bit identifier and the data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RxFrame {
    /// Standard (11-bit) identifier, as it came off the wire.
    pub id: u16,
    /// Data length, one of [`granite_proto::can::FD_LENGTHS`].
    pub len: u8,
    /// Data bytes, zero beyond `len`.
    pub data: [u8; MAX_FD_LEN],
}

impl RxFrame {
    /// A frame with `data` as its payload, or `None` when that does not
    /// fit a CAN FD frame.
    pub fn new(id: u16, data: &[u8]) -> Option<Self> {
        if data.len() > MAX_FD_LEN {
            return None;
        }
        let mut frame = RxFrame {
            id,
            len: data.len() as u8,
            data: [0u8; MAX_FD_LEN],
        };
        frame.data[..data.len()].copy_from_slice(data);
        Some(frame)
    }

    /// The data bytes that were on the wire.
    pub fn bytes(&self) -> &[u8] {
        &self.data[..self.len as usize]
    }
}

/// What a frame driver can fail with.
///
/// Deliberately two variants: the transport above only has to tell "no
/// room right now, try again" from "the peripheral is not doing its
/// job". A driver logs the detail itself; this crate has no logger.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriverError {
    /// The transmit FIFO is full. Nothing was queued and the call can be
    /// repeated.
    WouldBlock,
    /// The peripheral or its bus failed (SPI error, a controller in a
    /// state it should not be in).
    Bus,
}

/// The CAN controller's error state (ADR 0005 decision 8).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BusState {
    /// Transmit error counter.
    pub tec: u8,
    /// Receive error counter.
    pub rec: u8,
    /// True while the controller is bus-off. Recovery is automatic; the
    /// transport counts the transitions.
    pub bus_off: bool,
}

/// One CAN FD controller, as this transport needs it.
///
/// Blocking, synchronous, and no more than the link uses: no filters, no
/// interrupts, no modes. An implementation does its own logging and maps
/// its errors onto [`DriverError`].
pub trait CanFrames {
    /// Queue one frame with an 11-bit identifier and up to 64 data
    /// bytes, padded by the driver to the next valid CAN FD length.
    fn send(&mut self, id: u16, data: &[u8]) -> Result<(), DriverError>;

    /// Take one received frame, or `None` when none is waiting.
    fn receive(&mut self) -> Result<Option<RxFrame>, DriverError>;

    /// Monotonic milliseconds. Only differences are used, so any epoch
    /// will do as long as it never goes backwards.
    fn now_ms(&self) -> u64;

    /// The error counters and the bus-off flag.
    fn bus_state(&mut self) -> Result<BusState, DriverError>;

    /// Called between two polls while the transport is waiting, so a
    /// driver on an OS can yield instead of spinning. The default does
    /// nothing, which is what a bare-metal poll loop wants.
    fn idle(&mut self) {}
}

/// What the transport counted, for the diagnostics.
///
/// Every counter saturates rather than wraps: a diagnostic number that
/// goes back to zero is worse than one that stops.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CanCounters {
    /// Frames queued for transmission.
    pub tx_frames: u32,
    /// Frames taken off the receive FIFO.
    pub rx_frames: u32,
    /// Frames that went into the reassembler, because they carried the
    /// answer this transaction was waiting for.
    pub rx_in_messages: u32,
    /// Times the transmit FIFO was full and the send was repeated.
    pub tx_blocked: u32,
    /// Sends the driver refused outright.
    pub tx_errors: u32,
    /// Receives the driver refused outright.
    pub rx_errors: u32,
    /// Read transactions started.
    pub transactions: u32,
    /// Writes sent (no answer is expected, ADR 0003 decision 1).
    pub writes: u32,
    /// Transactions that got no answer in time.
    pub timeouts: u32,
    /// EVENT frames received (ADR 0005 decision 6).
    pub events: u32,
    /// Frames for somebody else: another controller's SYNC or REQ, an
    /// answer from a node this transaction is not with, a type that is
    /// not the one being waited for.
    pub foreign: u32,
    /// Frames whose identifier is none of ours (a reserved type or node,
    /// or an extended identifier the filter let through).
    pub unknown_id: u32,
    /// Times the controller went bus-off (the transitions, not the
    /// state: recovery is automatic, ADR 0005 decision 8).
    pub bus_off: u32,
    /// Partial messages the reassembler dropped, by reason
    /// (decision 4).
    pub reassembly: ReassemblyCounters,
    /// Last transmit error counter read off the chip.
    pub tec: u8,
    /// Last receive error counter read off the chip.
    pub rec: u8,
    /// True when the last read found the controller bus-off.
    pub bus_off_now: bool,
}

/// Add one, saturating.
fn bump(counter: &mut u32) {
    *counter = counter.saturating_add(1);
}

/// The CAN FD plane link.
///
/// Generic over its frame driver so the host tests drive the same code
/// the firmware does. One of these per image: ADR 0006's "one link per
/// image" keeps the I2C link and this one behind
/// [`crate::hal::PlaneLink`] and lets the build pick.
#[derive(Debug)]
pub struct CanPlaneLink<D> {
    driver: D,
    /// Our own node, for the frames we send from ourselves (SYNC).
    node: u8,
    rx: Reassembler<SLOTS>,
    /// Planes that sent EVENT since the last [`PlaneLink::event_nodes`].
    event_nodes: u8,
    counters: CanCounters,
    /// Whether the last bus-state read found the chip bus-off, so only
    /// the transitions are counted.
    was_bus_off: bool,
}

impl<D: CanFrames> CanPlaneLink<D> {
    /// The link of the master controller (node
    /// [`granite_proto::can::NODE_CONTROLLER`]).
    pub fn new(driver: D) -> Self {
        Self::with_node(driver, NODE_CONTROLLER)
    }

    /// The link of a controller at `node`: 14 for the master, 15 for the
    /// standby of hardware ADR 0006 decision 13 (nothing builds that
    /// one yet).
    pub fn with_node(driver: D, node: u8) -> Self {
        CanPlaneLink {
            driver,
            node,
            rx: Reassembler::new(),
            event_nodes: 0,
            counters: CanCounters::default(),
            was_bus_off: false,
        }
    }

    /// The node the link sends its own frames from.
    pub const fn node(&self) -> u8 {
        self.node
    }

    /// What the transport has counted so far, the reassembler's own
    /// counters included.
    pub fn counters(&self) -> CanCounters {
        CanCounters {
            reassembly: self.rx.counters(),
            ..self.counters
        }
    }

    /// The driver, for the hardware-specific parts a diagnostic wants
    /// (chip variant, interrupt flags).
    pub fn driver(&mut self) -> &mut D {
        &mut self.driver
    }

    /// Read the error counters and count a bus-off transition
    /// (decision 8: automatic recovery, counted).
    pub fn refresh_bus_state(&mut self) -> Option<BusState> {
        match self.driver.bus_state() {
            Ok(state) => {
                self.counters.tec = state.tec;
                self.counters.rec = state.rec;
                self.counters.bus_off_now = state.bus_off;
                if state.bus_off && !self.was_bus_off {
                    bump(&mut self.counters.bus_off);
                }
                self.was_bus_off = state.bus_off;
                Some(state)
            }
            Err(_) => {
                bump(&mut self.counters.rx_errors);
                None
            }
        }
    }

    /// Take every frame that is already waiting, so EVENT frames are
    /// never lost and a leftover answer never lands in the middle of the
    /// next transaction.
    pub fn pump(&mut self) {
        let mut sink = [0u8; 0];
        while let Some(frame) = self.next_frame() {
            self.feed(&frame, None, &mut sink);
        }
    }

    /// One frame off the driver, counting what the driver reports.
    fn next_frame(&mut self) -> Option<RxFrame> {
        match self.driver.receive() {
            Ok(Some(frame)) => {
                bump(&mut self.counters.rx_frames);
                Some(frame)
            }
            Ok(None) => None,
            Err(_) => {
                bump(&mut self.counters.rx_errors);
                None
            }
        }
    }

    /// Sort one frame.
    ///
    /// An EVENT from a plane sets its bit in the mask. A frame under
    /// `want` goes to the reassembler, and when it completes a message
    /// the bytes are copied into `out` and their count returned.
    /// Anything else is counted and dropped: another controller's SYNC
    /// or REQ, and an answer from a node this transaction is not with
    /// (which is what the ADR means by "a plane answers only REQ / BULK
    /// addressed to its own node", seen from the other end).
    fn feed(&mut self, frame: &RxFrame, want: Option<CanId>, out: &mut [u8]) -> Option<usize> {
        let Ok(id) = CanId::decode(frame.id) else {
            bump(&mut self.counters.unknown_id);
            return None;
        };
        if id.msg_type() == MsgType::Event && id.is_plane() {
            self.event_nodes |= 1u8 << id.node();
            bump(&mut self.counters.events);
            return None;
        }
        if Some(id) != want {
            bump(&mut self.counters.foreign);
            return None;
        }
        bump(&mut self.counters.rx_in_messages);
        let now = self.driver.now_ms() as u32;
        match self.rx.push(id, frame.bytes(), now) {
            Ok(Some(message)) => {
                let n = message.bytes().len().min(out.len());
                out[..n].copy_from_slice(&message.bytes()[..n]);
                Some(n)
            }
            // A dropped partial message is already in the reassembler's
            // own counters, which `counters()` folds in.
            Ok(None) | Err(_) => None,
        }
    }

    /// Cut a message into frames and queue every one of them.
    fn send_message(&mut self, id: CanId, msg: &[u8]) -> Result<(), LinkError> {
        let frames = can::segment(msg).map_err(|e| match e {
            // Neither can come from the plane client: a read request is
            // two bytes and a write frame is at most 258. Both are a
            // caller's bug, not a bus event.
            SegmentError::Empty => LinkError::Bus,
            SegmentError::TooLong => LinkError::BufferTooSmall,
        })?;
        for frame in frames {
            self.send_frame(id, frame.bytes())?;
        }
        Ok(())
    }

    /// Queue one frame, waiting out a full transmit FIFO.
    fn send_frame(&mut self, id: CanId, data: &[u8]) -> Result<(), LinkError> {
        let deadline = self.driver.now_ms().saturating_add(SEND_TIMEOUT_MS);
        loop {
            match self.driver.send(id.raw(), data) {
                Ok(()) => {
                    bump(&mut self.counters.tx_frames);
                    return Ok(());
                }
                Err(DriverError::WouldBlock) => {
                    bump(&mut self.counters.tx_blocked);
                    if self.driver.now_ms() >= deadline {
                        bump(&mut self.counters.timeouts);
                        return Err(LinkError::Timeout);
                    }
                    // Keep the receive side moving: a transmit FIFO that
                    // is full for milliseconds is long enough to
                    // overflow the receive one.
                    self.pump();
                    self.driver.idle();
                }
                Err(DriverError::Bus) => {
                    bump(&mut self.counters.tx_errors);
                    return Err(LinkError::Bus);
                }
            }
        }
    }

    /// The two identifiers of a transaction with `addr` about `block`:
    /// the request's and the answer's (ADR 0005 decision 2).
    fn ids(&self, addr: u8, block: u8) -> Result<(CanId, CanId), LinkError> {
        // An address outside 0x30-0x37 is not a plane's: no node is
        // strapped there, which is what the client reads as absence.
        let node = can::node_of_addr(addr).filter(|n| can::is_plane_node(*n));
        let node = node.ok_or(LinkError::Nak)?;
        let req = CanId::new(can::msg_type_for_block(block, true), node).ok_or(LinkError::Bus)?;
        let rsp = CanId::new(can::msg_type_for_block(block, false), node).ok_or(LinkError::Bus)?;
        Ok((req, rsp))
    }
}

impl<D: CanFrames> PlaneLink for CanPlaneLink<D> {
    fn write_read(&mut self, addr: u8, req: &[u8], resp: &mut [u8]) -> Result<usize, LinkError> {
        let block = *req.first().ok_or(LinkError::Bus)?;
        let (req_id, rsp_id) = self.ids(addr, block)?;
        bump(&mut self.counters.transactions);

        // Whatever is in the FIFO is from before this transaction.
        self.pump();
        self.send_message(req_id, req)?;

        let deadline = self
            .driver
            .now_ms()
            .saturating_add(TRANSACTION_TIMEOUT_MS);
        loop {
            while let Some(frame) = self.next_frame() {
                if let Some(n) = self.feed(&frame, Some(rsp_id), resp) {
                    self.refresh_bus_state();
                    return Ok(n);
                }
            }
            if self.driver.now_ms() >= deadline {
                bump(&mut self.counters.timeouts);
                self.refresh_bus_state();
                return Err(LinkError::Timeout);
            }
            self.driver.idle();
        }
    }

    fn write(&mut self, addr: u8, frame: &[u8]) -> Result<(), LinkError> {
        let block = *frame.first().ok_or(LinkError::Bus)?;
        let (req_id, _) = self.ids(addr, block)?;
        bump(&mut self.counters.writes);
        self.pump();
        let out = self.send_message(req_id, frame);
        self.refresh_bus_state();
        out
    }

    fn event_nodes(&mut self) -> u8 {
        self.pump();
        core::mem::take(&mut self.event_nodes)
    }

    fn sync(&mut self, epoch: u32) -> Result<(), LinkError> {
        // SYNC from our own node, payload `epoch: u32` little endian
        // (decision 7). It goes through the same segmenter as everything
        // else, so the one frame carries the index / last byte in front
        // of the four epoch bytes and a receiver needs one code path.
        let id = CanId::new(MsgType::Sync, self.node).ok_or(LinkError::Bus)?;
        self.send_message(id, &epoch.to_le_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::collections::VecDeque;
    use alloc::vec::Vec;
    use granite_proto::can::SEG_LAST;
    use granite_proto::{BLK_FW, BLK_LOG, BLK_STATUS, BLK_SUMMARY, INDEX_NONE, frame};

    /// A driver over two queues: what the link sent, and what the bus
    /// will hand it back. Time moves only when the link idles, so a
    /// test's timeout is deterministic.
    ///
    /// Frames the bus is told to [`FakeBus::deliver`] wait in `pending`
    /// until the link sends something, then land in `inbox`: that is
    /// what a transaction looks like from here (the request goes out,
    /// the answer comes back), and it is why the link's own `pump` at
    /// the start of a transaction does not swallow the answer to it.
    /// A frame put straight into `inbox` is one that was already on the
    /// bus before the call.
    #[derive(Debug, Default)]
    struct FakeBus {
        /// Frames the link queued, oldest first.
        sent: Vec<(u16, Vec<u8>)>,
        /// Frames waiting to be received.
        inbox: VecDeque<RxFrame>,
        /// Frames the bus hands over once the link has sent a frame.
        pending: VecDeque<RxFrame>,
        /// Milliseconds.
        now: u64,
        /// Milliseconds one [`CanFrames::idle`] costs.
        tick: u64,
        /// How many more sends report a full FIFO.
        block_sends: u32,
        /// Sends that fail outright.
        fail_sends: bool,
        /// Receives that fail outright.
        fail_receives: bool,
        /// What [`CanFrames::bus_state`] answers.
        state: BusState,
    }

    impl FakeBus {
        fn new() -> Self {
            FakeBus {
                tick: 1,
                ..FakeBus::default()
            }
        }

        /// Answer a whole message as the frames of `id`, once the link
        /// has sent something.
        fn deliver(&mut self, id: CanId, msg: &[u8]) {
            self.deliver_lossy(id, msg, usize::MAX);
        }

        /// The same, dropping segment `skip`.
        fn deliver_lossy(&mut self, id: CanId, msg: &[u8], skip: usize) {
            for (i, fd) in can::segment(msg).unwrap().enumerate() {
                if i == skip {
                    continue;
                }
                self.pending
                    .push_back(RxFrame::new(id.raw(), fd.bytes()).unwrap());
            }
        }

        /// A bare frame of `id`, once the link has sent something.
        fn deliver_frame(&mut self, id: CanId, data: &[u8]) {
            self.pending
                .push_back(RxFrame::new(id.raw(), data).unwrap());
        }

        fn ids_sent(&self) -> Vec<u16> {
            self.sent.iter().map(|(id, _)| *id).collect()
        }
    }

    impl CanFrames for FakeBus {
        fn send(&mut self, id: u16, data: &[u8]) -> Result<(), DriverError> {
            if self.fail_sends {
                return Err(DriverError::Bus);
            }
            if self.block_sends > 0 {
                self.block_sends -= 1;
                return Err(DriverError::WouldBlock);
            }
            self.sent.push((id, data.to_vec()));
            while let Some(frame) = self.pending.pop_front() {
                self.inbox.push_back(frame);
            }
            Ok(())
        }

        fn receive(&mut self) -> Result<Option<RxFrame>, DriverError> {
            if self.fail_receives {
                return Err(DriverError::Bus);
            }
            Ok(self.inbox.pop_front())
        }

        fn now_ms(&self) -> u64 {
            self.now
        }

        fn bus_state(&mut self) -> Result<BusState, DriverError> {
            Ok(self.state)
        }

        fn idle(&mut self) {
            self.now += self.tick;
        }
    }

    /// The answer a plane would send for `payload`, as ADR 0003 bytes.
    fn answer(node: u8, block: u8, payload: &[u8]) -> Vec<u8> {
        let mut out = [0u8; frame::MAX_ANSWER];
        frame::answer_frame(&mut out, can::node_addr(node), block, INDEX_NONE, payload)
            .unwrap()
            .to_vec()
    }

    #[test]
    fn a_read_segments_the_request_and_reassembles_the_answer() {
        let node = 3u8;
        let payload: Vec<u8> = (0..100u8).collect();
        let mut bus = FakeBus::new();
        bus.deliver(
            CanId::new(MsgType::Rsp, node).unwrap(),
            &answer(node, BLK_SUMMARY, &payload),
        );
        let mut link = CanPlaneLink::new(bus);

        let request = frame::read_request(BLK_SUMMARY, INDEX_NONE);
        let mut resp = [0u8; frame::MAX_ANSWER];
        let n = link
            .write_read(can::node_addr(node), &request, &mut resp)
            .unwrap();

        // The answer checks out against the node's address, which is
        // what keeps the CRC-8 of ADR 0003 unchanged (decision 1).
        let got = frame::check_answer(can::node_addr(node), &request, &resp[..n]).unwrap();
        assert_eq!(got, &payload[..]);

        // One request frame, under (REQ, node).
        let driver = link.driver();
        assert_eq!(
            driver.ids_sent(),
            [CanId::new(MsgType::Req, node).unwrap().raw()]
        );
        assert_eq!(driver.sent[0].1, [SEG_LAST, BLK_SUMMARY, INDEX_NONE]);

        let counters = link.counters();
        assert_eq!(counters.transactions, 1);
        assert_eq!(counters.tx_frames, 1);
        // 102 answer bytes is two frames.
        assert_eq!(counters.rx_frames, 2);
        assert_eq!(counters.timeouts, 0);
        assert_eq!(counters.foreign, 0);
        assert_eq!(counters.reassembly.total(), 0);
    }

    #[test]
    fn a_write_sends_every_segment_and_waits_for_nothing() {
        let node = 1u8;
        let payload: Vec<u8> = (0..200u8).collect();
        let mut buf = [0u8; frame::MAX_WRITE_FRAME];
        let bytes = frame::write_frame(
            &mut buf,
            can::node_addr(node),
            granite_proto::BLK_CTRL,
            INDEX_NONE,
            &payload,
        )
        .unwrap();

        let mut link = CanPlaneLink::new(FakeBus::new());
        link.write(can::node_addr(node), bytes).unwrap();

        let req = CanId::new(MsgType::Req, node).unwrap().raw();
        assert_eq!(link.driver().ids_sent(), [req, req, req, req]);
        // Every frame but the last is full, and the segment indices run.
        let frames: Vec<Vec<u8>> = link.driver().sent.iter().map(|(_, d)| d.clone()).collect();
        for (i, data) in frames.iter().enumerate() {
            assert_eq!(data[0] & !SEG_LAST, i as u8);
            assert_eq!(data[0] & SEG_LAST != 0, i + 1 == frames.len());
        }
        let counters = link.counters();
        assert_eq!((counters.writes, counters.tx_frames), (1, 4));
        assert_eq!(counters.transactions, 0);
    }

    /// FW and LOG travel as BULK in both directions (decision 2), so a
    /// LOG read must send BULK and wait for BULK, not RSP.
    #[test]
    fn fw_and_log_use_bulk_in_both_directions() {
        let node = 5u8;
        let mut bus = FakeBus::new();
        let msg = answer(node, BLK_LOG, &[1, 2, 3, 4]);
        // The plane answers under BULK. An RSP with the same bytes must
        // not be mistaken for it, so queue that first.
        bus.deliver(CanId::new(MsgType::Rsp, node).unwrap(), &msg);
        bus.deliver(CanId::new(MsgType::Bulk, node).unwrap(), &msg);
        let mut link = CanPlaneLink::new(bus);

        let request = frame::read_request(BLK_LOG, INDEX_NONE);
        let mut resp = [0u8; frame::MAX_ANSWER];
        let n = link
            .write_read(can::node_addr(node), &request, &mut resp)
            .unwrap();
        assert_eq!(
            frame::check_answer(can::node_addr(node), &request, &resp[..n]).unwrap(),
            &[1, 2, 3, 4]
        );
        assert_eq!(
            link.driver().ids_sent(),
            [CanId::new(MsgType::Bulk, node).unwrap().raw()]
        );
        // The RSP frame was somebody else's business.
        assert_eq!(link.counters().foreign, 1);

        // A firmware write goes out as BULK too.
        let mut buf = [0u8; frame::MAX_WRITE_FRAME];
        let bytes =
            frame::write_frame(&mut buf, can::node_addr(node), BLK_FW, 0, &[0xAA; 32]).unwrap();
        link.write(can::node_addr(node), bytes).unwrap();
        assert_eq!(
            *link.driver().ids_sent().last().unwrap(),
            CanId::new(MsgType::Bulk, node).unwrap().raw()
        );
    }

    #[test]
    fn no_answer_is_a_timeout_the_client_retries() {
        let mut link = CanPlaneLink::new(FakeBus::new());
        let request = frame::read_request(BLK_STATUS, INDEX_NONE);
        let mut resp = [0u8; frame::MAX_ANSWER];
        let err = link
            .write_read(can::node_addr(2), &request, &mut resp)
            .unwrap_err();
        // Timeout, not Nak: the client retries a timeout and reads a NAK
        // as "no plane is strapped there" (plane::link::READ_TRIES).
        assert_eq!(err, LinkError::Timeout);
        assert_eq!(link.counters().timeouts, 1);
        // It waited the ADR's 20 ms and no longer.
        assert!(link.driver().now_ms() >= TRANSACTION_TIMEOUT_MS);
        assert!(link.driver().now_ms() <= TRANSACTION_TIMEOUT_MS + link.driver().tick);
    }

    #[test]
    fn an_address_without_a_plane_naks_without_touching_the_bus() {
        let mut link = CanPlaneLink::new(FakeBus::new());
        let request = frame::read_request(BLK_STATUS, INDEX_NONE);
        let mut resp = [0u8; frame::MAX_ANSWER];
        // 0x38 is node 8, which the ADR reserves.
        for addr in [0x2F, 0x38, 0x3E, 0x3F] {
            assert_eq!(
                link.write_read(addr, &request, &mut resp),
                Err(LinkError::Nak)
            );
            assert_eq!(link.write(addr, &request), Err(LinkError::Nak));
        }
        assert!(link.driver().sent.is_empty());
        assert_eq!(link.counters().tx_frames, 0);
    }

    #[test]
    fn a_lost_segment_times_out_and_the_next_read_still_works() {
        let node = 4u8;
        let payload: Vec<u8> = (0..200u8).collect();
        let msg = answer(node, BLK_SUMMARY, &payload);
        let rsp = CanId::new(MsgType::Rsp, node).unwrap();
        let mut bus = FakeBus::new();
        // Segment 1 of four never arrives.
        bus.deliver_lossy(rsp, &msg, 1);
        let mut link = CanPlaneLink::new(bus);

        let request = frame::read_request(BLK_SUMMARY, INDEX_NONE);
        let mut resp = [0u8; frame::MAX_ANSWER];
        assert_eq!(
            link.write_read(can::node_addr(node), &request, &mut resp),
            Err(LinkError::Timeout)
        );
        // The reassembler noticed the gap rather than splicing it.
        assert!(link.counters().reassembly.total() > 0);

        // The retry the client would do: a whole answer this time.
        link.driver().deliver(rsp, &msg);
        let n = link
            .write_read(can::node_addr(node), &request, &mut resp)
            .unwrap();
        assert_eq!(
            frame::check_answer(can::node_addr(node), &request, &resp[..n]).unwrap(),
            &payload[..]
        );
    }

    /// An EVENT that arrives while the controller is waiting for an
    /// answer sets its bit and does not disturb the transaction
    /// (decision 6).
    #[test]
    fn an_event_during_a_transaction_is_kept() {
        let node = 6u8;
        let msg = answer(node, BLK_STATUS, &[0x12, 4, 0x10, 0x0E, 0, 0]);
        let mut bus = FakeBus::new();
        // Plane 2's EVENT lands between the request and the answer.
        bus.deliver_frame(CanId::new(MsgType::Event, 2).unwrap(), &[SEG_LAST, 0x01]);
        bus.deliver(CanId::new(MsgType::Rsp, node).unwrap(), &msg);
        let mut link = CanPlaneLink::new(bus);

        let request = frame::read_request(BLK_STATUS, INDEX_NONE);
        let mut resp = [0u8; frame::MAX_ANSWER];
        let n = link
            .write_read(can::node_addr(node), &request, &mut resp)
            .unwrap();
        assert_eq!(
            frame::check_answer(can::node_addr(node), &request, &resp[..n]).unwrap(),
            &[0x12, 4, 0x10, 0x0E, 0, 0]
        );
        // The mask survived the transaction and is taken once.
        assert_eq!(link.event_nodes(), 1 << 2);
        assert_eq!(link.event_nodes(), 0);
        assert_eq!(link.counters().events, 1);
        assert_eq!(link.counters().foreign, 0);
    }

    /// EVENT frames that arrive with no transaction running are picked
    /// up by `event_nodes` itself.
    #[test]
    fn events_are_collected_outside_a_transaction() {
        let mut bus = FakeBus::new();
        for node in [0u8, 7] {
            bus.inbox.push_back(
                RxFrame::new(
                    CanId::new(MsgType::Event, node).unwrap().raw(),
                    &[SEG_LAST, 0x01],
                )
                .unwrap(),
            );
        }
        // A controller's EVENT identifier cannot exist (node 14 is not a
        // plane), and a reserved identifier is counted, not trusted.
        bus.inbox
            .push_back(RxFrame::new((1 << 4) | 9, &[SEG_LAST, 0x01]).unwrap());
        let mut link = CanPlaneLink::new(bus);

        assert_eq!(link.event_nodes(), 0b1000_0001);
        assert_eq!(link.counters().unknown_id, 1);
    }

    /// An answer from the wrong plane is dropped, and the right one is
    /// still read: the node field names the plane (decision 2).
    #[test]
    fn an_answer_from_the_wrong_node_is_ignored() {
        let node = 1u8;
        let wrong = 2u8;
        let mut bus = FakeBus::new();
        bus.deliver(
            CanId::new(MsgType::Rsp, wrong).unwrap(),
            &answer(wrong, BLK_STATUS, &[0xFF; 6]),
        );
        bus.deliver(
            CanId::new(MsgType::Rsp, node).unwrap(),
            &answer(node, BLK_STATUS, &[1, 2, 3, 4, 5, 6]),
        );
        let mut link = CanPlaneLink::new(bus);

        let request = frame::read_request(BLK_STATUS, INDEX_NONE);
        let mut resp = [0u8; frame::MAX_ANSWER];
        let n = link
            .write_read(can::node_addr(node), &request, &mut resp)
            .unwrap();
        assert_eq!(
            frame::check_answer(can::node_addr(node), &request, &resp[..n]).unwrap(),
            &[1, 2, 3, 4, 5, 6]
        );
        assert_eq!(link.counters().foreign, 1);
    }

    /// Another controller's SYNC, and a standby's, are not ours to read.
    #[test]
    fn sync_and_foreign_requests_are_counted_and_dropped() {
        let mut bus = FakeBus::new();
        for id in [
            CanId::new(MsgType::Sync, NODE_CONTROLLER).unwrap(),
            CanId::new(MsgType::Sync, can::NODE_STANDBY).unwrap(),
            CanId::new(MsgType::Req, 3).unwrap(),
        ] {
            bus.inbox
                .push_back(RxFrame::new(id.raw(), &[SEG_LAST, 0, 0, 0, 0]).unwrap());
        }
        let mut link = CanPlaneLink::new(bus);
        assert_eq!(link.event_nodes(), 0);
        assert_eq!(link.counters().foreign, 3);
        assert_eq!(link.counters().rx_frames, 3);
    }

    /// SYNC: one frame from node 14, the epoch little endian behind the
    /// segment header (decision 7).
    #[test]
    fn sync_carries_the_epoch_little_endian() {
        let mut link = CanPlaneLink::new(FakeBus::new());
        link.sync(0x1234_5678).unwrap();
        let (id, data) = link.driver().sent[0].clone();
        assert_eq!(id, CanId::new(MsgType::Sync, NODE_CONTROLLER).unwrap().raw());
        assert_eq!(data, [SEG_LAST, 0x78, 0x56, 0x34, 0x12]);
        // The standby controller's link would send it from node 15.
        let mut standby = CanPlaneLink::with_node(FakeBus::new(), can::NODE_STANDBY);
        standby.sync(1).unwrap();
        assert_eq!(
            standby.driver().sent[0].0,
            CanId::new(MsgType::Sync, can::NODE_STANDBY).unwrap().raw()
        );
        assert_eq!(standby.node(), can::NODE_STANDBY);
    }

    #[test]
    fn a_full_transmit_fifo_is_waited_out_then_given_up() {
        let mut bus = FakeBus::new();
        bus.block_sends = 3;
        let mut link = CanPlaneLink::new(bus);
        link.sync(7).unwrap();
        assert_eq!(link.counters().tx_blocked, 3);
        assert_eq!(link.counters().tx_frames, 1);

        // A FIFO that never frees up is a timeout, not a hang.
        let mut bus = FakeBus::new();
        bus.block_sends = u32::MAX;
        let mut link = CanPlaneLink::new(bus);
        assert_eq!(link.sync(7), Err(LinkError::Timeout));
        assert!(link.driver().now_ms() >= SEND_TIMEOUT_MS);
    }

    #[test]
    fn driver_faults_are_counted_and_reported() {
        let mut bus = FakeBus::new();
        bus.fail_sends = true;
        let mut link = CanPlaneLink::new(bus);
        assert_eq!(link.sync(1), Err(LinkError::Bus));
        assert_eq!(link.counters().tx_errors, 1);

        let mut bus = FakeBus::new();
        bus.fail_receives = true;
        let mut link = CanPlaneLink::new(bus);
        assert_eq!(link.event_nodes(), 0);
        assert!(link.counters().rx_errors > 0);
    }

    /// Bus-off is counted once per transition, not once per read
    /// (decision 8: automatic recovery, counted).
    #[test]
    fn bus_off_is_counted_on_the_edge() {
        let mut link = CanPlaneLink::new(FakeBus::new());
        link.driver().state = BusState {
            tec: 200,
            rec: 10,
            bus_off: true,
        };
        for _ in 0..3 {
            link.refresh_bus_state();
        }
        assert_eq!(link.counters().bus_off, 1);
        assert_eq!((link.counters().tec, link.counters().rec), (200, 10));
        assert!(link.counters().bus_off_now);

        // Recovered, then off again: two transitions.
        link.driver().state.bus_off = false;
        link.refresh_bus_state();
        assert!(!link.counters().bus_off_now);
        link.driver().state.bus_off = true;
        link.refresh_bus_state();
        assert_eq!(link.counters().bus_off, 2);
    }

    /// A message longer than the transport can carry is refused before
    /// anything reaches the bus.
    #[test]
    fn an_impossible_message_is_refused() {
        let mut link = CanPlaneLink::new(FakeBus::new());
        let mut resp = [0u8; 4];
        assert_eq!(link.write_read(0x30, &[], &mut resp), Err(LinkError::Bus));
        let long = [0x40u8; can::MAX_MESSAGE + 1];
        assert_eq!(
            link.write(0x30, &long),
            Err(LinkError::BufferTooSmall),
            "a message that cannot be segmented must not be half-sent"
        );
        assert!(link.driver().sent.is_empty());
    }

    /// The padding of the last frame may be longer than the caller's
    /// buffer; what is copied is the answer, and `check_answer` reads
    /// the length from the answer itself.
    #[test]
    fn the_answer_is_copied_up_to_the_callers_buffer() {
        let node = 0u8;
        let payload: Vec<u8> = (0..60u8).collect();
        let msg = answer(node, BLK_SUMMARY, &payload);
        let mut bus = FakeBus::new();
        bus.deliver(CanId::new(MsgType::Rsp, node).unwrap(), &msg);
        let mut link = CanPlaneLink::new(bus);

        let request = frame::read_request(BLK_SUMMARY, INDEX_NONE);
        let mut resp = [0u8; frame::MAX_ANSWER];
        let n = link
            .write_read(can::node_addr(node), &request, &mut resp)
            .unwrap();
        // 62 answer bytes plus the segment header is 63 on the wire,
        // which pads to a 64-byte frame: one zero comes back with them.
        assert_eq!((msg.len(), n), (62, 63));
        assert_eq!(&resp[..msg.len()], &msg[..]);
        assert_eq!(
            frame::check_answer(can::node_addr(node), &request, &resp[..n]).unwrap(),
            &payload[..]
        );

        // A buffer that cannot hold the answer gets what fits, and the
        // frame check above the link is what refuses it.
        link.driver()
            .deliver(CanId::new(MsgType::Rsp, node).unwrap(), &msg);
        let mut small = [0u8; 8];
        let n = link
            .write_read(can::node_addr(node), &request, &mut small)
            .unwrap();
        assert_eq!(n, small.len());
        assert!(frame::check_answer(can::node_addr(node), &request, &small[..n]).is_err());
    }

    /// Nothing in the transport panics, whatever comes off the bus.
    #[test]
    fn no_frame_sequence_panics() {
        let mut state = 0x1234_5678u32;
        let mut rng = move || {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            state
        };
        let mut bus = FakeBus::new();
        for _ in 0..5_000 {
            let len = (rng() % (MAX_FD_LEN as u32 + 1)) as usize;
            let mut data = [0u8; MAX_FD_LEN];
            for byte in &mut data[..len] {
                *byte = rng() as u8;
            }
            bus.inbox
                .push_back(RxFrame::new((rng() % 0x0800) as u16, &data[..len]).unwrap());
        }
        let mut link = CanPlaneLink::new(bus);
        let request = frame::read_request(BLK_STATUS, INDEX_NONE);
        let mut resp = [0u8; frame::MAX_ANSWER];
        for node in 0..8u8 {
            let _ = link.write_read(can::node_addr(node), &request, &mut resp);
            let _ = link.event_nodes();
        }
        // A frame longer than a CAN FD frame cannot even be built.
        assert!(RxFrame::new(0, &[0u8; MAX_FD_LEN + 1]).is_none());
        // Every frame was looked at, and the counters stayed sane.
        let counters = link.counters();
        assert!(counters.rx_frames > 0);
        assert_eq!(
            counters.rx_frames,
            counters.events + counters.foreign + counters.unknown_id + counters.rx_in_messages
        );
    }
}
