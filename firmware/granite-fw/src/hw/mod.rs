//! Hardware layer (ADR 0001 components 1, 2, 3 backing, 4): MCP23017
//! expanders, relay presses with the hardware deadline, node and dry-contact
//! sense, DS18B20 probes, TMP1075, VIN ADC, status LED. Implements the
//! granite-core hal traits.
//!
//! # Calling it
//!
//! `main.rs` keeps its "EXP_nRESET_INT low before anything else"
//! guarantee: it creates the GPIO10 output driver itself, drives it low,
//! and hands that driver over here. Everything else this layer needs comes
//! straight off `Peripherals`.
//!
//! ```ignore
//! let peripherals = Peripherals::take()?;
//! let pins = peripherals.pins;
//!
//! let mut exp_reset = PinDriver::output(pins.gpio10)?;
//! exp_reset.set_low()?;                       // first GPIO action, unchanged
//!
//! let hw = granite_fw::hw::init(granite_fw::hw::HwInit {
//!     exp_reset,
//!     exp_int: pins.gpio0,
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
//! | `relays` | [`granite_core::hal::NodeSwitches`] |
//! | `sense` | [`granite_core::hal::NodeSense`] and [`granite_core::hal::DryInputs`] (cloneable) |
//! | `probes` | [`granite_core::hal::Probes`] |
//! | `board_temp` | [`granite_core::hal::BoardTemp`] |
//! | `vin` | [`granite_core::hal::BusVoltage`] |
//! | `led` | [`granite_core::hal::StatusLed`] (cloneable) |
//! | `clock` | [`granite_core::hal::Clock`] (copy) |
//!
//! `init` never fails because a device is missing: an absent expander,
//! probe or sensor is logged and shows up as an error from the trait call,
//! which is what keeps a board with a broken I2C bus reachable over the
//! network. The `Err` arm is reserved for peripherals the firmware cannot
//! run without at all (the I2C driver, the ADC unit, a thread).
//!
//! Three things happen as side effects of `init`, by design:
//!
//! - A `std::panic` hook is installed that drives GPIO10 low first
//!   ([`relays::install_panic_hook`]). `CONFIG_ESP_TASK_WDT_PANIC=y`, so
//!   the task watchdog goes through it too.
//! - The sense thread starts and keeps reading U15 (plus the 1 s expander
//!   readback from ADR component 2).
//! - The LED thread starts on GPIO1, replacing the blink loop in `main.rs`.

use std::sync::{Arc, Mutex};

use esp_idf_hal::adc::ADC1;
use esp_idf_hal::gpio::{
    Gpio0, Gpio1, Gpio2, Gpio3, Gpio5, Gpio6, Gpio7, Gpio16, Gpio17, Output, Pin, PinDriver, Pull,
};
use esp_idf_sys::i2c_port_t_I2C_NUM_0;
use esp_idf_sys::soc_periph_i2c_clk_src_t_I2C_CLK_SRC_DEFAULT;

pub mod clock;
pub mod expanders;
pub mod ext_i2c;
pub mod i2c;
pub mod led;
pub mod mcp23017;
pub mod probes;
pub mod relays;
pub mod sense;
pub mod tmp1075;
pub mod vin;

#[cfg(feature = "hwtest")]
pub mod hwtest;

/// Everything [`init`] needs. One field per pin or peripheral, so the call
/// site reads as a pin map and a missing pin is a compile error.
pub struct HwInit {
    /// GPIO10, EXP_nRESET_INT, already an output and already driven low by
    /// `main`.
    pub exp_reset: PinDriver<'static, Output>,
    /// GPIO0, EXP_INT: shared, open-drain, falling edge.
    pub exp_int: Gpio0<'static>,
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
    /// U14, the 16 relays, with the hardware press deadline.
    pub relays: relays::Relays,
    /// U15, the node power LEDs and the dry contacts.
    pub sense: sense::SenseHandle,
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
    /// The expanders, for anything that needs them directly (a bus scan, a
    /// forced reset pulse, the health page).
    pub expanders: expanders::Shared,
    /// The two internal-bus pins, held so nothing re-claims them. The
    /// ESP-IDF I2C driver owns the IOMUX for them.
    _int_pins: (Gpio2<'static>, Gpio3<'static>),
}

impl Hw {
    /// Release every relay and park U14 in reset. Safe to call at any time.
    pub fn release_all(&mut self) {
        use granite_core::hal::NodeSwitches as _;
        if let Err(err) = self.relays.release_all() {
            log::error!("hw: release_all failed: {err}");
        }
    }
}

/// Bring the hardware layer up. See the module docs for the contract.
pub fn init(cfg: HwInit) -> anyhow::Result<Hw> {
    relays::install_panic_hook();

    // Internal (HP) bus: 400 kHz, external 4.7k pull-ups on the board, so
    // the internal ones stay off.
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
        expanders::INT_BUS_HZ / 1000
    );

    let (exp, bring_up) = expanders::Expanders::new(int_bus.clone(), cfg.exp_reset)?;
    if let Err(err) = bring_up {
        log::error!("hw: expander bring-up failed: {err} (relays stay in reset)");
    }
    let expanders = Arc::new(Mutex::new(exp));

    let relays = relays::Relays::new(expanders.clone())?;

    // EXP_INT is open-drain and shared; the pull-up keeps it defined when
    // no expander drives it.
    let exp_int = PinDriver::input(cfg.exp_int, Pull::Up)?;
    let sense = sense::start(expanders.clone(), exp_int)?;

    let board_temp = tmp1075::Tmp1075::new(&int_bus, expanders::INT_BUS_HZ)?;
    let probes = probes::OneWireProbes::new(cfg.onewire, cfg.uart_rx)?;
    let vin = vin::Vin::new(cfg.adc1, cfg.vin, cfg.vin_trim)?;
    let led = led::start(PinDriver::output(cfg.status_led)?)?;
    let ext_i2c = ext_i2c::ExtI2c::new(cfg.ext_sda, cfg.ext_scl)?;

    Ok(Hw {
        relays,
        sense,
        probes,
        board_temp,
        vin,
        led,
        clock: clock::EspClock,
        ext_i2c,
        expanders,
        _int_pins: (cfg.int_sda, cfg.int_scl),
    })
}
