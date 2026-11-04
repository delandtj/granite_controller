//! CAN FD transport for the same messages (ADR 0005 decisions 1-5).
//!
//! The messages are the ADR 0003 frames of [`crate::frame`], byte for
//! byte: a read request is `block, index`, a write is
//! `block, index, L, records, crc`, an answer is `L, records, crc`.
//! The CRC-8 still covers the 7-bit address, which on CAN is
//! [`node_addr`] (`0x30 + node`), so neither end's CRC code changes.
//!
//! What this module adds is the transport under them:
//!
//! - [`MsgType`] and [`CanId`]: the 11-bit identifier
//!   `(type << 4) | node`, encode and decode.
//! - [`node_addr`] / [`node_of_addr`]: the ADR 0003 address of a node.
//! - [`msg_type_for_block`]: which type carries a block.
//! - [`fd_len`]: the CAN FD data lengths a frame can have.
//! - [`segment`]: a message into frames of at most 63 message bytes,
//!   each with an index / last byte in front.
//! - [`Reassembler`]: frames back into a message, with the drop rules
//!   of decision 4 and a counter per reason.
//!
//! Nothing here allocates, nothing here panics on any input and
//! nothing here touches a CAN peripheral: bytes in, bytes out. The
//! reassembler is the only part that keeps state, and its state is a
//! fixed array of slots sized by a const generic.
//!
//! ```
//! use granite_proto::can::{self, CanId, MsgType, Reassembler};
//! use granite_proto::{BLK_STATUS, INDEX_NONE, frame};
//!
//! // Plane 3 answers a STATUS read. The answer is an ADR 0003 frame,
//! // its CRC-8 over the node's address (0x30 + 3).
//! let mut out = [0u8; frame::MAX_ANSWER];
//! let answer = frame::answer_frame(
//!     &mut out,
//!     can::node_addr(3),
//!     BLK_STATUS,
//!     INDEX_NONE,
//!     &[0x12, 0x04, 0x10, 0x0E, 0x00, 0x00],
//! )
//! .unwrap();
//!
//! // Send it as RSP from node 3 and put it back together.
//! let id = CanId::new(can::msg_type_for_block(BLK_STATUS, false), 3).unwrap();
//! let mut rx: Reassembler = Reassembler::new();
//! let mut got = Vec::new();
//! for fd in can::segment(answer).unwrap() {
//!     if let Some(message) = rx.push(id, fd.bytes(), 0).unwrap() {
//!         assert_eq!((message.msg_type(), message.node()), (MsgType::Rsp, 3));
//!         got.extend_from_slice(message.bytes());
//!     }
//! }
//!
//! // The last frame is padded to a valid CAN FD length: the message's
//! // own length byte says where it ends (ADR 0003 `L`), so the
//! // controller's check works on the padded bytes as they are.
//! let request = frame::read_request(BLK_STATUS, INDEX_NONE);
//! let payload = frame::check_answer(id.addr(), &request, &got).unwrap();
//! assert_eq!(payload, &[0x12, 0x04, 0x10, 0x0E, 0x00, 0x00]);
//! ```

use crate::{ADDR_BASE, BLK_FW, BLK_LOG, MAX_PLANES};

// --- Nodes (ADR 0005 decision 2) -------------------------------------

/// Node number of the controller.
pub const NODE_CONTROLLER: u8 = 14;

/// Node number of the standby controller (hardware ADR 0006 decision
/// 13). Nothing sends from it yet; the number is reserved for it.
pub const NODE_STANDBY: u8 = 15;

/// Node numbers that fit the 4-bit field of an identifier.
pub const NODE_COUNT: usize = 16;

/// True for a node number the ADR gives a meaning: a plane (0-7), the
/// controller (14) or the standby controller (15). 8-13 are reserved.
pub const fn is_node(node: u8) -> bool {
    (node as usize) < MAX_PLANES || node == NODE_CONTROLLER || node == NODE_STANDBY
}

/// True for a node number that is a plane's (0-7).
pub const fn is_plane_node(node: u8) -> bool {
    (node as usize) < MAX_PLANES
}

/// True for a node number that is a controller's (14 or 15).
pub const fn is_controller_node(node: u8) -> bool {
    node == NODE_CONTROLLER || node == NODE_STANDBY
}

/// The ADR 0003 address of a node: `0x30 + node` (decision 1, so the
/// CRC-8 of a message does not change with the transport).
///
/// Only the low four bits of `node` are used, the width of the field in
/// an identifier; a node number outside 0-15 cannot reach the wire.
pub const fn node_addr(node: u8) -> u8 {
    ADDR_BASE + (node & 0x0F)
}

/// The node of an ADR 0003 address, if the address is a node's
/// (0x30-0x3F). The inverse of [`node_addr`].
pub const fn node_of_addr(addr: u8) -> Option<u8> {
    if addr >= ADDR_BASE && ((addr - ADDR_BASE) as usize) < NODE_COUNT {
        Some(addr - ADDR_BASE)
    } else {
        None
    }
}

// --- Message types (ADR 0005 decision 2) -----------------------------

/// The message type of an identifier, its high seven bits. A lower
/// identifier wins arbitration, so the order is the priority: SYNC
/// before EVENT before a transaction before a bulk transfer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum MsgType {
    /// Controller keepalive, once per second, payload `epoch: u32` LE
    /// (decision 7).
    Sync = 0,
    /// A plane has unacknowledged events, payload `0x01` (decision 6).
    Event = 1,
    /// A controller's read request or write (decision 5).
    Req = 2,
    /// A plane's answer to a [`MsgType::Req`].
    Rsp = 3,
    /// FW (0x13) and LOG (0x08) in either direction, so a long
    /// transfer never blocks a poll.
    Bulk = 4,
}

impl MsgType {
    /// The wire value.
    pub const fn value(self) -> u8 {
        self as u8
    }

    /// The type for a wire value; `None` for the reserved 5-15.
    pub const fn from_value(value: u8) -> Option<Self> {
        match value {
            0 => Some(MsgType::Sync),
            1 => Some(MsgType::Event),
            2 => Some(MsgType::Req),
            3 => Some(MsgType::Rsp),
            4 => Some(MsgType::Bulk),
            _ => None,
        }
    }
}

impl TryFrom<u8> for MsgType {
    type Error = crate::UnknownValue;

    fn try_from(value: u8) -> Result<Self, crate::UnknownValue> {
        Self::from_value(value).ok_or(crate::UnknownValue(value as u16))
    }
}

impl From<MsgType> for u8 {
    fn from(v: MsgType) -> u8 {
        v.value()
    }
}

