//! CONFIG records (tags 0x80-0x8F): read from the CONFIG block,
//! written to CONFIG_W.
//!
//! A CONFIG_W write is one set: the plane applies and stores it only
//! when the set ends with [`Config::Commit`] (0x8F), and answers the
//! write's sequence number in STATUS like any command. The controller
//! holds the desired config and writes it when the hash in INFO
//! differs (ADR 0002 decision 10).

use crate::record::{RecordError, RecordWriter, as_u32, fixed, u16_at};
use crate::tags::*;

/// Retry policy for one slot: CTRL 0x42 and CONFIG 0x81 carry the same
/// four fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryPolicy {
    /// Slot 0-7.
    pub slot: u8,
    /// How many PGD drops in the window the slot may have.
    pub max: u8,
    /// The window \[s\].
    pub window_s: u16,
    /// What the plane does past the maximum. The ADR does not
    /// enumerate the codes (ADR 0002 decision 7 describes the default:
    /// set the slot OFF and raise an event), so the byte is passed
    /// through as it is.
    pub action: u8,
}

impl RetryPolicy {
    /// The CONFIG tag. The same fields travel as [`TAG_RETRY_POLICY`]
    /// in a CTRL write.
    pub const TAG: u8 = TAG_CFG_RETRY_POLICY;

    /// Bytes of the value.
    pub const LEN: usize = 5;

    /// Append the record under the CONFIG tag.
    pub fn encode(&self, w: &mut RecordWriter) -> Result<(), RecordError> {
        self.encode_as(Self::TAG, w)
    }

    /// Append the record under `tag` ([`TAG_RETRY_POLICY`] in CTRL,
    /// [`TAG_CFG_RETRY_POLICY`] in CONFIG).
    pub fn encode_as(&self, tag: u8, w: &mut RecordWriter) -> Result<(), RecordError> {
        crate::check_slot(tag, self.slot)?;
        let mut v = [0u8; Self::LEN];
        v[0] = self.slot;
        v[1] = self.max;
        v[2..4].copy_from_slice(&self.window_s.to_le_bytes());
        v[4] = self.action;
        w.push(tag, &v)
    }

    /// Read the record's value (either tag).
    pub fn decode(value: &[u8]) -> Result<Self, RecordError> {
        let v = fixed::<{ Self::LEN }>(Self::TAG, value)?;
        crate::check_slot(Self::TAG, v[0])?;
        Ok(RetryPolicy {
            slot: v[0],
            max: v[1],
            window_s: u16_at(&v, 2),
            action: v[4],
        })
    }
}

/// CONFIG 0x83: the power limit of one slot \[0.1 W\]. The plane trips
/// the slot when it passes this.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PowerLimit {
    /// Slot 0-7.
    pub slot: u8,
    /// The limit \[0.1 W\]; 0 means no limit.
    pub watts: u16,
}

impl PowerLimit {
    /// The record's tag.
    pub const TAG: u8 = TAG_CFG_POWER_LIMIT;

    /// Append the record to a payload.
    pub fn encode(&self, w: &mut RecordWriter) -> Result<(), RecordError> {
        crate::check_slot(Self::TAG, self.slot)?;
        let p = self.watts.to_le_bytes();
        w.push_parts(Self::TAG, &[&[self.slot], &p])
    }

    /// Read the record's value.
    pub fn decode(value: &[u8]) -> Result<Self, RecordError> {
        let v = fixed::<3>(Self::TAG, value)?;
        crate::check_slot(Self::TAG, v[0])?;
        Ok(PowerLimit {
            slot: v[0],
            watts: u16_at(&v, 1),
        })
    }
}

/// CONFIG 0x84: the period window in which a host LED counts as
/// blinking (the host sleeps) rather than on or off.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlinkDetection {
    /// Shortest period that counts as a blink \[ms\].
    pub min_period_ms: u16,
    /// Longest period that counts as a blink \[ms\].
    pub max_period_ms: u16,
}

impl BlinkDetection {
    /// The record's tag.
    pub const TAG: u8 = TAG_CFG_LED_BLINK;

    /// Append the record to a payload.
    pub fn encode(&self, w: &mut RecordWriter) -> Result<(), RecordError> {
        let a = self.min_period_ms.to_le_bytes();
        let b = self.max_period_ms.to_le_bytes();
        w.push_parts(Self::TAG, &[&a, &b])
    }

    /// Read the record's value.
    pub fn decode(value: &[u8]) -> Result<Self, RecordError> {
        let v = fixed::<4>(Self::TAG, value)?;
        Ok(BlinkDetection {
            min_period_ms: u16_at(&v, 0),
            max_period_ms: u16_at(&v, 2),
        })
    }
}

