//! The unit hot-swap latch: the firmware half of an LM5069-**2**
//! (ADR 0005 decision 3, "Latch in firmware").
//!
//! The unit input stage (power/pp-48/tools/sch/blocks/uinput.py, U207)
//! is an auto-retry part: after a fault it restarts on its own at about
//! 0.5 % duty, for ever, and nothing in the hardware remembers that it
//! ever faulted. The latch-off that the -1 used to do is this module's
//! job: U_PGD (P27) falling while the unit's input is inside the
//! UVLO..OVLO window, and while the core is not itself holding the unit
//! off, is a unit fault. The core then drives U_OFF (P13) high - Q204
//! pulls UVLO low - and keeps it there until [`crate::Plane`] is told
//! `unit_reset` (CTRL 0x4A) or loses its own power.
//!
//! U_PGD low with the input *out* of range is the bus and not a fault:
//! the unit's own UVLO or OVLO opened the FETs and will close them again
//! when the bus comes back, so nothing is latched.
//!
//! ## Which voltage is "the input"
//!
//! Not the unit's own INA237. On the board (uinput.py U208) IN+ sits on
//! VIN and IN- on USENSE, but the **VBUS pin is on the hot-swap output**,
//! so [`crate::drivers::Ina238::unit`]'s VBUS register - the only
//! absolute voltage the part reports, see
//! [`crate::drivers::ina238`] - reads ~0 V exactly when the hot-swap is
//! open, which is every moment this module has to judge. The INA237 can
//! never witness its own input during a fault.
//!
//! What can: the core's own 48 V input (`CorePins::core_input_10mv`,
//! STATUS 0x11). Core and units hang on the same bus behind the same
//! breaker (ADR 0005 decision 6 and its chain drawing), the core reads
//! it at 20 Hz through the 220k / 10k divider, and that reading is live
//! whatever any unit's hot-swap is doing. So the bus witness is the core
//! input, and a core that has no reading yet (`core_volts == 0`, before
//! the first conversion) latches nothing.
//!
//! The one case this gets wrong is a unit whose *own* XT60 came out
//! while the rest of the bus is up: U_PGD falls with the bus in range
//! and the unit is latched off. That is deliberate - an input that
//! disappeared under load is worth an operator's `unit_reset` - and it
//! is the only reading the hardware allows.
//!
//! ## Timing
//!
//! Two numbers, both in whole 50 ms poll cycles:
//!
//! - [`FAULT_DEBOUNCE_MS`] (500 ms) is how long U_PGD must stay low
//!   before the core calls it a fault. It has to be longer than the
//!   160 ms insertion time of the stage (C203 220n), or every normal
//!   start would latch on its own ramp: a unit just plugged in, a
//!   `unit_reset` release, a core boot. It has to be shorter than the
//!   ~2 s retry period, so the latch lands in the first gap between
//!   restart attempts: the unit gets one retry (~10 ms of fault timer)
//!   before the core reacts, which is what ADR 0005 decision 3 budgets.
//! - [`RECOVER_MS`] (300 ms) is how long U_PGD must stay *high* before a
//!   fault window that was running is forgotten. A restart attempt is at
//!   most the 10 ms fault timer long, so a retry pulse that the 20 Hz
//!   poll happens to catch cannot clear the window and cannot stop the
//!   latch; a unit that really came back holds PGD for ever and clears
//!   it after six cycles.
//!
//! There is no separate rule for a core that boots with a unit already
//! retrying: the same window does it. A unit that is merely starting
//! reaches PGD within its insertion time and is never latched; a unit
//! that is retrying never does, and is latched 500 ms after the first
//! poll that reads it.

use granite_proto::{EventKind, EventTarget, UnitStatus};

use crate::device::Devices;
use crate::drivers::expander;
use crate::log::level;
use crate::plane::Plane;

/// The unit hot-swap's UVLO rising threshold \[10 mV\] (uinput.py R233 /
/// R234: 38.3 V).
pub const UVLO_ON_10MV: u16 = 3_830;

/// Its UVLO falling threshold \[10 mV\] (35.3 V).
pub const UVLO_OFF_10MV: u16 = 3_530;

