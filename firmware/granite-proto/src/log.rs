//! LOG block record (tag 0xA0): the plane's RAM ring, read in the
//! field where there is no SWD (ADR 0002 decision 11).
//!
//! The text is borrowed from the frame and is not required to be UTF-8:
//! [`LogLine::text`] stays bytes, and [`LogLine::as_str`] gives a
//! `&str` only when it happens to be valid.

use crate::record::{RecordError, RecordWriter, Records, u32_at};
use crate::tags::*;

/// LOG 0xA0: one log line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LogLine<'a> {
    /// The plane's uptime when the line was written \[ms\].
    pub uptime_ms: u32,
    /// Severity, the plane's own scale (defmt levels in ADR 0002); the
    /// ADR does not enumerate it, so the byte passes through.
    pub level: u8,
    /// The text, at most [`LOG_TEXT_MAX`] bytes, not NUL terminated.
    pub text: &'a [u8],
}

impl<'a> LogLine<'a> {
    /// The record's tag.
    pub const TAG: u8 = TAG_LOG_LINE;

    /// Append the record to a payload.
    pub fn encode(&self, w: &mut RecordWriter) -> Result<(), RecordError> {
        if self.text.len() > LOG_TEXT_MAX {
            return Err(RecordError::ValueTooLong);
        }
        let t = self.uptime_ms.to_le_bytes();
        w.push_parts(Self::TAG, &[&t, &[self.level], self.text])
    }

    /// Read the record's value, borrowing the text.
    pub fn decode(value: &'a [u8]) -> Result<Self, RecordError> {
        if value.len() < 5 || value.len() > 5 + LOG_TEXT_MAX {
            return Err(RecordError::BadLength {
                tag: Self::TAG,
                len: value.len().min(255) as u8,
            });
        }
        Ok(LogLine {
            uptime_ms: u32_at(value, 0),
            level: value[4],
            text: &value[5..],
        })
    }

    /// The text as a `&str`, when it is valid UTF-8.
    pub fn as_str(&self) -> Option<&'a str> {
        core::str::from_utf8(self.text).ok()
    }

    /// Every log line of a LOG payload, other tags skipped.
    pub fn records(payload: &'a [u8]) -> impl Iterator<Item = Result<Self, RecordError>> + 'a {
        Records::new(payload).filter_map(|r| match r {
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
    fn log_line_round_trip() {
        let line = LogLine {
            uptime_ms: 123_456,
            level: 3,
            text: b"unit 2 lost",
        };
        let mut buf = [0u8; 64];
        let mut w = RecordWriter::new(&mut buf);
        line.encode(&mut w).unwrap();
        assert_eq!(w.payload()[0], TAG_LOG_LINE);
        assert_eq!(w.payload()[1] as usize, 5 + line.text.len());
        let got = LogLine::records(w.payload()).next().unwrap().unwrap();
        assert_eq!(got, line);
        assert_eq!(got.as_str(), Some("unit 2 lost"));
    }

    #[test]
    fn long_text_and_short_values_are_rejected() {
        let mut buf = [0u8; 64];
        let mut w = RecordWriter::new(&mut buf);
        assert_eq!(
            LogLine {
                uptime_ms: 0,
                level: 0,
                text: &[b'x'; LOG_TEXT_MAX + 1],
            }
            .encode(&mut w),
            Err(RecordError::ValueTooLong)
        );
        assert!(matches!(
            LogLine::decode(&[0, 0, 0, 0]),
            Err(RecordError::BadLength { .. })
        ));
    }

    #[test]
    fn invalid_utf8_still_decodes() {
        let value = [0, 0, 0, 0, 1, 0xFF, 0xFE];
        let line = LogLine::decode(&value).unwrap();
        assert_eq!(line.text, &[0xFF, 0xFE]);
        assert_eq!(line.as_str(), None);
    }
}
