//! The plane's LOG block (0x08), on the controller's side.
//!
//! A plane keeps the last eight short records of what it did in a RAM
//! ring (ADR 0002 decision 11: "a LOG block (last N short records from a
//! RAM ring)"); ADR 0003 serves it as block 0x08, index 0xFF, one
//! [`granite_proto::LogLine`] record (tag 0xA0) per line, oldest first.
//! On the bench there is defmt over RTT; in a rack there is only this,
//! which is why the controller reads it at all.
//!
//! This module is the controller's half: what a line looks like once it
//! is off the wire ([`PlaneLogLine`]), what one plane's readout state is
//! ([`PlaneLogState`]: the cache, the rate limit and the de-duplication)
//! and which plane events are worth a read ([`read_after`]).
//!
//! # When a read happens
//!
//! Never on a schedule of its own: a LOG block is 8 x 37 bytes at worst
//! and nothing on a healthy plane wants it. [`PlaneLogState::request`]
//! arms one, and [`crate::plane::PlaneClient`] arms it after a plane
//! event that says something went wrong ([`read_after`]), after a plane
//! reboot, and when a client asks for one by hand. An armed read happens
//! on the next poll that has transaction budget left and that is at
//! least [`READ_PERIOD_MS`] after the last one, so a plane that faults in
//! a loop costs one extra read every few seconds and the 1 Hz poll
//! schedule is never displaced.
//!
//! # What is reported
//!
//! Every read returns the whole ring, so the same lines come back again
//! and again. A line is reported once: [`PlaneLogState::accept`] keeps
//! the last read and reports what the new one holds beyond it, matching
//! on the line itself (its uptime, its level and its text) and counting
//! copies, so two identical lines in one ring are both reported and
//! neither is reported twice.
//!
//! That is the whole rule, and it is why nothing here needs the boot
//! count. A plane that reboots has a new ring with a new clock, so none
//! of its lines match the kept ones and all of them are reported; a
//! plane whose boot count the controller has not re-read yet behaves the
//! same. (A mark on the newest uptime read was tried first and was
//! wrong: the boot count arrives with the next INFO, one poll after the
//! first read of the new ring, and clearing the mark on it then reported
//! that ring twice.)
//!
//! The one case this misses is the same plane writing the identical text
//! at the identical millisecond of two different boots, after the first
//! copy has scrolled out of the ring. The plane's boot line carries its
//! boot count, so that does not happen in practice.

use alloc::string::String;
use alloc::vec::Vec;

use granite_proto::{EventKind, LogLine};

use crate::config::LogLevel;

/// Lines the controller keeps per plane: the plane's whole ring
/// (`plane_logic::log::LOG_LINES` is 8) with room to spare, so a read
/// that catches a busy ring is kept whole.
pub const KEPT_LINES: usize = 16;

/// The shortest gap between two LOG reads of one plane \[ms\].
///
/// A fault storm raises an event per plane cycle (50 ms); this is what
/// keeps it to one extra transaction every few seconds.
pub const READ_PERIOD_MS: u64 = 3_000;

/// The plane's own severity scale. The LOG record passes the byte
/// through (ADR 0003 LOG 0xA0), and `plane-logic/src/log.rs` writes
/// these three.
pub mod level {
    /// Something the plane could not do.
    pub const ERROR: u8 = 1;
    /// Something unexpected that it worked around.
    pub const WARN: u8 = 2;
    /// A state change worth seeing.
    pub const INFO: u8 = 3;
}

/// One line of a plane's log ring.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PlaneLogLine {
    /// The plane's uptime when it was written \[ms\].
    pub uptime_ms: u32,
    /// Severity, the plane's own scale (see [`level`]).
    pub level: u8,
    /// The text as the plane wrote it, with the number it appended.
    pub text: String,
}

impl PlaneLogLine {
    /// Take one decoded LOG record. Text that is not UTF-8 is kept
    /// lossily rather than dropped: a line is evidence either way.
    pub fn of_record(record: LogLine<'_>) -> Self {
        PlaneLogLine {
            uptime_ms: record.uptime_ms,
            level: record.level,
            text: String::from_utf8_lossy(record.text).into_owned(),
        }
    }

    /// The controller log level this line is published at.
    pub const fn log_level(&self) -> LogLevel {
        log_level(self.level)
    }

