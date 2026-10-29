//! One transaction at a time: read a block, write a block, count what
//! went wrong (ADR 0003 decision 1 and its "two planes on one address"
//! expert question).
//!
//! The link itself only moves bytes ([`crate::hal::PlaneLink`]). The
//! framing and the CRC live in `granite_proto`, so this is where the
//! two meet:
//!
//! - a read builds the two request bytes, hands them to the link, and
//!   checks the answer against the address and the request;
//! - a bad CRC, a short answer or a transport fault is retried
//!   [`READ_TRIES`] times (ADR 0003: "default 100 kHz and 3 retries");
//! - a NAK is *not* retried: it means no plane is strapped there,
//!   which is how discovery reads absence (ADR 0004 decision 5);
//! - CRC failures with a valid length are counted on their own,
//!   because that is the signature of two planes answering one
//!   address: nothing in software can fix it, so after
//!   [`COLLISION_RUN`] of them in a row the address is flagged.

use granite_proto::frame::{self, FrameError};

use crate::hal::{LinkError, PlaneLink};

/// How often a read is tried before the client calls the transaction
/// failed.
pub const READ_TRIES: u32 = 3;

/// CRC failures in a row, with valid lengths, before an address is
/// flagged as a suspected collision. Three per failed read, so this is
/// two failed reads of the same address.
pub const COLLISION_RUN: u32 = 2 * READ_TRIES;

/// Failed transactions in a row before a plane the client knows is
/// called lost (ADR 0004 decision 5).
pub const LOST_AFTER_FAILURES: u32 = 2;

/// Why a read did not produce a payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaneLinkError {
    /// The transaction itself failed.
    Link(LinkError),
    /// What came back did not pass its length or CRC check.
    Frame(FrameError),
}

impl PlaneLinkError {
    /// True when the address was not acknowledged: no plane there.
    pub const fn is_nak(self) -> bool {
        matches!(self, PlaneLinkError::Link(LinkError::Nak))
    }

    /// True when the answer was readable but its CRC did not match: the
    /// case worth retrying, and the one a collision produces.
    pub const fn is_bad_crc(self) -> bool {
        matches!(self, PlaneLinkError::Frame(FrameError::BadCrc))
    }
}

impl core::fmt::Display for PlaneLinkError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            PlaneLinkError::Link(e) => write!(f, "{e}"),
            PlaneLinkError::Frame(FrameError::BadCrc) => f.write_str("bad crc"),
            PlaneLinkError::Frame(FrameError::Short) => f.write_str("short answer"),
            PlaneLinkError::Frame(FrameError::BadLength) => f.write_str("bad length"),
            PlaneLinkError::Frame(FrameError::BufferTooSmall) => f.write_str("buffer too small"),
        }
    }
}

/// What the client counted on one address.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PlaneErrors {
    /// Transactions the link refused to carry (NAK, bus, timeout).
    pub link: u32,
    /// Answers whose CRC did not match, with a valid length.
    pub bad_crc: u32,
    /// Answers whose length was impossible.
    pub bad_frame: u32,
    /// Reads that failed after every retry, and writes that failed.
    pub failed: u32,
    /// Failed transactions since the last good one.
    pub consecutive_failures: u32,
    /// CRC failures since the last good answer.
    pub crc_run: u32,
    /// Two planes may be strapped to this address: every answer fails
    /// its CRC while the length stays valid (ADR 0003, "Two planes in
    /// one frame"). Software cannot resolve it; the straps must be
    /// fixed.
    pub collision_suspected: bool,
}

impl PlaneErrors {
    /// Count one transaction that worked.
    fn good(&mut self) {
        self.consecutive_failures = 0;
        self.crc_run = 0;
        self.collision_suspected = false;
    }

    /// Count one failed try, without ending the transaction.
    fn attempt(&mut self, error: PlaneLinkError) {
        match error {
            PlaneLinkError::Link(_) => self.link = self.link.saturating_add(1),
            PlaneLinkError::Frame(FrameError::BadCrc) => {
                self.bad_crc = self.bad_crc.saturating_add(1);
                self.crc_run = self.crc_run.saturating_add(1);
                if self.crc_run >= COLLISION_RUN {
                    self.collision_suspected = true;
                }
            }
            PlaneLinkError::Frame(_) => self.bad_frame = self.bad_frame.saturating_add(1),
        }
    }

    /// Count one transaction that failed for good.
    fn failed(&mut self) {
        self.failed = self.failed.saturating_add(1);
        self.consecutive_failures = self.consecutive_failures.saturating_add(1);
    }

    /// True when the plane has missed enough transactions in a row to
    /// be called lost.
    pub const fn is_lost(&self) -> bool {
        self.consecutive_failures >= LOST_AFTER_FAILURES
    }
}

