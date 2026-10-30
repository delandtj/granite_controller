//! The poll schedule of ADR 0002 decision 5.
//!
//! One cycle every 50 ms (20 Hz) over every present unit: the expander's
//! three input ports, then each slot's INA238 bus voltage, current,
//! power and die temperature, and DIAG_ALRT whenever the slot's ALERT
//! node changed. The unit's own INA238 goes at 2 Hz and the ID ADCs at
//! 1 Hz, or at once on a PG or presence change.
//!
//! Everything a status bit says comes from this cycle, so a bit cannot
//! be stale: the only sticky ones are the latches (OV, power limit),
//! which ADR 0002 decision 7 says are never cleared automatically.

use granite_proto::{CrossCheck, EventKind, EventTarget, SEGMENT_CHANNELS, SlotStatus};

use crate::device::Devices;
use crate::drivers::expander::{self, In};
use crate::drivers::ina238::DIAG_POL;
use crate::drivers::Ina238;
use crate::log::level;
use crate::model::RetryOutcome;
use crate::plane::Plane;

/// The unit input INA238's period \[ms\] (2 Hz).
pub const UNIT_POLL_MS: u64 = 500;

/// The ID ADCs' period \[ms\] (1 Hz).
pub const ID_POLL_MS: u64 = 1_000;

impl Plane {
    /// One poll cycle over every present unit.
    pub(crate) fn poll_cycle<D: Devices>(&mut self, dev: &mut D) {
        let unit_due = self.now_ms >= self.next_unit_poll_ms;
        if unit_due {
            self.next_unit_poll_ms = self.now_ms + UNIT_POLL_MS;
        }
        let ids_due = self.now_ms >= self.next_id_poll_ms;
        if ids_due {
            self.next_id_poll_ms = self.now_ms + ID_POLL_MS;
        }

        for ch in 0..SEGMENT_CHANNELS {
            if !self.unit_present(ch) {
                continue;
            }
            if self.channels[ch].needs_config {
                self.configure_unit(dev, ch);
                continue;
            }
            if let Err(e) = self.mux.select(dev.bus(), ch as u8) {
                self.segment_error(dev, ch, e);
                continue;
            }
            if let Err(e) = self.poll_unit(dev, ch, unit_due, ids_due) {
                self.segment_error(dev, ch, e);
            } else {
                self.channels[ch].naks = 0;
            }
        }
    }

    /// Everything on one selected channel.
    fn poll_unit<D: Devices>(
        &mut self,
        dev: &mut D,
        ch: usize,
        unit_due: bool,
        ids_due: bool,
    ) -> crate::device::DevResult<()> {
        let mut expander = self.expanders[ch];
        expander.read_inputs(dev.bus())?;
        self.expanders[ch] = expander;

        // The unit's own hot-swap, from P27. The LM5069-2 retries by
        // itself for ever, so the latch is the firmware's
        // ([`crate::unit_latch`], ADR 0005 decision 3): U_PGD low with
        // the bus in range is a unit fault and the core holds U_OFF
        // high, U_PGD low with the bus out of range is the bus.
        let u_pgd = expander.get_input(expander::LINE_U_PGD);
        self.judge_unit_hot_swap(dev, ch, u_pgd);
        self.units[ch].status = self.unit_hot_swap_status(ch, u_pgd);

        for half in 0..2usize {
            self.poll_slot(dev, ch, half)?;
        }

        if unit_due {
            let sample = Ina238::unit().sample(dev.bus())?;
            self.units[ch].volts = sample.volts;
            self.units[ch].amps = sample.amps;
            self.units[ch].watts = sample.watts;
            self.units[ch].temperature = sample.temperature;
        }
        if ids_due {
            self.read_ids(dev, ch);
        }
        Ok(())
    }