/// Its OVLO threshold \[10 mV\] (uinput.py R235 / R236: 64.6 V).
pub const OVLO_10MV: u16 = 6_460;

/// Guard band kept away from both thresholds \[10 mV\] (2 V).
///
/// The core's divider, the unit's divider and the LM5069's own
/// comparator each have their tolerance, so the core only calls the
/// input "in range" where no reasonable part could disagree. Inside the
/// band nothing is latched, which is the safe way round: the unit keeps
/// retrying, as it would with no core at all.
pub const THRESHOLD_MARGIN_10MV: u16 = 200;

/// How long U_PGD must stay low, with the input in range, before the
/// core calls it a unit fault \[ms\].
pub const FAULT_DEBOUNCE_MS: u64 = 500;

/// How long U_PGD must stay high before a fault window is forgotten
/// \[ms\].
pub const RECOVER_MS: u64 = 300;

/// True while `volts_10mv` is a bus the unit's hot-swap should be
/// conducting on.
///
/// 0 is "no reading", not "no volts": the core's ADC has no conversion
/// yet at the first tick after a boot.
pub const fn input_in_range(volts_10mv: u16) -> bool {
    volts_10mv >= UVLO_ON_10MV + THRESHOLD_MARGIN_10MV
        && volts_10mv <= OVLO_10MV - THRESHOLD_MARGIN_10MV
}

/// What the poll cycle should do about one unit after a sample.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LatchAction {
    /// Nothing changed.
    Nothing,
    /// The unit faulted: drive U_OFF high and keep it there.
    Latch,
}

/// One unit's latch.
///
/// Plain data, no clock and no bus: [`UnitLatch::sample`] is fed from
/// the poll cycle and the caller does the driving, so the whole policy
/// is testable without a plane.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct UnitLatch {
    /// The core holds this unit off after a fault.
    held: bool,
    /// When U_PGD first read low with the input in range \[ms\], while a
    /// fault window runs.
    low_since_ms: Option<u64>,
    /// When U_PGD last read low \[ms\].
    last_low_ms: u64,
}

impl UnitLatch {
    /// A latch that holds nothing.
    pub const fn new() -> Self {
        UnitLatch {
            held: false,
            low_since_ms: None,
            last_low_ms: 0,
        }
    }

    /// True while the core holds this unit's hot-swap off.
    pub const fn held(&self) -> bool {
        self.held
    }

    /// Adopt the state the backup registers carry at boot: U_OFF in the
    /// intended state can only be this latch (ADR 0005 decision 3, and
    /// nothing else writes that bit), so a reset that kept VDD comes
    /// back still holding.
    pub fn restore(&mut self, held: bool) {
        *self = UnitLatch {
            held,
            ..UnitLatch::new()
        };
    }

    /// Take the hold back without a sample: a failed release, so the
    /// unit stays where it was.
    pub fn hold(&mut self) {
        self.held = true;
        self.low_since_ms = None;
    }

    /// Let go (CTRL 0x4A). The fault window starts from scratch, so the
    /// unit's 160 ms of insertion after U_OFF drops is not mistaken for
    /// the fault that is being cleared.
    pub fn release(&mut self) {
        *self = UnitLatch::new();
    }

    /// Feed one 20 Hz sample of U_PGD and the bus the core reads.
    ///
    /// Returns [`LatchAction::Latch`] once, on the cycle the fault
    /// window runs out.
    pub fn sample(&mut self, now_ms: u64, pgd: bool, input_10mv: u16) -> LatchAction {
        // Once held, only `release` lets go: a retry pulse cannot reach
        // this, since U_OFF keeps the part's UVLO low, but a unit that
        // somehow shows PGD anyway does not get to unlatch itself.
        if self.held {
            return LatchAction::Nothing;
        }
        if !input_in_range(input_10mv) {
            // The bus, not a fault (ADR 0005 decision 3).
            self.low_since_ms = None;
            return LatchAction::Nothing;
        }
        if pgd {
            if self
                .low_since_ms
                .is_some_and(|_| now_ms.saturating_sub(self.last_low_ms) >= RECOVER_MS)
            {
                self.low_since_ms = None;
            }
            return LatchAction::Nothing;
        }
        self.last_low_ms = now_ms;
        let since = *self.low_since_ms.get_or_insert(now_ms);
        if now_ms.saturating_sub(since) >= FAULT_DEBOUNCE_MS {
            self.held = true;
            self.low_since_ms = None;
            return LatchAction::Latch;
        }
        LatchAction::Nothing
    }
}

