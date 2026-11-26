//! The current / voltage / power monitors: one per slot (0x40 slot A,
//! 0x41 slot B) and one per unit input (0x44).
//!
//! Two parts answer there, pin-compatible in the same VSSOP-10 and
//! sharing the register numbers, the address map and the ADC_CONFIG /
//! CONFIG layout, but not the scaling:
//!
//! - **INA237 / INA238** (SBOS881), on the FD batch 1 units.
//! - **INA228** (SLYS021A), on the FD batch 2 units (pp-48-unit-fd
//!   U2 / U52 / U208): the INA237/238 were out of stock. 85 V, a 20-bit
//!   ADC, and the measurement registers 24 bits wide.
//!
//! One plane carries both kinds of unit, so the kind is a run-time
//! property of a chip, read from DEVICE_ID once in
//! [`Ina::configure`] and kept per channel by discovery. Every
//! quantity this module hands up is in the link's units (10 mV, mA,
//! 0.1 W, 0.1 C) and means the same thing whichever part produced it.
//!
//! What is not obvious, per part:
//!
//! - `SHUNT_CAL = 819.2e6 x CURRENT_LSB x R_shunt` (INA238) or
//!   `13107.2e6 x CURRENT_LSB x R_shunt` (INA228), both with
//!   ADCRANGE = 0 and both x4 for ADCRANGE = 1. The INA238 path picks
//!   `CURRENT_LSB = 1 mA`, so its CURRENT register *is* milliamps,
//!   which is the unit the link wants (ADR 0003 decision 3). The
//!   INA228's constant is 16x larger: 1 mA at the slot's 3.5 mOhm
//!   would want 45875, past the 15 bits of its SHUNT_CAL field, so the
//!   INA228 path halves it to `CURRENT_LSB = 0.5 mA` (22938 at
//!   3.5 mOhm, 19661 at the unit's 3 mOhm) and its CURRENT register is
//!   half-milliamps.
//! - ADCRANGE stays 0 on both (+/-163.84 mV). A slot shunt of 3.5 mOhm
//!   at its 30 A breaker is 105 mV, past the 40.96 mV of ADCRANGE = 1.
//! - VBUS LSB is fixed: 3.125 mV (INA238), 195.3125 uV (INA228).
//!   Power LSB is `0.2 x CURRENT_LSB` (INA238) or
//!   `3.2 x CURRENT_LSB` (INA228).
//! - VSHUNT, VBUS and CURRENT are 16-bit on the INA238 and 24-bit on
//!   the INA228, where the data sits in bits 23-4 and 3-0 read zero.
//!   POWER is 24-bit unsigned on both.
//! - DIETEMP is a 12-bit two's-complement value in bits 15-4 with a
//!   125 m C LSB on the INA238, and all 16 bits at 7.8125 m C on the
//!   INA228.
//! - PWR_LIMIT compares against POWER scaled by `256 x Power LSB` on
//!   both, so its step follows the part's Power LSB: 51.2 mW on the
//!   INA238, 409.6 mW on the INA228.
//! - DIAG_ALRT's bits are in the same places for everything the plane
//!   uses (ALATCH 15, APOL 12, POL 2, CNVRF 1). The INA228 moves
//!   MATHOF to bit 9 and puts ENERGYOF / CHARGEOF at 11 / 10.
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

