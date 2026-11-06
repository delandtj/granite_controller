//! The plane's side of the CAN FD transport (ADR 0005 decisions 2-8).
//!
//! The messages are the ADR 0003 frames of [`Plane::handle_frame`], byte
//! for byte; what this module owns is everything between them and a CAN
//! FD controller:
//!
//! - which frames are this plane's (REQ and BULK on its own node id, and
//!   nothing else: SYNC is ignored, a frame for another node is not even
//!   reassembled),
//! - reassembly of a request through [`granite_proto::can::Reassembler`],
//! - the answer, segmented back out as RSP - or as BULK for the blocks
//!   [`granite_proto::can::msg_type_for_block`] puts there (FW and LOG),
//! - the EVENT schedule: one frame the moment the first event is raised
//!   and one every [`EVENT_PERIOD_MS`] while anything is unacknowledged
//!   (ADR 0005 decision 6, which replaces the rev D EXP_INT line),
//! - the counters STATUS 0x1D carries: TEC / REC, bus-off recoveries,
//!   reassembly drops, frames a full transmit FIFO refused.
//!
//! The controller underneath is a [`CanFrames`] implementation, which is
//! three register reads and two FIFO operations wide and says nothing
//! about *how* the chip is reached: the G070 build drives an MCP2518FD
//! over SPI, the G0B1 build its own FDCAN, and the host tests a fake. The
//! trait is synchronous and takes the clock as an argument, so the whole
//! transport is a state machine with no executor in it; the firmware's
//! task does the awaiting around it.
//!
//! Nothing here allocates and nothing here keeps a frame longer than the
//! call it came in on. One [`CanLink`] is [`granite_proto::can::REASSEMBLY_CAP`]
//! bytes per slot plus a little, which on the plane's 36 KB is why the
//! default is two slots: the transaction being answered and one bulk
//! transfer beside it.
//!
//! ```
//! # use plane_logic::can_link::{CanFrames, CanLink, RawFrame, TxError};
//! # use plane_logic::{Plane, Setup};
//! # struct NoCan;
//! # impl CanFrames for NoCan {
//! #     type Error = ();
//! #     fn send(&mut self, _id: u16, _d: &[u8]) -> Result<(), TxError<()>> { Ok(()) }
//! #     fn receive(&mut self) -> Result<Option<RawFrame>, ()> { Ok(None) }
//! #     fn error_counters(&mut self) -> Result<(u8, u8), ()> { Ok((0, 0)) }
//! #     fn is_bus_off(&mut self) -> Result<bool, ()> { Ok(false) }
//! # }
//! // A plane strapped to node 3 (its ADR 0003 address is 0x33).
//! let mut plane = Plane::new(Setup::bare(0x33));
//! let mut link: CanLink = CanLink::new(3);
//! let mut can = NoCan;
//! // Once per firmware tick: answer what came in, then run the
//! // schedules.
//! link.poll(0, &mut plane, &mut can);
//! ```

use granite_proto::can::{
    self, CanId, MsgType, MAX_FD_LEN, Reassembler, ReassemblyError,
};
use granite_proto::{CanErrors, frame};

use crate::plane::Plane;

/// How often an unacknowledged event is announced (ADR 0005 decision 6).
pub const EVENT_PERIOD_MS: u32 = 1_000;

/// The one byte of an EVENT frame: "I have unacknowledged events". The
/// controller reads STATUS and then EVENTS, as on rev D, so the flag
/// carries nothing else (ADR 0005 open question, answered 2026-11-04).
pub const EVENT_PAYLOAD: u8 = 0x01;

/// How often the controller's own error counters are read back. The CAN
/// counters move on the wire, not in this code, and reading them costs
/// an SPI transaction on the G070 build, so they are sampled at the
/// poll rate the controller reads STATUS at and not per tick.
pub const COUNTER_PERIOD_MS: u32 = 1_000;

/// One CAN FD frame, as a driver hands it over or takes it.
///
/// The identifier is raw: [`CanId::decode`] is what turns it into a type
/// and a node, and a frame whose id is none of ours is counted and
/// dropped rather than refused by the driver.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawFrame {
    /// Standard (11-bit) identifier.
    pub id: u16,
    /// Data length, one of [`granite_proto::can::FD_LENGTHS`].
    pub len: u8,
    /// The data bytes, zero beyond `len`.
    pub data: [u8; MAX_FD_LEN],
}

