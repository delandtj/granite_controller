//! INFO block records (ADR 0003 tags 0x01-0x0B): who the plane is.
//!
//! Read at discovery, after a reboot (the boot count changed) and when
//! the config hash or the image state changes.

use crate::bits::{Capabilities, IndexBitmap};
use crate::record::{RecordError, RecordWriter, as_u8, as_u16, as_u32, fixed};
use crate::tags::*;

/// A three-byte version, major.minor.patch (INFO 0x04 and 0x0B, FW
/// begin).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Version {
    /// Major.
    pub major: u8,
    /// Minor.
    pub minor: u8,
    /// Patch.
    pub patch: u8,
}

impl Version {
    /// A version from its three parts.
    pub const fn new(major: u8, minor: u8, patch: u8) -> Self {
        Version {
            major,
            minor,
            patch,
        }
    }

    /// The three bytes as they travel.
    pub const fn to_bytes(self) -> [u8; 3] {
        [self.major, self.minor, self.patch]
    }

    /// A version from three bytes.
    pub const fn from_bytes(b: [u8; 3]) -> Self {
        Version::new(b[0], b[1], b[2])
    }
}

impl core::fmt::Display for Version {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// One INFO record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Info {
    /// 0x01 protocol format version ([`crate::FORMAT_VERSION`] here).
    FormatVersion(u8),
    /// 0x02 plane type.
    PlaneType(PlaneType),
    /// 0x03 slot capacity, 8 on the chained plane.
    SlotCapacity(u8),
    /// 0x04 firmware version.
    FirmwareVersion(Version),
    /// 0x05 serial: a hash of the STM32 UID.
    Serial(u32),
    /// 0x06 what the plane can do.
    Capabilities(Capabilities),
    /// 0x07 units present, bit n = unit n.
    UnitsPresent(IndexBitmap),
    /// 0x08 boot count.
    BootCount(u32),
    /// 0x09 config hash: CRC-32 of the stored config set.
    ConfigHash(u32),
    /// 0x0A image state of the OTA ladder.
    ImageState(ImageState),
    /// 0x0B bootloader version.
    BootloaderVersion(Version),
}

impl Info {
    /// The record's tag.
    pub const fn tag(&self) -> u8 {
        match self {
            Info::FormatVersion(_) => TAG_FORMAT_VERSION,
            Info::PlaneType(_) => TAG_PLANE_TYPE,
            Info::SlotCapacity(_) => TAG_SLOT_CAPACITY,
            Info::FirmwareVersion(_) => TAG_FIRMWARE_VERSION,
            Info::Serial(_) => TAG_SERIAL,
            Info::Capabilities(_) => TAG_CAPABILITIES,
            Info::UnitsPresent(_) => TAG_UNITS_PRESENT,
            Info::BootCount(_) => TAG_BOOT_COUNT,
            Info::ConfigHash(_) => TAG_CONFIG_HASH,
            Info::ImageState(_) => TAG_IMAGE_STATE,
            Info::BootloaderVersion(_) => TAG_BOOTLOADER_VERSION,
        }
    }

    /// Append the record to a payload.
    pub fn encode(&self, w: &mut RecordWriter) -> Result<(), RecordError> {
        let tag = self.tag();
        match self {
            Info::FormatVersion(v) | Info::SlotCapacity(v) => w.push(tag, &[*v]),
            Info::PlaneType(t) => w.push(tag, &t.value().to_le_bytes()),
            Info::FirmwareVersion(v) | Info::BootloaderVersion(v) => w.push(tag, &v.to_bytes()),
            Info::Serial(v) | Info::BootCount(v) | Info::ConfigHash(v) => {
                w.push(tag, &v.to_le_bytes())
            }
            Info::Capabilities(c) => w.push(tag, &c.bits().to_le_bytes()),
            Info::UnitsPresent(m) => w.push(tag, &[m.bits()]),
            Info::ImageState(s) => w.push(tag, &[s.value()]),
        }
    }

    /// Read one record. Tags outside the INFO table give
    /// [`RecordError::Unknown`]; a read skips those (see
    /// [`Info::records`]).
    pub fn decode(tag: u8, value: &[u8]) -> Result<Self, RecordError> {
        Ok(match tag {
            TAG_FORMAT_VERSION => Info::FormatVersion(as_u8(tag, value)?),
            TAG_PLANE_TYPE => Info::PlaneType(
                PlaneType::try_from(as_u16(tag, value)?)
                    .map_err(|_| RecordError::BadValue { tag })?,
            ),
            TAG_SLOT_CAPACITY => Info::SlotCapacity(as_u8(tag, value)?),
            TAG_FIRMWARE_VERSION => Info::FirmwareVersion(Version::from_bytes(fixed(tag, value)?)),
            TAG_SERIAL => Info::Serial(as_u32(tag, value)?),
            TAG_CAPABILITIES => Info::Capabilities(Capabilities::from_bits(as_u32(tag, value)?)),
            TAG_UNITS_PRESENT => Info::UnitsPresent(IndexBitmap::from_bits(as_u8(tag, value)?)),
            TAG_BOOT_COUNT => Info::BootCount(as_u32(tag, value)?),
            TAG_CONFIG_HASH => Info::ConfigHash(as_u32(tag, value)?),
            TAG_IMAGE_STATE => Info::ImageState(
                ImageState::try_from(as_u8(tag, value)?)
                    .map_err(|_| RecordError::BadValue { tag })?,
            ),
            TAG_BOOTLOADER_VERSION => {
                Info::BootloaderVersion(Version::from_bytes(fixed(tag, value)?))
            }
            _ => return Err(RecordError::Unknown { tag }),
        })
    }

    /// Every INFO record of a payload, unknown tags skipped. A
    /// truncated payload, or a known tag with the wrong length, yields
    /// one error item.
    pub fn records(payload: &[u8]) -> impl Iterator<Item = Result<Self, RecordError>> + '_ {
        crate::record::Records::new(payload).filter_map(|r| match r {
            Ok((tag, value)) => match Info::decode(tag, value) {
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
    fn version_bytes() {
        let v = Version::new(1, 4, 12);
        assert_eq!(v.to_bytes(), [1, 4, 12]);
        assert_eq!(Version::from_bytes([1, 4, 12]), v);
    }

    #[test]
    fn unknown_info_tag_is_skipped_on_read() {
        let mut buf = [0u8; 16];
        let mut w = RecordWriter::new(&mut buf);
        w.push(0xEE, &[0x01]).unwrap();
        w.push(TAG_SLOT_CAPACITY, &[8]).unwrap();
        let got: Result<Info, _> = Info::records(w.payload()).next().unwrap();
        assert_eq!(got, Ok(Info::SlotCapacity(8)));
    }

    #[test]
    fn bad_lengths_and_values() {
        assert_eq!(
            Info::decode(TAG_SERIAL, &[1, 2]),
            Err(RecordError::BadLength {
                tag: TAG_SERIAL,
                len: 2
            })
        );
        assert_eq!(
            Info::decode(TAG_IMAGE_STATE, &[9]),
            Err(RecordError::BadValue {
                tag: TAG_IMAGE_STATE
            })
        );
        assert_eq!(
            Info::decode(0x7E, &[]),
            Err(RecordError::Unknown { tag: 0x7E })
        );
    }
}
