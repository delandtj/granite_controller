//! The plane model: what the poll cycle writes and the link reads
//! (ADR 0002 decision 5, "The plane model keeps per slot V, I, P, min /
//! max since last read, energy ..., temperature, module ID and class,
//! status bits and the LED state").
//!
//! One [`SlotModel`] per slot 0-7 and one [`UnitModel`] per unit 0-3,
//! all plain data: the drivers fill them in, the link encodes them, and
//! nothing here touches a bus. Everything is in the link's units
//! (ADR 0003 decision 3: V in 10 mV, I in mA, P in 0.1 W, T in 0.1 C),
//! so encoding a block is a copy and not a conversion.

use granite_proto::{CrossCheck, LedMode, SequenceState, SlotStatus, UnitStatus};

use crate::config::{PlaneConfig, Retry};
use crate::drivers::Sample;

/// How often the 20 Hz cycle samples a slot \[ms\] (ADR 0002 decision
/// 5).
pub const POLL_PERIOD_MS: u32 = 50;

/// The host's power LED as the plane reads it off the opto.
///
/// The line is inverted in hardware (unit.py U8: "LED lit -> output
/// low"), so the caller passes `lit`, not the pin level. Blinking is
/// recognised after two edges whose spacing is inside the configured
/// window, and the period reported is the full cycle, which is twice
/// that spacing. A steady line is called on or off once it has held for
/// longer than the window's maximum, so a slow breathing LED is not
/// mistaken for off before then (ADR 0002, "What an expert would ask").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Led {
    /// The last sample.
    pub lit: bool,
    /// Off, on or blinking.
    pub mode: LedMode,
    /// Blink period \[ms\], 0 unless blinking.
    pub period_ms: u16,
    /// When the line last changed \[ms since boot\].
    last_edge_ms: u64,
    /// Edges seen since the line was last called steady.
    edges: u8,
}

impl Default for Led {
    fn default() -> Self {
        Led {
            lit: false,
            mode: LedMode::Off,
            period_ms: 0,
            last_edge_ms: 0,
            edges: 0,
        }
    }
}

impl Led {
    /// Feed one 20 Hz sample. Returns true when the reported mode or
    /// period changed, which is an event (EVENTS kind 7).
    pub fn sample(&mut self, now_ms: u64, lit: bool, cfg: &granite_proto::BlinkDetection) -> bool {
        let before = (self.mode, self.period_ms);
        if lit != self.lit {
            let spacing = now_ms.saturating_sub(self.last_edge_ms);
            self.lit = lit;
            self.last_edge_ms = now_ms;
            self.edges = self.edges.saturating_add(1);
            let period = spacing.saturating_mul(2);
            if self.edges >= 2
                && period >= cfg.min_period_ms as u64
                && period <= cfg.max_period_ms as u64
            {
                self.mode = LedMode::Blinking;
                self.period_ms = period as u16;
            }
        } else if now_ms.saturating_sub(self.last_edge_ms) > cfg.max_period_ms as u64 {
            // Held long enough to be steady.
            self.mode = if lit { LedMode::On } else { LedMode::Off };
            self.period_ms = 0;
            self.edges = 0;
        }
        before != (self.mode, self.period_ms)
    }

    /// Treat the line as steady from now, without waiting out the
    /// window: used when a sequence has just switched a rail, so the old
    /// blink is not carried over the change.
    pub fn restart(&mut self, now_ms: u64, lit: bool) {
        self.lit = lit;
        self.last_edge_ms = now_ms;
        self.edges = 0;
        self.mode = if lit { LedMode::On } else { LedMode::Off };
        self.period_ms = 0;
    }

    /// True while the host shows any light: on or blinking. The off
    /// sequences wait for this to go false.
    pub const fn shows_light(&self) -> bool {
        !matches!(self.mode, LedMode::Off)
    }
}

/// One slot (2 x unit + A/B), everything the SLOT and SUMMARY blocks
/// answer with.
#[derive(Debug, Clone, Copy)]
pub struct SlotModel {
    /// The status bits of SLOT 0x20.
    pub status: SlotStatus,
    /// Voltage \[10 mV\].
    pub volts: u16,
    /// Current \[mA\].
    pub amps: u16,
    /// Power \[0.1 W\].
    pub watts: u16,
    /// Smallest power since the last read \[0.1 W\]; [`u16::MAX`] while
    /// nothing has been sampled.
    pub watts_min: u16,
    /// Largest power since the last read \[0.1 W\].
    pub watts_max: u16,
    /// INA238 die temperature \[0.1 C\].
    pub temperature: i16,
    /// Energy \[uJ\]: integrated from the power samples against the
    /// millisecond count the firmware passes to `tick`. Reported in
    /// joules with the boot count, so a consumer sees a reset.
    pub energy_uj: u64,
    /// The ID ADC's raw count.
    pub module_id_raw: u16,
    /// The band the count falls in.
    pub module_class: u8,
    /// The host's power LED.
    pub led: Led,
    /// LED against rail cross-checks.
    pub cross: CrossCheck,
    /// What the sequencer is doing with this slot.
    pub seq_state: SequenceState,
    /// PGD drops counted in the retry window.
    pub retry_count: u16,
    /// The power limit in force \[0.1 W\].
    pub power_limit: u16,
    /// When the retry window started \[ms since boot\].
    pub retry_window_ms: u64,
    /// When the slot was last sampled, for the energy integral.
    last_sample_ms: u64,
}