impl RawFrame {
    /// A frame with `bytes` as its data; bytes past [`MAX_FD_LEN`] are
    /// dropped, which no CAN FD controller can produce.
    pub fn new(id: u16, bytes: &[u8]) -> Self {
        let n = bytes.len().min(MAX_FD_LEN);
        let mut data = [0u8; MAX_FD_LEN];
        data[..n].copy_from_slice(&bytes[..n]);
        RawFrame {
            id,
            len: n as u8,
            data,
        }
    }

    /// The data bytes on the wire.
    pub fn bytes(&self) -> &[u8] {
        &self.data[..(self.len as usize).min(MAX_FD_LEN)]
    }
}

/// Why a frame did not go out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TxError<E> {
    /// The transmit FIFO is full; nothing was queued and the call can be
    /// retried. Counted in [`Counters::tx_full`].
    Full,
    /// The driver's own error (an SPI fault, a chip in the wrong mode).
    Driver(E),
}

/// The CAN FD controller under the transport.
///
/// Synchronous on purpose: every operation is a register or FIFO access
/// that takes microseconds (an SPI burst on the G070 build, a memory
/// write on the G0B1), and making them `async` would put an executor
/// inside a state machine that is otherwise host-testable.
pub trait CanFrames {
    /// The driver's own error type.
    type Error;

    /// Queue one frame for transmission. The data is at most
    /// [`MAX_FD_LEN`] bytes and already padded to a valid CAN FD length
    /// by [`granite_proto::can::segment`].
    fn send(&mut self, id: u16, data: &[u8]) -> Result<(), TxError<Self::Error>>;

    /// Take one frame off the receive FIFO, or `None` when it is empty.
    fn receive(&mut self) -> Result<Option<RawFrame>, Self::Error>;

    /// The transmit and receive error counters, `(TEC, REC)`.
    fn error_counters(&mut self) -> Result<(u8, u8), Self::Error>;

    /// True while the node is bus-off. Recovery is automatic (ADR 0005
    /// decision 8), so the transport only counts the transitions.
    fn is_bus_off(&mut self) -> Result<bool, Self::Error>;
}

/// What the transport has seen since boot. Every counter saturates.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counters {
    /// Transmit error counter, as last sampled.
    pub tec: u8,
    /// Receive error counter, as last sampled.
    pub rec: u8,
    /// How often the node went bus-off (and recovered by itself).
    pub bus_off: u16,
    /// Frames not queued because the transmit FIFO was full.
    pub tx_full: u16,
    /// Frames the transport could make nothing of: no bytes at all, more
    /// than 64, or an identifier outside ADR 0005 decision 2.
    pub bad_frames: u16,
    /// Frames for another node, or of a type a plane does not answer
    /// (SYNC, EVENT, RSP). Not an error: a plane sees the whole bus.
    pub ignored: u16,
    /// Errors the driver itself reported, on any operation.
    pub driver_errors: u16,
    /// Messages reassembled and handed to [`Plane::handle_frame`].
    pub messages_in: u32,
    /// Answers segmented and queued.
    pub messages_out: u32,
    /// EVENT frames sent.
    pub events_sent: u16,
}

/// The transport: frames in, messages through the plane, frames out.
#[derive(Debug)]
pub struct CanLink<const N: usize = { can::DEFAULT_SLOTS }> {
    node: u8,
    rx: Reassembler<N>,
    counters: Counters,
    /// When the last EVENT went out; `None` while nothing is pending, so
    /// the next raise announces itself at once.
    event_ms: Option<u32>,
    /// When the error counters were last sampled.
    counter_ms: Option<u32>,
    /// Whether the node was bus-off at the last sample, so a recovery
    /// and a new bus-off are two counts and not one.
    bus_off: bool,
}