/// CONFIG: RST (bit 15), CONVDLY (13-6), ADCRANGE (bit 4). The INA228
/// also has RSTACC (14) and TEMPCOMP (5), both 0 in
/// [`CONFIG_VALUE`].
pub const REG_CONFIG: u8 = 0x00;
/// ADC_CONFIG: MODE (15-12), VBUSCT, VSHCT, VTCT, AVG.
pub const REG_ADC_CONFIG: u8 = 0x01;
/// SHUNT_CAL: the current scaling.
pub const REG_SHUNT_CAL: u8 = 0x02;
/// VSHUNT, signed; 16-bit (INA238) or 24-bit (INA228).
pub const REG_VSHUNT: u8 = 0x04;
/// VBUS; 16-bit at 3.125 mV / LSB (INA238) or 24-bit at
/// 195.3125 uV / LSB (INA228).
pub const REG_VBUS: u8 = 0x05;
/// DIETEMP; 12 bits in 15-4 at 125 m C (INA238) or all 16 bits at
/// 7.8125 m C (INA228).
pub const REG_DIETEMP: u8 = 0x06;
/// CURRENT, signed, CURRENT_LSB / LSB; 16-bit (INA238) or 24-bit
/// (INA228).
pub const REG_CURRENT: u8 = 0x07;
/// POWER, 24 bits unsigned; `0.2 x CURRENT_LSB` (INA238) or
/// `3.2 x CURRENT_LSB` (INA228) per LSB.
pub const REG_POWER: u8 = 0x08;
/// DIAG_ALRT: the alert configuration and the sticky fault flags.
pub const REG_DIAG_ALRT: u8 = 0x0B;
/// PWR_LIMIT, unsigned, `256 x Power LSB` / LSB.
pub const REG_PWR_LIMIT: u8 = 0x11;
/// MANUFACTURER_ID, always 0x5449 ("TI"): the presence probe.
pub const REG_MANUFACTURER_ID: u8 = 0x3E;
/// DEVICE_ID: DIEID in bits 15-4, the silicon revision in 3-0.
pub const REG_DEVICE_ID: u8 = 0x3F;

/// What MANUFACTURER_ID reads on a real part, either kind.
pub const MANUFACTURER_ID: u16 = 0x5449;

/// DEVICE_ID's DIEID (bits 15-4) on an INA228 (reset 0x2281).
pub const DIEID_INA228: u16 = 0x228;
/// DEVICE_ID's DIEID on an INA238 (reset 0x2381).
pub const DIEID_INA238: u16 = 0x238;
/// DEVICE_ID's DIEID on an INA237.
pub const DIEID_INA237: u16 = 0x237;

/// Which part sits at an address.
///
/// Only the scaling differs; everything the plane writes is the same
/// word on both, so the kind is read once per chip at configure time
/// and then only consulted by the conversions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Kind {
    /// INA237 / INA238, `CURRENT_LSB = 1 mA`. Also what an unreadable
    /// or unknown DEVICE_ID is treated as, which is the plane's
    /// behaviour from before the INA228 existed on a unit.
    #[default]
    Ina238,
    /// INA228, `CURRENT_LSB = 0.5 mA`.
    Ina228,
}

/// The kind a DEVICE_ID stands for; `None` is an id no part of either
/// family claims, which the caller logs and treats as [`Kind::Ina238`].
pub const fn kind_from_device_id(id: u16) -> Option<Kind> {
    match id >> 4 {
        DIEID_INA228 => Some(Kind::Ina228),
        DIEID_INA238 | DIEID_INA237 => Some(Kind::Ina238),
        _ => None,
    }
}

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
/// The INA228 has it at bit 9 ([`DIAG_MATHOF_228`]) and uses bit 10 for
/// CHARGEOF. The plane reads neither.
pub const DIAG_MATHOF: u16 = 1 << 10;
/// DIAG_ALRT bit 9: the INA228's MATHOF.
pub const DIAG_MATHOF_228: u16 = 1 << 9;
/// DIAG_ALRT bit 2: POWER passed PWR_LIMIT. The flag the plane reads to
/// call a slot's latch a power trip (SLOT 0x20 "power limit tripped").
pub const DIAG_POL: u16 = 1 << 2;
/// DIAG_ALRT bit 1: a conversion finished.
pub const DIAG_CNVRF: u16 = 1 << 1;

/// DIAG_ALRT as the plane programs it: latch the alert, pull low on it,
/// and no other source enabled (bus and shunt limits stay at their reset
/// values, which never trip).
pub const DIAG_ALRT_VALUE: u16 = DIAG_ALATCH;

