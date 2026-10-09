//! The external I2C bus on J8/J9 (GPIO6/GPIO7) at 100 kHz.
//!
//! docs/controller.md calls these pins "External I2C (software I2C, LP I2C
//! pins)" and leaves it as open item 3: "LP I2C from the HP core: if
//! ESP-IDF supports it, use it instead of software I2C for the external
//! bus (same pins 6/7)". It does. On the ESP32-C6, `LP_I2C_SDA_IOMUX_PAD`
//! is 6 and `LP_I2C_SCL_IOMUX_PAD` is 7 (ESP-IDF v5.5.5,
//! `components/hal/esp32c6/include/hal/i2c_ll.h`), and the v5 I2C master
//! driver addresses the LP instance as `i2c_port_t` `LP_I2C_NUM_0` from
//! the HP core, clocked from RTC_FAST. So this is the real LP I2C
//! peripheral, not a bit-bang, and the pin assignment is forced by the
//! IOMUX rather than chosen here.
//!
//! The bus runs at 100 kHz: docs/controller.md, "400 kHz does not meet the
//! rise time" with 4.7k pull-ups and ~200 pF of add-ons and cable.
//!
//! A fault here must never stop relay control, so nothing in this module
//! is on the internal bus's error path and no caller of [`ExtI2c::scan`]
//! holds the expander lock.

use esp_idf_hal::gpio::{Gpio6, Gpio7};
use esp_idf_sys::{
    EspError, i2c_port_t_LP_I2C_NUM_0, soc_periph_lp_i2c_clk_src_t_LP_I2C_SCLK_DEFAULT,
};

use super::i2c::{I2cBus, I2cDev};

/// External bus clock.
pub const EXT_BUS_HZ: u32 = 100_000;
/// SDA, fixed by the LP I2C IOMUX.
pub const SDA_GPIO: i32 = 6;
/// SCL, fixed by the LP I2C IOMUX.
pub const SCL_GPIO: i32 = 7;

/// The external bus.
pub struct ExtI2c {
    bus: I2cBus,
    /// The two pins, held so nothing else can claim them. The LP I2C IOMUX
    /// fixes which GPIOs the peripheral uses, so they are not passed to
    /// the driver.
    _pins: (Gpio6<'static>, Gpio7<'static>),
}

impl ExtI2c {
    /// Open the LP I2C port as an I2C master.
    ///
    /// The board has 4.7k pull-ups on both lines; the internal pull-ups are
    /// left off so a long cable's rise time is set by the known external
    /// resistors only.
    pub fn new(sda: Gpio6<'static>, scl: Gpio7<'static>) -> Result<Self, EspError> {
        let bus = I2cBus::new(
            i2c_port_t_LP_I2C_NUM_0 as i32,
            SDA_GPIO,
            SCL_GPIO,
            soc_periph_lp_i2c_clk_src_t_LP_I2C_SCLK_DEFAULT,
            false,
        )?;
        log::info!(
            "ext_i2c: LP_I2C_NUM_0 up on GPIO{SDA_GPIO}/GPIO{SCL_GPIO} at {} kHz",
            EXT_BUS_HZ / 1000
        );
        Ok(ExtI2c {
            bus,
            _pins: (sda, scl),
        })
    }

    /// Every 7-bit address on the external bus that answers.
    ///
    /// Expected inhabitants (docs/controller.md, "I2C buses"): add-on
    /// MCP23017 modules at 0x21-0x27 and the power board's eight LTC4282
    /// at 0x40-0x47.
    pub fn scan(&self) -> Vec<u8> {
        let found = self.bus.scan();
        if found.is_empty() {
            log::info!("ext_i2c: no device answered on the external bus");
        } else {
            let list: Vec<String> = found.iter().map(|a| format!("0x{a:02x}")).collect();
            log::info!("ext_i2c: {} device(s): {}", found.len(), list.join(" "));
        }
        found
    }

    /// Attach a device on the external bus, for the add-on drivers that
    /// come later (power board, Qwiic sensors).
    pub fn device(&self, addr: u8) -> Result<I2cDev, EspError> {
        self.bus.device(addr, EXT_BUS_HZ)
    }

    /// The underlying bus.
    pub fn bus(&self) -> &I2cBus {
        &self.bus
    }
}
