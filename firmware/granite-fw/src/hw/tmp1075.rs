//! Board temperature sensor at 0x48 on the internal bus (ADR 0001
//! component 4, "TMP1075 (0x48, internal bus) every 1 s").
//!
//! Two parts answer there, and the firmware does not tell them apart:
//! the TMP1075 of the earlier boards (SBOS854) and the TMP75A of
//! controller-fd U5 (SBOS288, the LM75 family). The module keeps the
//! TMP1075 name because that is what the ADRs, the README and the
//! Modbus map call this sensor.
//!
//! The temperature register is the same on both: 12 bits left justified
//! in a big-endian 16-bit word, 0.0625 C/LSB. The trait wants
//! centi-degrees, so the conversion is `(raw >> 4) * 625 / 100`, done
//! in integer arithmetic: 0.0625 C is exactly 6.25 centi-C, hence
//! `* 25 / 4`. A TMP75 starts in its 9-bit mode, where the low three
//! bits of the 12 read zero; the same arithmetic then lands on a
//! multiple of 0.5 C, which is resolution enough for a board sensor.
//!
//! What is *not* the same is the configuration register: 16 bits on the
//! TMP1075, 8 on the TMP75. See [`CFG_CONTINUOUS`] for what that means
//! for the one write this module makes.

use granite_core::hal::{BoardTemp, HalError, HalResult};

use super::i2c::{I2cBus, I2cDev, hal_err};

/// Address on the internal bus (docs/controller.md, "I2C buses").
pub const ADDR: u8 = 0x48;
/// Temperature result register.
const REG_TEMP: u8 = 0x00;
/// Configuration register.
const REG_CFG: u8 = 0x01;

/// CFG as a **single** byte: both parts' reset value for the bits it
/// covers. OS = 0, R1/R0 = 0, F1/F0 = 0, POL = 0, TM = 0 and - the one
/// that matters - SD = 0, continuous conversion.
///
/// One byte and not two. The TMP1075's CFG is 16 bits, but SBOS854
/// 7.5.1.2 says the register takes a single-byte write "for software
/// compatibility with xx75 standard temperature sensors", where the
/// byte updates bits 15-8 (and the low byte is reserved anyway); the
/// TMP75's CFG *is* those eight bits (SBOS288 7.5.3, reset 0x00). A
/// two-byte write of the old 0x00ff would have put the second byte -
/// every bit set, SD among them - into a TMP75's one and only
/// configuration byte and shut the sensor down.
///
/// The write is kept rather than dropped because it costs one
/// transaction at start-up and puts a part that somehow sits in
/// shutdown back into continuous mode. R1/R0 stay 0: that is 9-bit
/// resolution on a TMP75 and the 27.5 ms conversion rate the TMP1075
/// boards have always run.
const CFG_CONTINUOUS: u8 = 0x00;

/// [`BoardTemp`] over the TMP1075.
pub struct Tmp1075 {
    dev: I2cDev,
    /// The CFG write at start-up failed; retried once, on the first read
    /// that succeeds.
    unconfigured: bool,
    /// The last read failed. Only the change is logged: the sense task
    /// reads every second, and a sensor that is not there would otherwise
    /// fill the log ring and push everything else out of it.
    failing: bool,
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
        let mut sensor = Tmp1075 {
            dev,
            unconfigured: true,
            failing: false,
        };
        match sensor.configure() {
            Ok(()) => {
                sensor.unconfigured = false;
                log::info!("tmp1075: 0x{ADDR:02x} in continuous mode");
            }
            Err(err) => log::error!("tmp1075: configuring 0x{ADDR:02x} failed: {err}"),
        }
        Ok(sensor)
    }

    fn configure(&self) -> HalResult<()> {
        self.dev.write(&[REG_CFG, CFG_CONTINUOUS]).map_err(hal_err)
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
        let raw = match self.read_raw() {
            Ok(raw) => raw,
            Err(err) => {
                if !self.failing {
                    self.failing = true;
                    log::error!(
                        "tmp1075: read failed: {err} (not logged again until it answers)"
                    );
                }
                return Err(err);
            }
        };
        if self.failing {
            self.failing = false;
            log::info!("tmp1075: answering again");
        }
        if self.unconfigured {
            // One retry: both parts power up in continuous mode, so a
            // sensor that reads but will not take the write still works.
            self.unconfigured = false;
            match self.configure() {
                Ok(()) => log::info!("tmp1075: 0x{ADDR:02x} in continuous mode"),
                Err(err) => log::warn!("tmp1075: configuring 0x{ADDR:02x} failed: {err}"),
            }
        }
        // 0.0625 C/LSB = 6.25 centi-C/LSB, exact in integers as * 25 / 4.
        let centi = i32::from(raw) * 25 / 4;
        i16::try_from(centi).map_err(|_| HalError::OutOfRange)
    }
}