/// Which message type carries a block (decision 2): FW and LOG travel
/// as [`MsgType::Bulk`] in both directions, every other block as
/// [`MsgType::Req`] from a controller and [`MsgType::Rsp`] from a
/// plane.
pub const fn msg_type_for_block(block: u8, from_controller: bool) -> MsgType {
    if block == BLK_FW || block == BLK_LOG {
        MsgType::Bulk
    } else if from_controller {
        MsgType::Req
    } else {
        MsgType::Rsp
    }
}

// --- Identifiers (ADR 0005 decision 2) -------------------------------

/// Bits of the node field of an identifier.
pub const NODE_BITS: u32 = 4;

/// Mask of the node field of an identifier.
pub const NODE_MASK: u16 = 0x000F;

/// Largest 11-bit identifier.
pub const MAX_ID: u16 = 0x07FF;

/// Why a raw identifier is not one of ours.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdError {
    /// The value does not fit 11 bits (a CAN FD extended identifier,
    /// or a stray high bit).
    NotStandard,
    /// The type field is one of the reserved 5-15.
    ReservedType,
    /// The node field is one of the reserved 8-13.
    ReservedNode,
}

/// An 11-bit identifier: `(type << 4) | node`.
///
/// Only identifiers the ADR gives a meaning exist: [`CanId::new`] and
/// [`CanId::decode`] reject a reserved type and a reserved node, so a
/// `CanId` always names a type and a node that both ends know.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CanId {
    msg_type: MsgType,
    node: u8,
}

impl CanId {
    /// The identifier for a type and a node, if the node is one
    /// ([`is_node`]).
    pub const fn new(msg_type: MsgType, node: u8) -> Option<Self> {
        if is_node(node) {
            Some(CanId { msg_type, node })
        } else {
            None
        }
    }

    /// The type field.
    pub const fn msg_type(self) -> MsgType {
        self.msg_type
    }

    /// The node field: the plane a message is to or from, or the
    /// controller that sent it.
    pub const fn node(self) -> u8 {
        self.node
    }

    /// The 11 bits to put in the identifier register.
    pub const fn raw(self) -> u16 {
        ((self.msg_type.value() as u16) << NODE_BITS) | self.node as u16
    }

    /// The ADR 0003 address of the node, for the CRC-8 (decision 1).
    pub const fn addr(self) -> u8 {
        node_addr(self.node)
    }

    /// Decode a raw identifier off the bus.
    pub const fn decode(raw: u16) -> Result<Self, IdError> {
        if raw > MAX_ID {
            return Err(IdError::NotStandard);
        }
        let node = (raw & NODE_MASK) as u8;
        let Some(msg_type) = MsgType::from_value((raw >> NODE_BITS) as u8) else {
            return Err(IdError::ReservedType);
        };
        if !is_node(node) {
            return Err(IdError::ReservedNode);
        }
        Ok(CanId { msg_type, node })
    }

    /// True when the node field names a plane (0-7).
    pub const fn is_plane(self) -> bool {
        is_plane_node(self.node)
    }

    /// True when the node field names a controller (14 or 15).
    pub const fn is_controller(self) -> bool {
        is_controller_node(self.node)
    }
}

// --- Frame lengths ---------------------------------------------------

/// Every data length a CAN FD frame can have (decision 3).
pub const FD_LENGTHS: [usize; 16] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 12, 16, 20, 24, 32, 48, 64];

/// Largest CAN FD data length.
pub const MAX_FD_LEN: usize = 64;

/// The smallest valid CAN FD data length that holds `n` bytes. `n`
/// above [`MAX_FD_LEN`] has no frame, so it clamps to 64: a caller
/// never builds such a frame ([`segment`] cuts at 63 message bytes).
pub const fn fd_len(n: usize) -> usize {
    match n {
        0..=8 => n,
        9..=12 => 12,
        13..=16 => 16,
        17..=20 => 20,
        21..=24 => 24,
        25..=32 => 32,
        33..=48 => 48,
        _ => MAX_FD_LEN,
    }
}

// --- Segmentation (ADR 0005 decision 3) ------------------------------

/// Message bytes in one frame: 64 minus the index / last byte.
pub const SEGMENT_PAYLOAD: usize = MAX_FD_LEN - 1;

/// Longest message: a write frame of block, index, L = 254, records,
/// crc (ADR 0003), which is [`MAX_SEGMENTS`] frames.
pub const MAX_MESSAGE: usize = crate::frame::MAX_WRITE_FRAME;

/// Frames a message takes at most: `ceil(258 / 63)`.
pub const MAX_SEGMENTS: usize = 5;

/// Bit 7 of byte 0 of a frame: the last segment of the message.
pub const SEG_LAST: u8 = 0x80;

/// Bits 0-6 of byte 0 of a frame: the segment index.
pub const SEG_INDEX_MASK: u8 = 0x7F;

/// Why a message cannot be segmented.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SegmentError {
    /// An empty message. No ADR 0003 frame is empty (the shortest is a
    /// 2-byte read request), so this is a caller's bug, not a frame
    /// with only a header: a reader could not tell such a frame from a
    /// lost payload.
    Empty,
    /// Longer than [`MAX_MESSAGE`].
    TooLong,
}

/// How many frames a message of `msg_len` bytes takes.
pub const fn segment_count(msg_len: usize) -> usize {
    msg_len.div_ceil(SEGMENT_PAYLOAD)
}

/// One CAN FD frame: the data bytes and how many of them are on the
/// wire. The identifier is the caller's ([`CanId::raw`]): the same
/// frames go out under REQ, RSP or BULK.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FdFrame {
    /// The 64 data bytes, of which [`FdFrame::len`] are sent; the rest
    /// is the zero padding of decision 3.
    pub data: [u8; MAX_FD_LEN],
    /// Data length of the frame, one of [`FD_LENGTHS`].
    pub len: u8,
}

impl FdFrame {
    /// The bytes to send: byte 0 is the index / last byte, then the
    /// message bytes and the padding.
    pub fn bytes(&self) -> &[u8] {
        &self.data[..self.len as usize]
    }

    /// The segment index (0, 1, 2 ...).
    pub const fn index(&self) -> u8 {
        self.data[0] & SEG_INDEX_MASK
    }

    /// True on the last segment of its message.
    pub const fn is_last(&self) -> bool {
        self.data[0] & SEG_LAST != 0
    }

    /// The message bytes of this frame, padding included: a reader
    /// takes the message's length from its own content (ADR 0003 `L`).
    pub fn payload(&self) -> &[u8] {
        &self.data[1..self.len as usize]
    }
}