/// One CONFIG record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Config {
    /// 0x80 the hash of the stored set, read only.
    Hash(u32),
    /// 0x81 a slot's retry policy.
    RetryPolicy(RetryPolicy),
    /// 0x83 a slot's power limit.
    PowerLimit(PowerLimit),
    /// 0x84 the LED blink window.
    BlinkDetection(BlinkDetection),
    /// 0x8F commit: the last record of a CONFIG_W set.
    Commit,
}

impl Config {
    /// The record's tag.
    pub const fn tag(&self) -> u8 {
        match self {
            Config::Hash(_) => TAG_CFG_HASH,
            Config::RetryPolicy(_) => TAG_CFG_RETRY_POLICY,
            Config::PowerLimit(_) => TAG_CFG_POWER_LIMIT,
            Config::BlinkDetection(_) => TAG_CFG_LED_BLINK,
            Config::Commit => TAG_CFG_COMMIT,
        }
    }

    /// Append the record to a payload.
    pub fn encode(&self, w: &mut RecordWriter) -> Result<(), RecordError> {
        match self {
            Config::Hash(h) => w.push(TAG_CFG_HASH, &h.to_le_bytes()),
            Config::RetryPolicy(p) => p.encode(w),
            Config::PowerLimit(p) => p.encode(w),
            Config::BlinkDetection(b) => b.encode(w),
            Config::Commit => w.push(TAG_CFG_COMMIT, &[]),
        }
    }

    /// Read one record; tags outside the CONFIG table give
    /// [`RecordError::Unknown`].
    pub fn decode(tag: u8, value: &[u8]) -> Result<Self, RecordError> {
        Ok(match tag {
            TAG_CFG_HASH => Config::Hash(as_u32(tag, value)?),
            TAG_CFG_RETRY_POLICY => Config::RetryPolicy(RetryPolicy::decode(value)?),
            TAG_CFG_POWER_LIMIT => Config::PowerLimit(PowerLimit::decode(value)?),
            TAG_CFG_LED_BLINK => Config::BlinkDetection(BlinkDetection::decode(value)?),
            TAG_CFG_COMMIT => {
                let _ = fixed::<0>(tag, value)?;
                Config::Commit
            }
            _ => return Err(RecordError::Unknown { tag }),
        })
    }

    /// Every CONFIG record of a read payload, unknown tags skipped.
    pub fn records(payload: &[u8]) -> impl Iterator<Item = Result<Self, RecordError>> + '_ {
        crate::record::Records::new(payload).filter_map(|r| match r {
            Ok((tag, value)) => match Config::decode(tag, value) {
                Err(RecordError::Unknown { .. }) => None,
                other => Some(other),
            },
            Err(e) => Some(Err(e)),
        })
    }

    /// Every record of a CONFIG_W write, unknown tags reported: a write
    /// is answered per record, and an unknown tag is
    /// [`ResultCode::UnknownTag`], not something to skip.
    pub fn written(payload: &[u8]) -> impl Iterator<Item = Result<Self, RecordError>> + '_ {
        crate::record::Records::new(payload).map(|r| match r {
            Ok((tag, value)) => Config::decode(tag, value),
            Err(e) => Err(e),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_set_round_trip() {
        let set = [
            Config::RetryPolicy(RetryPolicy {
                slot: 5,
                max: 3,
                window_s: 60,
                action: 1,
            }),
            Config::PowerLimit(PowerLimit {
                slot: 5,
                watts: 7500,
            }),
            Config::BlinkDetection(BlinkDetection {
                min_period_ms: 300,
                max_period_ms: 3000,
            }),
            Config::Commit,
        ];
        let mut buf = [0u8; 64];
        let mut w = RecordWriter::new(&mut buf);
        for r in &set {
            r.encode(&mut w).unwrap();
        }
        let mut n = 0;
        for (got, want) in Config::written(w.payload()).zip(set.iter()) {
            assert_eq!(got, Ok(*want));
            n += 1;
        }
        assert_eq!(n, set.len());
    }

    #[test]
    fn a_write_reports_unknown_tags() {
        let mut buf = [0u8; 16];
        let mut w = RecordWriter::new(&mut buf);
        w.push(0x85, &[1]).unwrap();
        assert_eq!(
            Config::written(w.payload()).next(),
            Some(Err(RecordError::Unknown { tag: 0x85 }))
        );
        assert_eq!(Config::records(w.payload()).next(), None);
    }

    #[test]
    fn slot_out_of_range_is_rejected() {
        assert_eq!(
            PowerLimit::decode(&[9, 0x10, 0x00]),
            Err(RecordError::BadValue {
                tag: TAG_CFG_POWER_LIMIT
            })
        );
    }
}
