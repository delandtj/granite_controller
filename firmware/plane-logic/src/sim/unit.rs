//! A simulated unit board: its expander with real pin semantics, its
//! three INA238s, its two ID ADCs, and a node per slot whose power LED
//! follows the PWR presses and the rail.
//!
//! The point of the pin semantics is that the plane logic cannot tell
//! this from a board: an output line reads back what the latch holds
//! only while the configuration register says it is an output, an
//! unconfigured line reads its pull-down, the LED line is inverted, and
//! a slot whose OFF is asserted drops PGD, which drops the node.

use crate::drivers::expander::{self, In, Out};
use crate::drivers::ina238;

/// Register values a TCA6424A has after a reset: every line an input,
/// every output latch high (SCPS224).
pub const EXPANDER_RESET: [u8; 3] = [0xFF; 3];

/// A simulated TCA6424A.
#[derive(Debug, Clone, Copy)]
pub struct FakeExpander {
    /// Output Port 0-2.
    pub output: [u8; 3],
    /// Configuration Port 0-2; a 1 bit is an input.
    pub config: [u8; 3],
    /// Polarity Inversion Port 0-2.
    pub polarity: [u8; 3],
    /// Lines that refuse to be driven, bit per line: the part takes the
    /// write and the output register reads back zero, which is how a
    /// wedged expander looks from the core.
    pub wedged: u32,
}

impl Default for FakeExpander {
    fn default() -> Self {
        FakeExpander::after_reset()
    }
}

impl FakeExpander {
    /// An expander as a unit that just got its 3V3 has it.
    pub const fn after_reset() -> Self {
        FakeExpander {
            output: EXPANDER_RESET,
            config: EXPANDER_RESET,
            polarity: [0; 3],
            wedged: 0,
        }
    }

    /// True while the part has been configured by the core.
    pub fn configured(&self) -> bool {
        self.config != EXPANDER_RESET
    }

    /// The level a line drives, or `None` while it is an input.
    pub fn driven(&self, line: u8) -> Option<bool> {
        let (port, bit) = ((line >> 3) as usize, 1u8 << (line & 7));
        if self.config[port] & bit != 0 {
            return None;
        }
        if self.wedged & (1 << line) != 0 {
            return Some(false);
        }
        Some(self.output[port] & bit != 0)
    }

    /// True while the line is an output and asserted.
    pub fn asserted(&self, line: u8) -> bool {
        self.driven(line) == Some(true)
    }

    /// The output register as it reads back: a wedged line reads zero.
    pub fn output_readback(&self) -> [u8; 3] {
        let mut out = self.output;
        for line in 0..24u8 {
            if self.wedged & (1 << line) != 0 {
                out[(line >> 3) as usize] &= !(1 << (line & 7));
            }
        }
        out
    }
}

/// A simulated INA238.
#[derive(Debug, Clone, Copy)]
pub struct FakeIna {
    /// The part answers on the bus.
    pub present: bool,
    /// Bus voltage \[mV\].
    pub bus_mv: u32,
    /// Current \[mA\].
    pub current_ma: u32,
    /// Die temperature \[0.1 C\].
    pub temperature: i16,
    /// SHUNT_CAL as written.
    pub shunt_cal: u16,
    /// CONFIG as written.
    pub config: u16,
    /// ADC_CONFIG as written.
    pub adc_config: u16,
    /// PWR_LIMIT as written.
    pub power_limit: u16,
    /// DIAG_ALRT, including the sticky flags.
    pub diag: u16,
}

impl Default for FakeIna {
    fn default() -> Self {
        FakeIna {
            present: true,
            bus_mv: 0,
            current_ma: 0,
            temperature: 250,
            shunt_cal: 0,
            config: 0,
            adc_config: 0,
            // Reset value: no limit can trip.
            power_limit: u16::MAX,
            diag: 0,
        }
    }
}

impl FakeIna {
    /// Power \[mW\] from the voltage and current.
    pub const fn power_mw(&self) -> u32 {
        (self.bus_mv * self.current_ma) / 1000
    }

    /// True while POWER is past PWR_LIMIT, which on a slot pulls ALERT
    /// and sets the slot's OV latch.
    pub fn over_limit(&self) -> bool {
        if self.power_limit == u16::MAX {
            return false;
        }
        // PWR_LIMIT is compared against POWER / 256.
        self.power_register() / 256 > self.power_limit as u32
    }