/// Current per CURRENT LSB \[uA\] on an INA238: the plane's choice of
/// 1 mA.
pub const CURRENT_LSB_UA: u32 = 1000;

/// Current per CURRENT LSB \[uA\] on an INA228: 0.5 mA, the largest
/// round value whose SHUNT_CAL still fits the part's 15-bit field at
/// the slot's 3.5 mOhm.
pub const CURRENT_LSB_UA_228: u32 = 500;

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

/// SHUNT_CAL for a shunt of `shunt_uohm` on an INA228, at
/// `CURRENT_LSB = 0.5 mA` and ADCRANGE = 0.
///
/// `13107.2e6 x 0.5e-3 A x (shunt_uohm x 1e-6 Ohm) = 6.5536 x
/// shunt_uohm`, rounded: `shunt_uohm x 65536 / 10000`. The field is
/// 14-0, so the clamp is 15 bits and not 16.
pub const fn shunt_cal_228(shunt_uohm: u32) -> u16 {
    let raw = (shunt_uohm as u64 * 65_536 + 5_000) / 10_000;
    if raw > SHUNT_CAL_MAX_228 as u64 {
        SHUNT_CAL_MAX_228
    } else {
        raw as u16
    }
}

/// The largest SHUNT_CAL an INA228 takes: the field is bits 14-0.
pub const SHUNT_CAL_MAX_228: u16 = 0x7FFF;

/// A slot's SHUNT_CAL (3.5 mOhm) on an INA238.
pub const SLOT_SHUNT_CAL: u16 = shunt_cal(SLOT_SHUNT_UOHM);
/// A unit input's SHUNT_CAL (3 mOhm) on an INA238.
pub const UNIT_SHUNT_CAL: u16 = shunt_cal(UNIT_SHUNT_UOHM);
/// A slot's SHUNT_CAL on an INA228.
pub const SLOT_SHUNT_CAL_228: u16 = shunt_cal_228(SLOT_SHUNT_UOHM);
/// A unit input's SHUNT_CAL on an INA228.
pub const UNIT_SHUNT_CAL_228: u16 = shunt_cal_228(UNIT_SHUNT_UOHM);

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

/// The 20-bit signed payload of an INA228 measurement register, whose
/// data is in bits 23-4 and whose bits 3-0 always read 0.
pub const fn payload_228(reg: u32) -> i32 {
    // Shift the sign bit to the top of an i32, then back: the 32-bit
    // arithmetic shift sign-extends the 20 bits for free.
    ((reg << 8) as i32) >> 12
}

/// VBUS register to 10 mV: `reg x 3.125 mV / 10 mV = reg x 5 / 16`.
pub const fn volts_10mv(reg: u16) -> u16 {
    ((reg as u32 * 5) >> 4) as u16
}

/// INA228 VBUS register to 10 mV.
///
/// The LSB is 195.3125 uV, which is `10 mV x 5 / 256` exactly, and VBUS
/// is never negative (the part says so): `payload x 5 / 256`. Full
/// scale is 85 V, so nothing saturates.
pub const fn volts_10mv_228(reg: u32) -> u16 {
    let counts = payload_228(reg);
    if counts <= 0 {
        return 0;
    }
    ((counts as u32 * 5) >> 8) as u16
}

/// CURRENT register to mA. CURRENT_LSB is 1 mA, so the register is
/// already milliamps; a negative reading is clamped to 0.
pub const fn amps_ma(reg: u16) -> u16 {
    let signed = reg as i16;
    if signed < 0 { 0 } else { signed as u16 }
}

/// INA228 CURRENT register to mA: CURRENT_LSB is 0.5 mA, so the
/// payload is half-milliamps. Negative clamps to 0, and the top
/// saturates rather than wrapping (the shunt cannot get there: 163.84 mV
/// over 3 mOhm is 54.6 A).
pub const fn amps_ma_228(reg: u32) -> u16 {
    let counts = payload_228(reg);
    if counts <= 0 {
        return 0;
    }
    let ma = (counts as u32) / 2;
    if ma > u16::MAX as u32 { u16::MAX } else { ma as u16 }
}

