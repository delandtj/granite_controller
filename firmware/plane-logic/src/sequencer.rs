//! The slot actions of ADR 0002 decision 6 and the CTRL commands of
//! ADR 0003 (0x40-0x51).
//!
//! Every action is one per slot: a second action on a slot that is busy
//! answers [`ResultCode::Conflict`], and the outcome of one that runs is
//! an event ([`EventKind::SequenceDone`] or
//! [`EventKind::SequenceFailed`]) plus a result under the command's
//! sequence number. Nothing here sleeps: a sequence is a phase and a
//! deadline, and [`Plane::run_sequences`] moves it on every tick. That
//! is also what releases a press whose deadline has passed, whichever
//! phase set it (ADR 0002, "What an expert would ask").
//!
//! While any press is held, [`crate::plane::Plane::tick`] toggles PC6
//! PRESS_TRIG about every 200 ms. Without those edges the monostable
//! drops PRESS_EN and the photoMOS LEDs go dark about a second later, so
//! the toggling *is* the press: a firmware that stops ticking releases
//! every button on its own.

use granite_proto::{
    Escalate, EventKind, EventTarget, LedMode, ResultCode, SLOT_COUNT, SequenceState, SlotStatus,
    UNIT_COUNT, UnitStatus,
};

use crate::device::{CorePins, Devices, SegmentBus};
use crate::drivers::expander::{self, Out};
use crate::log::level;
use crate::plane::Plane;

/// How long a press lasts when the plane picks the duration itself (the
/// wake press of `node on` and `power-cycle`, and the short press of
/// `node off`).
pub const SHORT_PRESS_MS: u32 = 300;

/// How long PWR is held when `node off` escalates (ADR 0003 0x51
/// escalate 1 and 3: "hold PWR 5 s").
pub const HOLD_PRESS_MS: u32 = 5_000;

/// How long the OV latch's clear line is held (ADR 0002 decision 6 asks
/// for 10 ms; one tick is the shortest the bus task can do and a longer
/// clear is harmless).
pub const LCLR_PULSE_MS: u32 = 10;

/// How long U_OFF is held to re-arm a latched unit hot-swap (ADR 0002
/// decision 6: "unit reset | U_OFF pulse 100 ms").
pub const U_OFF_PULSE_MS: u32 = 100;

/// How long the plane waits after cutting a rail before it calls the
/// node off.
pub const CUT_SETTLE_MS: u32 = 1_000;

/// Which button a press holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    /// PWRD, the front panel's power contact.
    Pwr,
    /// RSTD, the reset contact.
    Rst,
}

impl Button {
    /// The expander output line of this button on half `b`.
    const fn line(self, b: bool) -> u8 {
        expander::out_line(
            b,
            match self {
                Button::Pwr => Out::Pwrd,
                Button::Rst => Out::Rstd,
            },
        )
    }
}

/// Where a slot's sequence stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// Nothing runs.
    Idle,
    /// A bare press of a fixed length (CTRL 0x45 / 0x46).
    RawPress,
    /// The OV latch's clear pulse (CTRL 0x41).
    ClearLatch,
    /// Rail off, waiting out the off delay (CTRL 0x47).
    CycleOff,
    /// Rail on, waiting for the host's LED (CTRL 0x47, 0x50).
    WaitLight,
    /// Holding PWR to wake a host that stayed dark.
    PressOn,
    /// A short PWR press, on its way to [`Phase::WaitDark`] (CTRL 0x51).
    PressOff,
    /// Waiting for the host's LED to go dark (CTRL 0x51).
    WaitDark,
    /// The escalation is done; waiting out the blink window to see
    /// whether the host really went dark.
    ConfirmDark,
    /// Holding PWR for five seconds (CTRL 0x51 escalate 1 and 3).
    HoldOff,
    /// The rail is cut, settling (CTRL 0x51 escalate 2 and 3).
    CutRail,
}

/// One slot's sequence: a phase, a deadline and the parameters of the
/// command that started it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sequence {
    /// Where it stands.
    pub phase: Phase,
    /// The CTRL sequence number that owns it, for the result.
    pub cmd_seq: u8,
    /// When the current phase must end \[ms since boot\].
    pub deadline_ms: u64,
    /// The button held, if any. This is what PRESS_TRIG answers to.
    pub press: Option<Button>,
    /// How long the rail stays off in a power-cycle \[ms\].
    pub off_delay_ms: u32,
    /// How long to wait for the host's LED \[ms\].
    pub led_timeout_ms: u32,
    /// Press PWR when the host stays dark.
    pub press_if_dark: bool,
    /// What `node off` does when the host stays up.
    pub escalate: Escalate,
    /// A wake press has already been tried in this sequence.
    pub pressed_once: bool,
    /// The state the link reports (SLOT 0x2D) once this ended.
    pub last: SequenceState,
}

