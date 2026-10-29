//! INA238 current / voltage / power monitor: one per slot (0x40 slot A,
//! 0x41 slot B) and one per unit input (0x44).
//!
//! Register numbers, the SHUNT_CAL equation, the LSB sizes and the
//! DIAG_ALRT field positions are from the INA238 datasheet (SBOS881,
//! "Register map" and the per-register field tables). The parts that are
//! not obvious:
//!
//! - `SHUNT_CAL = 819.2e6 x CURRENT_LSB x R_shunt` with ADCRANGE = 0
//!   (the equation is x4 for ADCRANGE = 1). The plane picks
//!   `CURRENT_LSB = 1 mA`, so the CURRENT register *is* milliamps, which
//!   is the unit the link wants (ADR 0003 decision 3) and keeps every
//!   conversion an integer shift.
//! - ADCRANGE stays 0 (+/-163.84 mV). A slot shunt of 3.5 mOhm at its
//!   30 A breaker is 105 mV, past the 40.96 mV of ADCRANGE = 1.
//! - VBUS LSB is 3.125 mV, fixed. Power LSB is `0.2 x CURRENT_LSB`.
//! - DIETEMP is a 12-bit two's-complement value in bits 15-4 with a
//!   125 m C LSB.
//! - PWR_LIMIT compares against POWER scaled by `256 x Power LSB`.
//!
//! The slot parts' ALERT is wired into the slot's OV latch (unit.py:
//! "the INA238 ALERT (power limit, ALATCH; firmware sets the limit at
//! boot, the reset value never trips)"), so the plane programs the limit
//! with ALATCH set: the alert latches until DIAG_ALRT is read, which is
//! how the firmware tells a power trip from the module's own OV_KILL.
//! The unit part's ALERT is not wired (uinput.py), so its limit is only
//! read back.

use crate::device::{DevResult, SegmentBus};

/// Slot A's monitor (A1 GND, A0 GND).
pub const INA_SLOT_A: u8 = 0x40;
/// Slot B's monitor (A0 to 3V3).
pub const INA_SLOT_B: u8 = 0x41;
/// The unit input's monitor (A1 to VS).
pub const INA_UNIT: u8 = 0x44;

/// CONFIG: RST (bit 15), CONVDLY (13-6), ADCRANGE (bit 4).
pub const REG_CONFIG: u8 = 0x00;
/// ADC_CONFIG: MODE (15-12), VBUSCT, VSHCT, VTCT, AVG.
pub const REG_ADC_CONFIG: u8 = 0x01;
/// SHUNT_CAL: the current scaling.
pub const REG_SHUNT_CAL: u8 = 0x02;
/// VSHUNT, signed.
pub const REG_VSHUNT: u8 = 0x04;
/// VBUS, 3.125 mV / LSB.
pub const REG_VBUS: u8 = 0x05;
/// DIETEMP, 12 bits in 15-4, 125 m C / LSB.
pub const REG_DIETEMP: u8 = 0x06;
/// CURRENT, signed, CURRENT_LSB / LSB.
pub const REG_CURRENT: u8 = 0x07;
/// POWER, 24 bits unsigned, `0.2 x CURRENT_LSB` / LSB.
pub const REG_POWER: u8 = 0x08;
/// DIAG_ALRT: the alert configuration and the sticky fault flags.
pub const REG_DIAG_ALRT: u8 = 0x0B;
/// PWR_LIMIT, unsigned, `256 x Power LSB` / LSB.
pub const REG_PWR_LIMIT: u8 = 0x11;
/// MANUFACTURER_ID, always 0x5449 ("TI"): the presence probe.
pub const REG_MANUFACTURER_ID: u8 = 0x3E;

/// What MANUFACTURER_ID reads on a real part.
pub const MANUFACTURER_ID: u16 = 0x5449;

/// CONFIG with ADCRANGE 0 and no conversion delay.
pub const CONFIG_VALUE: u16 = 0x0000;

/// CONFIG with the software reset bit, used by nothing in the field: a
/// unit that needs resetting lost its 3V3 anyway.
pub const CONFIG_RESET: u16 = 1 << 15;

