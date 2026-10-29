//! STATUS block records (tags 0x10-0x1B) and the SUMMARY record
//! (0x1C), the two blocks of the 1 Hz poll.
//!
//! STATUS carries the plane flags, the held command results (ADR 0003
//! decision 4: a command's outcome is read here, by its sequence
//! number) and the link counters.

use crate::bits::{SlotStatus, StatusFlags};
use crate::record::{
    RecordError, RecordWriter, as_i16, as_u8, as_u16, as_u32, fixed, u16_at, u32_at,
};
use crate::tags::*;

/// STATUS 0x16: the result the plane holds for one command sequence
/// number. The plane keeps the last [`crate::SEQ_MEMORY`] of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommandResult {
    /// The sequence number of the CTRL write (its 0x4F record).
    pub seq: u8,
    /// How it ended, or [`ResultCode::Accepted`] while it runs.
    pub code: ResultCode,
}

impl CommandResult {
    /// The record's tag.
    pub const TAG: u8 = TAG_COMMAND_RESULT;

    /// A result for a sequence number.
    pub const fn new(seq: u8, code: ResultCode) -> Self {
        CommandResult { seq, code }
    }

    /// Append the record to a payload.
    pub fn encode(&self, w: &mut RecordWriter) -> Result<(), RecordError> {
        w.push(Self::TAG, &[self.seq, self.code.value()])
    }

    /// Read the record's value.
    pub fn decode(value: &[u8]) -> Result<Self, RecordError> {
        let v = fixed::<2>(Self::TAG, value)?;
        Ok(CommandResult {
            seq: v[0],
            code: ResultCode::try_from(v[1])
                .map_err(|_| RecordError::BadValue { tag: Self::TAG })?,
        })
    }
}

/// STATUS 0x18: how far a firmware update has got (ADR 0002 decision 9:
/// the controller paces itself on the next offset).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UpdateProgress {
    /// What the plane is doing with the image.
    pub state: UpdateState,
    /// The offset the plane expects next.
    pub next_offset: u32,
}

impl UpdateProgress {
    /// The record's tag.
    pub const TAG: u8 = TAG_UPDATE_STATE;

    /// A progress record.
    pub const fn new(state: UpdateState, next_offset: u32) -> Self {
        UpdateProgress { state, next_offset }
    }

    /// Append the record to a payload.
    pub fn encode(&self, w: &mut RecordWriter) -> Result<(), RecordError> {
        let o = self.next_offset.to_le_bytes();
        w.push_parts(Self::TAG, &[&[self.state.value()], &o])
    }

    /// Read the record's value.
    pub fn decode(value: &[u8]) -> Result<Self, RecordError> {
        let v = fixed::<5>(Self::TAG, value)?;
        Ok(UpdateProgress {
            state: UpdateState::try_from(v[0])
                .map_err(|_| RecordError::BadValue { tag: Self::TAG })?,
            next_offset: u32_at(&v, 1),
        })
    }
}

/// STATUS 0x19: frames the plane dropped on the link.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LinkErrors {
    /// Frames whose CRC did not match.
    pub bad_crc: u16,
    /// Frames whose length was impossible.
    pub bad_length: u16,
}

impl LinkErrors {
    /// The record's tag.
    pub const TAG: u8 = TAG_LINK_ERRORS;

    /// Append the record to a payload.
    pub fn encode(&self, w: &mut RecordWriter) -> Result<(), RecordError> {
        let a = self.bad_crc.to_le_bytes();
        let b = self.bad_length.to_le_bytes();
        w.push_parts(Self::TAG, &[&a, &b])
    }

    /// Read the record's value.
    pub fn decode(value: &[u8]) -> Result<Self, RecordError> {
        let v = fixed::<4>(Self::TAG, value)?;
        Ok(LinkErrors {
            bad_crc: u16_at(&v, 0),
            bad_length: u16_at(&v, 2),
        })
    }
}

/// STATUS 0x1A: I2C errors on the plane's own segment bus, one counter
/// per mux channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SegmentErrors {
    /// One counter per channel, channel 0 first.
    pub per_channel: [u16; crate::SEGMENT_CHANNELS],
}

impl SegmentErrors {
    /// The record's tag.
    pub const TAG: u8 = TAG_SEGMENT_ERRORS;

    /// Append the record to a payload.
    pub fn encode(&self, w: &mut RecordWriter) -> Result<(), RecordError> {
        let mut v = [0u8; 2 * crate::SEGMENT_CHANNELS];
        for (i, c) in self.per_channel.iter().enumerate() {
            v[2 * i..2 * i + 2].copy_from_slice(&c.to_le_bytes());
        }
        w.push(Self::TAG, &v)
    }

