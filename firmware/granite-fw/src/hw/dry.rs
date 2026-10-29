//! The four dry-contact inputs.
//!
//! On rev C these sat on U15 (an MCP23017); on rev D they are straight on
//! MCU GPIOs (controller ADR 0004 decision 9) and there is no expander
//! left on the board. So this is a plain
//! [`granite_core::hal::DryInputs`] over four input pins and nothing
//! else.
//!
//! **The pins are not chosen yet.** ADR 0004 decision 9 says
//! "four dry-contact inputs straight on MCU GPIOs, non-strapping pins"
//! and leaves the numbers to the rev D hardware spec, which does not
//! exist. Until it does, [`DryGpio::unwired`] is what `main` builds: it
//! answers [`granite_core::hal::HalError::NotPresent`], the snapshot
//! keeps `dry_in: null` ("never read") instead of four plausible open
//! contacts, and a rule on a dry input never fires on an invention. One
//! call to [`DryGpio::wired`] replaces it when the pin map lands.

use esp_idf_hal::gpio::{AnyIOPin, Input, PinDriver, Pull};
use granite_core::DRY_COUNT;
use granite_core::hal::{DryInputs, HalError, HalResult};

/// The dry contacts, wired or not.
pub struct DryGpio {
    /// One input per contact, in order. `None` while the board's pin map
    /// is unknown.
    pins: Option<[PinDriver<'static, Input>; DRY_COUNT]>,
}

impl DryGpio {
    /// The rev D placeholder: no pins claimed, every read "not present".
    pub const fn unwired() -> Self {
        DryGpio { pins: None }
    }

    /// The real thing: four GPIOs, pulled up, closed contact to ground.
    pub fn wired(pins: [AnyIOPin<'static>; DRY_COUNT]) -> HalResult<Self> {
        let mut drivers: Vec<PinDriver<'static, Input>> = Vec::with_capacity(DRY_COUNT);
        for pin in pins {
            drivers.push(
                PinDriver::input(pin, Pull::Up).map_err(|e| HalError::Other(e.to_string()))?,
            );
        }
        let pins: [PinDriver<'static, Input>; DRY_COUNT] = drivers
            .try_into()
            .map_err(|_| HalError::OutOfRange)?;
        Ok(DryGpio { pins: Some(pins) })
    }

    /// True while the inputs are wired up.
    pub const fn is_wired(&self) -> bool {
        self.pins.is_some()
    }
}

impl DryInputs for DryGpio {
    fn read_dry(&mut self) -> HalResult<u8> {
        let Some(pins) = self.pins.as_mut() else {
            return Err(HalError::NotPresent);
        };
        let mut bits = 0u8;
        for (i, pin) in pins.iter().enumerate() {
            // Pulled up, so a closed contact reads low.
            if pin.is_low() {
                bits |= 1 << i;
            }
        }
        Ok(bits)
    }
}