impl Sequence {
    /// A slot with nothing running.
    pub const fn idle() -> Self {
        Sequence {
            phase: Phase::Idle,
            cmd_seq: 0,
            deadline_ms: 0,
            press: None,
            off_delay_ms: 0,
            led_timeout_ms: 0,
            press_if_dark: false,
            escalate: Escalate::None,
            pressed_once: false,
            last: SequenceState::Idle,
        }
    }

    /// True while something runs on this slot.
    pub const fn running(&self) -> bool {
        !matches!(self.phase, Phase::Idle)
    }

    /// What SLOT 0x2D reports.
    pub const fn state(&self) -> SequenceState {
        match self.phase {
            Phase::Idle => self.last,
            Phase::RawPress | Phase::PressOn | Phase::PressOff | Phase::HoldOff => {
                SequenceState::Pressing
            }
            Phase::CycleOff | Phase::WaitDark | Phase::ConfirmDark | Phase::CutRail => {
                SequenceState::OffWait
            }
            Phase::WaitLight => SequenceState::OnWait,
            Phase::ClearLatch => SequenceState::Pressing,
        }
    }
}

/// A plane-level stagger (CTRL 0x4B): every named slot off, then
/// released one by one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stagger {
    /// Slots still to release, bit per slot.
    pub remaining: u8,
    /// Delay between slots \[ms\].
    pub delay_ms: u16,
    /// When the next slot comes on.
    pub next_ms: u64,
    /// The CTRL sequence number that owns it.
    pub cmd_seq: u8,
}

/// A U_OFF pulse on one unit (CTRL 0x4A).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnitPulse {
    /// When U_OFF is released.
    pub until_ms: u64,
    /// The CTRL sequence number that owns it.
    pub cmd_seq: u8,
}

impl Plane {
    /// True while any slot runs a sequence, a stagger runs or a unit
    /// pulse is out (STATUS 0x13).
    pub fn any_sequence_running(&self) -> bool {
        self.sequences.iter().any(Sequence::running)
            || self.stagger.is_some()
            || self.unit_pulses.iter().any(Option::is_some)
    }

    /// True while any button is held: what PC6 answers to.
    pub fn any_press_running(&self) -> bool {
        self.sequences.iter().any(|s| s.press.is_some())
    }

    /// Where a slot's sequence stands (SLOT 0x2D).
    pub fn sequence_state(&self, slot: u8) -> SequenceState {
        self.sequences
            .get(slot as usize)
            .map_or(SequenceState::Idle, Sequence::state)
    }

    /// How long the plane must watch a line before it may call it
    /// steady.
    ///
    /// The LED mode comes from the 20 Hz samples and a line is only
    /// called on or off once it has held for longer than the blink
    /// window's maximum, so a 0.5 Hz breathing LED is not mistaken for
    /// off (ADR 0002, "What an expert would ask": "the LED timeout of
    /// the power-cycle sequence must be longer than that"). Every wait
    /// on an LED is therefore at least this long, whatever the command
    /// asked for, because a shorter one could only ever fail.
    pub(crate) fn led_settle_ms(&self) -> u32 {
        self.config.blink.max_period_ms as u32 + 1_000
    }

    /// A command's LED timeout, never shorter than the blink window.
    pub(crate) fn led_timeout_ms(&self, asked_s: u16) -> u32 {
        (asked_s as u32 * 1_000).max(self.led_settle_ms())
    }

    // --- the pieces every command is built from -----------------------

    /// The unit and half a slot index names.
    pub(crate) const fn split(slot: u8) -> (usize, bool) {
        ((slot / 2) as usize, slot % 2 == 1)
    }

    /// Write one expander output line and push the latch out.
    ///
    /// A write that fails leaves the latch as it was in RAM too, so the
    /// next cycle's `flush` tries again rather than believing a line it
    /// never set.
    pub(crate) fn drive<D: Devices>(&mut self, dev: &mut D, slot: u8, line: u8, on: bool) -> bool {
        let (unit, _) = Self::split(slot);
        self.drive_unit_line(dev, unit, line, on)
    }