    /// Read the record's value.
    pub fn decode(value: &[u8]) -> Result<Self, RecordError> {
        let v = fixed::<{ 2 * crate::SEGMENT_CHANNELS }>(Self::TAG, value)?;
        let mut per_channel = [0u16; crate::SEGMENT_CHANNELS];
        for (i, c) in per_channel.iter_mut().enumerate() {
            *c = u16_at(&v, 2 * i);
        }
        Ok(SegmentErrors { per_channel })
    }
}

/// One STATUS record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// 0x10 core input voltage \[10 mV\].
    CoreInputVoltage(u16),
    /// 0x11 core temperature \[0.1 C\] from the MCU sensor.
    CoreTemperature(i16),
    /// 0x12 uptime \[s\].
    Uptime(u32),
    /// 0x13 plane flags.
    Flags(StatusFlags),
    /// 0x16 one held command result.
    CommandResult(CommandResult),
    /// 0x17 the next event sequence number the plane will use.
    EventHead(u16),
    /// 0x18 firmware update progress.
    UpdateProgress(UpdateProgress),
    /// 0x19 link error counters.
    LinkErrors(LinkErrors),
    /// 0x1A segment bus error counters.
    SegmentErrors(SegmentErrors),
    /// 0x1B last reset cause, the plane's own code.
    LastResetCause(u8),
}

impl Status {
    /// The record's tag.
    pub const fn tag(&self) -> u8 {
        match self {
            Status::CoreInputVoltage(_) => TAG_CORE_INPUT_V,
            Status::CoreTemperature(_) => TAG_CORE_TEMPERATURE,
            Status::Uptime(_) => TAG_UPTIME,
            Status::Flags(_) => TAG_FLAGS,
            Status::CommandResult(_) => TAG_COMMAND_RESULT,
            Status::EventHead(_) => TAG_EVENT_HEAD,
            Status::UpdateProgress(_) => TAG_UPDATE_STATE,
            Status::LinkErrors(_) => TAG_LINK_ERRORS,
            Status::SegmentErrors(_) => TAG_SEGMENT_ERRORS,
            Status::LastResetCause(_) => TAG_LAST_RESET_CAUSE,
        }
    }

    /// Append the record to a payload.
    pub fn encode(&self, w: &mut RecordWriter) -> Result<(), RecordError> {
        let tag = self.tag();
        match self {
            Status::CoreInputVoltage(v) | Status::EventHead(v) => w.push(tag, &v.to_le_bytes()),
            Status::CoreTemperature(t) => w.push(tag, &t.to_le_bytes()),
            Status::Uptime(s) => w.push(tag, &s.to_le_bytes()),
            Status::Flags(f) => w.push(tag, &f.bits().to_le_bytes()),
            Status::CommandResult(r) => r.encode(w),
            Status::UpdateProgress(u) => u.encode(w),
            Status::LinkErrors(e) => e.encode(w),
            Status::SegmentErrors(e) => e.encode(w),
            Status::LastResetCause(c) => w.push(tag, &[*c]),
        }
    }

    /// Read one record; tags outside the STATUS table give
    /// [`RecordError::Unknown`].
    pub fn decode(tag: u8, value: &[u8]) -> Result<Self, RecordError> {
        Ok(match tag {
            TAG_CORE_INPUT_V => Status::CoreInputVoltage(as_u16(tag, value)?),
            TAG_CORE_TEMPERATURE => Status::CoreTemperature(as_i16(tag, value)?),
            TAG_UPTIME => Status::Uptime(as_u32(tag, value)?),
            TAG_FLAGS => Status::Flags(StatusFlags::from_bits(as_u16(tag, value)?)),
            TAG_COMMAND_RESULT => Status::CommandResult(CommandResult::decode(value)?),
            TAG_EVENT_HEAD => Status::EventHead(as_u16(tag, value)?),
            TAG_UPDATE_STATE => Status::UpdateProgress(UpdateProgress::decode(value)?),
            TAG_LINK_ERRORS => Status::LinkErrors(LinkErrors::decode(value)?),
            TAG_SEGMENT_ERRORS => Status::SegmentErrors(SegmentErrors::decode(value)?),
            TAG_LAST_RESET_CAUSE => Status::LastResetCause(as_u8(tag, value)?),
            _ => return Err(RecordError::Unknown { tag }),
        })
    }

    /// Every STATUS record of a payload, unknown tags skipped.
    pub fn records(payload: &[u8]) -> impl Iterator<Item = Result<Self, RecordError>> + '_ {
        crate::record::Records::new(payload).filter_map(|r| match r {
            Ok((tag, value)) => match Status::decode(tag, value) {
                Err(RecordError::Unknown { .. }) => None,
                other => Some(other),
            },
            Err(e) => Some(Err(e)),
        })
    }
}

