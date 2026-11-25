//! The power-up clear of the slot OV latches (ADR 0002 decision 7, dated
//! note of 2026-11-25).
//!
//! ## Why a unit needs one
//!
//! Each slot's OV latch (power/pp-48-fd/tools/sch/blocks/unit.py, U3 /
//! U53, a 74LVC1G74) has two asynchronous inputs. `/PRE` is the OV_KILL
//! node, pulled low by the module's own overvoltage comparator, by the
//! INA237's ALERT or by the PGOOD-loss trip Q6. `/CLR` is an RC
//! power-on clear - R11 100k over C5 100n, about 10 ms after the unit's
//! 3V3, which the unit gets from the core over the chain - in parallel
//! with Q4, the expander's LCLR line.
//!
//! The module's comparator (power/pm-19v-250w-b/tools/gensch.py, the
//! MCP65R46 on VAUX) runs from VIN through 47k into ~1.3 uF with a 4.7 V
//! zener. For the first ~2-3 ms after the slot hands the module VIN that
//! supply is below the part's minimum and OV_KILL glitches low. The
//! unit's RC clear is long finished by then: the 3V3 arrives with the
//! chain, and only afterwards does the unit's own hot-swap (LM5069-2)
//! and then the slot's hot-swap bring VIN to the module. So the glitch
//! lands *after* the only clear the hardware does by itself, and both
//! slots come up latched with the module's VIN at a volt or so.
//!
//! Measured on the first unit-fd with modules (2026-11-25): at every
//! power-up both slots read `ov_latched` with the module's VIN at 1.3 V
//! and OV_KILL sitting at 3.3 V statically; a commanded `clear_latch`
//! then left the slot ON with VIN 48.1 V and the output at 19.1 V.
//!
//! ## What this module does
//!
//! It watches the unit's input power-good, U_PGD (P27) - the same signal
//! [`crate::unit_latch`] judges - and on every rising edge, boot
//! included, arms one LCLR pulse per slot of that unit, due
//! [`SETTLE_MS`] later. The pulse itself is the existing clear-latch
//! mechanism of [`crate::sequencer`] ([`crate::sequencer::Phase::ClearLatch`],
//! [`crate::sequencer::LCLR_PULSE_MS`]), so the timing and the per-slot
//! conflict rules are the ones a commanded `clear_latch` already obeys.
//!
//! What it is **not**: an automatic fault reset. It runs once per U_PGD
//! rising edge and never on a latch that sets while the unit is up, so a
//! real overvoltage still latches the slot and stays latched until the
//! controller clears it (ADR 0002 decision 7). It does not touch the
//! intended state either: a slot commanded OFF stays OFF, because LCLR
//! only releases the latch's Q and the slot's OFF line is untouched.
//!
//! Two cases are left alone:
//!
//! - A unit the firmware unit latch holds off ([`crate::unit_latch`]):
//!   its slots carry nothing and its U_OFF is out on purpose.
//! - A slot that is busy with a sequence when the pulse comes due. It is
//!   **skipped**, with a log line, rather than deferred: deferring would
//!   fire the pulse at an unbounded later moment, and a latch that set
//!   in the meantime - a real fault, while the unit was up - must not be
//!   cleared by it. A slot that is mid-sequence 100 ms after its unit's
//!   input came up is being driven by the controller anyway, which can
//!   send `clear_latch` itself.

/// How long after U_PGD rises the plane waits before it clears the slot
/// OV latches \[ms\].
///
/// It has to outlast the module's comparator start-up, which is the
/// 2-3 ms of VAUX coming up through 47k plus whatever the slot's own
/// hot-swap takes to bring VIN there (the slot stage's insertion time is
/// of the same order as the unit's 160 ms, so the number is dominated by
/// the chain of hot-swaps, not by the comparator). 100 ms is two poll
/// cycles and still inside the window in which nothing else can have
/// commanded the slot. The glitch width itself has never been measured;
/// see docs/pm-interface.md.
pub const SETTLE_MS: u64 = 100;