    /// True while the ALERT pin is pulled.
    ///
    /// The plane programs DIAG_ALRT with ALATCH set, so the flag and the
    /// pin stay asserted until DIAG_ALRT is read, even though the slot's
    /// own latch has by then cut the current that caused it. That is the
    /// whole point of the latch: the core gets to see which source
    /// fired (ADR 0002 decision 6, unit.py U3).
    pub const fn alert(&self) -> bool {
        self.diag & ina238::DIAG_POL != 0
    }

    /// The POWER register: `0.2 mW` per count.
    pub const fn power_register(&self) -> u32 {
        self.power_mw() * 5
    }

    /// Answer a register read.
    pub fn read(&self, reg: u8, out: &mut [u8]) -> bool {
        let word = |v: u16, out: &mut [u8]| {
            out.copy_from_slice(&v.to_be_bytes());
            true
        };
        match (reg, out.len()) {
            (ina238::REG_CONFIG, 2) => word(self.config, out),
            (ina238::REG_ADC_CONFIG, 2) => word(self.adc_config, out),
            (ina238::REG_SHUNT_CAL, 2) => word(self.shunt_cal, out),
            // 3.125 mV per count: mV x 8 / 25.
            (ina238::REG_VBUS, 2) => word((self.bus_mv * 8 / 25) as u16, out),
            (ina238::REG_CURRENT, 2) => word(self.current_ma as u16, out),
            // 125 m C per count, in bits 15-4.
            (ina238::REG_DIETEMP, 2) => {
                word(((self.temperature as i32 * 4 / 5) as u16) << 4, out)
            }
            (ina238::REG_POWER, 3) => {
                let p = self.power_register();
                out[0] = (p >> 16) as u8;
                out[1] = (p >> 8) as u8;
                out[2] = p as u8;
                true
            }
            (ina238::REG_DIAG_ALRT, 2) => word(self.diag, out),
            (ina238::REG_PWR_LIMIT, 2) => word(self.power_limit, out),
            (ina238::REG_MANUFACTURER_ID, 2) => word(ina238::MANUFACTURER_ID, out),
            _ => false,
        }
    }

    /// Take a register write.
    pub fn write(&mut self, reg: u8, value: u16) -> bool {
        match reg {
            ina238::REG_CONFIG => self.config = value,
            ina238::REG_ADC_CONFIG => self.adc_config = value,
            ina238::REG_SHUNT_CAL => self.shunt_cal = value,
            ina238::REG_DIAG_ALRT => {
                // The configuration bits are written; the sticky flags
                // are not settable from outside.
                self.diag = (self.diag & 0x07FF) | (value & 0xF800);
            }
            ina238::REG_PWR_LIMIT => self.power_limit = value,
            _ => return false,
        }
        true
    }
}

/// A simulated ADC121C027 holding one slot's module ID.
#[derive(Debug, Clone, Copy)]
pub struct FakeIdAdc {
    /// The part answers on the bus.
    pub present: bool,
    /// The 12-bit count; full scale is an empty slot.
    pub counts: u16,
}

impl Default for FakeIdAdc {
    fn default() -> Self {
        FakeIdAdc {
            present: true,
            counts: crate::drivers::idadc::FULL_SCALE,
        }
    }
}

/// A simulated host behind one slot: its power LED, and how it answers
/// the front-panel contacts.
#[derive(Debug, Clone, Copy)]
pub struct FakeNode {
    /// The host is running and its power LED is lit.
    pub on: bool,
    /// How long PWR must be held to kill it \[ms\] (the ~4 s of a real
    /// motherboard, shortened by default so a test is not 4 s long).
    pub force_hold_ms: u32,
    /// Shortest press the host notices \[ms\].
    pub min_press_ms: u32,
    /// The host ignores a short press: an operating system that will not
    /// shut down, which is what CTRL 0x51's escalation is for.
    pub ignores_short_press: bool,
    /// The host survives even the long hold: only cutting the rail ends
    /// it (CTRL 0x51 escalate 3).
    pub ignores_hold: bool,
    /// When set, the LED blinks with this period while the host is on: a
    /// sleeping node.
    pub blink_period_ms: Option<u32>,
    /// How long PWR has been held \[ms\].
    held_ms: u32,
    /// How long the last completed press lasted \[ms\].
    last_press_ms: u32,
    /// Phase of the blink \[ms\].
    blink_ms: u32,
    /// The LED's level.
    led: bool,
}

