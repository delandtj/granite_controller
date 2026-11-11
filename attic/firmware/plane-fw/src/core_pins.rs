//! [`CorePins`]: PRESS_TRIG, EXP_RSTN and the VIN_SNS ADC.
//!
//! PRESS_TRIG is the press deadline (docs/press-deadline-line.md): the
//! plane logic toggles it from the task that owns the press and never
//! otherwise, so a hung firmware loses PRESS_EN after about a second
//! and every photoMOS goes dark. Nothing here may toggle it on its own,
//! which is why there is no timer and no DMA on this pin.

use embassy_stm32::adc::{Adc, SampleTime};
use embassy_stm32::gpio::{Level, Output, OutputOpenDrain, Speed};
use embassy_stm32::peripherals::ADC1;
use embassy_time::{Duration, block_for};
use plane_logic::device::CorePins;

use crate::pins;

/// How long EXP_RSTN is held low on CTRL 0x4D. The TCA6424A needs
/// 500 ns; 1 ms is safe across the chain's RC.
const EXP_RESET_MS: u64 = 1;

/// VIN_SNS divider: 220k over 10k, so the input is 23 times the pin.
const DIVIDER: u32 = 23;

/// ADC reference \[mV\]: the core's 3V3 rail (no VREF+ pin on the
/// LQFP-48 G070, VREF+ is tied to VDDA on the board).
const VREF_MV: u32 = 3300;

/// Full scale of a 12-bit conversion.
const FULL_SCALE: u32 = 4095;

/// The core's own pins and sensors.
pub struct Pins {
    press_trig: Output<'static>,
    exp_rstn: OutputOpenDrain<'static>,
    adc: Adc<'static, ADC1>,
    vin_sns: embassy_stm32::Peri<'static, pins::VIN_SNS>,
    /// What PRESS_TRIG is driving, so a toggle does not have to read
    /// the pin back.
    trig_level: bool,
}

/// What [`Pins::new`] needs out of [`crate::pins::CoreBoard`].
pub struct CorePinParts {
    /// PRESS_TRIG.
    pub press_trig: embassy_stm32::Peri<'static, pins::PRESS_TRIG>,
    /// EXP_RSTN.
    pub exp_rstn: embassy_stm32::Peri<'static, pins::EXP_RSTN>,
    /// VIN_SNS.
    pub vin_sns: embassy_stm32::Peri<'static, pins::VIN_SNS>,
    /// The ADC behind VIN_SNS.
    pub adc: embassy_stm32::Peri<'static, ADC1>,
}

impl Pins {
    /// Take the pins, with both outputs in their idle state: PRESS_TRIG
    /// low (no edge, so no press), EXP_RSTN released (the units' own
    /// pull-ups hold their expanders out of reset).
    pub fn new(parts: CorePinParts) -> Self {
        let press_trig = Output::new(parts.press_trig, Level::Low, Speed::Low);
        let exp_rstn = OutputOpenDrain::new(parts.exp_rstn, Level::High, Speed::Low);
        let adc = Adc::new(parts.adc);
        Pins {
            press_trig,
            exp_rstn,
            adc,
            vin_sns: parts.vin_sns,
            trig_level: false,
        }
    }
}

impl CorePins for Pins {
    fn toggle_press_trigger(&mut self) {
        self.trig_level = !self.trig_level;
        self.press_trig.set_level(Level::from(self.trig_level));
    }

    fn pulse_expander_reset(&mut self) {
        self.exp_rstn.set_low();
        block_for(Duration::from_millis(EXP_RESET_MS));
        self.exp_rstn.set_high();
    }

    fn core_input_10mv(&mut self) -> Option<u16> {
        // 160.5 cycles: the divider's ~9.6k source impedance needs a
        // long sample to charge the 12-bit sampling capacitor.
        let raw = self
            .adc
            .blocking_read(&mut self.vin_sns, SampleTime::CYCLES160_5);
        // raw -> mV at the pin -> mV at the input -> 10 mV units, all
        // in integers: no FPU on the M0+ and no soft float in the
        // image.
        let tenmv = (u32::from(raw) * VREF_MV * DIVIDER) / (FULL_SCALE * 10);
        Some(tenmv.min(u32::from(u16::MAX)) as u16)
    }

    fn core_temperature_decidegrees(&mut self) -> Option<i16> {
        // Not implemented: the G0 temperature sensor needs the two
        // factory calibration words and a division per sample, and
        // nothing in the plane logic decides on it (STATUS 0x11 is a
        // log value for the controller). The controller sees "no
        // reading" until it is wired up.
        None
    }
}