/// The transaction layer: the counters of every address plus the two
/// calls that touch the link.
#[derive(Debug, Clone, Copy, Default)]
pub struct Transactions {
    /// Index n is address 0x30 + n.
    errors: [PlaneErrors; granite_proto::MAX_PLANES],
    /// Reads the client has done, retries included.
    pub reads: u32,
    /// Writes the client has done.
    pub writes: u32,
}

impl Transactions {
    /// A fresh set of counters.
    pub fn new() -> Self {
        Transactions::default()
    }

    /// The counters of one address.
    pub fn errors(&self, address: u8) -> PlaneErrors {
        granite_proto::plane_of_address(address)
            .map(|n| self.errors[n as usize])
            .unwrap_or_default()
    }

    /// The counters of one address, to update.
    fn at(&mut self, address: u8) -> Option<&mut PlaneErrors> {
        let n = granite_proto::plane_of_address(address)?;
        Some(&mut self.errors[n as usize])
    }

    /// Forget what was counted on an address (a plane that came back is
    /// a fresh start).
    pub fn clear(&mut self, address: u8) {
        if let Some(e) = self.at(address) {
            *e = PlaneErrors::default();
        }
    }

    /// Read `block` / `index` from `address`, retried as ADR 0003 asks,
    /// and leave the payload at the front of `buf`.
    ///
    /// `buf` must hold [`frame::MAX_ANSWER`] bytes. The answer's
    /// `L, records, crc` is read into it and the records are moved to
    /// the front, so the payload is `buf[..n]` and nothing is
    /// allocated.
    pub fn read<L: PlaneLink>(
        &mut self,
        link: &mut L,
        address: u8,
        block: u8,
        index: u8,
        buf: &mut [u8],
    ) -> Result<usize, PlaneLinkError> {
        let request = frame::read_request(block, index);
        let mut last = PlaneLinkError::Link(LinkError::Timeout);
        for _ in 0..READ_TRIES {
            self.reads = self.reads.saturating_add(1);
            let outcome = match link.write_read(address, &request, buf) {
                Ok(n) => match frame::check_answer(address, &request, &buf[..n]) {
                    Ok(payload) => {
                        let len = payload.len();
                        buf.copy_within(1..1 + len, 0);
                        if let Some(e) = self.at(address) {
                            e.good();
                        }
                        return Ok(len);
                    }
                    Err(e) => PlaneLinkError::Frame(e),
                },
                Err(e) => PlaneLinkError::Link(e),
            };
            if let Some(e) = self.at(address) {
                e.attempt(outcome);
            }
            last = outcome;
            // A NAK is absence, not noise: nothing to retry.
            if outcome.is_nak() {
                break;
            }
        }
        if let Some(e) = self.at(address) {
            e.failed();
        }
        Err(last)
    }

    /// Write one payload to a block of `address`.
    ///
    /// A write has no answer (ADR 0003 decision 1): its outcome is read
    /// back from STATUS under its sequence number, so this is one
    /// attempt and the caller decides whether to resend.
    pub fn write<L: PlaneLink>(
        &mut self,
        link: &mut L,
        address: u8,
        block: u8,
        payload: &[u8],
    ) -> Result<(), PlaneLinkError> {
        let mut buf = [0u8; frame::MAX_WRITE_FRAME];
        let bytes =
            frame::write_frame(&mut buf, address, block, granite_proto::INDEX_NONE, payload)
                .map_err(PlaneLinkError::Frame)?;
        self.writes = self.writes.saturating_add(1);
        match link.write(address, bytes) {
            Ok(()) => {
                if let Some(e) = self.at(address) {
                    e.good();
                }
                Ok(())
            }
            Err(e) => {
                if let Some(err) = self.at(address) {
                    err.attempt(PlaneLinkError::Link(e));
                    err.failed();
                }
                Err(PlaneLinkError::Link(e))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_run_of_crc_failures_flags_a_collision() {
        let mut e = PlaneErrors::default();
        for _ in 0..COLLISION_RUN - 1 {
            e.attempt(PlaneLinkError::Frame(FrameError::BadCrc));
        }
        assert!(!e.collision_suspected);
        e.attempt(PlaneLinkError::Frame(FrameError::BadCrc));
        assert!(e.collision_suspected);
        assert_eq!(e.bad_crc, COLLISION_RUN);
        // One good answer and the suspicion is gone.
        e.good();
        assert!(!e.collision_suspected && e.crc_run == 0);
    }

    #[test]
    fn two_failed_transactions_lose_a_plane() {
        let mut e = PlaneErrors::default();
        e.failed();
        assert!(!e.is_lost());
        e.failed();
        assert!(e.is_lost());
        e.good();
        assert!(!e.is_lost());
    }
}
