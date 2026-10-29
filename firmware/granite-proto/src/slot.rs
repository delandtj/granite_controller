//! SLOT block records (tags 0x20-0x2D and 0x70): one slot in full.
//!
//! The slot index on the wire is `2 * unit + (A = 0, B = 1)`; see
//! [`crate::slot_of`]. Reading a slot that is not present answers with
//! a status record whose [`SlotStatus::PRESENT`] bit is clear, never an
//! I2C NAK (a NAK means the plane is gone).

use crate::bits::{CrossCheck, SlotStatus};
use crate::record::{RecordError, RecordWriter, as_i16, as_u8, as_u16, fixed, u16_at, u32_at};
use crate::tags::*;

/// SLOT 0x27: power min and max since the last read \[0.1 W\].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PowerMinMax {
    /// Smallest power seen \[0.1 W\].
    pub min: u16,
    /// Largest power seen \[0.1 W\].
    pub max: u16,
}

impl PowerMinMax {
    /// The record's tag.
    pub const TAG: u8 = TAG_SLOT_P_MIN_MAX;

    /// Append the record to a payload.
    pub fn encode(&self, w: &mut RecordWriter) -> Result<(), RecordError> {
        let a = self.min.to_le_bytes();
        let b = self.max.to_le_bytes();
        w.push_parts(Self::TAG, &[&a, &b])
    }

    /// Read the record's value.
    pub fn decode(value: &[u8]) -> Result<Self, RecordError> {
        let v = fixed::<4>(Self::TAG, value)?;
        Ok(PowerMinMax {
            min: u16_at(&v, 0),
            max: u16_at(&v, 2),
        })
    }
}

/// SLOT 0x29: energy since the plane's first boot, with the boot count
/// the figure belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Energy {
    /// Energy \[J\].
    pub joules: u64,
    /// The plane's boot count when this was accumulated.
    pub boot_count: u32,
}

impl Energy {
    /// The record's tag.
    pub const TAG: u8 = TAG_SLOT_ENERGY;

    /// Append the record to a payload.
    pub fn encode(&self, w: &mut RecordWriter) -> Result<(), RecordError> {
        let a = self.joules.to_le_bytes();
        let b = self.boot_count.to_le_bytes();
        w.push_parts(Self::TAG, &[&a, &b])
    }

    /// Read the record's value.
    pub fn decode(value: &[u8]) -> Result<Self, RecordError> {
        let v = fixed::<12>(Self::TAG, value)?;
        let mut j = [0u8; 8];
        j.copy_from_slice(&v[..8]);
        Ok(Energy {
            joules: u64::from_le_bytes(j),
            boot_count: u32_at(&v, 8),
        })
    }
}

/// SLOT 0x2B: the host's power LED as the plane reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NodeLed {
    /// Off, on or blinking.
    pub mode: LedMode,
    /// Blink period \[ms\]; 0 unless blinking.
    pub period_ms: u16,
}

impl NodeLed {
    /// The record's tag.
    pub const TAG: u8 = TAG_NODE_LED;

    /// Append the record to a payload.
    pub fn encode(&self, w: &mut RecordWriter) -> Result<(), RecordError> {
        let p = self.period_ms.to_le_bytes();
        w.push_parts(Self::TAG, &[&[self.mode.value()], &p])
    }

    /// Read the record's value.
    pub fn decode(value: &[u8]) -> Result<Self, RecordError> {
        let v = fixed::<3>(Self::TAG, value)?;
        Ok(NodeLed {
            mode: LedMode::try_from(v[0]).map_err(|_| RecordError::BadValue { tag: Self::TAG })?,
            period_ms: u16_at(&v, 1),
        })
    }
}

/// One SLOT record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slot {
    /// 0x20 the slot's status bits.
    Status(SlotStatus),
    /// 0x21 hot-swap retries counted in the policy window.
    RetryCount(u16),
    /// 0x22 module class, the plane's own code.
    ModuleClass(u8),
    /// 0x23 module ID as read, unmapped.
    ModuleIdRaw(u16),
    /// 0x24 voltage \[10 mV\].
    Volts(u16),
    /// 0x25 current \[mA\].
    Amps(u16),
    /// 0x26 power \[0.1 W\].
    Watts(u16),
    /// 0x27 power min / max since the last read.
    PowerMinMax(PowerMinMax),
    /// 0x28 temperature \[0.1 C\] from the INA238 die.
    Temperature(i16),
    /// 0x29 energy and its boot count.
    Energy(Energy),
    /// 0x2B the host's power LED.
    NodeLed(NodeLed),
    /// 0x2C LED against rail cross-checks.
    CrossCheck(CrossCheck),
    /// 0x2D the slot's sequence state.
    SequenceState(SequenceState),
    /// 0x70 power limit in force \[0.1 W\].
    PowerLimit(u16),
}