    /// Write one expander output line of a unit.
    pub(crate) fn drive_unit_line<D: Devices>(
        &mut self,
        dev: &mut D,
        unit: usize,
        line: u8,
        on: bool,
    ) -> bool {
        if !self.unit_present(unit) {
            return false;
        }
        if let Err(e) = self.mux.select(dev.bus(), unit as u8) {
            self.segment_error(dev, unit, e);
            return false;
        }
        let before = self.expanders[unit];
        let mut expander = before;
        expander.set_output(line, on);
        match expander.flush(dev.bus()) {
            Ok(()) => {
                self.expanders[unit] = expander;
                true
            }
            Err(e) => {
                // The part never took the byte, so the shadow goes back
                // to what it last accepted: the next flush then tries
                // the change again instead of believing a line it never
                // set.
                self.expanders[unit] = before;
                self.segment_error(dev, unit, e);
                false
            }
        }
    }

    /// Read the expander's output registers back and check that `line`
    /// holds `on`. The caller has the unit's channel selected (every
    /// caller comes through [`Plane::drive_unit_line`] first).
    ///
    /// This is how the plane refuses to assert RUNOFF without DISARM
    /// (ADR 0002 decision 6: "ordering enforced, never RUNOFF without
    /// DISARM"): a DISARM that does not stick is not a DISARM, and
    /// RUNOFF then never goes out.
    pub(crate) fn verify_output<D: Devices>(
        &mut self,
        dev: &mut D,
        unit: usize,
        line: u8,
        on: bool,
    ) -> bool {
        debug_assert_eq!(self.mux.selected(), Some(unit as u8));
        let mut out = [0u8; 3];
        if dev
            .bus()
            .read(
                expander::EXP_ADDR,
                &[expander::CMD_AUTO_INCREMENT | expander::REG_OUTPUT0],
                &mut out,
            )
            .is_err()
        {
            return false;
        }
        (out[(line >> 3) as usize] & (1 << (line & 7)) != 0) == on
    }

    /// Set or clear a slot's OFF, in the model, on the expander and in
    /// the backup registers.
    pub(crate) fn set_slot_off<D: Devices>(&mut self, dev: &mut D, slot: u8, off: bool) -> bool {
        let (unit, b) = Self::split(slot);
        self.intended.set_off(slot, off);
        self.save_intended(dev);
        let ok = self.drive(dev, slot, expander::out_line(b, Out::Off), off);
        let _ = unit;
        if ok {
            // The rail has just changed, so the LED tracking starts over
            // rather than carrying a blink across the switch.
            let lit = self.slots[slot as usize].led.lit;
            self.slots[slot as usize].led.restart(self.now_ms, lit && !off);
        }
        ok
    }

    /// Start a press on a slot, toggling PC6 at once so PRESS_EN is up
    /// before the photoMOS LED has current.
    pub(crate) fn start_press<D: Devices>(
        &mut self,
        dev: &mut D,
        slot: u8,
        button: Button,
        ms: u32,
    ) -> bool {
        let (_, b) = Self::split(slot);
        if !self.drive(dev, slot, button.line(b), true) {
            return false;
        }
        dev.pins().toggle_press_trigger();
        self.press_trig_ms = self.now_ms;
        let seq = &mut self.sequences[slot as usize];
        seq.press = Some(button);
        seq.deadline_ms = self.now_ms + ms as u64;
        true
    }

    /// Release whatever button a slot holds.
    pub(crate) fn release_press<D: Devices>(&mut self, dev: &mut D, slot: u8) {
        let Some(button) = self.sequences[slot as usize].press else {
            return;
        };
        let (_, b) = Self::split(slot);
        self.drive(dev, slot, button.line(b), false);
        self.sequences[slot as usize].press = None;
    }

    /// End a slot's sequence with a result.
    pub(crate) fn finish<D: Devices>(&mut self, dev: &mut D, slot: u8, code: ResultCode) {
        self.release_press(dev, slot);
        let seq = &mut self.sequences[slot as usize];
        let cmd = seq.cmd_seq;
        seq.phase = Phase::Idle;
        seq.last = if code.is_ok() {
            SequenceState::Done
        } else {
            SequenceState::Failed
        };
        self.results.finish(cmd, code);
        let kind = if code.is_ok() {
            EventKind::SequenceDone
        } else {
            EventKind::SequenceFailed
        };
        self.raise(EventTarget::Slot(slot), kind);
        self.slots[slot as usize].status = self.slots[slot as usize]
            .status
            .without(SlotStatus::SEQUENCE_RUNNING);
    }