/// SUMMARY 0x1C: one 10-byte record per present slot, the whole plane
/// in about 100 bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SlotSummary {
    /// Slot index 0-7 (2 x unit + A/B).
    pub slot: u8,
    /// The slot's status word, the same bits as SLOT 0x20.
    pub status: SlotStatus,
    /// Voltage \[10 mV\].
    pub volts: u16,
    /// Current \[mA\].
    pub amps: u16,
    /// Power \[0.1 W\].
    pub watts: u16,
    /// The host's power LED.
    pub led: LedMode,
}

impl SlotSummary {
    /// The record's tag.
    pub const TAG: u8 = TAG_SLOT_SUMMARY;

    /// Bytes of one summary record's value.
    pub const LEN: usize = 10;

    /// Append the record to a payload.
    pub fn encode(&self, w: &mut RecordWriter) -> Result<(), RecordError> {
        let mut v = [0u8; Self::LEN];
        v[0] = self.slot;
        v[1..3].copy_from_slice(&self.status.bits().to_le_bytes());
        v[3..5].copy_from_slice(&self.volts.to_le_bytes());
        v[5..7].copy_from_slice(&self.amps.to_le_bytes());
        v[7..9].copy_from_slice(&self.watts.to_le_bytes());
        v[9] = self.led.value();
        w.push(Self::TAG, &v)
    }

    /// Read the record's value.
    pub fn decode(value: &[u8]) -> Result<Self, RecordError> {
        let v = fixed::<{ Self::LEN }>(Self::TAG, value)?;
        Ok(SlotSummary {
            slot: v[0],
            status: SlotStatus::from_bits(u16_at(&v, 1)),
            volts: u16_at(&v, 3),
            amps: u16_at(&v, 5),
            watts: u16_at(&v, 7),
            led: LedMode::try_from(v[9]).map_err(|_| RecordError::BadValue { tag: Self::TAG })?,
        })
    }

    /// Every summary record of a SUMMARY payload, other tags skipped.
    pub fn records(payload: &[u8]) -> impl Iterator<Item = Result<Self, RecordError>> + '_ {
        crate::record::Records::new(payload).filter_map(|r| match r {
            Ok((tag, value)) if tag == Self::TAG => Some(Self::decode(value)),
            Ok(_) => None,
            Err(e) => Some(Err(e)),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_value_is_ten_bytes() {
        let s = SlotSummary {
            slot: 3,
            status: SlotStatus::PRESENT | SlotStatus::ENABLED | SlotStatus::PGD,
            volts: 4812,
            amps: 1500,
            watts: 721,
            led: LedMode::Blinking,
        };
        let mut buf = [0u8; 16];
        let mut w = RecordWriter::new(&mut buf);
        s.encode(&mut w).unwrap();
        assert_eq!(w.len(), SlotSummary::LEN + 2);
        assert_eq!(
            w.payload(),
            &[
                0x1C, 10, 3, 0x07, 0x00, 0xCC, 0x12, 0xDC, 0x05, 0xD1, 0x02, 2
            ]
        );
        assert_eq!(SlotSummary::decode(&w.payload()[2..]), Ok(s));
    }

    #[test]
    fn whole_summary_block_fits() {
        let mut buf = [0u8; crate::record::MAX_PAYLOAD];
        let mut w = RecordWriter::new(&mut buf);
        for slot in 0..crate::SLOT_COUNT as u8 {
            SlotSummary {
                slot,
                status: SlotStatus::PRESENT,
                volts: 4800,
                amps: 0,
                watts: 0,
                led: LedMode::Off,
            }
            .encode(&mut w)
            .unwrap();
        }
        assert_eq!(w.len(), 96);
        assert_eq!(SlotSummary::records(w.payload()).count(), 8);
    }

    #[test]
    fn command_results_round_trip() {
        let mut buf = [0u8; 32];
        let mut w = RecordWriter::new(&mut buf);
        CommandResult::new(0x07, ResultCode::Accepted)
            .encode(&mut w)
            .unwrap();
        CommandResult::new(0x08, ResultCode::Refused)
            .encode(&mut w)
            .unwrap();
        let mut it = Status::records(w.payload());
        assert_eq!(
            it.next(),
            Some(Ok(Status::CommandResult(CommandResult::new(
                7,
                ResultCode::Accepted
            ))))
        );
        assert_eq!(
            it.next(),
            Some(Ok(Status::CommandResult(CommandResult::new(
                8,
                ResultCode::Refused
            ))))
        );
        assert_eq!(it.next(), None);
    }
}
