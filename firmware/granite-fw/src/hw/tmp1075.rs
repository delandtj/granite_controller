//! TMP1075 board temperature sensor at 0x48 on the internal bus
//! (ADR 0001 component 4, "TMP1075 (0x48, internal bus) every 1 s").
//!
//! The temperature register is 12 bits, left justified in a big-endian
//! 16-bit word, 0.0625 C/LSB. The trait wants centi-degrees, so the
//! conversion is `(raw >> 4) * 625 / 100`, done in integer arithmetic:
//! 0.0625 C is exactly 6.25 centi-C, hence `* 25 / 4`.

use granite_core::hal::{BoardTemp, HalError, HalResult};

use super::i2c::{I2cBus, I2cDev, hal_err};

/// Address on the internal bus (docs/controller.md, "I2C buses").
pub const ADDR: u8 = 0x48;
/// Temperature result register.
const REG_TEMP: u8 = 0x00;
/// Configuration register.
const REG_CFG: u8 = 0x01;

/// CFG = the part's own reset value (SBOS734, Configuration Register):
/// OS = 0, conversion rate 27.5 ms, SD = 0 (continuous), ALERT unused, and
/// the reserved low byte left at its default 0xff. Written explicitly so a
/// warm restart after a shutdown command still ends up in continuous mode.
const CFG_CONTINUOUS: u16 = 0x00ff;

/// [`BoardTemp`] over the TMP1075.
pub struct Tmp1075 {
    dev: I2cDev,
}

impl Tmp1075 {
    /// Attach to the sensor on `bus` and put it into continuous mode.
    ///
    /// A missing sensor is reported, not fatal: the board temperature is a
    /// monitoring value, and the relays do not depend on it.
    pub fn new(bus: &I2cBus, scl_hz: u32) -> HalResult<Self> {
        let dev = bus.device(ADDR, scl_hz).map_err(|err| {
            log::error!("tmp1075: cannot attach at 0x{ADDR:02x}: {err}");
            hal_err(err)
        })?;
        let sensor = Tmp1075 { dev };
        match sensor.configure() {
            Ok(()) => log::info!("tmp1075: 0x{ADDR:02x} in continuous mode"),
            Err(err) => log::error!("tmp1075: configuring 0x{ADDR:02x} failed: {err}"),
        }
        Ok(sensor)
    }

    fn configure(&self) -> HalResult<()> {
        let bytes = [REG_CFG, (CFG_CONTINUOUS >> 8) as u8, CFG_CONTINUOUS as u8];
        self.dev.write(&bytes).map_err(hal_err)
    }

    /// Raw 12-bit signed reading, for logs and self-test.
    pub fn read_raw(&self) -> HalResult<i16> {
        let mut buf = [0u8; 2];
        self.dev
            .write_read(&[REG_TEMP], &mut buf)
            .map_err(hal_err)?;
        Ok(i16::from_be_bytes(buf) >> 4)
    }
}

impl BoardTemp for Tmp1075 {
    fn read_centi_c(&mut self) -> HalResult<i16> {
        let raw = self.read_raw().inspect_err(|err| {
            log::error!("tmp1075: read failed: {err}");
        })?;
        // 0.0625 C/LSB = 6.25 centi-C/LSB, exact in integers as * 25 / 4.
        let centi = i32::from(raw) * 25 / 4;
        i16::try_from(centi).map_err(|_| HalError::OutOfRange)
    }
}