    /// Begin a sequence on a slot, with the phase and deadline its first
    /// step wants.
    pub(crate) fn begin(&mut self, slot: u8, cmd_seq: u8, phase: Phase, deadline_ms: u64) {
        let seq = &mut self.sequences[slot as usize];
        *seq = Sequence {
            phase,
            cmd_seq,
            deadline_ms,
            ..Sequence::idle()
        };
        self.slots[slot as usize].status |= SlotStatus::SEQUENCE_RUNNING;
    }

    // --- the tick -----------------------------------------------------

    /// Move every running sequence on by one step.
    pub(crate) fn run_sequences<D: Devices>(&mut self, dev: &mut D) {
        for slot in 0..SLOT_COUNT as u8 {
            self.step_slot(dev, slot);
        }
        self.step_stagger(dev);
        self.step_unit_pulses(dev);
    }

    /// One slot's step.
    fn step_slot<D: Devices>(&mut self, dev: &mut D, slot: u8) {
        let seq = self.sequences[slot as usize];
        if !seq.running() {
            return;
        }
        // A press whose deadline has passed is released here, whatever
        // set it: this is the software half of the press bound.
        let due = self.now_ms >= seq.deadline_ms;
        let lit = self.slots[slot as usize].led.shows_light();

        match seq.phase {
            Phase::Idle => {}
            Phase::RawPress => {
                if due {
                    self.finish(dev, slot, ResultCode::Ok);
                }
            }
            Phase::ClearLatch => {
                if due {
                    let (_, b) = Self::split(slot);
                    self.drive(dev, slot, expander::out_line(b, Out::Lclr), false);
                    // The latch is cleared, so the power-limit trip that
                    // may have set it is cleared with it. Nothing else
                    // ever clears that bit (ADR 0002 decision 7).
                    self.slots[slot as usize].status = self.slots[slot as usize]
                        .status
                        .without(SlotStatus::POWER_LIMIT_TRIPPED);
                    self.finish(dev, slot, ResultCode::Ok);
                }
            }
            Phase::CycleOff => {
                if due {
                    self.set_slot_off(dev, slot, false);
                    self.sequences[slot as usize].phase = Phase::WaitLight;
                    self.sequences[slot as usize].deadline_ms =
                        self.now_ms + seq.led_timeout_ms as u64;
                }
            }
            Phase::WaitLight => {
                if lit {
                    self.slots[slot as usize].cross = self.slots[slot as usize]
                        .cross
                        .without(granite_proto::CrossCheck::NO_LED_AFTER_CYCLE);
                    self.finish(dev, slot, ResultCode::Ok);
                } else if due {
                    if seq.press_if_dark && !seq.pressed_once {
                        self.sequences[slot as usize].pressed_once = true;
                        if self.start_press(dev, slot, Button::Pwr, SHORT_PRESS_MS) {
                            self.sequences[slot as usize].phase = Phase::PressOn;
                        } else {
                            self.finish(dev, slot, ResultCode::Failed);
                        }
                    } else {
                        // The rail is on and the host never lit up.
                        self.slots[slot as usize].cross |=
                            granite_proto::CrossCheck::NO_LED_AFTER_CYCLE;
                        self.note(level::WARN, "no led after cycle", Some(slot as u32));
                        self.finish(dev, slot, ResultCode::Failed);
                    }
                }
            }
            Phase::PressOn => {
                if due {
                    self.release_press(dev, slot);
                    self.sequences[slot as usize].phase = Phase::WaitLight;
                    self.sequences[slot as usize].deadline_ms =
                        self.now_ms + seq.led_timeout_ms as u64;
                }
            }
            Phase::PressOff => {
                if due {
                    self.release_press(dev, slot);
                    self.sequences[slot as usize].phase = Phase::WaitDark;
                    self.sequences[slot as usize].deadline_ms =
                        self.now_ms + seq.led_timeout_ms as u64;
                }
            }
            Phase::WaitDark => {
                if !lit {
                    self.finish(dev, slot, ResultCode::Ok);
                } else if due {
                    self.escalate_off(dev, slot);
                }
            }
            Phase::HoldOff => {
                if due {
                    // The button comes out and the LED gets the blink
                    // window to settle: a host that died at second four
                    // of the hold still reads as lit for a while.
                    self.release_press(dev, slot);
                    self.sequences[slot as usize].phase = Phase::ConfirmDark;
                    self.sequences[slot as usize].deadline_ms =
                        self.now_ms + self.led_settle_ms() as u64;
                }
            }
            Phase::ConfirmDark => {
                if !lit {
                    self.finish(dev, slot, ResultCode::Ok);
                } else if due {
                    if matches!(seq.escalate, Escalate::HoldPwrThenCutRail) {
                        // The host took the button for five seconds and
                        // stayed up: cut the rail (ADR 0003 0x51
                        // escalate 3, controller ADR 0004 force_off).
                        self.note(level::WARN, "force off, cutting rail", Some(slot as u32));
                        self.set_slot_off(dev, slot, true);
                        self.sequences[slot as usize].phase = Phase::CutRail;
                        self.sequences[slot as usize].deadline_ms =
                            self.now_ms + CUT_SETTLE_MS as u64;
                    } else {
                        self.finish(dev, slot, ResultCode::Failed);
                    }
                }
            }
            Phase::CutRail => {
                if due {
                    self.finish(dev, slot, ResultCode::Ok);
                }
            }
        }
    }

