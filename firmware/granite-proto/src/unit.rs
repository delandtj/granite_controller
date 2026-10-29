//! UNIT block records (tags 0x30-0x36): one unit board of the chained
//! plane (ADR 0005), index 0-3.
//!
//! A unit holds two slots, its own hot-swap (LM5069-1) and its own
//! input INA238, so its input figures are the sum of its two slots plus
//! the unit's own losses.

use crate::bits::UnitStatus;
use crate::record::{RecordError, RecordWriter, as_i16, as_u16};
use crate::tags::*;

/// One UNIT record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    /// 0x30 the unit's status bits.
    Status(UnitStatus),
    /// 0x31 input voltage \[10 mV\].
    InputVolts(u16),
    /// 0x32 input current \[mA\].
    InputAmps(u16),
    /// 0x33 input power \[0.1 W\].
    InputWatts(u16),
    /// 0x34 input INA238 temperature \[0.1 C\].
    InputTemperature(i16),
    /// 0x35 I2C errors on this unit's segment.
    SegmentErrors(u16),
    /// 0x36 how often the unit stopped answering.
    TimesLost(u16),
}

impl Unit {
    /// The record's tag.
    pub const fn tag(&self) -> u8 {
        match self {
            Unit::Status(_) => TAG_UNIT_STATUS,
            Unit::InputVolts(_) => TAG_UNIT_V,
            Unit::InputAmps(_) => TAG_UNIT_I,
            Unit::InputWatts(_) => TAG_UNIT_P,
            Unit::InputTemperature(_) => TAG_UNIT_TEMPERATURE,
            Unit::SegmentErrors(_) => TAG_UNIT_SEGMENT_ERRORS,
            Unit::TimesLost(_) => TAG_UNIT_TIMES_LOST,
        }
    }

    /// Append the record to a payload.
    pub fn encode(&self, w: &mut RecordWriter) -> Result<(), RecordError> {
        let tag = self.tag();
        match self {
            Unit::Status(s) => w.push(tag, &s.bits().to_le_bytes()),
            Unit::InputVolts(v)
            | Unit::InputAmps(v)
            | Unit::InputWatts(v)
            | Unit::SegmentErrors(v)
            | Unit::TimesLost(v) => w.push(tag, &v.to_le_bytes()),
            Unit::InputTemperature(t) => w.push(tag, &t.to_le_bytes()),
        }
    }

    /// Read one record; tags outside the UNIT table give
    /// [`RecordError::Unknown`].
    pub fn decode(tag: u8, value: &[u8]) -> Result<Self, RecordError> {
        Ok(match tag {
            TAG_UNIT_STATUS => Unit::Status(UnitStatus::from_bits(as_u16(tag, value)?)),
            TAG_UNIT_V => Unit::InputVolts(as_u16(tag, value)?),
            TAG_UNIT_I => Unit::InputAmps(as_u16(tag, value)?),
            TAG_UNIT_P => Unit::InputWatts(as_u16(tag, value)?),
            TAG_UNIT_TEMPERATURE => Unit::InputTemperature(as_i16(tag, value)?),
            TAG_UNIT_SEGMENT_ERRORS => Unit::SegmentErrors(as_u16(tag, value)?),
            TAG_UNIT_TIMES_LOST => Unit::TimesLost(as_u16(tag, value)?),
            _ => return Err(RecordError::Unknown { tag }),
        })
    }

    /// Every UNIT record of a payload, unknown tags skipped.
    pub fn records(payload: &[u8]) -> impl Iterator<Item = Result<Self, RecordError>> + '_ {
        crate::record::Records::new(payload).filter_map(|r| match r {
            Ok((tag, value)) => match Unit::decode(tag, value) {
                Err(RecordError::Unknown { .. }) => None,
                other => Some(other),
            },
            Err(e) => Some(Err(e)),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_block_round_trip() {
        let records = [
            Unit::Status(UnitStatus::PRESENT | UnitStatus::EXPANDER_OK | UnitStatus::HOT_SWAP_PGD),
            Unit::InputVolts(4815),
            Unit::InputAmps(2300),
            Unit::InputWatts(1107),
            Unit::InputTemperature(-55),
            Unit::SegmentErrors(2),
            Unit::TimesLost(1),
        ];
        let mut buf = [0u8; 64];
        let mut w = RecordWriter::new(&mut buf);
        for r in &records {
            r.encode(&mut w).unwrap();
        }
        let mut seen = 0;
        for (got, want) in Unit::records(w.payload()).zip(records.iter()) {
            assert_eq!(got, Ok(*want));
            seen += 1;
        }
        assert_eq!(seen, records.len());
    }
}
