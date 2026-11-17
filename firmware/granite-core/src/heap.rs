//! The low-heap edge detector (ADR 0001 component 27, "Memory
//! budget").
//!
//! The ESP32-C6 has no external RAM and a Rust allocation that fails
//! aborts the firmware, so the margin is worth reporting before it is
//! gone. The platform samples the heap on a tick it already has and
//! hands the two figures that matter here; this decides whether that
//! sample is an edge worth one `sys` event.
//!
//! Two thresholds, because they fail differently: free heap is the
//! total left, the largest free block is what a single allocation can
//! still get, and a fragmented heap can have plenty of the first and
//! none of the second. Either one under its threshold is `heap_low`.
//!
//! Hysteresis is a quarter of the threshold on the way back up, so a
//! controller sitting exactly on the line does not publish an event per
//! sample. One event per crossing, report only: nothing here changes
//! what the firmware does.

use alloc::format;
use alloc::string::String;

/// Free heap under this many bytes is low.
pub const FREE_LOW: u32 = 32 * 1024;

/// A largest free block under this many bytes is low.
pub const LARGEST_LOW: u32 = 16 * 1024;

/// How far above a threshold a figure has to come back before the
/// condition clears: a quarter of the threshold.
const fn clear_at(threshold: u32) -> u32 {
    threshold + threshold / 4
}

/// What a sample changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeapEdge {
    /// The heap just went under a threshold.
    Low {
        /// Free heap at the crossing.
        free: u32,
        /// Largest free block at the crossing.
        largest: u32,
    },
    /// The heap is back over both thresholds plus the hysteresis.
    Ok {
        /// Free heap now.
        free: u32,
        /// Largest free block now.
        largest: u32,
    },
}

impl HeapEdge {
    /// The `what` of the `sys` event this edge publishes.
    pub const fn what(&self) -> &'static str {
        match self {
            HeapEdge::Low { .. } => "heap_low",
            HeapEdge::Ok { .. } => "heap_ok",
        }
    }

    /// The event's `detail`: the two figures and, for a crossing, which
    /// of them tripped.
    pub fn detail(&self) -> String {
        match self {
            HeapEdge::Low { free, largest } => {
                let which = match (*free < FREE_LOW, *largest < LARGEST_LOW) {
                    (true, true) => "free heap and the largest free block",
                    (true, false) => "free heap",
                    _ => "the largest free block",
                };
                format!("{which} is low: free {free} bytes, largest free block {largest} bytes")
            }
            HeapEdge::Ok { free, largest } => {
                format!("free {free} bytes, largest free block {largest} bytes")
            }
        }
    }
}

/// Remembers whether the heap counts as low, so an event goes out once
/// per crossing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HeapWatch {
    low: bool,
}

impl HeapWatch {
    /// A watch that starts in the healthy state: a board that boots
    /// already low reports `heap_low` on its first sample.
    pub const fn new() -> Self {
        HeapWatch { low: false }
    }

    /// True while the heap counts as low.
    pub const fn is_low(&self) -> bool {
        self.low
    }

    /// Feed one sample. `Some` only on a crossing.
    pub fn sample(&mut self, free: u32, largest: u32) -> Option<HeapEdge> {
        if self.low {
            if free >= clear_at(FREE_LOW) && largest >= clear_at(LARGEST_LOW) {
                self.low = false;
                return Some(HeapEdge::Ok { free, largest });
            }
            return None;
        }
        if free < FREE_LOW || largest < LARGEST_LOW {
            self.low = true;
            return Some(HeapEdge::Low { free, largest });
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PLENTY: (u32, u32) = (200 * 1024, 100 * 1024);

    #[test]
    fn plenty_of_heap_says_nothing() {
        let mut w = HeapWatch::new();
        assert_eq!(w.sample(PLENTY.0, PLENTY.1), None);
        assert_eq!(w.sample(PLENTY.0, PLENTY.1), None);
        assert!(!w.is_low());
    }

    #[test]
    fn one_event_per_crossing() {
        let mut w = HeapWatch::new();
        let edge = w.sample(20 * 1024, 18 * 1024).expect("a crossing");
        assert_eq!(edge.what(), "heap_low");
        assert!(w.is_low());
        // Still low, and lower: no second event.
        assert_eq!(w.sample(12 * 1024, 9 * 1024), None);
        let back = w.sample(PLENTY.0, PLENTY.1).expect("the way back");
        assert_eq!(back.what(), "heap_ok");
        assert!(!w.is_low());
        assert_eq!(w.sample(PLENTY.0, PLENTY.1), None);
    }

    #[test]
    fn a_fragmented_heap_trips_on_the_largest_block() {
        let mut w = HeapWatch::new();
        let edge = w.sample(120 * 1024, 15 * 1024).expect("a crossing");
        assert_eq!(edge.what(), "heap_low");
        assert!(edge.detail().contains("largest free block"));
    }

    #[test]
    fn the_hysteresis_holds_the_condition() {
        let mut w = HeapWatch::new();
        assert!(w.sample(FREE_LOW - 1, 100 * 1024).is_some());
        // Over the threshold but inside the hysteresis band.
        assert_eq!(w.sample(FREE_LOW + 1, 100 * 1024), None);
        assert_eq!(w.sample(clear_at(FREE_LOW) - 1, 100 * 1024), None);
        assert!(w.sample(clear_at(FREE_LOW), 100 * 1024).is_some());
    }

    #[test]
    fn both_figures_have_to_recover() {
        let mut w = HeapWatch::new();
        assert!(w.sample(10 * 1024, 8 * 1024).is_some());
        // Free heap is back, the block is not.
        assert_eq!(w.sample(200 * 1024, clear_at(LARGEST_LOW) - 1), None);
        assert!(w.sample(200 * 1024, clear_at(LARGEST_LOW)).is_some());
    }

    #[test]
    fn the_detail_names_both_figures() {
        let mut w = HeapWatch::new();
        let edge = w.sample(1_024, 512).expect("a crossing");
        let detail = edge.detail();
        assert!(detail.contains("1024"), "{detail}");
        assert!(detail.contains("512"), "{detail}");
    }
}
