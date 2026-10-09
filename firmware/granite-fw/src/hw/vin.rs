//! 19 V bus voltage on GPIO5 / ADC1 channel 5 (ADR 0001 component 4,
//! "VIN on ADC1 ch5 with the ESP-IDF curve calibration, 16-sample average
//! every 1 s, scaled by the 110/10 divider with a per-board trim factor").
//!
//! docs/controller.md, "Added inputs": R63 100k over R64 10k, so the pin
//! sees Vin/11 and 19 V lands at 1.73 V. 12 dB attenuation is the only
//! range that reaches the 3.23 V the TVS clamp allows.

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

/// GPIO5 is ADC1 channel 5 on the ESP32-C6. The channel driver owns the
/// unit driver, so one value holds the whole chain.
type Vin5 = AdcChannelDriver<'static, ADCCH5<ADCU1>, AdcDriver<'static, ADCU1>>;

/// [`BusVoltage`] over ADC1.
pub struct Vin {
    channel: Vin5,
    trim_milli: u32,
}

impl Vin {
    /// Claim ADC1 and GPIO5.
    ///
    /// `trim` is the per-board correction from config, 1.0 meaning "use
    /// the nominal divider". It is kept as parts per thousand internally so
    /// a reading needs no floating point.
    pub fn new(adc1: ADC1<'static>, pin: Gpio5<'static>, trim: f32) -> Result<Self, EspError> {
        let driver = AdcDriver::new(adc1)?;
        let config = AdcChannelConfig {
            attenuation: DB_12,
            calibration: Calibration::Curve,
            ..Default::default()
        };
        let channel = AdcChannelDriver::new(driver, pin, &config)?;
        let trim_milli = (trim.clamp(0.5, 2.0) * 1000.0) as u32;
        log::info!("vin: ADC1 ch5 on GPIO5, divider 1:{DIVIDER}, trim {trim_milli}/1000");
        Ok(Vin {
            channel,
            trim_milli,
        })
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
        Ok(pin_mv * DIVIDER * self.trim_milli / 1000)
    }
}