impl Default for FakeNode {
    fn default() -> Self {
        FakeNode {
            on: false,
            force_hold_ms: 4_000,
            min_press_ms: 50,
            ignores_short_press: false,
            ignores_hold: false,
            blink_period_ms: None,
            held_ms: 0,
            last_press_ms: 0,
            blink_ms: 0,
            led: false,
        }
    }
}

impl FakeNode {
    /// The power LED as the opto sees it.
    pub const fn led(&self) -> bool {
        self.led
    }

    /// How long PWR has been held, for a test that wants to see it.
    pub const fn held_ms(&self) -> u32 {
        self.held_ms
    }

    /// How long the last completed press lasted: what the host felt.
    pub const fn last_press_ms(&self) -> u32 {
        self.last_press_ms
    }

    /// Advance the host by `dt` with the contacts as they are.
    ///
    /// `rail` is its 19 V, `pwr` and `rst` the front-panel contacts.
    fn advance(&mut self, dt: u32, rail: bool, pwr: bool, rst: bool) {
        if !rail {
            self.on = false;
            self.led = false;
            self.held_ms = 0;
            return;
        }
        if pwr {
            self.held_ms = self.held_ms.saturating_add(dt);
            if self.held_ms >= self.force_hold_ms && !self.ignores_hold {
                self.on = false;
            }
        } else if self.held_ms > 0 {
            let held = core::mem::take(&mut self.held_ms);
            self.last_press_ms = held;
            if held >= self.min_press_ms && held < self.force_hold_ms {
                // A short press: on if it was off, a shutdown if it was
                // on and the host listens.
                if !self.on {
                    self.on = true;
                    self.blink_ms = 0;
                } else if !self.ignores_short_press {
                    self.on = false;
                }
            }
        }
        if rst && self.on {
            self.blink_ms = 0;
        }
        self.led = match (self.on, self.blink_period_ms) {
            (false, _) => false,
            (true, None) => true,
            (true, Some(period)) => {
                self.blink_ms = (self.blink_ms + dt) % period.max(1);
                self.blink_ms * 2 < period
            }
        };
    }
}

/// One simulated slot: its monitor, its ID ADC, its hot-swap and its
/// host.
#[derive(Debug, Clone, Copy, Default)]
pub struct FakeSlot {
    /// The slot's INA238 (0x40 or 0x41).
    pub ina: FakeIna,
    /// The slot's ID ADC (0x51 or 0x52).
    pub adc: FakeIdAdc,
    /// The hot-swap's power-good.
    pub pgd: bool,
    /// The OV latch, set by a trip source and cleared only by LCLR.
    pub ov_latched: bool,
    /// The module's PGOOD.
    pub module_pgood: bool,
    /// A module is screwed down (its ID resistor answers).
    pub module: bool,
    /// The host behind the slot.
    pub node: FakeNode,
    /// Drop PGD on the next advance, whatever the rail says: a hot-swap
    /// that retried, for the retry-policy test.
    pub glitch_pgd: bool,
}

/// A simulated LM5069-**2** unit hot-swap (uinput.py U207).
///
/// The auto-retry part: on a fault it opens the FETs, waits out its
/// retry period and tries again, for ever and with no memory of having
/// faulted. One attempt lasts the fault timer ([`HotSwap::attempt_ms`],
/// 10 ms on the board) and the whole cycle [`HotSwap::period_ms`]
/// (~2 s, the 0.5 % duty of ADR 0005 decision 3). Whether U_PGD rises
/// during an attempt is the fault's business - a dead short never lets
/// the output up, a power-limit trip does - so it is a knob.
///
/// The part is only enabled while U_OFF is low (Q204 off) and the input
/// is inside its UVLO / OVLO window; with neither, the TIMER sits in
/// reset and U_PGD is low.
#[derive(Debug, Clone, Copy)]
pub struct HotSwap {
    /// An overload the part faults on: the thing the firmware latch is
    /// there for.
    pub fault: bool,
    /// How long one restart attempt lasts \[ms\] (the fault timer).
    pub attempt_ms: u32,
    /// Attempt plus the wait before the next one \[ms\].
    pub period_ms: u32,
    /// U_PGD rises during an attempt.
    pub pgd_on_attempt: bool,
    /// Where the retry cycle stands \[ms\].
    phase_ms: u32,
    /// U_PGD as it stands.
    pgd: bool,
}

