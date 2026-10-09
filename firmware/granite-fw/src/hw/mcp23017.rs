//! MCP23017 driver (ADR 0001 component 2, "Expander driver").
//!
//! Bank 0 addressing only, which is the state the chip resets into, so the
//! driver never has to guess which map is live. Port A is the low byte of
//! every 16-bit value in this module, port B the high byte; bit 0 of port A
//! is GPA0.
//!
//! Three rules from docs/controller.md are enforced here rather than left
//! to the callers:
//!
//! - Every write is read back and compared against the shadow. A mismatch
//!   is [`HalError::ExpanderFault`], which is what the core treats as "the
//!   expander lied to us".
//! - GPA7 and GPB7 are output-only on this part (DS20001952D), so
//!   [`Mcp23017::set_iodir`] refuses to make them inputs.
//! - Outputs are driven through OLAT, never through the GPIO register, so
//!   the readback compares like with like.
//!
//! IOCON is written with SEQOP = 0 (sequential addressing on, the reset
//! default), which is what lets a port pair be written in one transaction.

use esp_idf_sys::EspError;
use granite_core::hal::{HalError, HalResult};

use super::i2c::{hal_err, I2cDev};

/// IODIRA. Every other register pair follows at a fixed offset in bank 0.
pub const REG_IODIR: u8 = 0x00;
/// IPOLA.
pub const REG_IPOL: u8 = 0x02;
/// GPINTENA.
pub const REG_GPINTEN: u8 = 0x04;
/// DEFVALA.
pub const REG_DEFVAL: u8 = 0x06;
/// INTCONA.
pub const REG_INTCON: u8 = 0x08;
/// IOCONA (IOCONB at 0x0b is the same physical register).
pub const REG_IOCON: u8 = 0x0a;
/// GPPUA.
pub const REG_GPPU: u8 = 0x0c;
/// INTFA.
pub const REG_INTF: u8 = 0x0e;
/// INTCAPA, cleared by reading it or GPIO.
pub const REG_INTCAP: u8 = 0x10;
/// GPIOA.
pub const REG_GPIO: u8 = 0x12;
/// OLATA.
pub const REG_OLAT: u8 = 0x14;

/// IOCON.BANK: 1 = split register map. Never set by this driver.
pub const IOCON_BANK: u8 = 1 << 7;
/// IOCON.MIRROR: 1 = INTA and INTB are internally joined.
pub const IOCON_MIRROR: u8 = 1 << 6;
/// IOCON.SEQOP: 1 = address pointer does not increment.
pub const IOCON_SEQOP: u8 = 1 << 5;
/// IOCON.DISSLW: 1 = SDA slew rate control disabled.
pub const IOCON_DISSLW: u8 = 1 << 4;
/// IOCON.HAEN: hardware address enable (no effect on the I2C part).
pub const IOCON_HAEN: u8 = 1 << 3;
/// IOCON.ODR: 1 = INT pin is open-drain. Required: EXP_INT is shared.
pub const IOCON_ODR: u8 = 1 << 2;
/// IOCON.INTPOL: interrupt polarity, ignored when ODR = 1.
pub const IOCON_INTPOL: u8 = 1 << 1;

/// Pins that are output-only on this part: GPA7 (bit 7) and GPB7 (bit 15).
pub const OUTPUT_ONLY_MASK: u16 = (1 << 7) | (1 << 15);

/// One MCP23017 at one address.
pub struct Mcp23017 {
    dev: I2cDev,
    name: &'static str,
    iodir: u16,
    olat: u16,
    iocon: u8,
    /// False until [`Mcp23017::configure`] has run since the last reset.
    configured: bool,
}

impl Mcp23017 {
    /// Wrap an addressed device. No I2C traffic happens here; the chip is
    /// only touched by [`Mcp23017::configure`] and later calls.
    pub fn new(dev: I2cDev, name: &'static str) -> Self {
        Mcp23017 {
            dev,
            name,
            // Reset state of the chip: all inputs, all latches low.
            iodir: 0xffff,
            olat: 0x0000,
            iocon: 0x00,
            configured: false,
        }
    }

    /// 7-bit address, for log lines.
    pub fn addr(&self) -> u8 {
        self.dev.addr()
    }