/// ADC_CONFIG: continuous bus, shunt and temperature (MODE 0xF), 1052 us
/// per conversion, 4 averages. One full cycle is ~12.6 ms, inside the
/// 50 ms poll of ADR 0002 decision 5.
pub const ADC_CONFIG_VALUE: u16 = (0xF << 12) | (0b101 << 9) | (0b101 << 6) | (0b101 << 3) | 0b001;

/// DIAG_ALRT bit 15: the alert pin latches until DIAG_ALRT is read.
pub const DIAG_ALATCH: u16 = 1 << 15;
/// DIAG_ALRT bit 12: alert polarity; 0 keeps the open-drain pull-down,
/// which is what the slot's OV latch wants.
pub const DIAG_APOL: u16 = 1 << 12;
/// DIAG_ALRT bit 10: an arithmetic overflow in the power calculation.
pub const DIAG_MATHOF: u16 = 1 << 10;
/// DIAG_ALRT bit 2: POWER passed PWR_LIMIT. The flag the plane reads to
/// call a slot's latch a power trip (SLOT 0x20 "power limit tripped").
pub const DIAG_POL: u16 = 1 << 2;
/// DIAG_ALRT bit 1: a conversion finished.
pub const DIAG_CNVRF: u16 = 1 << 1;

/// DIAG_ALRT as the plane programs it: latch the alert, pull low on it,
/// and no other source enabled (bus and shunt limits stay at their reset
/// values, which never trip).
pub const DIAG_ALRT_VALUE: u16 = DIAG_ALATCH;

/// Current per CURRENT LSB \[uA\]: the plane's choice of 1 mA.
pub const CURRENT_LSB_UA: u32 = 1000;

/// A slot's shunt \[uOhm\]: two 7 mOhm in parallel (unit.py R1 || R2).
pub const SLOT_SHUNT_UOHM: u32 = 3500;

/// A unit input's shunt \[uOhm\]: two 6 mOhm in parallel (uinput.py
/// R201 || R202).
pub const UNIT_SHUNT_UOHM: u32 = 3000;

/// SHUNT_CAL for a shunt of `shunt_uohm`, at `CURRENT_LSB = 1 mA` and
/// ADCRANGE = 0.
///
/// `819.2e6 x 1e-3 A x (shunt_uohm x 1e-6 Ohm) = 0.8192 x shunt_uohm`,
/// rounded: `shunt_uohm x 8192 / 10000`.
pub const fn shunt_cal(shunt_uohm: u32) -> u16 {
    let raw = (shunt_uohm as u64 * 8192 + 5000) / 10_000;
    if raw > u16::MAX as u64 {
        u16::MAX
    } else {
        raw as u16
    }
}

/// A slot's SHUNT_CAL (3.5 mOhm).
pub const SLOT_SHUNT_CAL: u16 = shunt_cal(SLOT_SHUNT_UOHM);
/// A unit input's SHUNT_CAL (3 mOhm).
pub const UNIT_SHUNT_CAL: u16 = shunt_cal(UNIT_SHUNT_UOHM);

/// One sample of a monitor, already in the link's units.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Sample {
    /// Bus voltage \[10 mV\].
    pub volts: u16,
    /// Current \[mA\], negative readings clamped to 0 (the plane only
    /// ever sources).
    pub amps: u16,
    /// Power \[0.1 W\].
    pub watts: u16,
    /// Die temperature \[0.1 C\].
    pub temperature: i16,
}

/// VBUS register to 10 mV: `reg x 3.125 mV / 10 mV = reg x 5 / 16`.
pub const fn volts_10mv(reg: u16) -> u16 {
    ((reg as u32 * 5) >> 4) as u16
}

/// CURRENT register to mA. CURRENT_LSB is 1 mA, so the register is
/// already milliamps; a negative reading is clamped to 0.
pub const fn amps_ma(reg: u16) -> u16 {
    let signed = reg as i16;
    if signed < 0 { 0 } else { signed as u16 }
}

