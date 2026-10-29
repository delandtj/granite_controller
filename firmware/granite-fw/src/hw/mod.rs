//! Hardware layer (ADR 0001 components 1, 4 and ADR 0004 decision 9):
//! the plane link, the dry-contact inputs, DS18B20 probes, TMP1075, the
//! VIN ADC and the status LED. Implements the granite-core hal traits.
//!
//! The rev C node hardware is gone with the rev C node model (ADR 0004
//! decision 1): no MCP23017s, no photoMOS relays, no press engine, no
//! LED sense. A node is a slot on a plane, and the plane does all of
//! that itself.
//!
//! Two parts of this layer are placeholders until the rev D board
//! exists, and both say so loudly: [`plane_link::StubPlaneLink`] (every
//! address NAKs) and [`dry::DryGpio::unwired`] (every read "not
//! present"). Nothing above them knows the difference, because the core
//! only sees the hal traits.
//!
//! # Calling it
//!
//! ```ignore
//! let peripherals = Peripherals::take()?;
//! let pins = peripherals.pins;
//!
//! let hw = granite_fw::hw::init(granite_fw::hw::HwInit {
//!     plane_int: pins.gpio0,
//!     status_led: pins.gpio1,
//!     int_sda: pins.gpio2,
//!     int_scl: pins.gpio3,
//!     ext_sda: pins.gpio6,
//!     ext_scl: pins.gpio7,
//!     vin: pins.gpio5,
//!     onewire: pins.gpio16,
//!     uart_rx: pins.gpio17,
//!     adc1: peripherals.adc1,
//!     vin_trim: 1.0,
//! })?;
//! ```
//!
//! The returned [`Hw`] owns everything. Its fields are the HAL
//! implementations the core wants:
//!
//! | field | trait |
//! |---|---|
//! | `plane` | [`granite_core::hal::PlaneLink`] |
//! | `dry` | [`granite_core::hal::DryInputs`] |
//! | `probes` | [`granite_core::hal::Probes`] |
//! | `board_temp` | [`granite_core::hal::BoardTemp`] |
//! | `vin` | [`granite_core::hal::BusVoltage`] |
//! | `led` | [`granite_core::hal::StatusLed`] (cloneable) |
//! | `clock` | [`granite_core::hal::Clock`] (copy) |
//!
//! `init` never fails because a device is missing: an absent plane,
//! probe or sensor is logged and shows up as an error from the trait
//! call, which is what keeps a board with a broken I2C bus reachable
//! over the network. The `Err` arm is reserved for peripherals the
//! firmware cannot run without at all (the I2C driver, the ADC unit, a
//! thread).
//!
//! The LED thread starts on GPIO1 as a side effect of `init`, replacing
//! the blink loop in `main.rs`.
//!
//! One more thread exists only in a devboard build: with the `rgb-led`
//! feature `main` also starts `rgb`, which paints the same patterns on
//! the WS2812 of an ESP32-C6 devkit. The controller board has no RGB LED.

use esp_idf_hal::adc::ADC1;
use esp_idf_hal::gpio::{
    Gpio0, Gpio1, Gpio2, Gpio3, Gpio5, Gpio6, Gpio7, Gpio16, Gpio17, Pin, PinDriver,
};
use esp_idf_sys::i2c_port_t_I2C_NUM_0;
use esp_idf_sys::soc_periph_i2c_clk_src_t_I2C_CLK_SRC_DEFAULT;

pub mod clock;
pub mod dry;
pub mod ext_i2c;
pub mod i2c;
pub mod led;
pub mod plane_link;
pub mod probes;
pub mod tmp1075;
pub mod vin;

#[cfg(feature = "hwtest")]
pub mod hwtest;

#[cfg(feature = "rgb-led")]
pub mod rgb;

