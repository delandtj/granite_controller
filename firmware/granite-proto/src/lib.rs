//! Wire format of the controller-to-plane link.
//!
//! See `firmware/docs/adr/0003-plane-link-protocol.md`: that ADR is the
//! authority for the framing, the block ids and every tag. This crate
//! is the only place either end encodes or decodes them, so the two
//! cannot drift: the plane firmware (plane-logic, ADR 0002), the
//! controller firmware (granite-core, ADR 0004) and the simulator all
//! use it.
//!
//! - [`crc`]: CRC-8 (SMBus PEC) over the address and the frame.
//! - [`frame`]: build a request, build a write, check and split an
//!   answer; the plane's mirror of all three.
//! - [`record`]: the TLV layer, [`RecordWriter`] and [`Records`].
//! - [`tags`]: block ids, every tag number, the wire enums.
//! - [`bits`]: the status words as bitflag newtypes.
//! - [`info`], [`status`], [`slot`], [`unit`], [`events`], [`ctrl`],
//!   [`config`], [`fw`], [`log`]: the typed records of each block, one
//!   enum per block plus a struct per multi-field record. The plane
//!   encodes them from its model, the controller decodes them into its
//!   own.
//!
//! Nothing here allocates, nothing here panics on any input and
//! nothing here touches I2C, timing or state: a frame goes in, records
//! come out.
//!
//! ```
//! use granite_proto::*;
//!
//! // Controller: ask plane 0x33 for STATUS.
//! let request = frame::read_request(BLK_STATUS, INDEX_NONE);
//!
//! // Plane: answer it.
//! let mut payload = [0u8; 64];
//! let mut w = RecordWriter::new(&mut payload);
//! Status::Uptime(3600).encode(&mut w).unwrap();
//! Status::Flags(StatusFlags::EVENTS_PENDING).encode(&mut w).unwrap();
//! let mut out = [0u8; frame::MAX_ANSWER];
//! let answer =
//!     frame::answer_frame(&mut out, 0x33, BLK_STATUS, INDEX_NONE, w.payload()).unwrap();
//!
//! // Controller: check the CRC against the address and the request.
//! let payload = frame::check_answer(0x33, &request, answer).unwrap();
//! for record in Status::records(payload) {
//!     if let Ok(Status::Uptime(s)) = record {
//!         assert_eq!(s, 3600);
//!     }
//! }
//! ```

#![no_std]
#![forbid(unsafe_code)]
#![warn(missing_docs)]

#[cfg(test)]
extern crate std;

pub mod bits;
pub mod config;
pub mod crc;
pub mod ctrl;
pub mod events;
pub mod frame;
pub mod fw;
pub mod info;
pub mod log;
pub mod record;
pub mod slot;
pub mod status;
pub mod tags;
pub mod unit;

pub use bits::{Capabilities, CrossCheck, IndexBitmap, SlotStatus, StatusFlags, UnitStatus};
pub use config::{BlinkDetection, Config, PowerLimit, RetryPolicy};
pub use crc::{crc8, crc8_update};
pub use ctrl::{Command, CtrlWrite};
pub use events::{AckThrough, Event, EventRecord, EventsLost};
pub use frame::{Frame, FrameError};
pub use fw::{Fw, FwBegin, FwData};
pub use info::{Info, Version};
pub use log::LogLine;
pub use record::{MAX_PAYLOAD, MAX_VALUE, RecordError, RecordWriter, Records};
pub use slot::{Energy, NodeLed, PowerMinMax, Slot};
pub use status::{CommandResult, LinkErrors, SegmentErrors, SlotSummary, Status, UpdateProgress};
pub use tags::*;
pub use unit::Unit;

/// Protocol format version this crate speaks (INFO 0x01).
pub const FORMAT_VERSION: u8 = 1;

/// First I2C address a plane answers on: 0x30, plus its three straps.
pub const ADDR_BASE: u8 = 0x30;

/// Planes on one link: 0x30 to 0x37.
pub const MAX_PLANES: usize = 8;

/// Slots on a plane: 2 per unit, 4 units.
pub const SLOT_COUNT: usize = 8;

/// Unit boards on a chained plane (ADR 0005).
pub const UNIT_COUNT: usize = 4;

/// Channels on the plane's segment mux (STATUS 0x1A).
pub const SEGMENT_CHANNELS: usize = 4;

/// How many command sequence numbers the plane remembers. The
/// controller keeps at most 4 commands in flight per plane, well under
/// this.
pub const SEQ_MEMORY: usize = 8;

/// The I2C address of plane `n` (0-7), if `n` is a plane.
pub const fn plane_address(n: u8) -> Option<u8> {
    if (n as usize) < MAX_PLANES {
        Some(ADDR_BASE + n)
    } else {
        None
    }
}

/// The plane number of an I2C address, if the address is a plane's.
pub const fn plane_of_address(addr: u8) -> Option<u8> {
    if addr >= ADDR_BASE && ((addr - ADDR_BASE) as usize) < MAX_PLANES {
        Some(addr - ADDR_BASE)
    } else {
        None
    }
}

/// The wire slot index of unit `unit` (0-3), half `b` (false = A, true
/// = B): `2 * unit + (A = 0, B = 1)`.
pub const fn slot_of(unit: u8, b: bool) -> Option<u8> {
    if (unit as usize) < UNIT_COUNT {
        Some(2 * unit + b as u8)
    } else {
        None
    }
}

/// The unit a wire slot index belongs to, if the index is a slot's.
pub const fn unit_of_slot(slot: u8) -> Option<u8> {
    if (slot as usize) < SLOT_COUNT {
        Some(slot / 2)
    } else {
        None
    }
}

/// True when a wire slot index is the B half of its unit.
pub const fn slot_is_b(slot: u8) -> bool {
    slot % 2 == 1
}

/// Check a slot number for a record of `tag`.
pub(crate) const fn check_slot(tag: u8, slot: u8) -> Result<(), RecordError> {
    if (slot as usize) < SLOT_COUNT {
        Ok(())
    } else {
        Err(RecordError::BadValue { tag })
    }
}

/// Check a unit number for a record of `tag`.
pub(crate) const fn check_unit(tag: u8, unit: u8) -> Result<(), RecordError> {
    if (unit as usize) < UNIT_COUNT {
        Ok(())
    } else {
        Err(RecordError::BadValue { tag })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_and_slots() {
        assert_eq!(plane_address(0), Some(0x30));
        assert_eq!(plane_address(7), Some(0x37));
        assert_eq!(plane_address(8), None);
        assert_eq!(plane_of_address(0x33), Some(3));
        assert_eq!(plane_of_address(0x38), None);
        assert_eq!(plane_of_address(0x2F), None);

        assert_eq!(slot_of(0, false), Some(0));
        assert_eq!(slot_of(0, true), Some(1));
        assert_eq!(slot_of(3, true), Some(7));
        assert_eq!(slot_of(4, false), None);
        assert_eq!(unit_of_slot(7), Some(3));
        assert_eq!(unit_of_slot(8), None);
        assert!(slot_is_b(5) && !slot_is_b(4));
    }
}
