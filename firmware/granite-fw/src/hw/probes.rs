//! DS18B20 probes on the 1-wire bus on GPIO16 (ADR 0001 component 4,
//! "DS18B20 on GPIO16 through the Espressif `onewire_bus` ... components
//! (RMT based)").
//!
//! The bus is driven by the Espressif `onewire_bus` managed component,
//! pulled in through `[[package.metadata.esp-idf-sys.extra_components]]`
//! in Cargo.toml and wrapped by `esp_idf_hal::onewire`. That component
//! does the RMT encoding, the reset/presence slot and the ROM search; the
//! DS18B20 commands themselves are four byte sequences, written out here
//! rather than taking the second `ds18b20` component as a dependency.
//!
//! GPIO17 is the other half of the same net (U0RXD; docs/controller.md,
//! "The 1-wire bus sits on U0TXD/U0RXD"). It is put into input mode with
//! no pulls so the UART receiver stays passive and does not fight the
//! 4.7k pull-up.
//!
//! A missing probe is reported, not an error: [`Probes::scan`] returns an
//! empty list on an empty bus and [`Probes::read`] answers
//! [`HalError::NotPresent`] for a ROM id that no longer responds.

use std::time::Duration;

use esp_idf_hal::gpio::{Gpio16, Gpio17, Input, PinDriver, Pull};
use esp_idf_sys::EspError;
use granite_core::hal::{HalError, HalResult, Probes, RomId, TEMP_MISSING};

use super::clock::now_ms;

/// MATCH ROM: address one device.
const CMD_MATCH_ROM: u8 = 0x55;
/// SKIP ROM: address every device at once.
const CMD_SKIP_ROM: u8 = 0xcc;
/// CONVERT T.
const CMD_CONVERT_T: u8 = 0x44;
/// READ SCRATCHPAD.
const CMD_READ_SCRATCH: u8 = 0xbe;
/// WRITE SCRATCHPAD.
const CMD_WRITE_SCRATCH: u8 = 0x4e;
/// DS18B20 family code; anything else on the bus is not a probe.
const FAMILY_DS18B20: u8 = 0x28;
/// Config byte for 12-bit resolution (R1 R0 = 11).
const CFG_12BIT: u8 = 0x7f;
/// 12-bit conversion time with margin (datasheet: 750 ms max).
const CONVERT_MS: u64 = 800;
/// How long a broadcast conversion stays usable for further reads.
const CONVERT_VALID_MS: u64 = 2_000;
/// Reading below this means "power-on default", i.e. no conversion yet.
const POWER_ON_RAW: i16 = 0x0550;

/// [`Probes`] over the 1-wire bus.
pub struct OneWireProbes {
    bus: Option<esp_idf_hal::onewire::OWDriver<'static>>,
    /// GPIO17 parked as a passive input; kept so nothing re-claims it.
    _rx_idle: PinDriver<'static, Input>,
    last_convert_ms: u64,
}

impl OneWireProbes {
    /// Claim GPIO16 for the bus and park GPIO17.
    ///
    /// A failure to create the RMT-based bus is logged and leaves a driver
    /// that reports an empty bus, because the probes are monitoring values
    /// and must not block bring-up.
    pub fn new(dq: Gpio16<'static>, uart_rx: Gpio17<'static>) -> Result<Self, EspError> {
        let rx_idle = PinDriver::input(uart_rx, Pull::Floating)?;
        let bus = match esp_idf_hal::onewire::OWDriver::new(dq) {
            Ok(bus) => {
                log::info!("probes: 1-wire bus up on GPIO16 (RMT)");
                Some(bus)
            }
            Err(err) => {
                log::error!("probes: 1-wire bus on GPIO16 failed: {err}");
                None
            }
        };
        let mut probes = OneWireProbes {
            bus,
            _rx_idle: rx_idle,
            last_convert_ms: 0,
        };
        if let Ok(found) = probes.scan() {
            log::info!("probes: {} device(s) on the bus", found.len());
        }
        Ok(probes)
    }