impl Default for HotSwap {
    fn default() -> Self {
        HotSwap {
            fault: false,
            attempt_ms: 10,
            period_ms: 2_000,
            pgd_on_attempt: true,
            phase_ms: 0,
            pgd: false,
        }
    }
}

impl HotSwap {
    /// U_PGD as the expander's P27 reads it.
    pub const fn pgd(&self) -> bool {
        self.pgd
    }

    /// True while the stage passes the full input to VBUS.
    pub const fn conducting(&self) -> bool {
        self.pgd && !self.fault
    }

    /// Advance the stage by `dt` with its enable and its input.
    fn advance(&mut self, dt: u32, enabled: bool) {
        if !enabled {
            self.phase_ms = 0;
            self.pgd = false;
            return;
        }
        if !self.fault {
            self.phase_ms = 0;
            self.pgd = true;
            return;
        }
        self.phase_ms = (self.phase_ms + dt) % self.period_ms.max(1);
        self.pgd = self.pgd_on_attempt && self.phase_ms < self.attempt_ms;
    }
}

/// One simulated unit board.
#[derive(Debug, Clone, Copy)]
pub struct FakeUnit {
    /// The unit has 3V3: with it gone nothing on the segment answers and
    /// the expander forgets its outputs.
    pub powered: bool,
    /// Its expander.
    pub expander: FakeExpander,
    /// Its two slots, A then B.
    pub slots: [FakeSlot; 2],
    /// The input monitor at 0x44. Its VBUS pin is on the hot-swap
    /// *output* (uinput.py U208), so the voltage it reports follows the
    /// stage and not the XT60.
    pub input: FakeIna,
    /// The unit's own 48 V input at the XT60 \[mV\]: the bus, which no
    /// part on the unit can read.
    pub input_mv: u32,
    /// Its hot-swap.
    pub hotswap: HotSwap,
}

impl Default for FakeUnit {
    fn default() -> Self {
        FakeUnit::new()
    }
}

impl FakeUnit {
    /// A powered unit with two empty slots at 48 V.
    pub fn new() -> Self {
        let mut unit = FakeUnit {
            powered: true,
            expander: FakeExpander::after_reset(),
            slots: [FakeSlot::default(); 2],
            input: FakeIna {
                bus_mv: 48_000,
                ..FakeIna::default()
            },
            input_mv: 48_000,
            hotswap: HotSwap {
                pgd: true,
                ..HotSwap::default()
            },
        };
        for slot in unit.slots.iter_mut() {
            slot.ina.bus_mv = 48_000;
        }
        unit
    }

    /// A unit with a module and a running host in both slots.
    pub fn populated() -> Self {
        let mut unit = FakeUnit::new();
        for slot in unit.slots.iter_mut() {
            slot.module = true;
            slot.adc.counts = 0x0400;
            slot.module_pgood = true;
            slot.pgd = true;
            slot.ina.current_ma = 1_000;
            slot.node.on = true;
        }
        unit
    }

    /// Cut the unit's 3V3: the expander resets to all inputs and every
    /// slot goes to its hardware default, which is ON.
    pub fn power_off(&mut self) {
        self.powered = false;
        self.expander = FakeExpander::after_reset();
    }

    /// Give the 3V3 back.
    pub fn power_on(&mut self) {
        self.powered = true;
    }

    /// True while the unit's input is inside the hot-swap's UVLO / OVLO
    /// window (uinput.py R233-R236: on at 38.3 V, off at 64.6 V).
    pub const fn input_in_range(&self) -> bool {
        self.input_mv >= 38_300 && self.input_mv <= 64_600
    }

    /// True while the hot-swap is enabled: U_OFF low (Q204 off, and a
    /// unit with no 3V3 drives nothing) and the input in range.
    pub fn hot_swap_enabled(&self) -> bool {
        !self.expander.asserted(expander::LINE_U_OFF) && self.input_in_range()
    }

    /// True while the unit's own hot-swap passes power to the slots.
    pub const fn unit_rail(&self) -> bool {
        self.hotswap.conducting()
    }