    /// Short name used in log lines ("U14", "U15").
    pub fn name(&self) -> &'static str {
        self.name
    }

    /// True when the shadows are known to match the chip.
    pub fn is_configured(&self) -> bool {
        self.configured
    }

    /// Forget the shadows: the chip has been (or may have been) reset.
    pub fn mark_reset(&mut self) {
        self.iodir = 0xffff;
        self.olat = 0x0000;
        self.iocon = 0x00;
        self.configured = false;
    }

    /// Apply a whole configuration in the order the datasheet wants:
    /// IOCON first (so INT is open-drain before GPINTEN is touched), then
    /// the input setup, then the output latches, then the directions.
    ///
    /// `iodir` uses 1 = input, as the chip does.
    #[allow(clippy::too_many_arguments)]
    pub fn configure(
        &mut self,
        iocon: u8,
        gppu: u16,
        ipol: u16,
        defval: u16,
        intcon: u16,
        gpinten: u16,
        olat: u16,
        iodir: u16,
    ) -> HalResult<()> {
        self.mark_reset();
        self.set_iocon(iocon)?;
        self.write_pair(REG_GPPU, gppu)?;
        self.write_pair(REG_IPOL, ipol)?;
        self.write_pair(REG_DEFVAL, defval)?;
        self.write_pair(REG_INTCON, intcon)?;
        self.write_pair(REG_GPINTEN, gpinten)?;
        self.set_olat(olat)?;
        self.set_iodir(iodir)?;
        self.configured = true;
        Ok(())
    }

    /// Write IOCON. BANK is masked off: this driver is bank 0 only.
    pub fn set_iocon(&mut self, iocon: u8) -> HalResult<()> {
        let want = iocon & !IOCON_BANK;
        self.write_reg(REG_IOCON, want)?;
        self.iocon = want;
        Ok(())
    }

    /// Write IODIR (1 = input). GPA7/GPB7 are forced to output.
    pub fn set_iodir(&mut self, iodir: u16) -> HalResult<()> {
        let want = iodir & !OUTPUT_ONLY_MASK;
        if want != iodir {
            log::warn!(
                "{}: GPA7/GPB7 are output-only on the MCP23017, refusing IODIR 0x{iodir:04x}",
                self.name
            );
        }
        self.write_pair(REG_IODIR, want)?;
        self.iodir = want;
        Ok(())
    }

    /// Write both output latches.
    pub fn set_olat(&mut self, olat: u16) -> HalResult<()> {
        self.write_pair(REG_OLAT, olat)?;
        self.olat = olat;
        Ok(())
    }

    /// Set or clear one output bit (0..=15, 0 = GPA0), writing only the
    /// port byte that changes.
    pub fn set_bit(&mut self, bit: u8, on: bool) -> HalResult<()> {
        if bit > 15 {
            return Err(HalError::OutOfRange);
        }
        let mask = 1u16 << bit;
        let want = if on { self.olat | mask } else { self.olat & !mask };
        let reg = if bit < 8 { REG_OLAT } else { REG_OLAT + 1 };
        let byte = if bit < 8 {
            (want & 0x00ff) as u8
        } else {
            (want >> 8) as u8
        };
        self.write_reg(reg, byte)?;
        self.olat = want;
        Ok(())
    }

    /// Shadowed output latches.
    pub fn olat(&self) -> u16 {
        self.olat
    }

    /// Shadowed directions.
    pub fn iodir(&self) -> u16 {
        self.iodir
    }

    /// Read both GPIO registers. Reading GPIO also clears INTCAP.
    pub fn read_gpio(&self) -> HalResult<u16> {
        self.read_pair(REG_GPIO)
    }

    /// Read both INTCAP registers: the port state captured at the last
    /// interrupt. Reading clears the interrupt.
    pub fn read_intcap(&self) -> HalResult<u16> {
        self.read_pair(REG_INTCAP)
    }

    /// Read both interrupt flag registers.
    pub fn read_intf(&self) -> HalResult<u16> {
        self.read_pair(REG_INTF)
    }

    /// Periodic 1 s check (ADR component 2): the three registers that can
    /// turn a released relay into a closed one still hold what we wrote.
    pub fn verify(&self) -> HalResult<()> {
        if !self.configured {
            return Err(HalError::ExpanderFault);
        }
        let iocon = self.read_reg(REG_IOCON)?;
        let iodir = self.read_pair(REG_IODIR)?;
        let olat = self.read_pair(REG_OLAT)?;
        if iocon != self.iocon || iodir != self.iodir || olat != self.olat {
            log::error!(
                "{} (0x{:02x}) verify mismatch: iocon {iocon:02x}/{:02x} iodir {iodir:04x}/{:04x} olat {olat:04x}/{:04x}",
                self.name,
                self.addr(),
                self.iocon,
                self.iodir,
                self.olat
            );
            return Err(HalError::ExpanderFault);
        }
        Ok(())
    }

    /// Write one register and read it back.
    pub fn write_reg(&self, reg: u8, val: u8) -> HalResult<()> {
        self.dev.write(&[reg, val]).map_err(|e| self.bus(reg, e))?;
        let got = self.read_reg(reg)?;
        if got != val {
            log::error!(
                "{} (0x{:02x}) reg 0x{reg:02x} readback 0x{got:02x}, wrote 0x{val:02x}",
                self.name,
                self.addr()
            );
            return Err(HalError::ExpanderFault);
        }
        Ok(())
    }

    /// Write a port pair (A then B) in one transaction and read it back.
    pub fn write_pair(&self, reg_a: u8, val: u16) -> HalResult<()> {
        let bytes = [reg_a, (val & 0x00ff) as u8, (val >> 8) as u8];
        self.dev.write(&bytes).map_err(|e| self.bus(reg_a, e))?;
        let got = self.read_pair(reg_a)?;
        if got != val {
            log::error!(
                "{} (0x{:02x}) reg pair 0x{reg_a:02x} readback 0x{got:04x}, wrote 0x{val:04x}",
                self.name,
                self.addr()
            );
            return Err(HalError::ExpanderFault);
        }
        Ok(())
    }

    /// Read one register.
    pub fn read_reg(&self, reg: u8) -> HalResult<u8> {
        let mut buf = [0u8; 1];
        self.dev
            .write_read(&[reg], &mut buf)
            .map_err(|e| self.bus(reg, e))?;
        Ok(buf[0])
    }

    /// Read a port pair (A then B).
    pub fn read_pair(&self, reg_a: u8) -> HalResult<u16> {
        let mut buf = [0u8; 2];
        self.dev
            .write_read(&[reg_a], &mut buf)
            .map_err(|e| self.bus(reg_a, e))?;
        Ok(u16::from(buf[0]) | (u16::from(buf[1]) << 8))
    }

    fn bus(&self, reg: u8, err: EspError) -> HalError {
        log::error!(
            "{} (0x{:02x}) i2c error on reg 0x{reg:02x}: {err}",
            self.name,
            self.addr()
        );
        hal_err(err)
    }
}
