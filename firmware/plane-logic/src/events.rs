//! The event ring and the command-result memory (ADR 0002 decision 8,
//! ADR 0003 decisions 3 and 4).
//!
//! Events carry a sequence number and are **not** cleared by a read: the
//! controller acknowledges through a number with EV_ACK, and INT_OUT
//! stays high while anything is unacknowledged, so a read that fails its
//! CRC loses nothing. An overflow drops the oldest and counts it; the
//! count travels as EVENTS 0x61 and is cleared when the controller has
//! acknowledged past the overflow.
//!
//! The command-result memory is the other half of the same contract: the
//! plane holds the last [`granite_proto::SEQ_MEMORY`] command sequence
//! numbers with their result code, answers them in STATUS 0x16, and
//! refuses to execute a write whose number it has already executed. An
//! entry that was *not* executed (the queue was full) is not a dedup
//! hit: a resend with the same number runs.

use granite_proto::{Event, EventKind, EventTarget, ResultCode, SEQ_MEMORY};

/// Events the ring holds. One EVENTS block carries 25
/// ([`granite_proto::EventRecord::CAPACITY`]), so 32 leaves a block's
/// worth of headroom between two reads.
pub const EVENT_CAPACITY: usize = 32;

/// The plane's event ring.
#[derive(Debug)]
pub struct Events {
    /// The ring, oldest first.
    ring: heapless::Deque<Event, EVENT_CAPACITY>,
    /// The next sequence number to hand out (STATUS 0x17).
    next_seq: u16,
    /// Events dropped to overflow since the last acknowledgement.
    lost: u16,
}

impl Default for Events {
    fn default() -> Self {
        Events::new()
    }
}

impl Events {
    /// An empty ring whose first event gets sequence number 1.
    pub const fn new() -> Self {
        Events {
            ring: heapless::Deque::new(),
            next_seq: 1,
            lost: 0,
        }
    }

    /// The sequence number the next event will take (STATUS 0x17).
    pub const fn next_seq(&self) -> u16 {
        self.next_seq
    }

    /// Events dropped to overflow (EVENTS 0x61).
    pub const fn lost(&self) -> u16 {
        self.lost
    }

    /// True while anything is unacknowledged: STATUS "events pending"
    /// and INT_OUT high.
    pub fn pending(&self) -> bool {
        !self.ring.is_empty() || self.lost > 0
    }

    /// How many events are unacknowledged.
    pub fn len(&self) -> usize {
        self.ring.len()
    }

    /// True while the ring holds nothing.
    pub fn is_empty(&self) -> bool {
        self.ring.is_empty()
    }

    /// Raise an event. The oldest goes when the ring is full.
    pub fn push(&mut self, target: EventTarget, kind: EventKind, uptime_s: u32) -> u16 {
        let seq = self.next_seq;
        self.next_seq = self.next_seq.wrapping_add(1);
        if self.ring.is_full() {
            let _ = self.ring.pop_front();
            self.lost = self.lost.saturating_add(1);
        }
        // Full was just handled, so this cannot fail.
        let _ = self.ring.push_back(Event::new(seq, target, kind, uptime_s));
        seq
    }

    /// The unacknowledged events, oldest first: what an EVENTS read
    /// answers with, up to what fits.
    pub fn iter(&self) -> impl Iterator<Item = &Event> {
        self.ring.iter()
    }

    /// Acknowledge every event up to and including `seq` (EV_ACK 0x62).
    ///
    /// Sequence numbers wrap, so "up to" is a wrapping comparison
    /// against the ring's own numbers rather than a plain `<=`.
    pub fn ack_through(&mut self, seq: u16) {
        while let Some(front) = self.ring.front() {
            if seq.wrapping_sub(front.seq) < u16::MAX / 2 {
                let _ = self.ring.pop_front();
            } else {
                break;
            }
        }
        if self.ring.is_empty() {
            self.lost = 0;
        }
    }
}

/// One remembered command and how it ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Held {
    /// The CTRL write's sequence number (its 0x4F record).
    pub seq: u8,
    /// Its result, or [`ResultCode::Accepted`] while it runs.
    pub code: ResultCode,
    /// False while the write was refused before it ran (a full queue):
    /// a resend with the same number is then executed, not deduplicated.
    pub executed: bool,
}