/// Cut a message into frames (decision 3): byte 0 is the segment
/// index, bit 7 set on the last one, then up to [`SEGMENT_PAYLOAD`]
/// message bytes, padded with 0x00 to the next [`fd_len`].
///
/// The iterator yields at least one and at most [`MAX_SEGMENTS`]
/// frames and borrows the message; it holds no copy of it.
pub const fn segment(msg: &[u8]) -> Result<Segments<'_>, SegmentError> {
    if msg.is_empty() {
        return Err(SegmentError::Empty);
    }
    if msg.len() > MAX_MESSAGE {
        return Err(SegmentError::TooLong);
    }
    Ok(Segments {
        msg,
        offset: 0,
        index: 0,
        done: false,
    })
}

/// The frames of one message, from [`segment`].
#[derive(Debug, Clone)]
pub struct Segments<'a> {
    msg: &'a [u8],
    offset: usize,
    index: u8,
    done: bool,
}

impl Iterator for Segments<'_> {
    type Item = FdFrame;

    fn next(&mut self) -> Option<FdFrame> {
        if self.done {
            return None;
        }
        let start = self.offset;
        let end = if self.msg.len() - start > SEGMENT_PAYLOAD {
            start + SEGMENT_PAYLOAD
        } else {
            self.msg.len()
        };
        let last = end == self.msg.len();

        let mut data = [0u8; MAX_FD_LEN];
        data[0] = self.index | if last { SEG_LAST } else { 0 };
        data[1..1 + (end - start)].copy_from_slice(&self.msg[start..end]);

        self.offset = end;
        self.index += 1;
        self.done = last;
        Some(FdFrame {
            data,
            len: fd_len(1 + end - start) as u8,
        })
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let left = if self.done {
            0
        } else {
            segment_count(self.msg.len() - self.offset)
        };
        (left, Some(left))
    }
}

impl ExactSizeIterator for Segments<'_> {}

// --- Reassembly (ADR 0005 decision 4) --------------------------------

/// Bytes a reassembly slot holds (decision 4: "fixed buffers of 320
/// bytes"). Five full segments are 315 bytes.
pub const REASSEMBLY_CAP: usize = 320;

/// Longest gap between two segments of one message, in milliseconds
/// (decision 4).
pub const SEGMENT_GAP_MS: u32 = 50;

/// Slots a [`Reassembler`] has unless the caller asks for more. Two is
/// what a plane needs: the transaction it is answering and a bulk
/// transfer beside it. A slot costs [`REASSEMBLY_CAP`] bytes, which on
/// the plane's 36 KB of RAM is why there is one slot per message in
/// flight and not one per (type, node) pair.
pub const DEFAULT_SLOTS: usize = 2;

/// Why a frame did not go into a message. Every one of these except
/// [`ReassemblyError::EmptyFrame`] and
/// [`ReassemblyError::FrameTooLong`] also drops the partial message of
/// its slot and counts it in [`ReassemblyCounters`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReassemblyError {
    /// A frame with no bytes at all: not even the index / last byte.
    EmptyFrame,
    /// More than [`MAX_FD_LEN`] bytes: not a CAN FD frame.
    FrameTooLong,
    /// The segment index is not the next one of its slot, or it is not
    /// 0 and no message is in flight for the identifier.
    OutOfOrder {
        /// The index the frame carried.
        got: u8,
        /// The index the slot was waiting for.
        expected: u8,
    },
    /// More than [`SEGMENT_GAP_MS`] since the slot's last segment.
    Timeout,
    /// A segment index of [`MAX_SEGMENTS`] or above: no ADR 0003
    /// message is that long.
    TooManySegments,
    /// The message would pass [`REASSEMBLY_CAP`] bytes.
    TooLong,
}

/// Partial messages dropped, by reason. Saturating: a counter that
/// reaches `u32::MAX` stays there.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ReassemblyCounters {
    /// Dropped because a segment did not follow the one before it
    /// ([`ReassemblyError::OutOfOrder`]).
    pub out_of_order: u32,
    /// Dropped because more than [`SEGMENT_GAP_MS`] passed
    /// ([`ReassemblyError::Timeout`]).
    pub timeout: u32,
    /// Dropped because the message grew past [`MAX_SEGMENTS`] or
    /// [`REASSEMBLY_CAP`].
    pub overflow: u32,
    /// Dropped because every slot was busy and this one was the least
    /// recently used.
    pub evicted: u32,
    /// Dropped because a segment 0 for the same identifier arrived
    /// while the slot still held a partial message.
    pub restarted: u32,
}

impl ReassemblyCounters {
    /// All partial messages dropped, whatever the reason.
    pub const fn total(&self) -> u32 {
        self.out_of_order
            .saturating_add(self.timeout)
            .saturating_add(self.overflow)
            .saturating_add(self.evicted)
            .saturating_add(self.restarted)
    }
}

/// A reassembled message: the identifier it came under and its bytes.
///
/// The bytes are every message byte of every segment, the zero padding
/// of the last frame included (decision 3): a caller takes the length
/// from the message's own content, which for an ADR 0003 frame is its
/// `L` byte, and [`crate::frame`] does exactly that.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Message<'a> {
    id: CanId,
    bytes: &'a [u8],
}

impl<'a> Message<'a> {
    /// The identifier the message came under.
    pub const fn id(&self) -> CanId {
        self.id
    }

    /// The type field of the identifier.
    pub const fn msg_type(&self) -> MsgType {
        self.id.msg_type()
    }

    /// The node field of the identifier: the plane the message is to
    /// or from.
    pub const fn node(&self) -> u8 {
        self.id.node()
    }

    /// The ADR 0003 address of that node, for the CRC-8.
    pub const fn addr(&self) -> u8 {
        self.id.addr()
    }

    /// The message bytes, padding of the last frame included.
    pub const fn bytes(&self) -> &'a [u8] {
        self.bytes
    }
}

/// One slot: the message in flight for one identifier.
#[derive(Debug)]
struct SlotState {
    /// The identifier, while a message is in flight.
    id: Option<CanId>,
    /// Bytes collected so far.
    len: usize,
    /// The index the next segment must carry.
    next: u8,
    /// When the last segment arrived.
    last_ms: u32,
    /// Use order: the highest `seq` is the slot used last.
    seq: u32,
    buf: [u8; REASSEMBLY_CAP],
}

impl SlotState {
    const fn new() -> Self {
        SlotState {
            id: None,
            len: 0,
            next: 0,
            last_ms: 0,
            seq: 0,
            buf: [0u8; REASSEMBLY_CAP],
        }
    }

    fn clear(&mut self) {
        self.id = None;
        self.len = 0;
        self.next = 0;
    }
}