    /// `node off` ran out of patience (CTRL 0x51).
    fn escalate_off<D: Devices>(&mut self, dev: &mut D, slot: u8) {
        let seq = self.sequences[slot as usize];
        match seq.escalate {
            Escalate::None => self.finish(dev, slot, ResultCode::Failed),
            Escalate::HoldPwr | Escalate::HoldPwrThenCutRail => {
                if self.start_press(dev, slot, Button::Pwr, HOLD_PRESS_MS) {
                    self.sequences[slot as usize].phase = Phase::HoldOff;
                } else {
                    self.finish(dev, slot, ResultCode::Failed);
                }
            }
            Escalate::CutRail => {
                self.set_slot_off(dev, slot, true);
                self.sequences[slot as usize].phase = Phase::CutRail;
                self.sequences[slot as usize].deadline_ms = self.now_ms + CUT_SETTLE_MS as u64;
            }
        }
    }

    /// Release the next slot of a stagger.
    fn step_stagger<D: Devices>(&mut self, dev: &mut D) {
        let Some(stagger) = self.stagger else { return };
        if self.now_ms < stagger.next_ms {
            return;
        }
        let Some(slot) = (0..SLOT_COUNT as u8).find(|s| stagger.remaining & (1 << s) != 0) else {
            self.results.finish(stagger.cmd_seq, ResultCode::Ok);
            self.raise(EventTarget::Plane, EventKind::SequenceDone);
            self.stagger = None;
            return;
        };
        self.set_slot_off(dev, slot, false);
        let next = Stagger {
            remaining: stagger.remaining & !(1 << slot),
            next_ms: self.now_ms + stagger.delay_ms as u64,
            ..stagger
        };
        self.stagger = Some(next);
    }

    /// Release a unit's U_OFF pulse when its time is up.
    fn step_unit_pulses<D: Devices>(&mut self, dev: &mut D) {
        for unit in 0..UNIT_COUNT {
            let Some(pulse) = self.unit_pulses[unit] else {
                continue;
            };
            if self.now_ms < pulse.until_ms {
                continue;
            }
            self.intended.set_u_off(unit as u8, false);
            self.save_intended(dev);
            let ok = self.drive_unit_line(dev, unit, expander::LINE_U_OFF, false);
            self.unit_pulses[unit] = None;
            self.units[unit].status = self
                .units[unit]
                .status
                .without(UnitStatus::U_OFF_ASSERTED);
            self.results.finish(
                pulse.cmd_seq,
                if ok {
                    ResultCode::Ok
                } else {
                    ResultCode::Failed
                },
            );
            self.raise(
                EventTarget::Unit(unit as u8),
                if ok {
                    EventKind::SequenceDone
                } else {
                    EventKind::SequenceFailed
                },
            );
        }
    }

    // --- the commands -------------------------------------------------

    /// Can a command touch this slot at all?
    pub(crate) fn slot_guard(&self, slot: u8) -> Option<ResultCode> {
        let (unit, _) = Self::split(slot);
        if (slot as usize) >= SLOT_COUNT {
            return Some(ResultCode::BadArgument);
        }
        if !self.unit_present(unit) {
            return Some(ResultCode::NotPresent);
        }
        if self.sequences[slot as usize].running() {
            return Some(ResultCode::Conflict);
        }
        None
    }