/// POWER register (24 bits) to 0.1 W.
///
/// Power LSB is `0.2 x CURRENT_LSB = 0.2 mW`, so `reg x 0.0002 W` and
/// one wire unit is 0.1 W: `reg / 500`. Saturates rather than wrapping.
pub const fn watts_deciwatt(reg: u32) -> u16 {
    let w = reg / 500;
    if w > u16::MAX as u32 { u16::MAX } else { w as u16 }
}

/// DIETEMP register to 0.1 C: 12 bits in 15-4, 125 m C per LSB, so
/// `(reg >> 4) x 1.25` wire units.
pub const fn temperature_decidegrees(reg: u16) -> i16 {
    let counts = (reg as i16) >> 4; // arithmetic: sign-extends the 12 bits
    ((counts as i32 * 5) / 4) as i16
}

/// A power limit of `deciwatt` \[0.1 W\] as PWR_LIMIT.
///
/// PWR_LIMIT is compared against POWER divided by 256, so one PWR_LIMIT
/// unit is `256 x 0.2 mW = 51.2 mW`: `deciwatt x 100 / 51.2`, that is
/// `deciwatt x 125 / 64`. 0 means "no limit" on the wire and is written
/// as the register's maximum, which no slot can reach.
pub const fn power_limit_register(deciwatt: u16) -> u16 {
    if deciwatt == 0 {
        return u16::MAX;
    }
    let raw = (deciwatt as u32 * 125) / 64;
    if raw > u16::MAX as u32 {
        u16::MAX
    } else {
        raw as u16
    }
}

/// The limit a PWR_LIMIT register stands for \[0.1 W\], the inverse of
/// [`power_limit_register`].
pub const fn power_limit_deciwatt(reg: u16) -> u16 {
    if reg == u16::MAX {
        return 0;
    }
    ((reg as u32 * 64) / 125) as u16
}

/// One INA238 on a unit segment.
#[derive(Debug, Clone, Copy)]
pub struct Ina238 {
    addr: u8,
    cal: u16,
}

impl Ina238 {
    /// A monitor at `addr` whose shunt asks for `cal`
    /// ([`SLOT_SHUNT_CAL`] or [`UNIT_SHUNT_CAL`]).
    pub const fn new(addr: u8, cal: u16) -> Self {
        Ina238 { addr, cal }
    }

    /// A slot monitor: address by half, slot shunt.
    pub const fn slot(b: bool) -> Self {
        Ina238::new(if b { INA_SLOT_B } else { INA_SLOT_A }, SLOT_SHUNT_CAL)
    }

    /// A unit input monitor.
    pub const fn unit() -> Self {
        Ina238::new(INA_UNIT, UNIT_SHUNT_CAL)
    }

    /// The part's I2C address.
    pub const fn addr(&self) -> u8 {
        self.addr
    }

    /// Read MANUFACTURER_ID: 0x5449 on a part that is there.
    pub fn manufacturer_id<B: SegmentBus>(&self, bus: &mut B) -> DevResult<u16> {
        self.read_u16(bus, REG_MANUFACTURER_ID)
    }

    /// Configure a monitor that just appeared: range, conversion,
    /// scaling, the latched alert and the power limit.
    ///
    /// `power_limit_deciwatt` of 0 leaves the limit unreachable.
    pub fn configure<B: SegmentBus>(
        &self,
        bus: &mut B,
        power_limit_deciwatt: u16,
    ) -> DevResult<()> {
        self.write_u16(bus, REG_CONFIG, CONFIG_VALUE)?;
        self.write_u16(bus, REG_ADC_CONFIG, ADC_CONFIG_VALUE)?;
        self.write_u16(bus, REG_SHUNT_CAL, self.cal)?;
        self.set_power_limit(bus, power_limit_deciwatt)?;
        // Last: arming the alert after the limit means the limit is
        // never compared against a stale threshold.
        self.write_u16(bus, REG_DIAG_ALRT, DIAG_ALRT_VALUE)
    }

    /// Write PWR_LIMIT \[0.1 W\]; 0 disables the limit.
    pub fn set_power_limit<B: SegmentBus>(&self, bus: &mut B, deciwatt: u16) -> DevResult<()> {
        self.write_u16(bus, REG_PWR_LIMIT, power_limit_register(deciwatt))
    }