    /// The lowercase level name, as the MQTT log topic spells it.
    pub const fn level_name(&self) -> &'static str {
        level_name(self.level)
    }
}

/// The controller level one of the plane's levels maps onto.
///
/// The plane has three; the controller has five and `sys.log_level`
/// filters on them. Anything the plane may add later is `debug`, so a
/// newer plane cannot flood an operator's log topic.
pub const fn log_level(plane_level: u8) -> LogLevel {
    match plane_level {
        level::ERROR => LogLevel::Error,
        level::WARN => LogLevel::Warn,
        level::INFO => LogLevel::Info,
        _ => LogLevel::Debug,
    }
}

/// The lowercase name of one of the plane's levels.
pub const fn level_name(plane_level: u8) -> &'static str {
    match plane_level {
        level::ERROR => "error",
        level::WARN => "warn",
        level::INFO => "info",
        _ => "debug",
    }
}

/// The `log` target a plane's lines are published under: `planeN`, the
/// same name a command target and a state payload use.
///
/// An address outside 0x30-0x37 cannot happen (the client only ever has
/// eight) and is named by its address, so a line is never lost to a
/// nameless target.
pub fn log_target(address: u8) -> String {
    match granite_proto::plane_of_address(address) {
        Some(n) => alloc::format!("plane{n}"),
        None => alloc::format!("plane@{address:#04x}"),
    }
}

/// The message text a plane line is logged with: what the plane wrote,
/// plus the plane's own clock, because the controller's timestamp on the
/// record is when it read the line and not when the plane wrote it.
pub fn log_message(text: &str, uptime_ms: u32) -> String {
    alloc::format!("{text} [plane {uptime_ms} ms]")
}

/// True when a plane event is one the LOG block is read after
/// (ADR 0004, "Plane log readout"): everything that says a slot, a unit
/// or a sequence did not do what it was told, plus a boot, which is the
/// plane telling the controller its ring has just started over.
pub const fn read_after(kind: EventKind) -> bool {
    matches!(
        kind,
        EventKind::OvLatched
            | EventKind::UnitHotSwapLatchedOff
            | EventKind::RetryLimitReached
            | EventKind::PowerLimitTrip
            | EventKind::SequenceFailed
            | EventKind::PgdLost
            | EventKind::Boot
            | EventKind::RolledBack
    )
}

/// Every line of a LOG block payload, oldest first, at most
/// [`KEPT_LINES`] of them. Records that are not log lines, and lines
/// that do not decode, are skipped: a TLV reader never assumes the whole
/// payload is for it.
pub fn decode_block(payload: &[u8]) -> Vec<PlaneLogLine> {
    let mut lines: Vec<PlaneLogLine> = LogLine::records(payload)
        .flatten()
        .map(PlaneLogLine::of_record)
        .collect();
    if lines.len() > KEPT_LINES {
        lines.drain(..lines.len() - KEPT_LINES);
    }
    lines
}

/// One plane's LOG readout: the cache, the rate limit and the last read
/// the next one is compared against.
#[derive(Debug, Clone, Default)]
pub struct PlaneLogState {
    /// The last read, oldest line first.
    lines: Vec<PlaneLogLine>,
    /// A read is wanted.
    requested: bool,
    /// The earliest a read may happen \[ms\].
    next_ms: u64,
    /// Anything has been read at all.
    read: bool,
}

impl PlaneLogState {
    /// A plane whose log has never been read.
    pub const fn new() -> Self {
        PlaneLogState {
            lines: Vec::new(),
            requested: false,
            next_ms: 0,
            read: false,
        }
    }

    /// Ask for a read. The rate limit still applies.
    pub fn request(&mut self) {
        self.requested = true;
    }

    /// True when a read is wanted and the rate limit allows one.
    pub const fn is_due(&self, now_ms: u64) -> bool {
        self.requested && now_ms >= self.next_ms
    }

    /// A read has been attempted: it is no longer wanted and the next
    /// one waits out [`READ_PERIOD_MS`]. Called whether the transaction
    /// succeeded or not, so a plane that is falling off the link is not
    /// hammered with log reads.
    pub fn note_read(&mut self, now_ms: u64) {
        self.requested = false;
        self.next_ms = now_ms.saturating_add(READ_PERIOD_MS);
    }

