//! FW block records (tags 0x90-0x94): the firmware update of ADR 0002
//! decision 9.
//!
//! Begin erases the DFU slot, data records fill it (at most
//! [`FW_DATA_MAX`] bytes each, paced by the next offset in STATUS
//! 0x18), commit checks the CRC-32 over DFU and marks the swap, a CTRL
//! reboot performs it, and confirm ends the probation of the new image.
//! Data is borrowed: nothing here copies an image.

use crate::info::Version;
use crate::record::{RecordError, RecordWriter, Records, as_u32, fixed, u32_at};
use crate::tags::*;

/// FW 0x90: start an update.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FwBegin {
    /// Image size \[bytes\].
    pub size: u32,
    /// CRC-32 over the whole image.
    pub crc32: u32,
    /// The version the image carries.
    pub version: Version,
}

impl FwBegin {
    /// The record's tag.
    pub const TAG: u8 = TAG_FW_BEGIN;

    /// Bytes of the value.
    pub const LEN: usize = 11;

    /// Append the record to a payload.
    pub fn encode(&self, w: &mut RecordWriter) -> Result<(), RecordError> {
        let mut v = [0u8; Self::LEN];
        v[0..4].copy_from_slice(&self.size.to_le_bytes());
        v[4..8].copy_from_slice(&self.crc32.to_le_bytes());
        v[8..11].copy_from_slice(&self.version.to_bytes());
        w.push(Self::TAG, &v)
    }

    /// Read the record's value.
    pub fn decode(value: &[u8]) -> Result<Self, RecordError> {
        let v = fixed::<{ Self::LEN }>(Self::TAG, value)?;
        Ok(FwBegin {
            size: u32_at(&v, 0),
            crc32: u32_at(&v, 4),
            version: Version::new(v[8], v[9], v[10]),
        })
    }
}

/// FW 0x91: a piece of the image, borrowed from the frame it came in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FwData<'a> {
    /// Where this piece belongs in the image.
    pub offset: u32,
    /// The bytes, at most [`FW_DATA_MAX`].
    pub data: &'a [u8],
}

impl<'a> FwData<'a> {
    /// The record's tag.
    pub const TAG: u8 = TAG_FW_DATA;

    /// Append the record to a payload.
    pub fn encode(&self, w: &mut RecordWriter) -> Result<(), RecordError> {
        if self.data.len() > FW_DATA_MAX {
            return Err(RecordError::ValueTooLong);
        }
        let o = self.offset.to_le_bytes();
        w.push_parts(Self::TAG, &[&o, self.data])
    }

    /// Read the record's value, borrowing the data.
    pub fn decode(value: &'a [u8]) -> Result<Self, RecordError> {
        if value.len() < 4 || value.len() > 4 + FW_DATA_MAX {
            return Err(RecordError::BadLength {
                tag: Self::TAG,
                len: value.len().min(255) as u8,
            });
        }
        Ok(FwData {
            offset: u32_at(value, 0),
            data: &value[4..],
        })
    }

    /// The offset the plane expects after this record.
    pub const fn next_offset(&self) -> u32 {
        self.offset.wrapping_add(self.data.len() as u32)
    }
}

/// One FW record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fw<'a> {
    /// 0x90 begin: erases DFU.
    Begin(FwBegin),
    /// 0x91 data.
    Data(FwData<'a>),
    /// 0x92 commit: verify DFU against this CRC-32 and mark the swap.
    Commit {
        /// CRC-32 over the image, as in [`FwBegin`].
        crc32: u32,
    },
    /// 0x93 confirm: the running image is good, end its probation.
    Confirm,
    /// 0x94 abort: forget the transfer.
    Abort,
}

impl<'a> Fw<'a> {
    /// The record's tag.
    pub const fn tag(&self) -> u8 {
        match self {
            Fw::Begin(_) => TAG_FW_BEGIN,
            Fw::Data(_) => TAG_FW_DATA,
            Fw::Commit { .. } => TAG_FW_COMMIT,
            Fw::Confirm => TAG_FW_CONFIRM,
            Fw::Abort => TAG_FW_ABORT,
        }
    }

