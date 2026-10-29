//! The TLV layer: `tag u8, len u8, value[len]`, packed back to back into
//! a payload of at most [`MAX_PAYLOAD`] bytes (ADR 0003 decision 1).
//!
//! A reader skips tags it does not know by their length and never
//! assumes an order, so a plane may add records and an older controller
//! still parses the ones it knows. A writer fills a caller-owned buffer;
//! nothing here allocates and nothing here panics.

/// Largest payload length byte `L` on the wire (ADR 0003: `L <= 254`).
pub const MAX_PAYLOAD: usize = 254;

/// Largest value a single record can carry: `MAX_PAYLOAD` minus the tag
/// and length bytes.
pub const MAX_VALUE: usize = MAX_PAYLOAD - 2;

/// What can go wrong building or reading records.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordError {
    /// The destination buffer has no room for the record.
    Full,
    /// The record would push `L` past [`MAX_PAYLOAD`].
    TooLong,
    /// The value is longer than [`MAX_VALUE`] (or than the tag allows).
    ValueTooLong,
    /// The payload ends inside a record: a length byte promised more
    /// bytes than are there, or a tag byte stands alone.
    Truncated,
    /// A known tag carried a value of the wrong length.
    BadLength {
        /// The tag that was read.
        tag: u8,
        /// The value length that came with it.
        len: u8,
    },
    /// A tag this block has no meaning for. Reads skip unknown tags;
    /// the typed decoders report them, and a CTRL write answers with
    /// [`crate::ResultCode::UnknownTag`].
    Unknown {
        /// The tag that was read.
        tag: u8,
    },
    /// A field held a value outside the range the ADR gives it (an
    /// enum discriminant, a slot number, a press duration).
    BadValue {
        /// The tag the bad field belongs to.
        tag: u8,
    },
    /// A CTRL write did not start with its 0x4F sequence record. The
    /// plane cannot name such a write in a result, so it drops it.
    NoSeq,
}

/// Builds a record payload in a caller-owned buffer.
///
/// The writer never writes past the buffer and never past
/// [`MAX_PAYLOAD`]; a record that does not fit is rejected and the
/// payload keeps whatever was written before it.
#[derive(Debug)]
pub struct RecordWriter<'a> {
    buf: &'a mut [u8],
    len: usize,
}

impl<'a> RecordWriter<'a> {
    /// A writer over `buf`. Only the first [`MAX_PAYLOAD`] bytes of a
    /// larger buffer are ever used.
    pub fn new(buf: &'a mut [u8]) -> Self {
        RecordWriter { buf, len: 0 }
    }

    /// Payload bytes written so far: this is the frame's `L` records.
    pub fn payload(&self) -> &[u8] {
        &self.buf[..self.len]
    }

    /// Length of the payload, the frame's `L`.
    pub fn len(&self) -> usize {
        self.len
    }

    /// True while nothing has been written.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Value bytes still available for one more record (0 when not even
    /// an empty record fits).
    pub fn remaining(&self) -> usize {
        let cap = if self.buf.len() < MAX_PAYLOAD {
            self.buf.len()
        } else {
            MAX_PAYLOAD
        };
        cap.saturating_sub(self.len).saturating_sub(2)
    }

    /// Forget everything written, keeping the buffer.
    pub fn clear(&mut self) {
        self.len = 0;
    }

    /// Append one record.
    pub fn push(&mut self, tag: u8, value: &[u8]) -> Result<(), RecordError> {
        self.push_parts(tag, &[value])
    }

    /// Append one record whose value is the concatenation of `parts`.
    /// Used by records that carry a borrowed tail (firmware data, a log
    /// line) after a fixed head.
    pub fn push_parts(&mut self, tag: u8, parts: &[&[u8]]) -> Result<(), RecordError> {
        let mut value_len = 0usize;
        for part in parts {
            value_len += part.len();
        }
        if value_len > MAX_VALUE {
            return Err(RecordError::ValueTooLong);
        }
        let end = self.len + 2 + value_len;
        if end > MAX_PAYLOAD {
            return Err(RecordError::TooLong);
        }
        if end > self.buf.len() {
            return Err(RecordError::Full);
        }
        self.buf[self.len] = tag;
        self.buf[self.len + 1] = value_len as u8;
        let mut at = self.len + 2;
        for part in parts {
            self.buf[at..at + part.len()].copy_from_slice(part);
            at += part.len();
        }
        self.len = end;
        Ok(())
    }
}

/// Iterator over the records of a payload.
///
/// Yields `Ok((tag, value))` per record and exactly one
/// `Err(RecordError::Truncated)` when the payload ends inside a record,
/// then stops. It borrows the payload and copies nothing.
#[derive(Debug, Clone)]
pub struct Records<'a> {
    payload: &'a [u8],
    at: usize,
}

impl<'a> Records<'a> {
    /// Read the records of `payload`.
    pub fn new(payload: &'a [u8]) -> Self {
        Records { payload, at: 0 }
    }

    /// Bytes not consumed yet.
    pub fn rest(&self) -> &'a [u8] {
        &self.payload[self.at..]
    }
}