    /// One slot's expander lines, monitor and derived status.
    fn poll_slot<D: Devices>(
        &mut self,
        dev: &mut D,
        ch: usize,
        half: usize,
    ) -> crate::device::DevResult<()> {
        let slot = 2 * ch + half;
        let b = half == 1;
        let exp = self.expanders[ch];

        let pgd = exp.get_input(expander::in_line(b, In::Pgd));
        let pg = exp.get_input(expander::in_line(b, In::Pg));
        let lq = exp.get_input(expander::in_line(b, In::Lq));
        let ovk = exp.get_input(expander::in_line(b, In::Ovk));
        // The opto pulls the line low while the host's LED is lit
        // (unit.py U8).
        let lit = !exp.get_input(expander::in_line(b, In::Led));

        let was = self.slots[slot].status;
        let enabled = !self.intended.off(slot as u8);

        // A PGD drop while the slot is enabled is a retry (ADR 0002
        // decision 7); the hot-swap retries by itself, the plane only
        // counts.
        if was.contains(SlotStatus::PGD) && !pgd && enabled && !lq {
            let policy = self.config.retry[slot];
            let outcome = self.slots[slot].count_retry(self.now_ms, &policy);
            self.raise(EventTarget::Slot(slot as u8), EventKind::PgdLost);
            if outcome == RetryOutcome::LimitReached {
                self.note(level::ERROR, "retry limit, slot off", Some(slot as u32));
                self.raise(EventTarget::Slot(slot as u8), EventKind::RetryLimitReached);
                self.set_slot_off(dev, slot as u8, true);
            }
        }
        if !was.contains(SlotStatus::OV_LATCHED) && lq {
            self.note(level::ERROR, "ov latched", Some(slot as u32));
            self.raise(EventTarget::Slot(slot as u8), EventKind::OvLatched);
        }

        // DIAG_ALRT only when the ALERT node changed: the slot's INA238
        // alert is one of the three open-drain sources on OVK, and
        // DIAG_ALRT is what tells a power trip from the module's own
        // OV_KILL (unit.py U3).
        if !ovk && self.ovk_was_high(slot) {
            let ina = Ina238::slot(b);
            if let Ok(diag) = ina.diag_alrt(dev.bus())
                && diag & DIAG_POL != 0
            {
                self.note(level::ERROR, "power limit trip", Some(slot as u32));
                self.raise(EventTarget::Slot(slot as u8), EventKind::PowerLimitTrip);
                self.slots[slot].status |= SlotStatus::POWER_LIMIT_TRIPPED;
            }
        }
        self.set_ovk_high(slot, ovk);

        let sample = Ina238::slot(b).sample(dev.bus())?;
        self.slots[slot].take_sample(self.now_ms, sample);

        let blink = self.config.blink;
        if self.slots[slot].led.sample(self.now_ms, lit, &blink) {
            self.raise(EventTarget::Slot(slot as u8), EventKind::LedChanged);
        }

        // Status bits that are a function of the sample, not of
        // history. The two sticky ones are kept from the status as it
        // stands *now* and not from `was`: the power-limit trip may
        // have been set a few lines up, by this very cycle's DIAG_ALRT,
        // and PRESENT comes from the ID ADC on its own 1 Hz schedule.
        let retrying = self.slots[slot].retry_count > 0 && !pgd && enabled;
        let keep = self.slots[slot].status.bits()
            & (SlotStatus::PRESENT | SlotStatus::POWER_LIMIT_TRIPPED).bits();
        self.slots[slot].status = SlotStatus::from_bits(keep)
            .set(SlotStatus::ENABLED, enabled)
            .set(SlotStatus::PGD, pgd)
            .set(SlotStatus::MODULE_PGOOD, pg)
            .set(SlotStatus::OV_LATCHED, lq)
            .set(SlotStatus::RETRYING, retrying)
            .set(
                SlotStatus::SEQUENCE_RUNNING,
                self.sequences[slot].running(),
            )
            .set(SlotStatus::MODULE_RUN_OFF, self.intended.runoff(slot as u8))
            .set(SlotStatus::DISARMED, self.intended.disarm(slot as u8));

        // Cross-checks: a lit LED with no current, or on a slot that is
        // commanded off, means the wiring or the module is not what the
        // plane thinks.
        let shows = self.slots[slot].led.shows_light();
        self.slots[slot].cross = self.slots[slot]
            .cross
            .set(CrossCheck::LED_AT_ZERO_CURRENT, shows && sample.amps == 0)
            .set(CrossCheck::LED_WHILE_OFF, shows && !enabled);

        // A module that came or went changes the ID reading, so the
        // 1 Hz schedule is pulled forward to the next cycle (ADR 0002
        // decision 5: "ID ADCs at 1 Hz and on a PG / presence change").
        if was.contains(SlotStatus::MODULE_PGOOD) != pg {
            self.next_id_poll_ms = self.now_ms;
        }

        // Rail transitions are events in their own right.
        if was.contains(SlotStatus::PGD) != pgd {
            let kind = if pgd {
                EventKind::SlotOn
            } else {
                EventKind::SlotOff
            };
            self.raise(EventTarget::Slot(slot as u8), kind);
        }
        Ok(())
    }

    /// Whether OVK read high on the last cycle, kept out of the wire
    /// status word because the link has no bit for it.
    fn ovk_was_high(&self, slot: usize) -> bool {
        self.ovk_high & (1 << slot) != 0
    }

    /// Remember this cycle's OVK level.
    fn set_ovk_high(&mut self, slot: usize, high: bool) {
        if high {
            self.ovk_high |= 1 << slot;
        } else {
            self.ovk_high &= !(1u8 << slot);
        }
    }
}
