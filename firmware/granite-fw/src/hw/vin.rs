//! 19 V bus voltage on GPIO5 / ADC1 channel 5 (ADR 0001 component 4,
//! "VIN on ADC1 ch5 with the ESP-IDF curve calibration, 16-sample average
//! every 1 s, scaled by the 110/10 divider with a per-board trim factor").
//!
//! docs/controller.md, "Added inputs": R63 100k over R64 10k, so the pin
//! sees Vin/11 and 19 V lands at 1.73 V. 12 dB attenuation is the only
//! range that reaches the 3.23 V the TVS clamp allows.

use std::sync::atomic::{AtomicU32, Ordering};

use esp_idf_hal::adc::attenuation::DB_12;
use esp_idf_hal::adc::oneshot::config::{AdcChannelConfig, Calibration};
use esp_idf_hal::adc::oneshot::{AdcChannelDriver, AdcDriver};
use esp_idf_hal::adc::{ADC1, ADCCH5, ADCU1};
use esp_idf_hal::gpio::Gpio5;
use esp_idf_sys::EspError;
use granite_core::hal::{BusVoltage, HalError, HalResult};

/// Divider ratio: (R63 + R64) / R64 = (100k + 10k) / 10k.
pub const DIVIDER: u32 = 11;
/// Samples averaged per reading.
pub const SAMPLES: u32 = 16;

/// `nodes.vin_trim` in parts per thousand, live.
///
/// The ADC is owned by the sense thread and the trim comes from a config
/// section an operator can save at any moment, so the two meet in an
/// atomic: [`set_trim`] is called from the dispatcher's `apply_config`
/// and every reading loads it. A trim change therefore shows up on the
/// next status refresh instead of after a reboot.
static TRIM_MILLI: AtomicU32 = AtomicU32::new(1000);

/// Clamp a config trim factor to the range the reader accepts and return
/// it in parts per thousand.
fn trim_milli(trim: f32) -> u32 {
    (trim.clamp(0.5, 2.0) * 1000.0) as u32
}

/// Push a new `nodes.vin_trim` into the reader. Takes effect on the next
/// reading, no reboot.
pub fn set_trim(trim: f32) {
    let milli = trim_milli(trim);
    if TRIM_MILLI.swap(milli, Ordering::Relaxed) != milli {
        log::info!("vin: trim now {milli}/1000");
    }
}

/// The trim the reader is using, parts per thousand.
pub fn trim() -> u32 {
    TRIM_MILLI.load(Ordering::Relaxed)
}

/// GPIO5 is ADC1 channel 5 on the ESP32-C6. The channel driver owns the
/// unit driver, so one value holds the whole chain.
type Vin5 = AdcChannelDriver<'static, ADCCH5<ADCU1>, AdcDriver<'static, ADCU1>>;

/// [`BusVoltage`] over ADC1.
pub struct Vin {
    channel: Vin5,
}

impl Vin {
    /// Claim ADC1 and GPIO5.
    ///
    /// `trim` is the per-board correction from config, 1.0 meaning "use
    /// the nominal divider". It is kept as parts per thousand in
    /// [`TRIM_MILLI`] so a reading needs no floating point and so a later
    /// config save can change it live ([`set_trim`]).
    pub fn new(adc1: ADC1<'static>, pin: Gpio5<'static>, trim: f32) -> Result<Self, EspError> {
        let driver = AdcDriver::new(adc1)?;
        let config = AdcChannelConfig {
            attenuation: DB_12,
            calibration: Calibration::Curve,
            ..Default::default()
        };
        let channel = AdcChannelDriver::new(driver, pin, &config)?;
        let milli = trim_milli(trim);
        TRIM_MILLI.store(milli, Ordering::Relaxed);
        log::info!("vin: ADC1 ch5 on GPIO5, divider 1:{DIVIDER}, trim {milli}/1000");
        Ok(Vin { channel })
    }

    /// Averaged voltage at the pin, in millivolts.
    pub fn read_pin_mv(&mut self) -> HalResult<u32> {
        let mut total = 0u32;
        for _ in 0..SAMPLES {
            let mv = self.channel.read().map_err(|err| {
                log::error!("vin: ADC read failed: {err}");
                HalError::Other(format!("adc: {err}"))
            })?;
            total += u32::from(mv);
        }
        Ok(total / SAMPLES)
    }
}

impl BusVoltage for Vin {
    fn read_mv(&mut self) -> HalResult<u32> {
        let pin_mv = self.read_pin_mv()?;
        Ok(pin_mv * DIVIDER * trim() / 1000)
    }
}