impl<'a> Iterator for Records<'a> {
    type Item = Result<(u8, &'a [u8]), RecordError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.at >= self.payload.len() {
            return None;
        }
        if self.at + 2 > self.payload.len() {
            // A lone tag byte: report once, then end.
            self.at = self.payload.len();
            return Some(Err(RecordError::Truncated));
        }
        let tag = self.payload[self.at];
        let len = self.payload[self.at + 1] as usize;
        let from = self.at + 2;
        let to = from + len;
        if to > self.payload.len() {
            self.at = self.payload.len();
            return Some(Err(RecordError::Truncated));
        }
        self.at = to;
        Some(Ok((tag, &self.payload[from..to])))
    }
}

/// Read a value of exactly `N` bytes, or report the length.
pub(crate) fn fixed<const N: usize>(tag: u8, value: &[u8]) -> Result<[u8; N], RecordError> {
    if value.len() == N {
        let mut out = [0u8; N];
        out.copy_from_slice(value);
        Ok(out)
    } else {
        Err(RecordError::BadLength {
            tag,
            len: value.len().min(255) as u8,
        })
    }
}

/// Read a one-byte value.
pub(crate) fn as_u8(tag: u8, value: &[u8]) -> Result<u8, RecordError> {
    Ok(fixed::<1>(tag, value)?[0])
}

/// Read a little-endian `u16` value.
pub(crate) fn as_u16(tag: u8, value: &[u8]) -> Result<u16, RecordError> {
    Ok(u16::from_le_bytes(fixed::<2>(tag, value)?))
}

/// Read a little-endian `i16` value.
pub(crate) fn as_i16(tag: u8, value: &[u8]) -> Result<i16, RecordError> {
    Ok(i16::from_le_bytes(fixed::<2>(tag, value)?))
}

/// Read a little-endian `u32` value.
pub(crate) fn as_u32(tag: u8, value: &[u8]) -> Result<u32, RecordError> {
    Ok(u32::from_le_bytes(fixed::<4>(tag, value)?))
}

/// Little-endian `u16` at `at` in a slice known to be long enough.
pub(crate) fn u16_at(value: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([value[at], value[at + 1]])
}

/// Little-endian `u32` at `at` in a slice known to be long enough.
pub(crate) fn u32_at(value: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([value[at], value[at + 1], value[at + 2], value[at + 3]])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_and_reads_back() {
        let mut buf = [0u8; 32];
        let mut w = RecordWriter::new(&mut buf);
        w.push(0x4F, &[0x07]).unwrap();
        w.push(0x40, &[0x03, 0x01]).unwrap();
        w.push(0x90, &[]).unwrap();
        assert_eq!(w.len(), 3 + 4 + 2);
        assert_eq!(w.payload(), &[0x4F, 1, 0x07, 0x40, 2, 0x03, 0x01, 0x90, 0]);

        let mut it = Records::new(w.payload());
        assert_eq!(it.next(), Some(Ok((0x4F, &[0x07][..]))));
        assert_eq!(it.next(), Some(Ok((0x40, &[0x03, 0x01][..]))));
        assert_eq!(it.next(), Some(Ok((0x90, &[][..]))));
        assert_eq!(it.next(), None);
    }

    #[test]
    fn full_buffer_is_an_error_not_a_panic() {
        let mut buf = [0u8; 4];
        let mut w = RecordWriter::new(&mut buf);
        w.push(0x01, &[0x01]).unwrap();
        assert_eq!(w.push(0x02, &[0x02]), Err(RecordError::Full));
        assert_eq!(w.len(), 3);
        assert_eq!(w.remaining(), 0);
    }

    #[test]
    fn payload_stops_at_254() {
        let mut buf = [0u8; 300];
        let mut w = RecordWriter::new(&mut buf);
        for _ in 0..42 {
            w.push(0x24, &[0u8; 4]).unwrap();
        }
        assert_eq!(w.len(), 252);
        // Two more bytes fit (an empty record), four do not.
        assert_eq!(w.remaining(), 0);
        assert_eq!(w.push(0x24, &[0u8; 4]), Err(RecordError::TooLong));
        w.push(0x8F, &[]).unwrap();
        assert_eq!(w.len(), 254);
        assert_eq!(w.push(0x8F, &[]), Err(RecordError::TooLong));
    }

    #[test]
    fn value_longer_than_a_payload_is_rejected() {
        let mut buf = [0u8; 300];
        let mut w = RecordWriter::new(&mut buf);
        assert_eq!(
            w.push(0x91, &[0u8; MAX_VALUE + 1]),
            Err(RecordError::ValueTooLong)
        );
    }

    #[test]
    fn truncated_records_end_the_iterator() {
        // Length byte promises 3 bytes, only 1 follows.
        let mut it = Records::new(&[0x20, 3, 0x01]);
        assert_eq!(it.next(), Some(Err(RecordError::Truncated)));
        assert_eq!(it.next(), None);

        // A lone tag byte.
        let mut it = Records::new(&[0x20, 2, 0x01, 0x02, 0x21]);
        assert_eq!(it.next(), Some(Ok((0x20, &[0x01, 0x02][..]))));
        assert_eq!(it.next(), Some(Err(RecordError::Truncated)));
        assert_eq!(it.next(), None);
    }

    #[test]
    fn unknown_tags_are_yielded() {
        let mut it = Records::new(&[0xEE, 1, 0x05]);
        assert_eq!(it.next(), Some(Ok((0xEE, &[0x05][..]))));
        assert_eq!(it.next(), None);
    }
}