    /// CTRL 0x40 enable / disable.
    pub(crate) fn cmd_enable<D: Devices>(&mut self, dev: &mut D, slot: u8, on: bool) -> ResultCode {
        if let Some(code) = self.slot_guard(slot) {
            return code;
        }
        if !self.set_slot_off(dev, slot, !on) {
            return ResultCode::Failed;
        }
        self.note(
            level::INFO,
            if on { "slot on" } else { "slot off" },
            Some(slot as u32),
        );
        self.raise(
            EventTarget::Slot(slot),
            if on {
                EventKind::SlotOn
            } else {
                EventKind::SlotOff
            },
        );
        ResultCode::Ok
    }

    /// CTRL 0x41 clear the OV latch: a pulse, so it answers Accepted and
    /// ends on the next tick.
    pub(crate) fn cmd_clear_ov<D: Devices>(
        &mut self,
        dev: &mut D,
        slot: u8,
        cmd_seq: u8,
    ) -> ResultCode {
        if let Some(code) = self.slot_guard(slot) {
            return code;
        }
        let (_, b) = Self::split(slot);
        if !self.drive(dev, slot, expander::out_line(b, Out::Lclr), true) {
            return ResultCode::Failed;
        }
        self.begin(
            slot,
            cmd_seq,
            Phase::ClearLatch,
            self.now_ms + LCLR_PULSE_MS as u64,
        );
        ResultCode::Accepted
    }

    /// CTRL 0x44 clear the power min / max.
    pub(crate) fn cmd_clear_min_max(&mut self, slot: u8) -> ResultCode {
        if slot == granite_proto::SLOT_ALL {
            for s in self.slots.iter_mut() {
                s.clear_min_max();
            }
            return ResultCode::Ok;
        }
        if (slot as usize) >= SLOT_COUNT {
            return ResultCode::BadArgument;
        }
        self.slots[slot as usize].clear_min_max();
        ResultCode::Ok
    }

    /// CTRL 0x45 / 0x46 press PWR or RST.
    pub(crate) fn cmd_press<D: Devices>(
        &mut self,
        dev: &mut D,
        slot: u8,
        button: Button,
        ms: u16,
        cmd_seq: u8,
    ) -> ResultCode {
        if !(granite_proto::PRESS_MS_MIN..=granite_proto::PRESS_MS_MAX).contains(&ms) {
            return ResultCode::BadArgument;
        }
        if let Some(code) = self.slot_guard(slot) {
            return code;
        }
        self.begin(slot, cmd_seq, Phase::RawPress, self.now_ms);
        if !self.start_press(dev, slot, button, ms as u32) {
            self.sequences[slot as usize] = Sequence::idle();
            return ResultCode::Failed;
        }
        ResultCode::Accepted
    }

    /// CTRL 0x47 power-cycle.
    pub(crate) fn cmd_power_cycle<D: Devices>(
        &mut self,
        dev: &mut D,
        slot: u8,
        off_delay_s: u16,
        press_if_dark: bool,
        led_timeout_s: u16,
        cmd_seq: u8,
    ) -> ResultCode {
        if let Some(code) = self.slot_guard(slot) {
            return code;
        }
        let off_delay_ms = off_delay_s as u32 * 1000;
        if !self.set_slot_off(dev, slot, true) {
            return ResultCode::Failed;
        }
        self.begin(
            slot,
            cmd_seq,
            Phase::CycleOff,
            self.now_ms + off_delay_ms as u64,
        );
        let led_timeout_ms = self.led_timeout_ms(led_timeout_s);
        let seq = &mut self.sequences[slot as usize];
        seq.off_delay_ms = off_delay_ms;
        seq.led_timeout_ms = led_timeout_ms;
        seq.press_if_dark = press_if_dark;
        ResultCode::Accepted
    }

    /// CTRL 0x48 abort.
    pub(crate) fn cmd_abort<D: Devices>(&mut self, dev: &mut D, slot: u8) -> ResultCode {
        if (slot as usize) >= SLOT_COUNT {
            return ResultCode::BadArgument;
        }
        if !self.sequences[slot as usize].running() {
            return ResultCode::Ok;
        }
        self.finish(dev, slot, ResultCode::Failed);
        ResultCode::Ok
    }