    /// The three input port bytes the expander answers with.
    pub fn input_ports(&self) -> [u8; 3] {
        let mut ports = [0u8; 3];
        let mut set = |line: u8, level: bool| {
            if level {
                ports[(line >> 3) as usize] |= 1 << (line & 7);
            }
        };
        for half in 0..2usize {
            let b = half == 1;
            let slot = &self.slots[half];
            // An output line reads back what it drives; an input line
            // reads the pin, and every unconnected gate has a
            // pull-down.
            for out in [
                Out::Off,
                Out::Lclr,
                Out::Runoff,
                Out::Disarm,
                Out::Pwrd,
                Out::Rstd,
            ] {
                let line = expander::out_line(b, out);
                set(line, self.expander.driven(line).unwrap_or(false));
            }
            set(expander::in_line(b, In::Pgd), slot.pgd);
            set(expander::in_line(b, In::Pg), slot.module_pgood);
            set(expander::in_line(b, In::Lq), slot.ov_latched);
            // The OV_KILL node idles high and is pulled low by a trip
            // source; the latch holds the slot down afterwards.
            set(expander::in_line(b, In::Ovk), !slot.ina.alert());
            // The opto pulls the LED line low while the host's LED is
            // lit.
            set(expander::in_line(b, In::Led), !slot.node.led());
        }
        set(
            expander::LINE_U_OFF,
            self.expander.driven(expander::LINE_U_OFF).unwrap_or(false),
        );
        set(expander::LINE_U_PGD, self.hotswap.pgd());
        ports
    }

    /// Advance the unit's hardware by `dt` milliseconds.
    ///
    /// `press_en` is the core's press deadline: without it the photoMOS
    /// LEDs have no current and no contact closes, whatever the expander
    /// holds.
    pub fn advance(&mut self, dt: u32, press_en: bool) {
        // The hot-swap runs on the unit's own 48 V and does not care
        // about the chain's 3V3: a unit whose cable is out still passes
        // power, because U_OFF then drives nothing (R214).
        self.hotswap.advance(dt, self.hot_swap_enabled());
        let unit_rail = self.unit_rail();
        // The INA237's VBUS pin is on the hot-swap output, so it reads
        // the rail and not the XT60 (uinput.py U208).
        self.input.bus_mv = if unit_rail { self.input_mv } else { 0 };
        if !self.powered {
            for slot in self.slots.iter_mut() {
                slot.pgd = false;
                slot.module_pgood = false;
                // No 3V3 on the unit means no expander, so every slot
                // sits at its hardware default: on. The node keeps
                // running on its rail.
                slot.node.advance(dt, true, false, false);
            }
            return;
        }
        for half in 0..2usize {
            let b = half == 1;
            let off = self.expander.asserted(expander::out_line(b, Out::Off));
            let lclr = self.expander.asserted(expander::out_line(b, Out::Lclr));
            let runoff = self.expander.asserted(expander::out_line(b, Out::Runoff));
            let disarm = self.expander.asserted(expander::out_line(b, Out::Disarm));
            let pwrd = self.expander.asserted(expander::out_line(b, Out::Pwrd)) && press_en;
            let rstd = self.expander.asserted(expander::out_line(b, Out::Rstd)) && press_en;
            let slot = &mut self.slots[half];

            if lclr {
                slot.ov_latched = false;
                slot.ina.diag &= !ina238::DIAG_POL;
            }
            // The INA238's alert is one of the three sources on OV_KILL.
            if slot.ina.over_limit() {
                slot.ina.diag |= ina238::DIAG_POL;
                slot.ov_latched = true;
            }
            // The PGOOD-loss trip, unless the slot is disarmed.
            if !disarm && slot.pgd && slot.module && !slot.module_pgood {
                slot.ov_latched = true;
            }

            let rail = unit_rail && !off && !slot.ov_latched;
            slot.pgd = rail && !core::mem::take(&mut slot.glitch_pgd);
            slot.module_pgood = slot.pgd && slot.module && !runoff;
            if !rail {
                slot.ina.current_ma = 0;
            }
            let node_rail = rail && slot.module && !runoff;
            slot.node.advance(dt, node_rail, pwrd, rstd);
        }
        // The unit monitor's shunt is on the input side, so it carries
        // the two slots' current.
        self.input.current_ma = self.slots.iter().map(|s| s.ina.current_ma).sum();
    }
}
