//! Discovery, the 2 s re-scan and the stuck-bus recovery (ADR 0002
//! decision 4).
//!
//! At boot and then every [`RESCAN_MS`] for the channels that have no
//! unit: select channel k, probe the expander at 0x22. A unit is present
//! when the expander acknowledges; its INA238s and ADCs are then probed
//! and their absence is a unit fault, not "no unit".
//!
//! A present unit that NAKs twice in a row is lost (event), and its
//! channel goes back on the re-scan. A unit that appears or comes back
//! is configured from scratch and then put back in its intended state:
//! a slot that was OFF goes back OFF. While the unit had no 3V3 its
//! slots were ON, which is the hardware default and cannot be avoided;
//! the "unit reset seen" event records it.
//!
//! A channel whose bus stays stuck gets nine SCL clocks, then a MUX_RST
//! pulse, then it is skipped until the next re-scan.

use granite_proto::{EventKind, EventTarget, SEGMENT_CHANNELS, SlotStatus, UnitStatus};

use crate::device::{DeviceError, DevResult, Devices, SegmentBus};
use crate::drivers::expander::{self, Expander};
use crate::drivers::{IdAdc, Ina238};
use crate::log::level;
use crate::plane::Plane;

/// How often a channel without a unit is probed again.
pub const RESCAN_MS: u64 = 2_000;

/// NAKs in a row that make a present unit lost.
pub const LOST_AFTER_NAKS: u8 = 2;

/// Where a channel stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelState {
    /// Nothing ever answered here.
    Absent,
    /// A unit answers.
    Present,
    /// A unit answered and stopped.
    Lost,
    /// The segment is stuck; skipped until the next re-scan.
    Stuck,
}

/// Discovery state of one mux channel.
#[derive(Debug, Clone, Copy)]
pub struct Channel {
    /// Where the channel stands.
    pub state: ChannelState,
    /// NAKs in a row from a unit that was present.
    pub naks: u8,
    /// When this channel may be probed again.
    pub next_scan_ms: u64,
    /// The unit needs configuring from scratch before it is polled.
    pub needs_config: bool,
}

impl Default for Channel {
    fn default() -> Self {
        Channel::new()
    }
}

impl Channel {
    /// A channel nothing has answered on yet.
    pub const fn new() -> Self {
        Channel {
            state: ChannelState::Absent,
            naks: 0,
            next_scan_ms: 0,
            needs_config: false,
        }
    }

    /// True while the channel is on the re-scan rather than the poll.
    pub const fn scanning(&self) -> bool {
        !matches!(self.state, ChannelState::Present)
    }
}

impl Plane {
    /// Probe the channels that are not polled, and recover the stuck
    /// ones. `force` ignores the re-scan timer (boot).
    pub(crate) fn scan_channels<D: Devices>(&mut self, dev: &mut D, force: bool) {
        for ch in 0..SEGMENT_CHANNELS {
            if !self.channels[ch].scanning() {
                continue;
            }
            if !force && self.now_ms < self.channels[ch].next_scan_ms {
                continue;
            }
            self.channels[ch].next_scan_ms = self.now_ms + RESCAN_MS;
            match self.probe_channel(dev, ch) {
                Ok(true) => self.unit_appeared(dev, ch),
                Ok(false) => {}
                Err(e) => self.segment_error(dev, ch, e),
            }
        }
    }

    /// Select the channel and probe 0x22. `Ok(false)` is "no unit".
    fn probe_channel<D: Devices>(&mut self, dev: &mut D, ch: usize) -> DevResult<bool> {
        self.mux.select(dev.bus(), ch as u8)?;
        let mut probe = Expander::new();
        match probe.probe(dev.bus()) {
            Ok(()) => Ok(true),
            Err(DeviceError::Nak) => Ok(false),
            Err(e) => Err(e),
        }
    }

    /// A unit answered on a channel that had none: configure it from
    /// scratch and put it back in its intended state.
    fn unit_appeared<D: Devices>(&mut self, dev: &mut D, ch: usize) {
        self.channels[ch].state = ChannelState::Present;
        self.channels[ch].naks = 0;
        self.units[ch].status |= UnitStatus::PRESENT;
        self.units[ch].status = self.units[ch].status.without(UnitStatus::LOST);
        self.note(level::INFO, "unit present", Some(ch as u32));
        self.raise(EventTarget::Unit(ch as u8), EventKind::UnitPresent);
        // Whether it came back at hardware defaults is its expander's
        // own configuration registers to say, not this channel's
        // history: a unit can be lost to a cable glitch without ever
        // losing its 3V3.
        self.configure_unit(dev, ch);
    }