    /// CTRL 0x49 module run off / on, with the DISARM ordering of
    /// ADR 0002 decision 6.
    ///
    /// Off: DISARM first, verified on the part, then RUNOFF. On: RUNOFF
    /// released first, then DISARM. A DISARM that does not stick refuses
    /// the command rather than asserting RUNOFF without it.
    pub(crate) fn cmd_module_run<D: Devices>(
        &mut self,
        dev: &mut D,
        slot: u8,
        on: bool,
    ) -> ResultCode {
        if let Some(code) = self.slot_guard(slot) {
            return code;
        }
        let (unit, b) = Self::split(slot);
        let disarm = expander::out_line(b, Out::Disarm);
        let runoff = expander::out_line(b, Out::Runoff);
        if on {
            if !self.drive(dev, slot, runoff, false) {
                return ResultCode::Failed;
            }
            self.intended.set_runoff(slot, false);
            if !self.drive(dev, slot, disarm, false) {
                self.save_intended(dev);
                return ResultCode::Failed;
            }
            self.intended.set_disarm(slot, false);
            self.save_intended(dev);
            ResultCode::Ok
        } else {
            if !self.drive(dev, slot, disarm, true) {
                return ResultCode::Failed;
            }
            if !self.verify_output(dev, unit, disarm, true) {
                // Never RUNOFF without DISARM: the PGOOD-loss trip would
                // fire the moment the module stops.
                self.drive(dev, slot, disarm, false);
                self.note(level::ERROR, "disarm did not hold", Some(slot as u32));
                return ResultCode::Refused;
            }
            self.intended.set_disarm(slot, true);
            if !self.drive(dev, slot, runoff, true) {
                self.save_intended(dev);
                return ResultCode::Failed;
            }
            self.intended.set_runoff(slot, true);
            self.save_intended(dev);
            ResultCode::Ok
        }
    }

    /// CTRL 0x4A unit hot-swap reset: a U_OFF pulse.
    pub(crate) fn cmd_unit_reset<D: Devices>(
        &mut self,
        dev: &mut D,
        unit: u8,
        cmd_seq: u8,
    ) -> ResultCode {
        let u = unit as usize;
        if u >= UNIT_COUNT {
            return ResultCode::BadArgument;
        }
        if !self.unit_present(u) {
            return ResultCode::NotPresent;
        }
        if self.unit_pulses[u].is_some() {
            return ResultCode::Conflict;
        }
        self.intended.set_u_off(unit, true);
        self.save_intended(dev);
        if !self.drive_unit_line(dev, u, expander::LINE_U_OFF, true) {
            self.intended.set_u_off(unit, false);
            self.save_intended(dev);
            return ResultCode::Failed;
        }
        self.units[u].status |= UnitStatus::U_OFF_ASSERTED;
        self.unit_pulses[u] = Some(UnitPulse {
            until_ms: self.now_ms + U_OFF_PULSE_MS as u64,
            cmd_seq,
        });
        self.note(level::INFO, "unit reset", Some(unit as u32));
        ResultCode::Accepted
    }

    /// CTRL 0x4B stagger on.
    ///
    /// Every named slot goes off first and comes back one at a time
    /// (ADR 0002 decision 6), so naming a slot that already carries a
    /// board cuts it. That is this command's contract and it stays
    /// here: which slots may be cut is the caller's decision. The
    /// controller's `on_all` names only the slots whose rail is down
    /// (controller ADR 0004 decision 4, `NodeCommand::OnAll` of
    /// granite-core).
    pub(crate) fn cmd_stagger<D: Devices>(
        &mut self,
        dev: &mut D,
        slots: granite_proto::IndexBitmap,
        delay_ms: u16,
        cmd_seq: u8,
    ) -> ResultCode {
        if self.stagger.is_some() {
            return ResultCode::Busy;
        }
        let mut remaining = 0u8;
        for slot in 0..SLOT_COUNT as u8 {
            if !slots.has(slot) {
                continue;
            }
            let (unit, _) = Self::split(slot);
            if !self.unit_present(unit) || self.sequences[slot as usize].running() {
                continue;
            }
            // Everything named goes off first, then comes back one at a
            // time (ADR 0002 decision 6).
            self.set_slot_off(dev, slot, true);
            remaining |= 1 << slot;
        }
        if remaining == 0 {
            return ResultCode::NotPresent;
        }
        self.stagger = Some(Stagger {
            remaining,
            delay_ms,
            next_ms: self.now_ms + delay_ms as u64,
            cmd_seq,
        });
        ResultCode::Accepted
    }