    /// The lines of the last read, oldest first.
    pub fn lines(&self) -> &[PlaneLogLine] {
        &self.lines
    }

    /// True once a read has landed.
    pub const fn has_read(&self) -> bool {
        self.read
    }

    /// Take a fresh read of the ring: keep it as the cache and give back
    /// the lines that have not been reported before.
    ///
    /// The new read is compared against the kept one line by line, with
    /// copies counted, so a ring that has scrolled reports only what
    /// came in and a ring that started over (a plane that rebooted)
    /// reports every line of it.
    pub fn accept(&mut self, lines: Vec<PlaneLogLine>) -> Vec<PlaneLogLine> {
        let mut fresh: Vec<PlaneLogLine> = Vec::new();
        for (i, line) in lines.iter().enumerate() {
            let kept = self.lines.iter().filter(|l| *l == line).count();
            let earlier = lines[..i].iter().filter(|l| *l == line).count();
            if earlier >= kept {
                fresh.push(line.clone());
            }
        }
        self.lines = lines;
        self.read = true;
        fresh
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use granite_proto::RecordWriter;

    /// One line, as the plane would have written it.
    fn line(uptime_ms: u32, level: u8, text: &str) -> PlaneLogLine {
        PlaneLogLine {
            uptime_ms,
            level,
            text: String::from(text),
        }
    }

    /// A LOG block payload holding `lines`, the way the plane encodes
    /// one.
    fn block(lines: &[PlaneLogLine]) -> Vec<u8> {
        let mut buf = [0u8; granite_proto::MAX_PAYLOAD];
        let mut w = RecordWriter::new(&mut buf);
        for l in lines {
            LogLine {
                uptime_ms: l.uptime_ms,
                level: l.level,
                text: l.text.as_bytes(),
            }
            .encode(&mut w)
            .expect("the line fits");
        }
        w.payload().to_vec()
    }

    #[test]
    fn a_block_decodes_oldest_first_and_other_records_are_skipped() {
        let lines = [
            line(100, level::INFO, "boot 1"),
            line(2_500, level::ERROR, "ov latched 5"),
        ];
        let mut payload = block(&lines);
        // A record of another tag in the same payload is not ours.
        payload.extend_from_slice(&[0x13, 2, 0, 0]);
        let got = decode_block(&payload);
        assert_eq!(got, lines);
        assert_eq!(got[1].level_name(), "error");
        assert_eq!(got[1].log_level(), LogLevel::Error);
        assert_eq!(got[0].log_level(), LogLevel::Info);
        assert_eq!(line(0, level::WARN, "x").log_level(), LogLevel::Warn);
        assert_eq!(line(0, 9, "x").log_level(), LogLevel::Debug);
        assert_eq!(level_name(9), "debug");
    }

    #[test]
    fn text_that_is_not_utf8_is_kept_lossily() {
        let value = [0, 0, 0, 0, 1, 0xFF, b'x'];
        let record = LogLine::decode(&value).expect("a line");
        let kept = PlaneLogLine::of_record(record);
        assert!(kept.text.ends_with('x'), "{}", kept.text);
    }

    #[test]
    fn the_first_read_reports_the_whole_ring() {
        let mut state = PlaneLogState::new();
        let lines = alloc::vec![
            line(100, level::INFO, "boot 1"),
            line(2_500, level::ERROR, "ov latched 5"),
        ];
        assert_eq!(state.accept(lines.clone()), lines);
        assert_eq!(state.lines(), lines.as_slice());
        assert!(state.has_read());
    }

    #[test]
    fn a_second_read_of_the_same_ring_reports_nothing() {
        let mut state = PlaneLogState::new();
        let first = alloc::vec![
            line(100, level::INFO, "boot 1"),
            line(2_500, level::ERROR, "ov latched 5"),
        ];
        state.accept(first.clone());
        assert!(state.accept(first.clone()).is_empty());

        // The ring moved on: only the new line is reported, and the old
        // ones that are still in it are not.
        let mut second = first;
        second.push(line(3_000, level::WARN, "unit lost 1"));
        assert_eq!(
            state.accept(second.clone()),
            alloc::vec![line(3_000, level::WARN, "unit lost 1")]
        );
        assert!(state.accept(second).is_empty());
    }

    #[test]
    fn a_line_that_scrolled_out_of_the_ring_is_not_reported_again() {
        let mut state = PlaneLogState::new();
        let first: Vec<PlaneLogLine> = (0..KEPT_LINES as u32)
            .map(|i| line(100 * i, level::INFO, "unit present 0"))
            .collect();
        assert_eq!(state.accept(first.clone()).len(), KEPT_LINES);
        // The oldest line is gone and a new one is in: one report.
        let mut second: Vec<PlaneLogLine> = first[1..].to_vec();
        second.push(line(9_000, level::WARN, "unit lost 1"));
        assert_eq!(
            state.accept(second.clone()),
            alloc::vec![line(9_000, level::WARN, "unit lost 1")]
        );
        assert!(state.accept(second).is_empty());
    }

    #[test]
    fn two_lines_of_one_millisecond_are_both_reported_once() {
        let mut state = PlaneLogState::new();
        let first = alloc::vec![line(700, level::ERROR, "ov latched 5")];
        assert_eq!(state.accept(first), alloc::vec![
            line(700, level::ERROR, "ov latched 5")
        ]);
        // The plane wrote a second line in the same millisecond, and
        // then the identical line once more.
        let second = alloc::vec![
            line(700, level::ERROR, "ov latched 5"),
            line(700, level::ERROR, "ov latched 6"),
            line(700, level::ERROR, "ov latched 6"),
        ];
        assert_eq!(state.accept(second.clone()), alloc::vec![
            line(700, level::ERROR, "ov latched 6"),
            line(700, level::ERROR, "ov latched 6"),
        ]);
        assert!(state.accept(second).is_empty());
    }

    #[test]
    fn a_plane_that_rebooted_has_its_new_ring_reported_whole() {
        let mut state = PlaneLogState::new();
        state.accept(alloc::vec![
            line(100, level::INFO, "boot 1"),
            line(9_000, level::ERROR, "ov latched 5"),
        ]);
        // The ring started over on a clock that started over: none of
        // its lines are the kept ones, so all of them are reported.
        let after = alloc::vec![line(80, level::INFO, "boot 2")];
        assert_eq!(state.accept(after.clone()), after);
        assert!(state.accept(after).is_empty());

        // The boot count the controller holds plays no part in this: a
        // read taken before the next INFO and one taken after it are
        // the same read.
        let same = alloc::vec![
            line(80, level::INFO, "boot 2"),
            line(2_000, level::WARN, "unit lost 1"),
        ];
        assert_eq!(state.accept(same.clone()), alloc::vec![
            line(2_000, level::WARN, "unit lost 1")
        ]);
        assert!(state.accept(same).is_empty());
    }

    #[test]
    fn a_read_is_armed_by_hand_and_rate_limited() {
        let mut state = PlaneLogState::new();
        assert!(!state.is_due(0), "nothing wants a read");
        state.request();
        assert!(state.is_due(0));
        state.note_read(1_000);
        assert!(!state.is_due(1_000), "the read is no longer wanted");
        state.request();
        assert!(!state.is_due(1_000 + READ_PERIOD_MS - 1), "too soon");
        assert!(state.is_due(1_000 + READ_PERIOD_MS));
    }

    #[test]
    fn the_events_a_read_follows_are_the_troubled_ones() {
        for kind in [
            EventKind::OvLatched,
            EventKind::UnitHotSwapLatchedOff,
            EventKind::RetryLimitReached,
            EventKind::PowerLimitTrip,
            EventKind::SequenceFailed,
            EventKind::PgdLost,
            EventKind::Boot,
        ] {
            assert!(read_after(kind), "{kind:?} is trouble");
        }
        for kind in [
            EventKind::SlotOn,
            EventKind::SlotOff,
            EventKind::LedChanged,
            EventKind::SequenceDone,
            EventKind::UnitPresent,
            EventKind::ConfigStored,
        ] {
            assert!(!read_after(kind), "{kind:?} is routine");
        }
    }

    #[test]
    fn a_plane_is_named_by_its_number() {
        assert_eq!(log_target(0x30), "plane0");
        assert_eq!(log_target(0x37), "plane7");
        assert_eq!(log_target(0x3F), "plane@0x3f");
        assert_eq!(log_message("ov latched 5", 1234), "ov latched 5 [plane 1234 ms]");
    }
}