/// Frames into messages (decision 4), with `N` messages in flight.
///
/// A slot is keyed by the whole identifier, so a transaction with one
/// plane and a bulk transfer with another run side by side. A segment 0
/// always starts a new message; a segment that is not the next one, or
/// that comes more than [`SEGMENT_GAP_MS`] after the one before it,
/// drops the partial message and counts it. When every slot is busy
/// and a new message starts, the least recently used slot is dropped
/// and counted: the alternative, a slot per (type, node) pair, is 80
/// buffers of 320 bytes, which the plane does not have.
///
/// The index / last byte carries no message identity, so a message
/// that loses its segment 0 continues whatever partial message the
/// slot still held, if its first surviving index is the one that slot
/// waits for and it comes within [`SEGMENT_GAP_MS`]. The transport
/// cannot see that; the ADR 0003 CRC over the spliced bytes is what
/// refuses it, which is why every message carries one.
///
/// ```
/// use granite_proto::can::{CanId, MsgType, Reassembler};
///
/// // Four messages in flight instead of the default two.
/// let mut rx: Reassembler<4> = Reassembler::new();
/// let id = CanId::new(MsgType::Req, 0).unwrap();
/// assert!(rx.push(id, &[0x80, 0x02, 0xFF], 0).unwrap().is_some());
/// ```
#[derive(Debug)]
pub struct Reassembler<const N: usize = DEFAULT_SLOTS> {
    slots: [SlotState; N],
    counters: ReassemblyCounters,
    clock: u32,
}

impl<const N: usize> Default for Reassembler<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> Reassembler<N> {
    /// An empty reassembler. `N` must be at least 1.
    pub const fn new() -> Self {
        assert!(N > 0, "a reassembler needs at least one slot");
        Reassembler {
            slots: [const { SlotState::new() }; N],
            counters: ReassemblyCounters {
                out_of_order: 0,
                timeout: 0,
                overflow: 0,
                evicted: 0,
                restarted: 0,
            },
            clock: 0,
        }
    }

    /// Partial messages dropped so far, by reason.
    pub const fn counters(&self) -> ReassemblyCounters {
        self.counters
    }

    /// Forget every message in flight, keeping the counters.
    pub fn clear(&mut self) {
        for slot in &mut self.slots {
            slot.clear();
        }
    }

    /// How many messages are in flight.
    pub fn in_flight(&self) -> usize {
        self.slots.iter().filter(|s| s.id.is_some()).count()
    }

    /// Feed one frame: its identifier, its data bytes (byte 0 is the
    /// index / last byte) and the time it arrived in milliseconds.
    ///
    /// Returns the message on the last segment of one, `Ok(None)` while
    /// a message is still coming, and an error when the frame dropped a
    /// partial message (see [`ReassemblyError`]). `now_ms` may wrap:
    /// only the difference to the slot's last segment is used.
    ///
    /// The bytes after byte 0 are appended as they are, however many
    /// there are: [`segment`] fills every frame but the last to 64
    /// bytes, and a sender that does not is not refused here but
    /// produces a message its ADR 0003 CRC fails on.
    pub fn push(
        &mut self,
        id: CanId,
        data: &[u8],
        now_ms: u32,
    ) -> Result<Option<Message<'_>>, ReassemblyError> {
        if data.is_empty() {
            return Err(ReassemblyError::EmptyFrame);
        }
        if data.len() > MAX_FD_LEN {
            return Err(ReassemblyError::FrameTooLong);
        }
        let index = data[0] & SEG_INDEX_MASK;
        let last = data[0] & SEG_LAST != 0;
        let payload = &data[1..];

        if index as usize >= MAX_SEGMENTS {
            if let Some(i) = self.find(id) {
                self.slots[i].clear();
                self.counters.overflow = self.counters.overflow.saturating_add(1);
            }
            return Err(ReassemblyError::TooManySegments);
        }

        let i = if index == 0 {
            self.start(id, now_ms)
        } else {
            match self.find(id) {
                Some(i) => i,
                None => {
                    return Err(ReassemblyError::OutOfOrder {
                        got: index,
                        expected: 0,
                    });
                }
            }
        };

        if index != 0 {
            let slot = &mut self.slots[i];
            if now_ms.wrapping_sub(slot.last_ms) > SEGMENT_GAP_MS {
                slot.clear();
                self.counters.timeout = self.counters.timeout.saturating_add(1);
                return Err(ReassemblyError::Timeout);
            }
            if index != slot.next {
                let expected = slot.next;
                slot.clear();
                self.counters.out_of_order = self.counters.out_of_order.saturating_add(1);
                return Err(ReassemblyError::OutOfOrder {
                    got: index,
                    expected,
                });
            }
        }

        let slot = &mut self.slots[i];
        if slot.len + payload.len() > REASSEMBLY_CAP {
            slot.clear();
            self.counters.overflow = self.counters.overflow.saturating_add(1);
            return Err(ReassemblyError::TooLong);
        }
        slot.buf[slot.len..slot.len + payload.len()].copy_from_slice(payload);
        slot.len += payload.len();
        slot.next = index + 1;
        slot.last_ms = now_ms;