impl<const N: usize> CanLink<N> {
    /// A transport for the plane strapped to `node` (its ADR 0003
    /// address is `0x30 + node`, [`granite_proto::can::node_addr`]).
    pub const fn new(node: u8) -> Self {
        CanLink {
            node: node & can::NODE_MASK as u8,
            rx: Reassembler::new(),
            counters: Counters {
                tec: 0,
                rec: 0,
                bus_off: 0,
                tx_full: 0,
                bad_frames: 0,
                ignored: 0,
                driver_errors: 0,
                messages_in: 0,
                messages_out: 0,
                events_sent: 0,
            },
            event_ms: None,
            counter_ms: None,
            bus_off: false,
        }
    }

    /// The node this plane answers for.
    pub const fn node(&self) -> u8 {
        self.node
    }

    /// What the transport has seen.
    pub const fn counters(&self) -> Counters {
        self.counters
    }

    /// Partial messages reassembly dropped, by reason.
    pub const fn reassembly_counters(&self) -> can::ReassemblyCounters {
        self.rx.counters()
    }

    /// The counters as STATUS 0x1D carries them: the two error counters,
    /// the bus-off count, every partial message dropped and every frame
    /// a full FIFO refused, each saturated into its field.
    pub fn can_errors(&self) -> CanErrors {
        CanErrors {
            tec: self.counters.tec,
            rec: self.counters.rec,
            bus_off: self.counters.bus_off,
            dropped: u16::try_from(self.rx.counters().total()).unwrap_or(u16::MAX),
            tx_full: self.counters.tx_full,
        }
    }

    /// One round: take everything the controller has, answer what is
    /// this plane's, then run the EVENT and counter schedules.
    ///
    /// `now_ms` is a free-running millisecond clock and may wrap: only
    /// differences are used.
    pub fn poll<C: CanFrames>(&mut self, now_ms: u32, plane: &mut Plane, can: &mut C) {
        self.drain(now_ms, plane, can);
        self.run_events(now_ms, plane.event_pending(), can);
        self.sample_counters(now_ms, can);
    }

    /// Take every frame waiting in the receive FIFO and answer the ones
    /// addressed to this plane.
    pub fn drain<C: CanFrames>(&mut self, now_ms: u32, plane: &mut Plane, can: &mut C) {
        loop {
            match can.receive() {
                Ok(Some(frame)) => self.on_frame(now_ms, &frame, plane, can),
                Ok(None) => return,
                Err(_) => {
                    self.counters.driver_errors = self.counters.driver_errors.saturating_add(1);
                    return;
                }
            }
        }
    }

    /// Feed one received frame.
    ///
    /// A plane answers only REQ and BULK addressed to its own node
    /// (ADR 0005 decision 5); SYNC is ignored (decision 7) and so is
    /// every frame of another node, which a plane sees because the bus
    /// is shared and the acceptance filter is only as narrow as the node
    /// field.
    pub fn on_frame<C: CanFrames>(
        &mut self,
        now_ms: u32,
        frame: &RawFrame,
        plane: &mut Plane,
        can: &mut C,
    ) {
        let Ok(id) = CanId::decode(frame.id) else {
            self.counters.bad_frames = self.counters.bad_frames.saturating_add(1);
            return;
        };
        if id.node() != self.node || !matches!(id.msg_type(), MsgType::Req | MsgType::Bulk) {
            self.counters.ignored = self.counters.ignored.saturating_add(1);
            return;
        }

        // The answer is built while the reassembled message is still
        // borrowed, and sent after: `out` is this frame's own buffer, so
        // the borrow of the reassembler ends with the match.
        let mut out = [0u8; frame::MAX_ANSWER];
        let answer = match self.rx.push(id, frame.bytes(), now_ms) {
            Ok(Some(message)) => {
                // The last segment arrived padded to a valid CAN FD
                // length (decision 3); the message's own content says
                // where it ends, and `parse_frame` takes a length, so
                // the padding comes off here.
                let bytes = frame::trim_padded(message.bytes());
                // Byte 0 of every ADR 0003 frame is the block, which is
                // what decides whether the answer travels as RSP or as
                // BULK (FW and LOG).
                let block = bytes.first().copied().unwrap_or(0);
                let kind = can::msg_type_for_block(block, false);
                let len = plane.handle_frame(bytes, &mut out).map_or(0, <[u8]>::len);
                self.counters.messages_in = self.counters.messages_in.saturating_add(1);
                Some((kind, len))
            }
            Ok(None) => None,
            Err(ReassemblyError::EmptyFrame | ReassemblyError::FrameTooLong) => {
                self.counters.bad_frames = self.counters.bad_frames.saturating_add(1);
                None
            }
            // Every other reason already counted the partial message it
            // dropped (`reassembly_counters`), which is what STATUS
            // reports; there is nothing to answer.
            Err(_) => None,
        };

        if let Some((kind, len)) = answer
            && len > 0
        {
            self.send_message(can, kind, &out[..len]);
        }
    }