    fn bus(&self) -> HalResult<&esp_idf_hal::onewire::OWDriver<'static>> {
        self.bus.as_ref().ok_or(HalError::NotPresent)
    }

    /// Reset the bus and address one device.
    fn select(&self, rom: RomId) -> HalResult<()> {
        let bus = self.bus()?;
        bus.reset().map_err(presence_err)?;
        let mut frame = [0u8; 9];
        frame[0] = CMD_MATCH_ROM;
        frame[1..9].copy_from_slice(&rom.to_le_bytes());
        bus.write(&frame).map_err(|err| {
            log::error!("probes: addressing {rom:016x} failed: {err}");
            HalError::Bus
        })
    }

    /// Reset the bus and address every device.
    fn select_all(&self) -> HalResult<()> {
        let bus = self.bus()?;
        bus.reset().map_err(presence_err)?;
        bus.write(&[CMD_SKIP_ROM]).map_err(|_| HalError::Bus)
    }

    /// Put every probe on the bus into 12-bit mode.
    pub fn set_resolution_12bit(&self) -> HalResult<()> {
        self.select_all()?;
        self.bus()?
            // TH = +75 C, TL = +70 C: the alarm outputs are unused, these
            // are the datasheet defaults.
            .write(&[CMD_WRITE_SCRATCH, 0x4b, 0x46, CFG_12BIT])
            .map_err(|_| HalError::Bus)
    }

    /// Broadcast CONVERT T and wait for it, unless a recent broadcast is
    /// still good. One conversion serves every probe, so a full read cycle
    /// costs 800 ms rather than 800 ms per probe.
    fn convert_all(&mut self) -> HalResult<()> {
        let now = now_ms();
        if self.last_convert_ms != 0 && now.saturating_sub(self.last_convert_ms) < CONVERT_VALID_MS
        {
            return Ok(());
        }
        self.select_all()?;
        self.bus()?
            .write(&[CMD_CONVERT_T])
            .map_err(|_| HalError::Bus)?;
        std::thread::sleep(Duration::from_millis(CONVERT_MS));
        self.last_convert_ms = now_ms();
        Ok(())
    }

    /// Read the 9-byte scratchpad of one probe and check its CRC.
    fn scratchpad(&self, rom: RomId) -> HalResult<[u8; 9]> {
        self.select(rom)?;
        self.bus()?
            .write(&[CMD_READ_SCRATCH])
            .map_err(|_| HalError::Bus)?;
        let mut buf = [0u8; 9];
        self.bus()?.read(&mut buf).map_err(|err| {
            log::error!("probes: reading {rom:016x} failed: {err}");
            HalError::Bus
        })?;
        if crc8(&buf[..8]) != buf[8] {
            log::error!("probes: {rom:016x} scratchpad CRC mismatch");
            return Err(HalError::Bus);
        }
        Ok(buf)
    }
}

impl Probes for OneWireProbes {
    fn scan(&mut self) -> HalResult<Vec<RomId>> {
        let bus = match self.bus.as_mut() {
            Some(bus) => bus,
            None => return Ok(Vec::new()),
        };
        let mut found = Vec::new();
        match bus.search() {
            Ok(search) => {
                for device in search {
                    match device {
                        Ok(address) => {
                            let rom = address.address();
                            if address.family_code() == FAMILY_DS18B20 {
                                found.push(rom);
                            } else {
                                log::warn!(
                                    "probes: ignoring {rom:016x}, family 0x{:02x} is not a DS18B20",
                                    address.family_code()
                                );
                            }
                        }
                        Err(err) => {
                            log::error!("probes: ROM search failed: {err}");
                            break;
                        }
                    }
                }
            }
            Err(err) => {
                // An empty bus fails the presence detect; that is "no
                // probes", not a fault.
                log::info!("probes: no device answered the ROM search ({err})");
                return Ok(Vec::new());
            }
        }
        if !found.is_empty()
            && let Err(err) = self.set_resolution_12bit()
        {
            log::error!("probes: setting 12-bit resolution failed: {err}");
        }
        Ok(found)
    }

    fn read(&mut self, rom: RomId) -> HalResult<i16> {
        self.convert_all()?;
        let pad = self.scratchpad(rom)?;
        let raw = i16::from_le_bytes([pad[0], pad[1]]);
        if raw == POWER_ON_RAW && pad[4] & CFG_12BIT != CFG_12BIT {
            // 85.0 C with an unwritten config register is the reset value,
            // not a reading.
            log::warn!("probes: {rom:016x} returned its power-on default");
            return Ok(TEMP_MISSING);
        }
        // 1/16 C per LSB -> centi-degrees, rounding to nearest.
        let centi = (i32::from(raw) * 100 + raw.signum() as i32 * 8) / 16;
        i16::try_from(centi).map_err(|_| HalError::OutOfRange)
    }
}

/// A failed reset means nothing answered the presence slot.
fn presence_err(err: EspError) -> HalError {
    log::debug!("probes: 1-wire presence detect failed: {err}");
    HalError::NotPresent
}

/// Maxim/Dallas CRC-8, polynomial x^8 + x^5 + x^4 + 1.
pub fn crc8(data: &[u8]) -> u8 {
    let mut crc = 0u8;
    for byte in data {
        crc ^= byte;
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0x8c
            } else {
                crc >> 1
            };
        }
    }
    crc
}