        if !last {
            return Ok(None);
        }
        let len = slot.len;
        slot.clear();
        Ok(Some(Message {
            id,
            bytes: &self.slots[i].buf[..len],
        }))
    }

    /// The slot holding `id`, if a message of it is in flight.
    fn find(&self, id: CanId) -> Option<usize> {
        self.slots.iter().position(|s| s.id == Some(id))
    }

    /// The slot to start a message of `id` in: its own if it has one
    /// (dropping what is in it), else a free one, else the least
    /// recently used one (dropping what is in it).
    fn start(&mut self, id: CanId, now_ms: u32) -> usize {
        let i = if let Some(i) = self.find(id) {
            // The slot still held a partial message of this very
            // identifier: the sender restarted it.
            self.counters.restarted = self.counters.restarted.saturating_add(1);
            i
        } else if let Some(i) = self.slots.iter().position(|s| s.id.is_none()) {
            i
        } else {
            let i = self.least_recently_used();
            self.counters.evicted = self.counters.evicted.saturating_add(1);
            i
        };
        self.clock = self.clock.wrapping_add(1);
        let clock = self.clock;
        let slot = &mut self.slots[i];
        slot.id = Some(id);
        slot.len = 0;
        slot.next = 0;
        slot.last_ms = now_ms;
        slot.seq = clock;
        i
    }

    /// The busy slot used longest ago. Use order is a counter of
    /// started messages, not the clock the caller passes, so a caller
    /// with no clock at all still evicts in order; after 2^32 messages
    /// the counter wraps and one eviction picks the wrong slot.
    fn least_recently_used(&self) -> usize {
        let mut best = 0;
        for (i, slot) in self.slots.iter().enumerate() {
            if slot.seq < self.slots[best].seq {
                best = i;
            }
        }
        best
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BLK_CTRL, BLK_INFO, BLK_STATUS, BLK_SUMMARY, INDEX_NONE};
    use std::vec::Vec;

    /// A message of `len` bytes whose content depends on `len`, so a
    /// test that mixes two of them notices a mix-up.
    fn message(len: usize, salt: u8) -> Vec<u8> {
        (0..len)
            .map(|i| (i as u8).wrapping_mul(7).wrapping_add(salt))
            .collect()
    }

    /// Push every frame of `msg` under `id` and return what came out.
    fn round_trip(rx: &mut Reassembler, id: CanId, msg: &[u8], now_ms: u32) -> Vec<u8> {
        let frames: Vec<FdFrame> = segment(msg).unwrap().collect();
        let mut out = None;
        for (i, frame) in frames.iter().enumerate() {
            let got = rx.push(id, frame.bytes(), now_ms).unwrap();
            assert_eq!(got.is_some(), i + 1 == frames.len(), "finished too early");
            if let Some(m) = got {
                assert_eq!(m.id(), id);
                out = Some(m.bytes().to_vec());
            }
        }
        out.unwrap()
    }

    /// The reassembled bytes are the message plus the last frame's
    /// zero padding.
    fn assert_same_message(msg: &[u8], got: &[u8]) {
        assert!(got.len() >= msg.len(), "{} < {}", got.len(), msg.len());
        assert_eq!(&got[..msg.len()], msg);
        assert!(got[msg.len()..].iter().all(|b| *b == 0), "padding not zero");
    }

    // --- Identifiers -------------------------------------------------

    #[test]
    fn id_round_trip_and_priority() {
        let types = [
            MsgType::Sync,
            MsgType::Event,
            MsgType::Req,
            MsgType::Rsp,
            MsgType::Bulk,
        ];
        for t in types {
            for node in [0u8, 1, 7, NODE_CONTROLLER, NODE_STANDBY] {
                let id = CanId::new(t, node).unwrap();
                assert_eq!(id.raw(), ((t.value() as u16) << 4) | node as u16);
                assert_eq!(CanId::decode(id.raw()), Ok(id));
                assert_eq!(id.addr(), 0x30 + node);
                assert_eq!(node_of_addr(id.addr()), Some(node));
            }
        }
        // Lower identifier wins arbitration: SYNC before EVENT before
        // REQ before RSP before BULK.
        assert!(
            CanId::new(MsgType::Sync, NODE_CONTROLLER).unwrap().raw()
                < CanId::new(MsgType::Event, 0).unwrap().raw()
        );
        assert!(
            CanId::new(MsgType::Rsp, 7).unwrap().raw()
                < CanId::new(MsgType::Bulk, 0).unwrap().raw()
        );
    }

    #[test]
    fn reserved_ids_are_rejected() {
        for node in 8u8..=13 {
            assert_eq!(CanId::new(MsgType::Req, node), None);
            assert_eq!(
                CanId::decode((2 << 4) | node as u16),
                Err(IdError::ReservedNode)
            );
        }
        assert_eq!(CanId::new(MsgType::Req, 16), None);
        for t in 5u16..=15 {
            assert_eq!(CanId::decode((t << 4) | 3), Err(IdError::ReservedType));
        }
        assert_eq!(CanId::decode(0x0800), Err(IdError::NotStandard));
        assert_eq!(CanId::decode(u16::MAX), Err(IdError::NotStandard));
        assert_eq!(MsgType::try_from(5), Err(crate::UnknownValue(5)));
    }

    /// Every 11-bit identifier either decodes back to itself or is
    /// refused; nothing panics.
    #[test]
    fn every_eleven_bit_id() {
        let mut ours = 0;
        for raw in 0..=MAX_ID {
            if let Ok(id) = CanId::decode(raw) {
                assert_eq!(id.raw(), raw);
                ours += 1;
            }
        }
        // 5 types x 10 nodes.
        assert_eq!(ours, 50);
    }

    #[test]
    fn nodes_and_addresses() {
        assert!(is_plane_node(0) && is_plane_node(7) && !is_plane_node(8));
        assert!(is_controller_node(NODE_CONTROLLER) && is_controller_node(NODE_STANDBY));
        assert!(!is_node(8) && !is_node(13) && is_node(14));
        assert_eq!(node_addr(0), crate::ADDR_BASE);
        assert_eq!(node_addr(7), 0x37);
        assert_eq!(node_addr(NODE_CONTROLLER), 0x3E);
        assert_eq!(node_addr(NODE_STANDBY), 0x3F);
        // The plane addresses are ADR 0003's.
        for n in 0..8 {
            assert_eq!(Some(node_addr(n)), crate::plane_address(n));
        }
        assert_eq!(node_of_addr(0x2F), None);
        assert_eq!(node_of_addr(0x40), None);
        for n in 0..NODE_COUNT as u8 {
            assert_eq!(node_of_addr(node_addr(n)), Some(n));
        }
    }

    #[test]
    fn blocks_map_to_types() {
        assert_eq!(msg_type_for_block(BLK_FW, true), MsgType::Bulk);
        assert_eq!(msg_type_for_block(BLK_FW, false), MsgType::Bulk);
        assert_eq!(msg_type_for_block(BLK_LOG, true), MsgType::Bulk);
        assert_eq!(msg_type_for_block(BLK_LOG, false), MsgType::Bulk);
        for block in [BLK_INFO, BLK_STATUS, BLK_SUMMARY, BLK_CTRL] {
            assert_eq!(msg_type_for_block(block, true), MsgType::Req);
            assert_eq!(msg_type_for_block(block, false), MsgType::Rsp);
        }
    }

    // --- Lengths -----------------------------------------------------

    #[test]
    fn fd_lengths() {
        for n in 0..=8 {
            assert_eq!(fd_len(n), n);
        }
        assert_eq!(fd_len(9), 12);
        assert_eq!(fd_len(12), 12);
        assert_eq!(fd_len(13), 16);
        assert_eq!(fd_len(17), 20);
        assert_eq!(fd_len(21), 24);
        assert_eq!(fd_len(25), 32);
        assert_eq!(fd_len(33), 48);
        assert_eq!(fd_len(49), 64);
        assert_eq!(fd_len(64), 64);
        assert_eq!(fd_len(65), MAX_FD_LEN);
        assert_eq!(fd_len(usize::MAX), MAX_FD_LEN);
        // Every result is a valid CAN FD length and holds the bytes.
        for n in 0..=MAX_FD_LEN {
            let l = fd_len(n);
            assert!(FD_LENGTHS.contains(&l), "{l} is not a CAN FD length");
            assert!(l >= n);
        }
    }

    // --- Segmentation ------------------------------------------------

    #[test]
    fn segment_header_and_padding() {
        // One frame: header plus 5 bytes is 6, already a valid length.
        let frames: Vec<FdFrame> = segment(&[1, 2, 3, 4, 5]).unwrap().collect();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].bytes(), &[0x80, 1, 2, 3, 4, 5]);
        assert!(frames[0].is_last() && frames[0].index() == 0);

        // Ten bytes: 11 on the wire, padded to 12.
        let frames: Vec<FdFrame> = segment(&message(10, 1)).unwrap().collect();
        assert_eq!(frames[0].len, 12);
        assert_eq!(&frames[0].bytes()[11..], &[0]);

        // Two frames: 64 message bytes.
        let msg = message(64, 2);
        let frames: Vec<FdFrame> = segment(&msg).unwrap().collect();
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].len, 64);
        assert_eq!((frames[0].index(), frames[0].is_last()), (0, false));
        assert_eq!(frames[0].payload(), &msg[..63]);
        assert_eq!((frames[1].index(), frames[1].is_last()), (1, true));
        assert_eq!(frames[1].bytes(), &[0x81, msg[63]]);
    }

    #[test]
    fn longest_message_takes_five_frames() {
        assert_eq!(MAX_MESSAGE, 258);
        let msg = message(MAX_MESSAGE, 3);
        let frames: Vec<FdFrame> = segment(&msg).unwrap().collect();
        assert_eq!(frames.len(), MAX_SEGMENTS);
        assert_eq!(segment_count(MAX_MESSAGE), MAX_SEGMENTS);
        assert!(frames[MAX_SEGMENTS - 1].is_last());
        // 63 * 4 + 6: this message needs no padding at all.
        assert_eq!(
            frames.iter().map(|f| f.len as usize - 1).sum::<usize>(),
            MAX_MESSAGE
        );
    }

    #[test]
    fn empty_and_oversized_messages_are_refused() {
        assert_eq!(segment(&[]).unwrap_err(), SegmentError::Empty);
        let long = message(MAX_MESSAGE + 1, 0);
        assert_eq!(segment(&long).unwrap_err(), SegmentError::TooLong);
        assert!(segment(&long[..MAX_MESSAGE]).is_ok());
    }

    #[test]
    fn segments_report_their_count() {
        for len in 1..=MAX_MESSAGE {
            let msg = message(len, 0);
            let it = segment(&msg).unwrap();
            assert_eq!(it.len(), segment_count(len));
            assert_eq!(it.count(), segment_count(len));
        }
    }

    // --- Reassembly --------------------------------------------------

    #[test]
    fn round_trip_every_length() {
        let mut rx: Reassembler = Reassembler::new();
        let id = CanId::new(MsgType::Req, 2).unwrap();
        for len in 1..=MAX_MESSAGE {
            let msg = message(len, len as u8);
            let got = round_trip(&mut rx, id, &msg, len as u32);
            assert_same_message(&msg, &got);
            assert_eq!(rx.in_flight(), 0);
        }
        assert_eq!(rx.counters(), ReassemblyCounters::default());
    }

    #[test]
    fn round_trip_every_type_and_node() {
        let mut rx: Reassembler = Reassembler::new();
        for t in [
            MsgType::Sync,
            MsgType::Event,
            MsgType::Req,
            MsgType::Rsp,
            MsgType::Bulk,
        ] {
            for node in [0u8, 7, NODE_CONTROLLER, NODE_STANDBY] {
                let id = CanId::new(t, node).unwrap();
                let msg = message(100, node);
                let got = round_trip(&mut rx, id, &msg, 0);
                assert_same_message(&msg, &got);
            }
        }
        assert_eq!(rx.counters().total(), 0);
    }

    /// An ADR 0003 answer survives the transport: the CRC still checks
    /// against `0x30 + node` (decision 1), padding and all.
    #[test]
    fn adr_0003_answer_through_the_transport() {
        let node = 5u8;
        let request = crate::frame::read_request(BLK_STATUS, INDEX_NONE);
        let mut out = [0u8; crate::frame::MAX_ANSWER];
        let answer = crate::frame::answer_frame(
            &mut out,
            node_addr(node),
            BLK_STATUS,
            INDEX_NONE,
            &[0x12, 4, 0x10, 0x0E, 0, 0],
        )
        .unwrap()
        .to_vec();

        let mut rx: Reassembler = Reassembler::new();
        let id = CanId::new(msg_type_for_block(BLK_STATUS, false), node).unwrap();
        let got = round_trip(&mut rx, id, &answer, 0);
        let payload = crate::frame::check_answer(id.addr(), &request, &got).unwrap();
        assert_eq!(payload, &[0x12, 4, 0x10, 0x0E, 0, 0]);
    }

    #[test]
    fn two_nodes_interleaved() {
        let mut rx: Reassembler = Reassembler::new();
        let a = CanId::new(MsgType::Rsp, 1).unwrap();
        let b = CanId::new(MsgType::Rsp, 6).unwrap();
        let ma = message(200, 0xA0);
        let mb = message(130, 0x0B);
        let fa: Vec<FdFrame> = segment(&ma).unwrap().collect();
        let fb: Vec<FdFrame> = segment(&mb).unwrap().collect();
        assert_eq!((fa.len(), fb.len()), (4, 3));

        let mut out_a = None;
        let mut out_b = None;
        for (i, frame_a) in fa.iter().enumerate() {
            if let Some(m) = rx.push(a, frame_a.bytes(), i as u32).unwrap() {
                out_a = Some(m.bytes().to_vec());
            }
            if let Some(frame_b) = fb.get(i)
                && let Some(m) = rx.push(b, frame_b.bytes(), i as u32).unwrap()
            {
                out_b = Some(m.bytes().to_vec());
            }
        }
        assert_same_message(&ma, &out_a.unwrap());
        assert_same_message(&mb, &out_b.unwrap());
        assert_eq!(rx.counters().total(), 0);
    }

    #[test]
    fn a_missing_segment_drops_the_message() {
        let mut rx: Reassembler = Reassembler::new();
        let id = CanId::new(MsgType::Bulk, 0).unwrap();
        let msg = message(200, 1);
        let frames: Vec<FdFrame> = segment(&msg).unwrap().collect();

        assert_eq!(rx.push(id, frames[0].bytes(), 0), Ok(None));
        // Segment 1 is lost: 2 arrives instead.
        assert_eq!(
            rx.push(id, frames[2].bytes(), 1),
            Err(ReassemblyError::OutOfOrder {
                got: 2,
                expected: 1
            })
        );
        assert_eq!(rx.counters().out_of_order, 1);
        assert_eq!(rx.in_flight(), 0);
        // The rest of the message finds no slot.
        assert_eq!(
            rx.push(id, frames[3].bytes(), 2),
            Err(ReassemblyError::OutOfOrder {
                got: 3,
                expected: 0
            })
        );
        // Still one drop: there was nothing left to drop.
        assert_eq!(rx.counters().out_of_order, 1);

        // A fresh send goes through.
        let got = round_trip(&mut rx, id, &msg, 3);
        assert_same_message(&msg, &got);
    }

    #[test]
    fn a_duplicated_segment_drops_the_message() {
        let mut rx: Reassembler = Reassembler::new();
        let id = CanId::new(MsgType::Req, 4).unwrap();
        let msg = message(150, 2);
        let frames: Vec<FdFrame> = segment(&msg).unwrap().collect();

        assert_eq!(rx.push(id, frames[0].bytes(), 0), Ok(None));
        assert_eq!(rx.push(id, frames[1].bytes(), 1), Ok(None));
        assert_eq!(
            rx.push(id, frames[1].bytes(), 2),
            Err(ReassemblyError::OutOfOrder {
                got: 1,
                expected: 2
            })
        );
        assert_eq!(rx.counters().out_of_order, 1);

        // A duplicated segment 0 restarts instead of erroring.
        assert_eq!(rx.push(id, frames[0].bytes(), 3), Ok(None));
        assert_eq!(rx.push(id, frames[0].bytes(), 4), Ok(None));
        assert_eq!(rx.counters().restarted, 1);
        assert_eq!(rx.in_flight(), 1);
    }

    #[test]
    fn a_late_segment_times_out() {
        let mut rx: Reassembler = Reassembler::new();
        let id = CanId::new(MsgType::Req, 3).unwrap();
        let msg = message(130, 3);
        let frames: Vec<FdFrame> = segment(&msg).unwrap().collect();

        assert_eq!(rx.push(id, frames[0].bytes(), 1_000), Ok(None));
        // Exactly 50 ms is still in time.
        assert_eq!(rx.push(id, frames[1].bytes(), 1_050), Ok(None));
        // 51 ms is not.
        assert_eq!(
            rx.push(id, frames[2].bytes(), 1_101),
            Err(ReassemblyError::Timeout)
        );
        assert_eq!(rx.counters().timeout, 1);
        assert_eq!(rx.in_flight(), 0);

        // The clock may wrap between two segments.
        let late = u32::MAX - 10;
        assert_eq!(rx.push(id, frames[0].bytes(), late), Ok(None));
        assert_eq!(
            rx.push(id, frames[1].bytes(), late.wrapping_add(20)),
            Ok(None)
        );
        assert_eq!(
            rx.push(id, frames[2].bytes(), late.wrapping_add(80)),
            Err(ReassemblyError::Timeout)
        );
    }

    #[test]
    fn too_many_segments_and_too_many_bytes() {
        let mut rx: Reassembler = Reassembler::new();
        let id = CanId::new(MsgType::Bulk, 1).unwrap();
        // Five full segments is the most a message takes; a sixth is
        // refused whatever it holds.
        let mut data = [0u8; MAX_FD_LEN];
        for index in 0..MAX_SEGMENTS as u8 {
            data[0] = index;
            assert_eq!(rx.push(id, &data, 0), Ok(None));
        }
        data[0] = MAX_SEGMENTS as u8;
        assert_eq!(rx.push(id, &data, 0), Err(ReassemblyError::TooManySegments));
        assert_eq!(rx.counters().overflow, 1);
        assert_eq!(rx.in_flight(), 0);
        // And the index field cannot name a sixth segment either.
        data[0] = SEG_INDEX_MASK | SEG_LAST;
        assert_eq!(rx.push(id, &data, 0), Err(ReassemblyError::TooManySegments));
    }

    #[test]
    fn malformed_frames_are_refused() {
        let mut rx: Reassembler = Reassembler::new();
        let id = CanId::new(MsgType::Event, 2).unwrap();
        assert_eq!(rx.push(id, &[], 0), Err(ReassemblyError::EmptyFrame));
        assert_eq!(
            rx.push(id, &[0u8; MAX_FD_LEN + 1], 0),
            Err(ReassemblyError::FrameTooLong)
        );
        // Neither touched a slot or a counter.
        assert_eq!(rx.in_flight(), 0);
        assert_eq!(rx.counters().total(), 0);
        // A header-only frame is a message of no bytes: the caller's
        // ADR 0003 check refuses it, the transport does not.
        assert_eq!(
            rx.push(id, &[SEG_LAST], 0)
                .unwrap()
                .map(|m| m.bytes().len()),
            Some(0)
        );
    }

    #[test]
    fn a_third_message_evicts_the_oldest() {
        let mut rx: Reassembler = Reassembler::new();
        let a = CanId::new(MsgType::Req, 0).unwrap();
        let b = CanId::new(MsgType::Req, 1).unwrap();
        let c = CanId::new(MsgType::Req, 2).unwrap();
        let msg = message(130, 7);
        let frames: Vec<FdFrame> = segment(&msg).unwrap().collect();

        assert_eq!(rx.push(a, frames[0].bytes(), 0), Ok(None));
        assert_eq!(rx.push(b, frames[0].bytes(), 1), Ok(None));
        // Both slots are busy: a's is the older one and goes.
        assert_eq!(rx.push(c, frames[0].bytes(), 2), Ok(None));
        assert_eq!(rx.counters().evicted, 1);
        assert_eq!(rx.in_flight(), 2);
        assert_eq!(
            rx.push(a, frames[1].bytes(), 3),
            Err(ReassemblyError::OutOfOrder {
                got: 1,
                expected: 0
            })
        );
        // b and c still finish.
        for id in [b, c] {
            assert_eq!(rx.push(id, frames[1].bytes(), 4), Ok(None));
            let out = rx.push(id, frames[2].bytes(), 5).unwrap().unwrap();
            assert_eq!(out.node(), id.node());
            assert_same_message(&msg, out.bytes());
        }

        // With a slot per message in flight there is no eviction.
        let mut wide: Reassembler<3> = Reassembler::new();
        for id in [a, b, c] {
            assert_eq!(wide.push(id, frames[0].bytes(), 0), Ok(None));
        }
        assert_eq!(wide.counters().evicted, 0);
        assert_eq!(wide.in_flight(), 3);
        wide.clear();
        assert_eq!(wide.in_flight(), 0);
        assert_eq!(wide.counters().total(), 0);
    }

    /// The index / last byte alone cannot tell a continuation from a
    /// splice: a message whose segment 0 is lost continues the partial
    /// message that was already in the slot, if its first surviving
    /// index happens to be the one that slot waits for and it comes
    /// within [`SEGMENT_GAP_MS`]. The ADR 0003 CRC over the spliced
    /// bytes is what refuses it; the transport counts nothing.
    #[test]
    fn a_lost_first_segment_can_splice_onto_a_partial() {
        let mut rx: Reassembler = Reassembler::new();
        let id = CanId::new(MsgType::Rsp, 0).unwrap();
        let first = message(130, 0x11);
        let second = message(130, 0x22);
        let f1: Vec<FdFrame> = segment(&first).unwrap().collect();
        let f2: Vec<FdFrame> = segment(&second).unwrap().collect();

        // The first message loses its tail after segment 0.
        assert_eq!(rx.push(id, f1[0].bytes(), 0), Ok(None));
        // The second loses segment 0: its 1 and 2 land in that slot.
        assert_eq!(rx.push(id, f2[1].bytes(), 1), Ok(None));
        let spliced = rx.push(id, f2[2].bytes(), 2).unwrap().unwrap();
        assert_eq!(&spliced.bytes()[..63], &first[..63]);
        assert_eq!(&spliced.bytes()[63..126], &second[63..126]);
        assert_eq!(rx.counters().total(), 0);

        // 51 ms later the same frames are refused instead.
        assert_eq!(rx.push(id, f1[0].bytes(), 100), Ok(None));
        assert_eq!(
            rx.push(id, f2[1].bytes(), 151),
            Err(ReassemblyError::Timeout)
        );
        assert_eq!(rx.counters().timeout, 1);
    }

    /// The same node with two types at once is two messages, not one.
    #[test]
    fn type_is_part_of_the_key() {
        let mut rx: Reassembler = Reassembler::new();
        let node = 3;
        let req = CanId::new(MsgType::Req, node).unwrap();
        let bulk = CanId::new(MsgType::Bulk, node).unwrap();
        let m1 = message(70, 0x11);
        let m2 = message(70, 0x22);
        let f1: Vec<FdFrame> = segment(&m1).unwrap().collect();
        let f2: Vec<FdFrame> = segment(&m2).unwrap().collect();

        assert_eq!(rx.push(req, f1[0].bytes(), 0), Ok(None));
        assert_eq!(rx.push(bulk, f2[0].bytes(), 0), Ok(None));
        let got1 = rx.push(req, f1[1].bytes(), 1).unwrap().unwrap();
        assert_same_message(&m1, got1.bytes());
        let got2 = rx.push(bulk, f2[1].bytes(), 1).unwrap().unwrap();
        assert_same_message(&m2, got2.bytes());
        assert_eq!(rx.counters().total(), 0);
    }

    // --- Fuzz --------------------------------------------------------

    /// A deterministic xorshift: the fuzz test below must run the same
    /// way on every machine and must not pull in a crate.
    struct Rng(u32);

    impl Rng {
        fn next(&mut self) -> u32 {
            let mut x = self.0;
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            self.0 = x;
            x
        }

        fn below(&mut self, n: u32) -> u32 {
            self.next() % n
        }
    }

    /// No frame sequence makes the reassembler panic, and whatever it
    /// returns stays inside its own rules: at most [`REASSEMBLY_CAP`]
    /// bytes, never more slots in flight than it has.
    #[test]
    fn fuzz_no_panic_on_any_frame_sequence() {
        let mut rng = Rng(0x1234_5678);
        let mut rx: Reassembler<2> = Reassembler::new();
        let mut now = 0u32;
        let mut messages = 0u32;

        for _ in 0..200_000 {
            // Sometimes a legal identifier, sometimes any 16 bits.
            let id = match CanId::decode(rng.below(0x1_0000) as u16) {
                Ok(id) => id,
                Err(_) => CanId::new(
                    MsgType::from_value(rng.below(5) as u8).unwrap(),
                    match rng.below(3) {
                        0 => rng.below(8) as u8,
                        1 => NODE_CONTROLLER,
                        _ => NODE_STANDBY,
                    },
                )
                .unwrap(),
            };

            let mut data = [0u8; MAX_FD_LEN + 4];
            let len = rng.below(data.len() as u32 + 1) as usize;
            for byte in &mut data[..len] {
                *byte = rng.next() as u8;
            }
            now = now.wrapping_add(rng.below(200));

            // Anything may come back; nothing may panic.
            if let Ok(Some(m)) = rx.push(id, &data[..len], now) {
                assert!(m.bytes().len() <= REASSEMBLY_CAP);
                assert_eq!(m.id(), id);
                assert!(is_node(m.node()));
                messages += 1;
            }
            assert!(rx.in_flight() <= 2);
        }

        // The run must have exercised both outcomes.
        assert!(messages > 0, "no message ever completed");
        assert!(rx.counters().total() > 0, "nothing was ever dropped");
    }

    /// Whole messages through random frame loss, reordering and
    /// duplication: every message either comes out byte for byte or
    /// does not come out at all, and a lost one never corrupts the
    /// next.
    #[test]
    fn fuzz_message_streams_survive_loss_and_reorder() {
        let mut rng = Rng(0x0BAD_C0DE);
        let mut rx: Reassembler<2> = Reassembler::new();
        let mut now = 0u32;
        let mut delivered = 0u32;

        for round in 0..20_000u32 {
            // A poll is far apart from the one before it, so a partial
            // message left over from the round before times out
            // instead of swallowing this round's segments (see
            // `a_lost_first_segment_can_splice_onto_a_partial`).
            now = now.wrapping_add(SEGMENT_GAP_MS + 1);
            let node = rng.below(8) as u8;
            let id =
                CanId::new(MsgType::from_value(2 + rng.below(3) as u8).unwrap(), node).unwrap();
            let len = 1 + rng.below(MAX_MESSAGE as u32) as usize;
            let msg = message(len, round as u8);
            let mut frames: Vec<FdFrame> = segment(&msg).unwrap().collect();

            // Drop, duplicate or swap a frame now and then.
            match rng.below(8) {
                0 if frames.len() > 1 => {
                    let i = rng.below(frames.len() as u32) as usize;
                    frames.remove(i);
                }
                1 => {
                    let i = rng.below(frames.len() as u32) as usize;
                    let f = frames[i];
                    frames.insert(i, f);
                }
                2 if frames.len() > 1 => {
                    let i = rng.below(frames.len() as u32 - 1) as usize;
                    frames.swap(i, i + 1);
                }
                _ => {}
            }

            for frame in &frames {
                now = now.wrapping_add(rng.below(3));
                if let Ok(Some(m)) = rx.push(id, frame.bytes(), now) {
                    // A message that completes is the one that was
                    // sent, or nothing: a partial message never
                    // becomes a short one.
                    assert_eq!(m.id(), id);
                    assert_same_message(&msg, m.bytes());
                    delivered += 1;
                }
            }
        }

        assert!(delivered > 8_000, "too few messages got through");
        assert!(rx.counters().total() > 0, "no loss was ever noticed");
    }
}