    /// Append the record to a payload.
    pub fn encode(&self, w: &mut RecordWriter) -> Result<(), RecordError> {
        match self {
            Fw::Begin(b) => b.encode(w),
            Fw::Data(d) => d.encode(w),
            Fw::Commit { crc32 } => w.push(TAG_FW_COMMIT, &crc32.to_le_bytes()),
            Fw::Confirm => w.push(TAG_FW_CONFIRM, &[]),
            Fw::Abort => w.push(TAG_FW_ABORT, &[]),
        }
    }

    /// Read one record; tags outside the FW table give
    /// [`RecordError::Unknown`].
    pub fn decode(tag: u8, value: &'a [u8]) -> Result<Self, RecordError> {
        Ok(match tag {
            TAG_FW_BEGIN => Fw::Begin(FwBegin::decode(value)?),
            TAG_FW_DATA => Fw::Data(FwData::decode(value)?),
            TAG_FW_COMMIT => Fw::Commit {
                crc32: as_u32(tag, value)?,
            },
            TAG_FW_CONFIRM => {
                let _ = fixed::<0>(tag, value)?;
                Fw::Confirm
            }
            TAG_FW_ABORT => {
                let _ = fixed::<0>(tag, value)?;
                Fw::Abort
            }
            _ => return Err(RecordError::Unknown { tag }),
        })
    }

    /// Every record of an FW write, unknown tags reported (a write is
    /// answered per record, like CTRL).
    pub fn records(payload: &'a [u8]) -> impl Iterator<Item = Result<Self, RecordError>> + 'a {
        Records::new(payload).map(|r| match r {
            Ok((tag, value)) => Fw::decode(tag, value),
            Err(e) => Err(e),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_record_borrows_and_fits() {
        let image = [0xA5u8; FW_DATA_MAX];
        let mut buf = [0u8; crate::record::MAX_PAYLOAD];
        let mut w = RecordWriter::new(&mut buf);
        FwData {
            offset: 0x1000,
            data: &image,
        }
        .encode(&mut w)
        .unwrap();
        assert_eq!(w.len(), 2 + 4 + FW_DATA_MAX);
        match Fw::records(w.payload()).next().unwrap().unwrap() {
            Fw::Data(d) => {
                assert_eq!(d.offset, 0x1000);
                assert_eq!(d.data, &image);
                assert_eq!(d.next_offset(), 0x1080);
            }
            other => panic!("not data: {other:?}"),
        }
    }

    #[test]
    fn oversized_data_is_rejected() {
        let big = [0u8; FW_DATA_MAX + 1];
        let mut buf = [0u8; crate::record::MAX_PAYLOAD];
        let mut w = RecordWriter::new(&mut buf);
        assert_eq!(
            FwData {
                offset: 0,
                data: &big
            }
            .encode(&mut w),
            Err(RecordError::ValueTooLong)
        );
        let mut value = [0u8; 4 + FW_DATA_MAX + 1];
        value[0] = 0;
        assert!(matches!(
            FwData::decode(&value),
            Err(RecordError::BadLength { .. })
        ));
        assert!(matches!(
            FwData::decode(&[0, 0, 0]),
            Err(RecordError::BadLength { .. })
        ));
    }

    #[test]
    fn begin_commit_confirm_round_trip() {
        let mut buf = [0u8; 64];
        let mut w = RecordWriter::new(&mut buf);
        let begin = FwBegin {
            size: 55_000,
            crc32: 0xDEAD_BEEF,
            version: Version::new(0, 2, 1),
        };
        Fw::Begin(begin).encode(&mut w).unwrap();
        Fw::Commit { crc32: 0xDEAD_BEEF }.encode(&mut w).unwrap();
        Fw::Confirm.encode(&mut w).unwrap();
        Fw::Abort.encode(&mut w).unwrap();
        let mut it = Fw::records(w.payload());
        assert_eq!(it.next(), Some(Ok(Fw::Begin(begin))));
        assert_eq!(it.next(), Some(Ok(Fw::Commit { crc32: 0xDEAD_BEEF })));
        assert_eq!(it.next(), Some(Ok(Fw::Confirm)));
        assert_eq!(it.next(), Some(Ok(Fw::Abort)));
        assert_eq!(it.next(), None);
    }
}