/// One unit's power-up clear.
///
/// Plain data, no clock and no bus: the poll cycle feeds
/// [`PowerUpClear::sample`] and the sequencer asks whether a pulse is
/// due, so the whole policy is testable without a plane.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PowerUpClear {
    /// U_PGD as the last sample read it.
    pgd: bool,
    /// Halves of this unit still to clear: bit 0 is slot A, bit 1 slot B.
    pending: u8,
    /// When the pulses may go out \[ms\].
    due_ms: u64,
}

impl PowerUpClear {
    /// A unit whose input has never read good, so the first high sample
    /// is a rising edge. This is also the state at boot, which is why
    /// the clear runs once per boot.
    pub const fn new() -> Self {
        PowerUpClear {
            pgd: false,
            pending: 0,
            due_ms: 0,
        }
    }

    /// Feed one poll sample of U_PGD. Returns true on the rising edge
    /// that armed the clear.
    pub fn sample(&mut self, now_ms: u64, pgd: bool) -> bool {
        let rising = pgd && !self.pgd;
        self.pgd = pgd;
        if rising {
            self.pending = 0b11;
            self.due_ms = now_ms + SETTLE_MS;
        }
        rising
    }

    /// True while a pulse is armed and its settle time has passed.
    pub const fn due(&self, now_ms: u64) -> bool {
        self.pending != 0 && now_ms >= self.due_ms
    }

    /// True while half `half` (0 = A, 1 = B) still wants its pulse.
    pub const fn wants(&self, half: usize) -> bool {
        self.pending & (1 << half) != 0
    }

    /// Mark one half dealt with, whether the pulse went out or the slot
    /// was skipped: the clear is once per rising edge either way.
    pub fn taken(&mut self, half: usize) {
        self.pending &= !(1u8 << half);
    }

    /// Drop the whole unit's clear without firing it.
    pub fn cancel(&mut self) {
        self.pending = 0;
    }

    /// True while anything is still armed, for a test or a status line.
    pub const fn armed(&self) -> bool {
        self.pending != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The settle time has to be a whole number of poll cycles' worth of
    /// waiting, and short enough that nothing else can have commanded
    /// the slot in between.
    const _TIMING: () = {
        assert!(SETTLE_MS >= crate::model::POLL_PERIOD_MS as u64);
        assert!(SETTLE_MS < 1_000);
    };

    #[test]
    fn a_fresh_unit_arms_on_its_first_good_sample() {
        let mut clear = PowerUpClear::new();
        assert!(!clear.armed());
        assert!(clear.sample(0, true), "the first high sample is an edge");
        assert!(clear.armed());
        assert!(clear.wants(0) && clear.wants(1));
        assert!(!clear.due(0));
        assert!(!clear.due(SETTLE_MS - 1));
        assert!(clear.due(SETTLE_MS));
    }

    #[test]
    fn a_level_that_does_not_change_arms_nothing() {
        let mut clear = PowerUpClear::new();
        clear.sample(0, true);
        clear.cancel();
        for t in 1..50u64 {
            assert!(!clear.sample(t * 50, true), "no edge at t = {t}");
        }
        assert!(!clear.armed());
    }

    #[test]
    fn a_falling_then_rising_input_arms_once_more() {
        let mut clear = PowerUpClear::new();
        clear.sample(0, true);
        clear.taken(0);
        clear.taken(1);
        assert!(!clear.armed());
        // The unit's input stage drops out and comes back.
        assert!(!clear.sample(1_000, false));
        assert!(clear.sample(2_000, true));
        assert!(clear.wants(0) && clear.wants(1));
        assert!(clear.due(2_000 + SETTLE_MS));
    }

    #[test]
    fn each_half_is_taken_on_its_own() {
        let mut clear = PowerUpClear::new();
        clear.sample(0, true);
        clear.taken(1);
        assert!(clear.wants(0) && !clear.wants(1));
        assert!(clear.due(SETTLE_MS));
        clear.taken(0);
        assert!(!clear.armed());
        assert!(!clear.due(10_000));
    }
}