impl Slot {
    /// The record's tag.
    pub const fn tag(&self) -> u8 {
        match self {
            Slot::Status(_) => TAG_SLOT_STATUS,
            Slot::RetryCount(_) => TAG_RETRY_COUNT,
            Slot::ModuleClass(_) => TAG_MODULE_CLASS,
            Slot::ModuleIdRaw(_) => TAG_MODULE_ID_RAW,
            Slot::Volts(_) => TAG_SLOT_V,
            Slot::Amps(_) => TAG_SLOT_I,
            Slot::Watts(_) => TAG_SLOT_P,
            Slot::PowerMinMax(_) => TAG_SLOT_P_MIN_MAX,
            Slot::Temperature(_) => TAG_SLOT_TEMPERATURE,
            Slot::Energy(_) => TAG_SLOT_ENERGY,
            Slot::NodeLed(_) => TAG_NODE_LED,
            Slot::CrossCheck(_) => TAG_CROSS_CHECK,
            Slot::SequenceState(_) => TAG_SEQUENCE_STATE,
            Slot::PowerLimit(_) => TAG_POWER_LIMIT_IN_FORCE,
        }
    }

    /// Append the record to a payload.
    pub fn encode(&self, w: &mut RecordWriter) -> Result<(), RecordError> {
        let tag = self.tag();
        match self {
            Slot::Status(s) => w.push(tag, &s.bits().to_le_bytes()),
            Slot::RetryCount(v)
            | Slot::ModuleIdRaw(v)
            | Slot::Volts(v)
            | Slot::Amps(v)
            | Slot::Watts(v)
            | Slot::PowerLimit(v) => w.push(tag, &v.to_le_bytes()),
            Slot::ModuleClass(c) => w.push(tag, &[*c]),
            Slot::PowerMinMax(p) => p.encode(w),
            Slot::Temperature(t) => w.push(tag, &t.to_le_bytes()),
            Slot::Energy(e) => e.encode(w),
            Slot::NodeLed(l) => l.encode(w),
            Slot::CrossCheck(c) => w.push(tag, &[c.bits()]),
            Slot::SequenceState(s) => w.push(tag, &[s.value()]),
        }
    }

    /// Read one record; tags outside the SLOT table give
    /// [`RecordError::Unknown`].
    pub fn decode(tag: u8, value: &[u8]) -> Result<Self, RecordError> {
        Ok(match tag {
            TAG_SLOT_STATUS => Slot::Status(SlotStatus::from_bits(as_u16(tag, value)?)),
            TAG_RETRY_COUNT => Slot::RetryCount(as_u16(tag, value)?),
            TAG_MODULE_CLASS => Slot::ModuleClass(as_u8(tag, value)?),
            TAG_MODULE_ID_RAW => Slot::ModuleIdRaw(as_u16(tag, value)?),
            TAG_SLOT_V => Slot::Volts(as_u16(tag, value)?),
            TAG_SLOT_I => Slot::Amps(as_u16(tag, value)?),
            TAG_SLOT_P => Slot::Watts(as_u16(tag, value)?),
            TAG_SLOT_P_MIN_MAX => Slot::PowerMinMax(PowerMinMax::decode(value)?),
            TAG_SLOT_TEMPERATURE => Slot::Temperature(as_i16(tag, value)?),
            TAG_SLOT_ENERGY => Slot::Energy(Energy::decode(value)?),
            TAG_NODE_LED => Slot::NodeLed(NodeLed::decode(value)?),
            TAG_CROSS_CHECK => Slot::CrossCheck(CrossCheck::from_bits(as_u8(tag, value)?)),
            TAG_SEQUENCE_STATE => Slot::SequenceState(
                SequenceState::try_from(as_u8(tag, value)?)
                    .map_err(|_| RecordError::BadValue { tag })?,
            ),
            TAG_POWER_LIMIT_IN_FORCE => Slot::PowerLimit(as_u16(tag, value)?),
            _ => return Err(RecordError::Unknown { tag }),
        })
    }

    /// Every SLOT record of a payload, unknown tags skipped.
    pub fn records(payload: &[u8]) -> impl Iterator<Item = Result<Self, RecordError>> + '_ {
        crate::record::Records::new(payload).filter_map(|r| match r {
            Ok((tag, value)) => match Slot::decode(tag, value) {
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
    fn absent_slot_answers_with_an_empty_status() {
        let mut buf = [0u8; 8];
        let mut w = RecordWriter::new(&mut buf);
        Slot::Status(SlotStatus::NONE).encode(&mut w).unwrap();
        assert_eq!(w.payload(), &[0x20, 2, 0x00, 0x00]);
        let got: Slot = Slot::records(w.payload()).next().unwrap().unwrap();
        match got {
            Slot::Status(s) => assert!(!s.contains(SlotStatus::PRESENT)),
            other => panic!("not a status: {other:?}"),
        }
    }

    #[test]
    fn energy_is_twelve_bytes() {
        let e = Energy {
            joules: 0x0102_0304_0506_0708,
            boot_count: 7,
        };
        let mut buf = [0u8; 32];
        let mut w = RecordWriter::new(&mut buf);
        e.encode(&mut w).unwrap();
        assert_eq!(w.payload()[1], 12);
        assert_eq!(Energy::decode(&w.payload()[2..]), Ok(e));
    }
}