impl Default for SlotModel {
    fn default() -> Self {
        SlotModel {
            status: SlotStatus::NONE,
            volts: 0,
            amps: 0,
            watts: 0,
            watts_min: u16::MAX,
            watts_max: 0,
            temperature: 0,
            energy_uj: 0,
            module_id_raw: 0,
            module_class: 0,
            led: Led::default(),
            cross: CrossCheck::NONE,
            seq_state: SequenceState::Idle,
            retry_count: 0,
            power_limit: 0,
            retry_window_ms: 0,
            last_sample_ms: 0,
        }
    }
}

impl SlotModel {
    /// Take one INA238 sample: telemetry, min / max and the energy
    /// integral.
    ///
    /// Energy is integer throughout: one 0.1 W unit held for one
    /// millisecond is 100 uJ, so `watts x dt x 100` with no rounding
    /// anywhere. The first sample after a gap longer than four poll
    /// periods only sets the clock, so a unit that was away does not
    /// bill the whole absence at its last power.
    pub fn take_sample(&mut self, now_ms: u64, sample: Sample) {
        self.volts = sample.volts;
        self.amps = sample.amps;
        self.watts = sample.watts;
        self.temperature = sample.temperature;
        if self.watts < self.watts_min {
            self.watts_min = self.watts;
        }
        if self.watts > self.watts_max {
            self.watts_max = self.watts;
        }
        let dt = now_ms.saturating_sub(self.last_sample_ms);
        if self.last_sample_ms != 0 && dt <= 4 * POLL_PERIOD_MS as u64 {
            self.energy_uj = self
                .energy_uj
                .saturating_add(sample.watts as u64 * dt * 100);
        }
        self.last_sample_ms = now_ms;
    }

    /// Energy in whole joules, the unit SLOT 0x29 carries.
    pub const fn energy_joules(&self) -> u64 {
        self.energy_uj / 1_000_000
    }

    /// Forget the min / max window (CTRL 0x44).
    pub fn clear_min_max(&mut self) {
        self.watts_min = u16::MAX;
        self.watts_max = 0;
    }

    /// The min to report: 0 while nothing has been sampled.
    pub const fn reported_min(&self) -> u16 {
        if self.watts_min == u16::MAX {
            0
        } else {
            self.watts_min
        }
    }

    /// Stop claiming anything about a slot whose unit is away: the
    /// telemetry goes to zero and only the sticky counters stay.
    pub fn mark_absent(&mut self) {
        self.status = SlotStatus::NONE;
        self.volts = 0;
        self.amps = 0;
        self.watts = 0;
        self.temperature = 0;
        self.module_id_raw = 0;
        self.module_class = 0;
        self.cross = CrossCheck::NONE;
        self.last_sample_ms = 0;
        self.led = Led::default();
    }

    /// True for a slot in a state the controller should look at
    /// (STATUS 0x13 "any slot fault").
    pub fn in_fault(&self) -> bool {
        self.status.intersects(
            SlotStatus::OV_LATCHED | SlotStatus::POWER_LIMIT_TRIPPED | SlotStatus::RETRYING,
        )
    }
}

/// One unit board (ADR 0005): its presence, its hot-swap and its input
/// monitor.
#[derive(Debug, Clone, Copy, Default)]
pub struct UnitModel {
    /// The status bits of UNIT 0x30.
    pub status: UnitStatus,
    /// Input voltage \[10 mV\].
    pub volts: u16,
    /// Input current \[mA\].
    pub amps: u16,
    /// Input power \[0.1 W\].
    pub watts: u16,
    /// Input INA238 die temperature \[0.1 C\].
    pub temperature: i16,
    /// I2C errors on this unit's channel (also STATUS 0x1A).
    pub segment_errors: u16,
    /// How often the unit stopped answering.
    pub times_lost: u16,
}

impl UnitModel {
    /// True while the unit answers.
    pub fn present(&self) -> bool {
        self.status.contains(UnitStatus::PRESENT)
    }

    /// Clear everything a unit that went away cannot claim.
    pub fn mark_absent(&mut self) {
        self.status = UnitStatus::from_bits(
            self.status.bits()
                & (UnitStatus::LOST | UnitStatus::RESET_SEEN | UnitStatus::U_OFF_ASSERTED).bits(),
        );
        self.volts = 0;
        self.amps = 0;
        self.watts = 0;
        self.temperature = 0;
    }

    /// True for a unit the controller should look at (STATUS 0x13 "any
    /// unit fault").
    pub fn in_fault(&self) -> bool {
        self.status.contains(UnitStatus::LOST)
            || self.status.contains(UnitStatus::HOT_SWAP_LATCHED_OFF)
            || (self.present()
                && !self
                    .status
                    .contains(UnitStatus::EXPANDER_OK | UnitStatus::INPUT_INA238_OK))
    }
}