    /// Cut a message into frames and queue them all under one id.
    ///
    /// A frame the transmit FIFO refuses abandons the rest of the
    /// message: half an answer is one the controller's CRC refuses
    /// anyway, and its retry (ADR 0003) is what recovers.
    fn send_message<C: CanFrames>(&mut self, can: &mut C, kind: MsgType, msg: &[u8]) -> bool {
        let Some(id) = CanId::new(kind, self.node) else {
            return false;
        };
        let Ok(frames) = can::segment(msg) else {
            return false;
        };
        for fd in frames {
            match can.send(id.raw(), fd.bytes()) {
                Ok(()) => {}
                Err(TxError::Full) => {
                    self.counters.tx_full = self.counters.tx_full.saturating_add(1);
                    return false;
                }
                Err(TxError::Driver(_)) => {
                    self.counters.driver_errors = self.counters.driver_errors.saturating_add(1);
                    return false;
                }
            }
        }
        self.counters.messages_out = self.counters.messages_out.saturating_add(1);
        true
    }

    /// The EVENT schedule: announce the first unacknowledged event at
    /// once and repeat every [`EVENT_PERIOD_MS`] until an EV_ACK covers
    /// them all (ADR 0005 decision 6).
    ///
    /// A frame a full FIFO refused does not advance the schedule, so the
    /// next call retries immediately.
    pub fn run_events<C: CanFrames>(&mut self, now_ms: u32, pending: bool, can: &mut C) {
        if !pending {
            self.event_ms = None;
            return;
        }
        let due = match self.event_ms {
            None => true,
            Some(sent) => now_ms.wrapping_sub(sent) >= EVENT_PERIOD_MS,
        };
        if !due {
            return;
        }
        let Some(id) = CanId::new(MsgType::Event, self.node) else {
            return;
        };
        // An EVENT is one byte and is not segmented: there is no message
        // under it to reassemble, only the flag of decision 6.
        match can.send(id.raw(), &[EVENT_PAYLOAD]) {
            Ok(()) => {
                self.counters.events_sent = self.counters.events_sent.saturating_add(1);
                self.event_ms = Some(now_ms);
            }
            Err(TxError::Full) => {
                self.counters.tx_full = self.counters.tx_full.saturating_add(1);
            }
            Err(TxError::Driver(_)) => {
                self.counters.driver_errors = self.counters.driver_errors.saturating_add(1);
            }
        }
    }

    /// Sample TEC / REC and the bus-off state, at most every
    /// [`COUNTER_PERIOD_MS`].
    pub fn sample_counters<C: CanFrames>(&mut self, now_ms: u32, can: &mut C) {
        if let Some(last) = self.counter_ms
            && now_ms.wrapping_sub(last) < COUNTER_PERIOD_MS
        {
            return;
        }
        self.counter_ms = Some(now_ms);
        match can.error_counters() {
            Ok((tec, rec)) => {
                self.counters.tec = tec;
                self.counters.rec = rec;
            }
            Err(_) => self.counters.driver_errors = self.counters.driver_errors.saturating_add(1),
        }
        match can.is_bus_off() {
            Ok(off) => {
                // Recovery is the controller's own (128 x 11 recessive
                // bits), so what is worth a counter is the edge into
                // bus-off and not the state.
                if off && !self.bus_off {
                    self.counters.bus_off = self.counters.bus_off.saturating_add(1);
                }
                self.bus_off = off;
            }
            Err(_) => self.counters.driver_errors = self.counters.driver_errors.saturating_add(1),
        }
    }

    /// True while the node was bus-off at the last sample.
    pub const fn is_bus_off(&self) -> bool {
        self.bus_off
    }

    /// Forget every message in flight (after a bus-off, say); the
    /// counters stay.
    pub fn clear(&mut self) {
        self.rx.clear();
    }
}
