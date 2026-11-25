//! Switch-on blanking of a slot's OV latch (ADR 0002 decision 7c, dated
//! 2026-11-25).
//!
//! ## Why
//!
//! The power-up clear of [`crate::power_up_clear`] fixed the unit's
//! *power-up*: the module's OV comparator glitches OV_KILL low while its
//! own VAUX comes up, the unit's RC clear is long over by then, and both
//! slots came up latched. The same glitch happens at **every** switch-on
//! of a slot's rail, because every switch-on is the first time *that*
//! module gets VIN since it last lost it. Measured on the bench
//! (2026-11-25, after the power-up clear was in): a commanded `on` on a
//! slot whose rail had been switched off latched that slot at once.
//!
//! ## The rule
//!
//! Whenever the plane releases a slot's OFF line - every path goes
//! through [`crate::plane::Plane::set_slot_off`], so there is one place
//! to do it - and the slot's latch is **not** set at that moment, LCLR is
//! asserted first and held until [`SWITCH_ON_BLANK_MS`] after OFF was
//! released. The 74LVC1G74 (unit.py U3 / U53) then rides out the glitch:
//! while the module pulls `/PRE` low its Q is high whatever `/CLR` does,
//! so the slot is cut for the width of the glitch, and the moment OV_KILL
//! goes back high the held `/CLR` puts Q low again and the slot stays on.
//! A clear released while OV_KILL is *still* low would leave Q high -
//! which is exactly what the bench saw.
//!
//! If the latch **is** set when the switch-on is asked for, nothing is
//! blanked and nothing is cleared: the slot stays latched and the action
//! reports as it always did. Latches are cleared by `clear_latch` and by
//! the power-up clear, and by nothing else (ADR 0002 decision 7).
//!
//! ## What it is not
//!
//! It is not a sequence. It owns no [`crate::sequencer::Phase`], answers
//! no command, emits no result and no event, so whatever the switch-on
//! was part of - the wake press of `node on`, the LED wait of a
//! power-cycle, the next slot of a stagger - runs on undisturbed while
//! LCLR is held. The only thing it says out loud is one log line, when
//! the window ends with the latch set anyway: that is a real overvoltage
//! or a glitch wider than the window, and the slot stays latched.
//!
//! The line itself is shared with the clear of
//! [`crate::sequencer::Phase::ClearLatch`] and with the hold while the
//! slot is off (ADR 0002 decision 7d): whichever of them holds LCLR
//! last is the one that releases it, and
//! [`crate::plane::Plane::lclr_wanted`] is the list of holders.
//!
//! ## What decision 7d changed here
//!
//! The bench then found that a switched-off slot latches on its own,
//! about 10 ms after the cut: its module's input sits at about 1.2 V,
//! the comparator runs under its minimum supply and OV_KILL can go low
//! at any moment while the slot is off. The slot was therefore already
//! latched at the next switch-on and the rule above, by its own "a slot
//! that is already latched is not touched", did nothing. Decision 7d
//! keeps LCLR asserted for as long as OFF is high, so the latch cannot
//! be set while the slot is off and the window below is what takes over
//! at the switch-on - which is why the window is still the thing that
//! releases the line.

/// How long LCLR is held after a slot's OFF line was released \[ms\].
///
/// It has to outlast the module's OV_KILL glitch, which is the few ms of
/// VAUX coming up through 47k into ~1.3 uF plus whatever the slot's own
/// hot-swap takes to bring VIN there. The glitch has never been measured
/// directly (see docs/pm-interface.md), so this starts at 100 ms - two
/// bus cycles, and the same order as the power-up clear's settle time -
/// and is a one-line change.
///
/// **The trade-off**: while LCLR is held the slot's OV latch cannot keep
/// the slot cut, so for this long after every switch-on a real
/// overvoltage would not cut it. The module's own comparator still sees
/// the overvoltage and still pulls OV_KILL, so the slot is cut for as
/// long as the module pulls - what the blanking takes away is the
/// *latching*, not the cut. It is bounded, it only ever happens in the
/// first moments of a rail that is coming up anyway, and the hardware
/// latch takes over the instant the window ends.
///
/// **Resolution**: the line comes down at the first
/// [`crate::Plane::tick`] at or after the deadline, so the real window is
/// this value rounded up to the bus task's period
/// ([`crate::model::POLL_PERIOD_MS`], 50 ms).
pub const SWITCH_ON_BLANK_MS: u32 = 100;

/// One slot's switch-on blanking window.
///
/// Plain data, no clock and no bus: the sequencer starts it when it
/// releases OFF and ends it from the tick, so the policy is testable
/// without a plane.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SwitchOnBlank {
    /// When LCLR may come down \[ms\], or `None` while nothing blanks.
    until_ms: Option<u64>,
}

impl SwitchOnBlank {
    /// A slot with no window open.
    pub const fn new() -> Self {
        SwitchOnBlank { until_ms: None }
    }

    /// Open the window at `now_ms`, which is the moment OFF is released.
    ///
    /// A second switch-on inside an open window simply moves the
    /// deadline out: the module got VIN again, so the glitch can come
    /// again.
    pub const fn start(&mut self, now_ms: u64) {
        self.until_ms = Some(now_ms + SWITCH_ON_BLANK_MS as u64);
    }

    /// True while LCLR is held for a switch-on.
    pub const fn active(&self) -> bool {
        self.until_ms.is_some()
    }

    /// True while the window is open and its deadline has passed.
    pub const fn over(&self, now_ms: u64) -> bool {
        match self.until_ms {
            Some(until) => now_ms >= until,
            None => false,
        }
    }

    /// Close the window.
    pub const fn end(&mut self) {
        self.until_ms = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The window is a window in which the latch cannot hold the slot
    /// cut, so it stays short - and it is no use at all if it is shorter
    /// than the bus cycle that has to end it.
    const _TIMING: () = {
        assert!(SWITCH_ON_BLANK_MS >= crate::model::POLL_PERIOD_MS);
        assert!(SWITCH_ON_BLANK_MS < 1_000);
    };

    #[test]
    fn a_window_ends_at_its_deadline() {
        let mut blank = SwitchOnBlank::new();
        assert!(!blank.active());
        assert!(!blank.over(10_000), "a closed window never ends");

        blank.start(1_000);
        assert!(blank.active());
        assert!(!blank.over(1_000));
        assert!(!blank.over(1_000 + SWITCH_ON_BLANK_MS as u64 - 1));
        assert!(blank.over(1_000 + SWITCH_ON_BLANK_MS as u64));

        blank.end();
        assert!(!blank.active() && !blank.over(100_000));
    }

    #[test]
    fn a_second_switch_on_moves_the_deadline_out() {
        let mut blank = SwitchOnBlank::new();
        blank.start(0);
        blank.start(500);
        assert!(!blank.over(500 + SWITCH_ON_BLANK_MS as u64 - 1));
        assert!(blank.over(500 + SWITCH_ON_BLANK_MS as u64));
    }
}