/// The retry policy of a slot as the poll cycle applies it (ADR 0002
/// decision 7).
///
/// A PGD drop while the slot is enabled counts; past the maximum inside
/// the window the slot is set OFF and an event is raised. The window
/// restarts whenever it has run out, so a slot that drops once a day
/// never reaches a limit of three in a minute.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryOutcome {
    /// Nothing to do.
    Nothing,
    /// The drop was counted and the slot is still allowed to retry.
    Counted,
    /// The count passed the maximum: the caller sets the slot OFF and
    /// raises the event.
    LimitReached,
}

impl SlotModel {
    /// Count a PGD drop against the policy.
    pub fn count_retry(&mut self, now_ms: u64, policy: &Retry) -> RetryOutcome {
        let window_ms = policy.window_s as u64 * 1000;
        if self.retry_window_ms == 0 || now_ms.saturating_sub(self.retry_window_ms) > window_ms {
            self.retry_window_ms = now_ms;
            self.retry_count = 0;
        }
        self.retry_count = self.retry_count.saturating_add(1);
        if policy.max > 0 && self.retry_count > policy.max as u16 {
            RetryOutcome::LimitReached
        } else {
            RetryOutcome::Counted
        }
    }
}

/// The LED thresholds a slot's LED tracking uses.
pub fn blink_config(cfg: &PlaneConfig) -> granite_proto::BlinkDetection {
    cfg.blink
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::PlaneConfig;

    fn blink() -> granite_proto::BlinkDetection {
        PlaneConfig::default().blink
    }

    #[test]
    fn a_steady_line_is_on_after_the_window() {
        let mut led = Led::default();
        let cfg = blink();
        // Lit, then held: the first sample is an edge, the mode only
        // settles once the window has passed.
        assert!(!led.sample(0, true, &cfg));
        assert_eq!(led.mode, LedMode::Off);
        for t in (50..=4000).step_by(50) {
            led.sample(t as u64, true, &cfg);
        }
        assert_eq!(led.mode, LedMode::Off);
        led.sample(4051, true, &cfg);
        assert_eq!(led.mode, LedMode::On);
        assert!(led.shows_light());
    }

    #[test]
    fn two_edges_make_a_blink_with_its_period() {
        let mut led = Led::default();
        let cfg = blink();
        // 1 Hz: an edge every 500 ms.
        let mut now = 0u64;
        let mut lit = false;
        for _ in 0..4 {
            now += 500;
            lit = !lit;
            led.sample(now, lit, &cfg);
        }
        assert_eq!(led.mode, LedMode::Blinking);
        assert_eq!(led.period_ms, 1000);
        assert!(led.shows_light());
    }

    #[test]
    fn a_blink_faster_than_the_window_is_not_one() {
        let mut led = Led::default();
        let cfg = granite_proto::BlinkDetection {
            min_period_ms: 200,
            max_period_ms: 4000,
        };
        let mut now = 0u64;
        let mut lit = false;
        for _ in 0..6 {
            now += 50; // 100 ms period, under the minimum
            lit = !lit;
            led.sample(now, lit, &cfg);
        }
        assert_ne!(led.mode, LedMode::Blinking);
    }

    #[test]
    fn energy_is_integer_and_ignores_a_gap() {
        let mut slot = SlotModel::default();
        let s = Sample {
            volts: 4800,
            amps: 1000,
            watts: 480, // 48.0 W
            temperature: 250,
        };
        // The first sample only sets the clock.
        slot.take_sample(1000, s);
        assert_eq!(slot.energy_uj, 0);
        // Ten 50 ms steps: 48 W for half a second is 24 J.
        for i in 1..=10 {
            slot.take_sample(1000 + i * 50, s);
        }
        assert_eq!(slot.energy_uj, 24_000_000);
        assert_eq!(slot.energy_joules(), 24);
        // A long gap is not billed.
        slot.take_sample(60_000, s);
        assert_eq!(slot.energy_joules(), 24);
    }

    #[test]
    fn min_max_tracks_and_clears() {
        let mut slot = SlotModel::default();
        assert_eq!(slot.reported_min(), 0);
        for w in [100u16, 40, 900] {
            slot.take_sample(1000, Sample {
                watts: w,
                ..Default::default()
            });
        }
        assert_eq!((slot.reported_min(), slot.watts_max), (40, 900));
        slot.clear_min_max();
        assert_eq!((slot.reported_min(), slot.watts_max), (0, 0));
    }

    #[test]
    fn the_retry_window_restarts() {
        let mut slot = SlotModel::default();
        let policy = Retry {
            max: 2,
            window_s: 10,
            action: 0,
        };
        assert_eq!(slot.count_retry(1_000, &policy), RetryOutcome::Counted);
        assert_eq!(slot.count_retry(2_000, &policy), RetryOutcome::Counted);
        assert_eq!(slot.count_retry(3_000, &policy), RetryOutcome::LimitReached);
        // Past the window the count starts again.
        assert_eq!(slot.count_retry(30_000, &policy), RetryOutcome::Counted);
        assert_eq!(slot.retry_count, 1);
    }
}
