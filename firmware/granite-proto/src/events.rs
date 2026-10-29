//! EVENTS block records (0x60, 0x61) and the EV_ACK record (0x62).
//!
//! Reading EVENTS clears nothing: the controller acknowledges with
//! EV_ACK once it has processed them, and EXP_INT stays asserted while
//! unacknowledged events exist (ADR 0003 decision 3, "Events cleared on
//! read" rejected in the alternatives).

use crate::record::{RecordError, RecordWriter, as_u16, fixed, u16_at, u32_at};
use crate::tags::*;

/// EVENTS 0x60: one event from the plane's ring.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Event {
    /// Sequence number; EV_ACK acknowledges through one of these.
    pub seq: u16,
    /// What the event is about.
    pub target: EventTarget,
    /// What happened.
    pub kind: EventKind,
    /// The plane's uptime when it happened \[s\].
    pub uptime_s: u32,
}

impl Event {
    /// The record's tag.
    pub const TAG: u8 = TAG_EVENT;

    /// Bytes of one event record's value.
    pub const LEN: usize = 8;

    /// An event.
    pub const fn new(seq: u16, target: EventTarget, kind: EventKind, uptime_s: u32) -> Self {
        Event {
            seq,
            target,
            kind,
            uptime_s,
        }
    }

    /// Append the record to a payload.
    pub fn encode(&self, w: &mut RecordWriter) -> Result<(), RecordError> {
        let mut v = [0u8; Self::LEN];
        v[0..2].copy_from_slice(&self.seq.to_le_bytes());
        v[2] = self.target.value();
        v[3] = self.kind.value();
        v[4..8].copy_from_slice(&self.uptime_s.to_le_bytes());
        w.push(Self::TAG, &v)
    }

    /// Read the record's value.
    pub fn decode(value: &[u8]) -> Result<Self, RecordError> {
        let v = fixed::<{ Self::LEN }>(Self::TAG, value)?;
        Ok(Event {
            seq: u16_at(&v, 0),
            target: EventTarget::try_from(v[2])
                .map_err(|_| RecordError::BadValue { tag: Self::TAG })?,
            kind: EventKind::try_from(v[3])
                .map_err(|_| RecordError::BadValue { tag: Self::TAG })?,
            uptime_s: u32_at(&v, 4),
        })
    }
}

/// EVENTS 0x61: how many events the ring dropped before the controller
/// read them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EventsLost {
    /// Events lost to overflow.
    pub count: u16,
}

impl EventsLost {
    /// The record's tag.
    pub const TAG: u8 = TAG_EVENTS_LOST;

    /// Append the record to a payload.
    pub fn encode(&self, w: &mut RecordWriter) -> Result<(), RecordError> {
        w.push(Self::TAG, &self.count.to_le_bytes())
    }

    /// Read the record's value.
    pub fn decode(value: &[u8]) -> Result<Self, RecordError> {
        Ok(EventsLost {
            count: as_u16(Self::TAG, value)?,
        })
    }
}

/// EV_ACK 0x62: acknowledge every event up to and including `seq`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AckThrough {
    /// The last event sequence number the controller has processed.
    pub seq: u16,
}

impl AckThrough {
    /// The record's tag.
    pub const TAG: u8 = TAG_ACK_THROUGH;

    /// An acknowledgement.
    pub const fn new(seq: u16) -> Self {
        AckThrough { seq }
    }

    /// Append the record to a payload.
    pub fn encode(&self, w: &mut RecordWriter) -> Result<(), RecordError> {
        w.push(Self::TAG, &self.seq.to_le_bytes())
    }

    /// Read the record's value.
    pub fn decode(value: &[u8]) -> Result<Self, RecordError> {
        Ok(AckThrough {
            seq: as_u16(Self::TAG, value)?,
        })
    }
}

/// One record of the EVENTS block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventRecord {
    /// 0x60 an event.
    Event(Event),
    /// 0x61 the lost counter.
    Lost(EventsLost),
}

impl EventRecord {
    /// The record's tag.
    pub const fn tag(&self) -> u8 {
        match self {
            EventRecord::Event(_) => TAG_EVENT,
            EventRecord::Lost(_) => TAG_EVENTS_LOST,
        }
    }

    /// Append the record to a payload.
    pub fn encode(&self, w: &mut RecordWriter) -> Result<(), RecordError> {
        match self {
            EventRecord::Event(e) => e.encode(w),
            EventRecord::Lost(l) => l.encode(w),
        }
    }

    /// Read one record; other tags give [`RecordError::Unknown`].
    pub fn decode(tag: u8, value: &[u8]) -> Result<Self, RecordError> {
        Ok(match tag {
            TAG_EVENT => EventRecord::Event(Event::decode(value)?),
            TAG_EVENTS_LOST => EventRecord::Lost(EventsLost::decode(value)?),
            _ => return Err(RecordError::Unknown { tag }),
        })
    }

    /// Every EVENTS record of a payload, unknown tags skipped.
    pub fn records(payload: &[u8]) -> impl Iterator<Item = Result<Self, RecordError>> + '_ {
        crate::record::Records::new(payload).filter_map(|r| match r {
            Ok((tag, value)) => match EventRecord::decode(tag, value) {
                Err(RecordError::Unknown { .. }) => None,
                other => Some(other),
            },
            Err(e) => Some(Err(e)),
        })
    }

    /// How many events fit in one EVENTS block.
    pub const CAPACITY: usize = crate::record::MAX_PAYLOAD / (Event::LEN + 2);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_round_trip() {
        let e = Event::new(0x0141, EventTarget::Unit(2), EventKind::UnitLost, 86_400);
        let mut buf = [0u8; 16];
        let mut w = RecordWriter::new(&mut buf);
        e.encode(&mut w).unwrap();
        assert_eq!(
            w.payload(),
            &[0x60, 8, 0x41, 0x01, 0x12, 17, 0x80, 0x51, 0x01, 0x00]
        );
        assert_eq!(Event::decode(&w.payload()[2..]), Ok(e));
    }

    #[test]
    fn events_block_holds_twenty_five() {
        assert_eq!(EventRecord::CAPACITY, 25);
        let mut buf = [0u8; crate::record::MAX_PAYLOAD];
        let mut w = RecordWriter::new(&mut buf);
        for seq in 0..EventRecord::CAPACITY as u16 {
            Event::new(seq, EventTarget::Plane, EventKind::Boot, seq as u32)
                .encode(&mut w)
                .unwrap();
        }
        assert_eq!(EventRecord::records(w.payload()).count(), 25);
        assert_eq!(
            Event::new(99, EventTarget::Plane, EventKind::Boot, 0).encode(&mut w),
            Err(RecordError::TooLong)
        );
    }

    #[test]
    fn bad_target_and_kind() {
        assert_eq!(
            Event::decode(&[0, 0, 0x09, 1, 0, 0, 0, 0]),
            Err(RecordError::BadValue { tag: TAG_EVENT })
        );
        assert_eq!(
            Event::decode(&[0, 0, 0xFF, 99, 0, 0, 0, 0]),
            Err(RecordError::BadValue { tag: TAG_EVENT })
        );
    }
}