/// POWER register (24 bits) to 0.1 W.
///
/// Power LSB is `0.2 x CURRENT_LSB = 0.2 mW`, so `reg x 0.0002 W` and
/// one wire unit is 0.1 W: `reg / 500`. Saturates rather than wrapping.
pub const fn watts_deciwatt(reg: u32) -> u16 {
    let w = reg / 500;
    if w > u16::MAX as u32 { u16::MAX } else { w as u16 }
}

/// INA228 POWER register (24 bits, unsigned) to 0.1 W.
///
/// Power LSB is `3.2 x CURRENT_LSB = 1.6 mW`, so `reg x 0.0016 W` and
/// one wire unit is 0.1 W: `reg x 2 / 125`. A full 24-bit POWER is
/// 26.8 kW, well past the wire's u16, so this saturates.
pub const fn watts_deciwatt_228(reg: u32) -> u16 {
    let w = (reg & 0x00FF_FFFF) * 2 / 125;
    if w > u16::MAX as u32 { u16::MAX } else { w as u16 }
}

/// DIETEMP register to 0.1 C: 12 bits in 15-4, 125 m C per LSB, so
/// `(reg >> 4) x 1.25` wire units.
pub const fn temperature_decidegrees(reg: u16) -> i16 {
    let counts = (reg as i16) >> 4; // arithmetic: sign-extends the 12 bits
    ((counts as i32 * 5) / 4) as i16
}