/// Everything [`init`] needs. One field per pin or peripheral, so the call
/// site reads as a pin map and a missing pin is a compile error.
pub struct HwInit {
    /// GPIO0, EXP_INT: the planes' shared interrupt line, open-drain.
    pub plane_int: Gpio0<'static>,
    /// GPIO1, status LED.
    pub status_led: Gpio1<'static>,
    /// GPIO2, internal bus SDA.
    pub int_sda: Gpio2<'static>,
    /// GPIO3, internal bus SCL.
    pub int_scl: Gpio3<'static>,
    /// GPIO6, external bus SDA (fixed by the LP I2C IOMUX).
    pub ext_sda: Gpio6<'static>,
    /// GPIO7, external bus SCL (fixed by the LP I2C IOMUX).
    pub ext_scl: Gpio7<'static>,
    /// GPIO5, VIN_SENSE.
    pub vin: Gpio5<'static>,
    /// GPIO16, the 1-wire probe bus.
    pub onewire: Gpio16<'static>,
    /// GPIO17, the UART RX half of the 1-wire net; parked as an input.
    pub uart_rx: Gpio17<'static>,
    /// ADC1, for VIN_SENSE.
    pub adc1: ADC1<'static>,
    /// Per-board VIN correction from config; 1.0 = nominal divider.
    pub vin_trim: f32,
}

/// The whole hardware layer.
pub struct Hw {
    /// The plane link (a stub until the rev D board exists).
    pub plane: plane_link::StubPlaneLink,
    /// The four dry contacts (unwired until the pin map exists).
    pub dry: dry::DryGpio,
    /// The DS18B20 bus.
    pub probes: probes::OneWireProbes,
    /// The TMP1075.
    pub board_temp: tmp1075::Tmp1075,
    /// VIN on ADC1.
    pub vin: vin::Vin,
    /// The status LED.
    pub led: led::LedHandle,
    /// Monotonic clock.
    pub clock: clock::EspClock,
    /// The external bus on J8/J9.
    pub ext_i2c: ext_i2c::ExtI2c,
    /// The two internal-bus pins, held so nothing re-claims them. The
    /// ESP-IDF I2C driver owns the IOMUX for them.
    _int_pins: (Gpio2<'static>, Gpio3<'static>),
}

/// Internal (HP) I2C bus speed \[Hz\].
///
/// ADR 0004 decision 9 puts the plane link on this peripheral at
/// 100 kHz. Until the stub link becomes a driver the bus carries only
/// the TMP1075, which is happy either way.
pub const INT_BUS_HZ: u32 = 100_000;

/// Bring the hardware layer up. See the module docs for the contract.
pub fn init(cfg: HwInit) -> anyhow::Result<Hw> {
    // Internal (HP) bus: external 4.7k pull-ups on the board, so the
    // internal ones stay off.
    let int_bus = i2c::I2cBus::new(
        i2c_port_t_I2C_NUM_0 as i32,
        cfg.int_sda.pin() as i32,
        cfg.int_scl.pin() as i32,
        soc_periph_i2c_clk_src_t_I2C_CLK_SRC_DEFAULT,
        false,
    )?;
    log::info!(
        "hw: internal bus on GPIO{}/GPIO{} at {} kHz",
        cfg.int_sda.pin(),
        cfg.int_scl.pin(),
        INT_BUS_HZ / 1000
    );

    // The planes' side of that bus, once there is a board to drive: the
    // stub only claims EXP_INT.
    let plane = plane_link::StubPlaneLink::new(cfg.plane_int);
    let dry = dry::DryGpio::unwired();

    let board_temp = tmp1075::Tmp1075::new(&int_bus, INT_BUS_HZ)?;
    let probes = probes::OneWireProbes::new(cfg.onewire, cfg.uart_rx)?;
    let vin = vin::Vin::new(cfg.adc1, cfg.vin, cfg.vin_trim)?;
    let led = led::start(PinDriver::output(cfg.status_led)?)?;
    let ext_i2c = ext_i2c::ExtI2c::new(cfg.ext_sda, cfg.ext_scl)?;

    Ok(Hw {
        plane,
        dry,
        probes,
        board_temp,
        vin,
        led,
        clock: clock::EspClock,
        ext_i2c,
        _int_pins: (cfg.int_sda, cfg.int_scl),
    })
}