    /// CTRL 0x4C reboot: the intended state goes to the backup registers
    /// marked as a commanded reset, and the firmware does the rest.
    pub(crate) fn cmd_reboot<D: Devices>(&mut self, dev: &mut D) -> ResultCode {
        self.backup.set_commanded_reboot(true);
        self.save_intended(dev);
        self.note(level::INFO, "reboot requested", None);
        self.reset_request = true;
        ResultCode::Ok
    }

    /// CTRL 0x4D reset the unit expanders: EXP_RSTN is bused, so every
    /// slot goes ON for the pulse before the plane re-applies its state.
    pub(crate) fn cmd_reset_expanders<D: Devices>(&mut self, dev: &mut D) -> ResultCode {
        self.note(level::WARN, "expander reset", None);
        // Any press is released first: the expander is about to forget
        // its outputs and the photoMOS would be left to PRESS_EN alone.
        for slot in 0..SLOT_COUNT as u8 {
            if self.sequences[slot as usize].running() {
                self.finish(dev, slot, ResultCode::Failed);
            }
        }
        dev.pins().pulse_expander_reset();
        for unit in 0..UNIT_COUNT {
            if !self.unit_present(unit) {
                continue;
            }
            self.expanders[unit].release_all();
            self.units[unit].status |= UnitStatus::RESET_SEEN;
            self.raise(EventTarget::Unit(unit as u8), EventKind::UnitResetSeen);
            self.configure_unit(dev, unit);
        }
        ResultCode::Ok
    }

    /// CTRL 0x50 node on: the rail on if it is off, then a PWR press if
    /// the host stays dark.
    pub(crate) fn cmd_node_on<D: Devices>(
        &mut self,
        dev: &mut D,
        slot: u8,
        led_timeout_s: u16,
        cmd_seq: u8,
    ) -> ResultCode {
        if let Some(code) = self.slot_guard(slot) {
            return code;
        }
        if self.slots[slot as usize].led.mode == LedMode::On {
            // Already up: nothing to do, as the controller's `on` verb
            // asks (controller ADR 0004).
            return ResultCode::Ok;
        }
        if self.intended.off(slot) && !self.set_slot_off(dev, slot, false) {
            return ResultCode::Failed;
        }
        let led_timeout_ms = self.led_timeout_ms(led_timeout_s);
        self.begin(
            slot,
            cmd_seq,
            Phase::WaitLight,
            self.now_ms + led_timeout_ms as u64,
        );
        let seq = &mut self.sequences[slot as usize];
        seq.led_timeout_ms = led_timeout_ms;
        seq.press_if_dark = true;
        ResultCode::Accepted
    }

    /// CTRL 0x51 node off: a short press, then the escalation ladder. A
    /// timeout of 0 skips the short press and escalates at once, which
    /// is the controller's `force_off`.
    pub(crate) fn cmd_node_off<D: Devices>(
        &mut self,
        dev: &mut D,
        slot: u8,
        timeout_s: u16,
        escalate: Escalate,
        cmd_seq: u8,
    ) -> ResultCode {
        if let Some(code) = self.slot_guard(slot) {
            return code;
        }
        if !self.slots[slot as usize].led.shows_light() && timeout_s != 0 {
            // Already dark.
            return ResultCode::Ok;
        }
        self.begin(slot, cmd_seq, Phase::PressOff, self.now_ms);
        let led_timeout_ms = self.led_timeout_ms(timeout_s);
        let seq = &mut self.sequences[slot as usize];
        seq.escalate = escalate;
        seq.led_timeout_ms = led_timeout_ms;
        if timeout_s == 0 {
            self.sequences[slot as usize].phase = Phase::WaitDark;
            self.sequences[slot as usize].deadline_ms = self.now_ms;
            self.escalate_off(dev, slot);
            if !self.sequences[slot as usize].running() {
                // The escalation finished inside this call (it failed to
                // start): its result is already recorded.
                return self
                    .results
                    .get(cmd_seq)
                    .map_or(ResultCode::Failed, |h| h.code);
            }
            return ResultCode::Accepted;
        }
        if !self.start_press(dev, slot, Button::Pwr, SHORT_PRESS_MS) {
            self.sequences[slot as usize] = Sequence::idle();
            return ResultCode::Failed;
        }
        ResultCode::Accepted
    }
}