/// INA228 DIETEMP register to 0.1 C: all 16 bits, 7.8125 m C per LSB,
/// so `reg x 5 / 64` wire units. The part's range is -40 to +125 C, so
/// the i16 cannot be reached.
pub const fn temperature_decidegrees_228(reg: u16) -> i16 {
    ((reg as i16) as i32 * 5 / 64) as i16
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

/// A power limit of `deciwatt` \[0.1 W\] as an INA228 PWR_LIMIT.
///
/// One PWR_LIMIT unit is `256 x 1.6 mW = 409.6 mW`, so
/// `deciwatt x 100 / 409.6`, that is `deciwatt x 125 / 512`. The step
/// is 8x the INA238's, which is as fine as the part goes; 0 is "no
/// limit" and written as the register's maximum, which no real
/// deciwatt maps onto.
pub const fn power_limit_register_228(deciwatt: u16) -> u16 {
    if deciwatt == 0 {
        return u16::MAX;
    }
    (deciwatt as u32 * 125 / 512) as u16
}

/// The limit an INA228 PWR_LIMIT register stands for \[0.1 W\], the
/// inverse of [`power_limit_register_228`].
pub const fn power_limit_deciwatt_228(reg: u16) -> u16 {
    if reg == u16::MAX {
        return 0;
    }
    let dw = reg as u32 * 512 / 125;
    if dw > u16::MAX as u32 { u16::MAX } else { dw as u16 }
}

/// One monitor on a unit segment, of either kind.
#[derive(Debug, Clone, Copy)]
pub struct Ina {
    addr: u8,
    kind: Kind,
    cal: u16,
}

impl Ina {
    /// A monitor at `addr` of kind `kind`, on a shunt of
    /// `shunt_uohm` ([`SLOT_SHUNT_UOHM`] or [`UNIT_SHUNT_UOHM`]).
    pub const fn new(addr: u8, kind: Kind, shunt_uohm: u32) -> Self {
        let cal = match kind {
            Kind::Ina238 => shunt_cal(shunt_uohm),
            Kind::Ina228 => shunt_cal_228(shunt_uohm),
        };
        Ina { addr, kind, cal }
    }

    /// A slot monitor: address by half, slot shunt.
    pub const fn slot(b: bool, kind: Kind) -> Self {
        Ina::new(
            if b { INA_SLOT_B } else { INA_SLOT_A },
            kind,
            SLOT_SHUNT_UOHM,
        )
    }

    /// A unit input monitor.
    pub const fn unit(kind: Kind) -> Self {
        Ina::new(INA_UNIT, kind, UNIT_SHUNT_UOHM)
    }

    /// The part's I2C address.
    pub const fn addr(&self) -> u8 {
        self.addr
    }

    /// Which part this is.
    pub const fn kind(&self) -> Kind {
        self.kind
    }

    /// Read MANUFACTURER_ID: 0x5449 on a part that is there.
    pub fn manufacturer_id<B: SegmentBus>(&self, bus: &mut B) -> DevResult<u16> {
        self.read_u16(bus, REG_MANUFACTURER_ID)
    }

    /// Read DEVICE_ID and say which part answered.
    ///
    /// `None` is an id neither family claims: the caller logs it and
    /// goes on as [`Kind::Ina238`].
    pub fn device_kind<B: SegmentBus>(&self, bus: &mut B) -> DevResult<(u16, Option<Kind>)> {
        let id = self.read_u16(bus, REG_DEVICE_ID)?;
        Ok((id, kind_from_device_id(id)))
    }

    /// Configure a monitor that just appeared: range, conversion,
    /// scaling, the latched alert and the power limit.
    ///
    /// Every word here is the same on both kinds except SHUNT_CAL,
    /// which this part's [`Ina::new`] already picked.
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
        let reg = match self.kind {
            Kind::Ina238 => power_limit_register(deciwatt),
            Kind::Ina228 => power_limit_register_228(deciwatt),
        };
        self.write_u16(bus, REG_PWR_LIMIT, reg)
    }

    /// Read PWR_LIMIT back \[0.1 W\].
    pub fn power_limit<B: SegmentBus>(&self, bus: &mut B) -> DevResult<u16> {
        let reg = self.read_u16(bus, REG_PWR_LIMIT)?;
        Ok(match self.kind {
            Kind::Ina238 => power_limit_deciwatt(reg),
            Kind::Ina228 => power_limit_deciwatt_228(reg),
        })
    }

    /// Read VBUS, CURRENT, POWER and DIETEMP as the link's units.
    ///
    /// Four transactions: the registers are not contiguous, and neither
    /// part has a block read that spans them. On an INA228 three of the
    /// four are 24 bits wide.
    pub fn sample<B: SegmentBus>(&self, bus: &mut B) -> DevResult<Sample> {
        if self.kind == Kind::Ina228 {
            let volts = volts_10mv_228(self.read_u24(bus, REG_VBUS)?);
            let amps = amps_ma_228(self.read_u24(bus, REG_CURRENT)?);
            let watts = watts_deciwatt_228(self.read_u24(bus, REG_POWER)?);
            let temperature = temperature_decidegrees_228(self.read_u16(bus, REG_DIETEMP)?);
            return Ok(Sample {
                volts,
                amps,
                watts,
                temperature,
            });
        }
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
    fn ina228_shunt_cal_fits_its_fifteen_bits() {
        // 6.5536 x 3500 = 22937.6, 6.5536 x 3000 = 19660.8, both under
        // the 0x7FFF of the SHUNT_CAL field. At CURRENT_LSB = 1 mA the
        // slot would have wanted 45875, which does not fit: that is why
        // the INA228 path halves the LSB.
        assert_eq!(SLOT_SHUNT_CAL_228, 22_938);
        assert_eq!(UNIT_SHUNT_CAL_228, 19_661);
        const { assert!(SLOT_SHUNT_CAL_228 <= SHUNT_CAL_MAX_228) };
        const { assert!(UNIT_SHUNT_CAL_228 <= SHUNT_CAL_MAX_228) };
        // Nothing wraps on an absurd shunt.
        assert_eq!(shunt_cal_228(100_000), SHUNT_CAL_MAX_228);
    }

    #[test]
    fn device_id_picks_the_kind() {
        // The reset values of the two datasheets.
        assert_eq!(kind_from_device_id(0x2281), Some(Kind::Ina228));
        assert_eq!(kind_from_device_id(0x2381), Some(Kind::Ina238));
        // An INA237 and any other revision of either die.
        assert_eq!(kind_from_device_id(0x2370), Some(Kind::Ina238));
        assert_eq!(kind_from_device_id(0x228F), Some(Kind::Ina228));
        // Not a part this driver knows.
        assert_eq!(kind_from_device_id(0x0000), None);
        assert_eq!(kind_from_device_id(0x2260), None);
        // The fallback an unknown id gets.
        assert_eq!(Kind::default(), Kind::Ina238);
    }

    #[test]
    fn ina228_scaling_lands_on_the_same_wire_units() {
        // 48.00 V is 245760 counts of 195.3125 uV (SLYS021A Table 8-2),
        // in bits 23-4.
        assert_eq!(volts_10mv_228(245_760 << 4), 4800);
        assert_eq!(volts_10mv_228(0), 0);
        // 85 V, the part's full scale, still fits the wire's u16.
        assert_eq!(volts_10mv_228(435_200 << 4), 8500);
        // CURRENT_LSB is 0.5 mA: 2.300 A is 4600 counts. A negative
        // reading reads as zero.
        assert_eq!(amps_ma_228(4_600 << 4), 2300);
        assert_eq!(amps_ma_228((-4_600i32 << 4) as u32 & 0x00FF_FFFF), 0);
        // 110.4 W is 69000 power counts of 1.6 mW.
        assert_eq!(watts_deciwatt_228(69_000), 1104);
        // A full 24-bit POWER is 26.8 kW, past the wire: saturate.
        assert_eq!(watts_deciwatt_228(0x00FF_FFFF), u16::MAX);
        // 25.0 C is 3200 counts of 7.8125 m C (SLYS021A Table 8-2),
        // all 16 bits.
        assert_eq!(temperature_decidegrees_228(3_200), 250);
        // -40.0 C is -5120 counts.
        assert_eq!(temperature_decidegrees_228((-5_120i16) as u16), -400);
    }

    #[test]
    fn both_kinds_agree_on_the_same_physical_values() {
        // 48.00 V, 2.300 A, 110.4 W, 25.0 C through each part's own
        // registers has to come out as the same wire record.
        assert_eq!(volts_10mv(15_360), volts_10mv_228(245_760 << 4));
        assert_eq!(amps_ma(2_300), amps_ma_228(4_600 << 4));
        assert_eq!(watts_deciwatt(552_000), watts_deciwatt_228(69_000));
        assert_eq!(
            temperature_decidegrees(200 << 4),
            temperature_decidegrees_228(3_200)
        );
    }

    #[test]
    fn ina228_power_limit_round_trips_near_enough() {
        // 75 W, the unit input's design limit: 183 x 409.6 mW is the
        // same 74.9568 W threshold the INA238's 1464 x 51.2 mW sets.
        let reg = power_limit_register_228(750);
        assert_eq!(reg, 183);
        assert_eq!(power_limit_deciwatt_228(reg), 749);
        assert_eq!(reg as u32 * 4096, power_limit_register(750) as u32 * 512);
        // A limit that is exact in both: 51.2 W.
        assert_eq!(power_limit_register_228(512), 125);
        assert_eq!(power_limit_deciwatt_228(125), 512);
        // Every limit a slot can be given comes back inside one
        // register step (4.096 wire units), at either shunt - the
        // scaling has no shunt in it, the SHUNT_CAL does.
        let mut dw = 1u16;
        while dw < 10_000 {
            let back = power_limit_deciwatt_228(power_limit_register_228(dw));
            assert!(back <= dw && dw - back <= 5, "{dw} -> {back}");
            dw += 1;
        }
        // 0 is "no limit" both ways, and nothing wraps at the top.
        assert_eq!(power_limit_register_228(0), u16::MAX);
        assert_eq!(power_limit_deciwatt_228(u16::MAX), 0);
        assert_eq!(power_limit_deciwatt_228(u16::MAX - 1), u16::MAX);
        assert_eq!(power_limit_register_228(u16::MAX), 15_999);
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