    /// Configure a unit's expander, monitors and ADCs, then re-apply the
    /// intended state.
    ///
    /// The expander write order is the one thing here that is not
    /// interchangeable: [`Expander::configure`] writes the released
    /// output latches, then the direction registers, then the intended
    /// latch (ADR 0002 decision 3 step 2 before step 3).
    pub(crate) fn configure_unit<D: Devices>(&mut self, dev: &mut D, ch: usize) {
        if self.mux.select(dev.bus(), ch as u8).is_err() {
            self.channels[ch].needs_config = true;
            return;
        }

        let latch = self.intended_latch(ch);
        let mut expander = Expander::new();
        match expander.configure(dev.bus(), latch) {
            Ok(at_defaults) => {
                self.expanders[ch] = expander;
                self.units[ch].status |= UnitStatus::EXPANDER_OK;
                self.channels[ch].needs_config = false;
                // All-inputs configuration registers mean the expander
                // came back at its hardware defaults: every slot was ON
                // while it did. That cannot be avoided; the event
                // records it.
                if at_defaults {
                    self.units[ch].status |= UnitStatus::RESET_SEEN;
                    self.raise(EventTarget::Unit(ch as u8), EventKind::UnitResetSeen);
                    self.note(level::WARN, "unit reset seen", Some(ch as u32));
                }
            }
            Err(e) => {
                self.units[ch].status = self.units[ch].status.without(UnitStatus::EXPANDER_OK);
                self.channels[ch].needs_config = true;
                self.count_segment_error(ch, e);
                return;
            }
        }

        // The two slot monitors and the unit's own.
        let mut monitors_ok = true;
        for half in 0..2u8 {
            let slot = (2 * ch) as u8 + half;
            let ina = Ina238::slot(half == 1);
            let limit = self.config.power_limit[slot as usize];
            let ok = ina
                .manufacturer_id(dev.bus())
                .is_ok_and(|id| id == crate::drivers::ina238::MANUFACTURER_ID)
                && ina.configure(dev.bus(), limit).is_ok();
            if ok {
                self.slots[slot as usize].power_limit = limit;
            } else {
                monitors_ok = false;
            }
        }
        let unit_ina = Ina238::unit();
        let unit_ok = unit_ina
            .manufacturer_id(dev.bus())
            .is_ok_and(|id| id == crate::drivers::ina238::MANUFACTURER_ID)
            && unit_ina.configure(dev.bus(), 0).is_ok();
        self.units[ch].status = self
            .units[ch]
            .status
            .set(UnitStatus::INPUT_INA238_OK, unit_ok);
        if !monitors_ok || !unit_ok {
            self.note(level::WARN, "unit monitor missing", Some(ch as u32));
        }

        // The ID ADCs, so a module's presence is known before the first
        // sequence can ask about it.
        self.read_ids(dev, ch);
    }

    /// Read both slots' ID ADCs on a selected channel.
    pub(crate) fn read_ids<D: Devices>(&mut self, dev: &mut D, ch: usize) {
        for half in 0..2usize {
            let slot = 2 * ch + half;
            match IdAdc::slot(half == 1).read_id(dev.bus()) {
                Ok(counts) => {
                    self.slots[slot].module_id_raw = counts;
                    self.slots[slot].module_class = crate::drivers::idadc::module_class(counts);
                    let present = !crate::drivers::idadc::is_empty(counts);
                    self.slots[slot].status =
                        self.slots[slot].status.set(SlotStatus::PRESENT, present);
                }
                Err(e) => self.count_segment_error(ch, e),
            }
        }
    }

    /// Count a NAK from a unit that was present; two in a row lose it.
    pub(crate) fn unit_nak<D: Devices>(&mut self, dev: &mut D, ch: usize) {
        let _ = dev;
        self.channels[ch].naks = self.channels[ch].naks.saturating_add(1);
        if self.channels[ch].naks < LOST_AFTER_NAKS {
            return;
        }
        self.channels[ch].state = ChannelState::Lost;
        self.channels[ch].next_scan_ms = self.now_ms + RESCAN_MS;
        self.units[ch].status |= UnitStatus::LOST;
        self.units[ch].mark_absent();
        self.units[ch].times_lost = self.units[ch].times_lost.saturating_add(1);
        for half in 0..2usize {
            self.slots[2 * ch + half].mark_absent();
        }
        self.mux.invalidate();
        self.note(level::WARN, "unit lost", Some(ch as u32));
        self.raise(EventTarget::Unit(ch as u8), EventKind::UnitLost);
    }

    /// Handle an error from a channel: count it, and recover a stuck
    /// bus.
    pub(crate) fn segment_error<D: Devices>(&mut self, dev: &mut D, ch: usize, e: DeviceError) {
        self.count_segment_error(ch, e);
        match e {
            DeviceError::Nak => self.unit_nak(dev, ch),
            DeviceError::Stuck => {
                // Nine clocks, then the mux, then leave the channel
                // alone until the re-scan.
                let _ = dev.bus().unstick();
                let _ = self.mux.reset(dev.bus());
                self.channels[ch].state = ChannelState::Stuck;
                self.channels[ch].next_scan_ms = self.now_ms + RESCAN_MS;
                self.units[ch].mark_absent();
                for half in 0..2usize {
                    self.slots[2 * ch + half].mark_absent();
                }
                self.note(level::ERROR, "segment stuck", Some(ch as u32));
            }
            _ => {
                self.mux.invalidate();
                self.unit_nak(dev, ch);
            }
        }
    }

    /// Count one error against a channel (STATUS 0x1A, UNIT 0x35).
    ///
    /// A NAK while *probing* an absent channel never reaches this: it is
    /// absence and [`Plane::probe_channel`] turns it into `Ok(false)`. A
    /// NAK from a part of a unit that is present does, because then
    /// something that should answer did not.
    pub(crate) fn count_segment_error(&mut self, ch: usize, _e: DeviceError) {
        self.units[ch].segment_errors = self.units[ch].segment_errors.saturating_add(1);
    }

    /// The expander output latch the intended state asks for on unit
    /// `ch`.
    pub(crate) fn intended_latch(&self, ch: usize) -> [u8; 3] {
        let mut latch = [0u8; 3];
        let mut set = |line: u8| latch[(line >> 3) as usize] |= 1 << (line & 7);
        for half in 0..2u8 {
            let slot = (2 * ch) as u8 + half;
            let b = half == 1;
            if self.intended.off(slot) {
                set(expander::out_line(b, expander::Out::Off));
            }
            if self.intended.runoff(slot) {
                set(expander::out_line(b, expander::Out::Runoff));
            }
            if self.intended.disarm(slot) {
                set(expander::out_line(b, expander::Out::Disarm));
            }
        }
        if self.intended.u_off(ch as u8) {
            set(expander::LINE_U_OFF);
        }
        latch
    }
}