    /// Read PWR_LIMIT back \[0.1 W\].
    pub fn power_limit<B: SegmentBus>(&self, bus: &mut B) -> DevResult<u16> {
        Ok(power_limit_deciwatt(self.read_u16(bus, REG_PWR_LIMIT)?))
    }

    /// Read VBUS, CURRENT, POWER and DIETEMP as the link's units.
    ///
    /// Four transactions: the registers are not contiguous, and the
    /// INA238 has no block read that spans them.
    pub fn sample<B: SegmentBus>(&self, bus: &mut B) -> DevResult<Sample> {
        let volts = volts_10mv(self.read_u16(bus, REG_VBUS)?);
        let amps = amps_ma(self.read_u16(bus, REG_CURRENT)?);
        let watts = watts_deciwatt(self.read_u24(bus, REG_POWER)?);
        let temperature = temperature_decidegrees(self.read_u16(bus, REG_DIETEMP)?);
        Ok(Sample {
            volts,
            amps,
            watts,
            temperature,
        })
    }

    /// Read DIAG_ALRT. The read clears the latched flags, so the caller
    /// keeps what it saw.
    pub fn diag_alrt<B: SegmentBus>(&self, bus: &mut B) -> DevResult<u16> {
        self.read_u16(bus, REG_DIAG_ALRT)
    }

    fn read_u16<B: SegmentBus>(&self, bus: &mut B, reg: u8) -> DevResult<u16> {
        let mut v = [0u8; 2];
        bus.read(self.addr, &[reg], &mut v)?;
        Ok(u16::from_be_bytes(v))
    }

    fn read_u24<B: SegmentBus>(&self, bus: &mut B, reg: u8) -> DevResult<u32> {
        let mut v = [0u8; 3];
        bus.read(self.addr, &[reg], &mut v)?;
        Ok((v[0] as u32) << 16 | (v[1] as u32) << 8 | v[2] as u32)
    }

    fn write_u16<B: SegmentBus>(&self, bus: &mut B, reg: u8, value: u16) -> DevResult<()> {
        let v = value.to_be_bytes();
        bus.write(self.addr, &[reg, v[0], v[1]])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shunt_cal_for_both_shunts() {
        // 0.8192 x 3500 = 2867.2, 0.8192 x 3000 = 2457.6.
        assert_eq!(SLOT_SHUNT_CAL, 2867);
        assert_eq!(UNIT_SHUNT_CAL, 2458);
    }

    #[test]
    fn scaling_lands_on_the_wire_units() {
        // 48.00 V is 15360 counts of 3.125 mV.
        assert_eq!(volts_10mv(15_360), 4800);
        assert_eq!(volts_10mv(0), 0);
        // CURRENT_LSB is 1 mA; a negative reading reads as zero.
        assert_eq!(amps_ma(2_300), 2300);
        assert_eq!(amps_ma(0xFFFF), 0);
        // 110.4 W is 552000 power counts of 0.2 mW.
        assert_eq!(watts_deciwatt(552_000), 1104);
        // A full 24-bit POWER is 3355.4 W, inside the wire's u16.
        assert_eq!(watts_deciwatt(0x00FF_FFFF), 33_554);
        assert_eq!(watts_deciwatt(u32::MAX), u16::MAX);
        // 25.0 C is 200 counts of 125 m C, in bits 15-4.
        assert_eq!(temperature_decidegrees(200 << 4), 250);
        // -40.0 C is -320 counts of 125 m C.
        assert_eq!(temperature_decidegrees((-320i16 << 4) as u16), -400);
    }

    #[test]
    fn power_limit_round_trips_near_enough() {
        // 75 W, the unit input's design limit.
        let reg = power_limit_register(750);
        assert_eq!(reg, 1464);
        assert_eq!(power_limit_deciwatt(reg), 749);
        // 0 is "no limit" both ways.
        assert_eq!(power_limit_register(0), u16::MAX);
        assert_eq!(power_limit_deciwatt(u16::MAX), 0);
        // Nothing wraps at the top.
        assert_eq!(power_limit_register(u16::MAX), u16::MAX);
    }
}