/// The last [`SEQ_MEMORY`] command results, oldest first.
#[derive(Debug, Default)]
pub struct Results {
    held: heapless::Deque<Held, SEQ_MEMORY>,
}

impl Results {
    /// An empty memory.
    pub const fn new() -> Self {
        Results {
            held: heapless::Deque::new(),
        }
    }

    /// The result held for `seq`, if the plane remembers it.
    pub fn get(&self, seq: u8) -> Option<Held> {
        self.held.iter().find(|h| h.seq == seq).copied()
    }

    /// True when a write with this number has already been executed and
    /// must not run again (ADR 0003 decision 3).
    pub fn is_duplicate(&self, seq: u8) -> bool {
        self.get(seq).is_some_and(|h| h.executed)
    }

    /// Record a result, replacing any the plane held for that number.
    pub fn record(&mut self, seq: u8, code: ResultCode, executed: bool) {
        if let Some(slot) = self.held.iter_mut().find(|h| h.seq == seq) {
            slot.code = code;
            slot.executed |= executed;
            return;
        }
        if self.held.is_full() {
            let _ = self.held.pop_front();
        }
        let _ = self.held.push_back(Held {
            seq,
            code,
            executed,
        });
    }

    /// Finish a command that answered [`ResultCode::Accepted`].
    pub fn finish(&mut self, seq: u8, code: ResultCode) {
        self.record(seq, code, true);
    }

    /// The held results, oldest first: what STATUS 0x16 answers with.
    pub fn iter(&self) -> impl Iterator<Item = &Held> {
        self.held.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_get_consecutive_numbers_and_survive_reads() {
        let mut e = Events::new();
        assert!(!e.pending());
        let first = e.push(EventTarget::Plane, EventKind::Boot, 0);
        let second = e.push(EventTarget::Slot(3), EventKind::SlotOff, 1);
        assert_eq!((first, second), (1, 2));
        assert_eq!(e.next_seq(), 3);
        // Reading twice gives the same two events.
        assert_eq!(e.iter().count(), 2);
        assert_eq!(e.iter().count(), 2);
        assert!(e.pending());
        e.ack_through(1);
        assert_eq!(e.iter().count(), 1);
        e.ack_through(2);
        assert!(!e.pending());
    }

    #[test]
    fn an_overflow_drops_the_oldest_and_is_counted() {
        let mut e = Events::new();
        for i in 0..EVENT_CAPACITY as u32 + 5 {
            e.push(EventTarget::Plane, EventKind::Boot, i);
        }
        assert_eq!(e.len(), EVENT_CAPACITY);
        assert_eq!(e.lost(), 5);
        // The oldest kept is the sixth raised.
        assert_eq!(e.iter().next().unwrap().seq, 6);
        // The count clears once everything is acknowledged.
        e.ack_through(e.next_seq().wrapping_sub(1));
        assert_eq!(e.lost(), 0);
        assert!(!e.pending());
    }

    #[test]
    fn a_held_result_is_replaced_not_duplicated() {
        let mut r = Results::new();
        r.record(7, ResultCode::Accepted, true);
        assert!(r.is_duplicate(7));
        r.finish(7, ResultCode::Ok);
        assert_eq!(r.iter().count(), 1);
        assert_eq!(r.get(7).unwrap().code, ResultCode::Ok);
    }

    #[test]
    fn a_refused_write_is_not_a_duplicate() {
        let mut r = Results::new();
        r.record(9, ResultCode::Busy, false);
        assert!(!r.is_duplicate(9));
        assert_eq!(r.get(9).unwrap().code, ResultCode::Busy);
        r.record(9, ResultCode::Ok, true);
        assert!(r.is_duplicate(9));
    }

    #[test]
    fn only_the_last_eight_results_are_held() {
        let mut r = Results::new();
        for seq in 0..12u8 {
            r.record(seq, ResultCode::Ok, true);
        }
        assert_eq!(r.iter().count(), SEQ_MEMORY);
        assert!(r.get(3).is_none());
        assert!(r.get(11).is_some());
    }
}
