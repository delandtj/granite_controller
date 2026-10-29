//! The RAM log ring behind the LOG block (ADR 0002 decision 11: "a LOG
//! block (last N short records from a RAM ring)").
//!
//! defmt over RTT is the bench tool; in the tank the only way to see
//! what a plane did is to read this. The lines come from the plane's own
//! code, so they are short static texts plus at most one number, and the
//! ring holds the last [`LOG_LINES`] of them.

/// Lines the ring holds. Eight is one LOG block's worth at this text
/// length and costs under 400 bytes of RAM.
pub const LOG_LINES: usize = 8;

/// Bytes of text one line keeps. The wire allows 48
/// ([`granite_proto::LOG_TEXT_MAX`]); the plane's own messages are
/// shorter and the ring is RAM.
pub const LOG_TEXT: usize = 32;

/// Levels, the plane's own scale (LOG 0xA0 passes the byte through).
pub mod level {
    /// Something the plane could not do.
    pub const ERROR: u8 = 1;
    /// Something unexpected that it worked around.
    pub const WARN: u8 = 2;
    /// A state change worth seeing.
    pub const INFO: u8 = 3;
}

/// One line of the ring.
#[derive(Debug, Clone, Copy)]
pub struct Line {
    /// Uptime when it was written \[ms\].
    pub uptime_ms: u32,
    /// Severity.
    pub level: u8,
    /// The text, `len` bytes of it.
    text: [u8; LOG_TEXT],
    /// Bytes of `text` in use.
    len: u8,
}

impl Line {
    /// The text as it was written.
    pub fn text(&self) -> &[u8] {
        &self.text[..self.len as usize]
    }
}

/// The ring itself.
#[derive(Debug, Default)]
pub struct Log {
    ring: heapless::Deque<Line, LOG_LINES>,
}

impl Log {
    /// An empty ring.
    pub const fn new() -> Self {
        Log {
            ring: heapless::Deque::new(),
        }
    }

    /// Write a line, dropping the oldest when the ring is full.
    ///
    /// `n` is appended as a decimal when it is `Some`, which covers
    /// every message the plane has ("unit 2 lost", "slot 5 off"),
    /// without a formatter.
    pub fn push(&mut self, uptime_ms: u32, level: u8, message: &str, n: Option<u32>) {
        let mut text = [0u8; LOG_TEXT];
        let mut len = 0usize;
        for b in message.as_bytes() {
            if len == LOG_TEXT {
                break;
            }
            text[len] = *b;
            len += 1;
        }
        if let Some(mut value) = n {
            if len < LOG_TEXT {
                text[len] = b' ';
                len += 1;
            }
            // Decimal, most significant digit first, without alloc.
            let mut digits = [0u8; 10];
            let mut d = 0;
            loop {
                digits[d] = b'0' + (value % 10) as u8;
                value /= 10;
                d += 1;
                if value == 0 {
                    break;
                }
            }
            while d > 0 && len < LOG_TEXT {
                d -= 1;
                text[len] = digits[d];
                len += 1;
            }
        }
        if self.ring.is_full() {
            let _ = self.ring.pop_front();
        }
        let _ = self.ring.push_back(Line {
            uptime_ms,
            level,
            text,
            len: len as u8,
        });
    }

    /// The lines, oldest first.
    pub fn iter(&self) -> impl Iterator<Item = &Line> {
        self.ring.iter()
    }

    /// How many lines the ring holds.
    pub fn len(&self) -> usize {
        self.ring.len()
    }

    /// True while nothing has been logged.
    pub fn is_empty(&self) -> bool {
        self.ring.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_number_is_appended() {
        let mut log = Log::new();
        log.push(1234, level::WARN, "unit lost", Some(2));
        let line = log.iter().next().unwrap();
        assert_eq!(line.text(), b"unit lost 2");
        assert_eq!((line.uptime_ms, line.level), (1234, level::WARN));
        log.push(0, level::INFO, "boot", None);
        assert_eq!(log.iter().nth(1).unwrap().text(), b"boot");
        log.push(0, level::INFO, "n", Some(0));
        assert_eq!(log.iter().nth(2).unwrap().text(), b"n 0");
        log.push(0, level::INFO, "n", Some(4_294_967_295));
        assert_eq!(log.iter().nth(3).unwrap().text(), b"n 4294967295");
    }

    #[test]
    fn long_text_is_cut_not_overflowed() {
        let mut log = Log::new();
        log.push(0, level::ERROR, "0123456789012345678901234567890123456789", Some(7));
        assert_eq!(log.iter().next().unwrap().text().len(), LOG_TEXT);
    }

    #[test]
    fn the_ring_keeps_the_last_lines() {
        let mut log = Log::new();
        for i in 0..LOG_LINES as u32 + 3 {
            log.push(i, level::INFO, "x", Some(i));
        }
        assert_eq!(log.len(), LOG_LINES);
        assert_eq!(log.iter().next().unwrap().text(), b"x 3");
    }
}