impl Plane {
    /// Judge one unit's U_PGD against its input, and hold the unit off
    /// when the window runs out.
    ///
    /// Called from the poll cycle with the unit's channel selected.
    pub(crate) fn judge_unit_hot_swap<D: Devices>(&mut self, dev: &mut D, unit: usize, pgd: bool) {
        let action = self.unit_latch[unit].sample(self.now_ms, pgd, self.core_volts);
        if action == LatchAction::Latch {
            self.note(level::ERROR, "unit fault, hot-swap held off", Some(unit as u32));
            self.raise(
                EventTarget::Unit(unit as u8),
                EventKind::UnitHotSwapLatchedOff,
            );
            self.intended.set_u_off(unit as u8, true);
            self.save_intended(dev);
        }
        // Whether the hold was just taken or has been in force for a
        // week, the line has to be out. Driving it from the state and
        // not from the transition is what repairs a write the bus ate
        // and what re-asserts the hold after an expander reset.
        if self.unit_latch[unit].held() && !self.expanders[unit].get_output(expander::LINE_U_OFF) {
            self.drive_unit_line(dev, unit, expander::LINE_U_OFF, true);
        }
    }

    /// The status bits a unit's hot-swap stands at.
    pub(crate) fn unit_hot_swap_status(&self, unit: usize, pgd: bool) -> UnitStatus {
        self.units[unit]
            .status
            .set(UnitStatus::HOT_SWAP_PGD, pgd)
            .set(UnitStatus::U_OFF_ASSERTED, self.intended.u_off(unit as u8))
            .set(
                UnitStatus::HOT_SWAP_LATCHED_OFF,
                self.unit_latch[unit].held(),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 48 V, the bus every test runs on unless it says otherwise.
    const BUS: u16 = 4_800;

    /// Feed `ms` of samples at the poll period and return the last
    /// action that was not `Nothing`.
    fn feed(latch: &mut UnitLatch, from_ms: u64, ms: u64, pgd: bool, bus: u16) -> (u64, LatchAction) {
        let mut now = from_ms;
        let mut seen = LatchAction::Nothing;
        let end = from_ms + ms;
        while now < end {
            now += crate::model::POLL_PERIOD_MS as u64;
            if latch.sample(now, pgd, bus) == LatchAction::Latch {
                seen = LatchAction::Latch;
            }
        }
        (now, seen)
    }

    /// uinput.py C203: 160 ms insertion, 10 ms fault timer, ~0.5 % duty,
    /// so ~2 s between restart attempts. The windows have to sit
    /// between those, and the compiler is the right place to say so.
    const _TIMING: () = {
        assert!(FAULT_DEBOUNCE_MS > 160);
        assert!(FAULT_DEBOUNCE_MS < 2_000);
        assert!(RECOVER_MS > 10);
        assert!(RECOVER_MS < FAULT_DEBOUNCE_MS);
    };

    #[test]
    fn the_window_covers_the_thresholds_with_their_margin() {
        assert!(input_in_range(BUS));
        assert!(input_in_range(4_030) && !input_in_range(4_029));
        assert!(input_in_range(6_260) && !input_in_range(6_261));
        // No reading is not "no volts".
        assert!(!input_in_range(0));
        // Both sides of the unit's own thresholds are out.
        assert!(!input_in_range(UVLO_OFF_10MV));
        assert!(!input_in_range(OVLO_10MV));
    }

    #[test]
    fn a_fault_in_range_latches_after_the_window() {
        let mut latch = UnitLatch::new();
        let (now, action) = feed(&mut latch, 0, FAULT_DEBOUNCE_MS - 100, false, BUS);
        assert_eq!(action, LatchAction::Nothing, "not yet");
        assert!(!latch.held());
        let (_, action) = feed(&mut latch, now, 200, false, BUS);
        assert_eq!(action, LatchAction::Latch);
        assert!(latch.held());
    }

    #[test]
    fn a_bus_dip_is_not_a_fault() {
        let mut latch = UnitLatch::new();
        // Ten seconds of PGD low with the bus under the unit's UVLO.
        let (now, action) = feed(&mut latch, 0, 10_000, false, 3_000);
        assert_eq!(action, LatchAction::Nothing);
        assert!(!latch.held());
        // And over the OVLO.
        let (now, action) = feed(&mut latch, now, 10_000, false, 6_600);
        assert_eq!(action, LatchAction::Nothing);
        assert!(!latch.held());
        // The bus comes back and the unit does not: now it is a fault.
        let (_, action) = feed(&mut latch, now, 1_000, false, BUS);
        assert_eq!(action, LatchAction::Latch);
    }

    #[test]
    fn a_dip_part_way_through_the_window_starts_it_again() {
        let mut latch = UnitLatch::new();
        let (now, _) = feed(&mut latch, 0, 400, false, BUS);
        // One sample out of range forgets the window.
        let (now, _) = feed(&mut latch, now, 50, false, 3_000);
        let (now, action) = feed(&mut latch, now, 400, false, BUS);
        assert_eq!(action, LatchAction::Nothing, "the window restarted");
        let (_, action) = feed(&mut latch, now, 200, false, BUS);
        assert_eq!(action, LatchAction::Latch);
    }

    #[test]
    fn a_retry_pulse_does_not_stop_the_latch() {
        let mut latch = UnitLatch::new();
        let mut now = 0u64;
        // 400 ms of fault, then one caught restart attempt (a single
        // 50 ms sample reading PGD high), then more fault.
        let (next, _) = feed(&mut latch, now, 400, false, BUS);
        now = next;
        now += crate::model::POLL_PERIOD_MS as u64;
        assert_eq!(latch.sample(now, true, BUS), LatchAction::Nothing);
        let (_, action) = feed(&mut latch, now, 150, false, BUS);
        assert_eq!(action, LatchAction::Latch, "the pulse was not a recovery");
    }

    #[test]
    fn pgd_held_high_forgets_the_window() {
        let mut latch = UnitLatch::new();
        let (now, _) = feed(&mut latch, 0, 400, false, BUS);
        let (now, _) = feed(&mut latch, now, RECOVER_MS + 100, true, BUS);
        let (_, action) = feed(&mut latch, now, 400, false, BUS);
        assert_eq!(action, LatchAction::Nothing, "the window started again");
    }

    #[test]
    fn a_held_latch_never_lets_go_on_its_own() {
        let mut latch = UnitLatch::new();
        feed(&mut latch, 0, 1_000, false, BUS);
        assert!(latch.held());
        // Retry pulses, a dead bus, a long time: it stays held.
        feed(&mut latch, 1_000, 60_000, true, BUS);
        assert!(latch.held());
        feed(&mut latch, 61_000, 10_000, true, 0);
        assert!(latch.held());
        latch.release();
        assert!(!latch.held());
    }

    #[test]
    fn a_release_does_not_latch_on_the_insertion_ramp() {
        let mut latch = UnitLatch::new();
        feed(&mut latch, 0, 1_000, false, BUS);
        latch.release();
        // 160 ms of insertion with PGD still low, then the unit is up.
        let (now, action) = feed(&mut latch, 1_000, 200, false, BUS);
        assert_eq!(action, LatchAction::Nothing);
        let (_, action) = feed(&mut latch, now, 1_000, true, BUS);
        assert_eq!(action, LatchAction::Nothing);
        assert!(!latch.held());
    }

    #[test]
    fn a_restored_latch_is_held_and_clean() {
        let mut latch = UnitLatch::new();
        latch.restore(true);
        assert!(latch.held());
        assert_eq!(latch.sample(50, false, BUS), LatchAction::Nothing);
        let mut latch = UnitLatch::new();
        latch.restore(false);
        assert!(!latch.held());
    }
}
