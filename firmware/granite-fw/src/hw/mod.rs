//! Hardware layer (ADR 0001 components 1, 4 and ADR 0004 decision 9):
//! the plane link, the dry-contact inputs, DS18B20 probes, TMP1075, the
//! VIN ADC and the status LED. Implements the granite-core hal traits.
//!
//! The rev C node hardware is gone with the rev C node model (ADR 0004
//! decision 1): no MCP23017s, no photoMOS relays, no press engine, no
//! LED sense. A node is a slot on a plane, and the plane does all of
//! that itself.
//!
//! # The two links, and the pins they take
//!
//! One image, one link (ADR 0006, architecture component 7). The cargo
//! feature decides what GPIO0, GPIO2 and GPIO3 are and which plane link
//! [`Hw::plane`] holds:
//!
//! | pin | `link-i2c` (rev D, rev C) | `link-canfd` (controller-fd) |
//! |---|---|---|
//! | GPIO0 | EXP_INT, the planes' wired-OR interrupt | TCAN1044V STB, driven low for normal mode |
//! | GPIO2 | plane link SDA (HP I2C) | MCP2518FD nCS |
//! | GPIO3 | plane link SCL (HP I2C) | MCP2518FD nINT |
//! | GPIO6 / 7 | local I2C (LP): J9 | local I2C (LP): J9 **and the TMP1075** |
//!
//! That is the board, not a choice made here: controller-fd has no HP
//! I2C bus at all (its TMP1075 sits on the local bus with J9, schematic
//! `mcu.kicad_sch`: "Pull-ups: local I2C (TMP1075, J9) 4.7k"), while
//! rev D keeps the TMP1075 on the HP bus beside the planes. So an FD
//! build does not open `I2C_NUM_0`, and the board temperature comes off
//! the LP bus.
//!
//! The rev C bench board has the `link-i2c` pin map (TMP1075 U5 and the
//! U14 / U15 expanders on GPIO2 / GPIO3, EXP_INT on GPIO0) and no
//! MCP2518FD, so its image is the `rev-c` feature, an alias for
//! `link-i2c`. An FD image on rev C runs (with `link-bench`) but cannot
//! read the board temperature: it looks for the TMP1075 on GPIO6 / GPIO7.
//!
//! Two parts of this layer are placeholders until the rev D board
//! exists, and both say so loudly: [`dry::DryGpio::unwired`] (every read
//! "not present") and, in an I2C build, `plane_link::StubPlaneLink`
//! (every address NAKs). Nothing above them knows the difference,
//! because the core only sees the hal traits.
//!
//! # Calling it
//!
//! ```ignore
//! let peripherals = Peripherals::take()?;
//! let pins = peripherals.pins;
//! let spi = platform::net::spi_bus(peripherals.spi2, pins.gpio19, pins.gpio20, pins.gpio21)?;
//!
//! let hw = granite_fw::hw::init(granite_fw::hw::HwInit {
//!     // link-canfd: the MCP2518FD on the W5500's bus.
//!     spi,
//!     can_stb: pins.gpio0,
//!     can_cs: pins.gpio2,
//!     can_int: pins.gpio3,
//!     // link-i2c instead: plane_int: pins.gpio0, int_sda: pins.gpio2,
//!     // int_scl: pins.gpio3.
//!     status_led: pins.gpio1,
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
use esp_idf_hal::gpio::{Gpio0, Gpio1, Gpio2, Gpio3, Gpio5, Gpio6, Gpio7, Gpio16, Gpio17, PinDriver};
#[cfg(feature = "link-i2c")]
use esp_idf_hal::gpio::Pin;
#[cfg(feature = "link-canfd")]
use esp_idf_hal::spi::SpiDriver;
#[cfg(feature = "link-i2c")]
use esp_idf_sys::i2c_port_t_I2C_NUM_0;
#[cfg(feature = "link-i2c")]
use esp_idf_sys::soc_periph_i2c_clk_src_t_I2C_CLK_SRC_DEFAULT;

#[cfg(feature = "link-canfd")]
pub mod can_link;
pub mod clock;
pub mod dry;
pub mod ext_i2c;
pub mod i2c;
pub mod led;
#[cfg(feature = "link-i2c")]
pub mod plane_link;
pub mod probes;
pub mod tmp1075;
pub mod vin;

/// The plane link this image has (ADR 0006, architecture component 7).
#[cfg(feature = "link-canfd")]
pub type PlaneLinkImpl = can_link::CanLink;
/// The plane link this image has (ADR 0006, architecture component 7).
#[cfg(feature = "link-i2c")]
pub type PlaneLinkImpl = plane_link::StubPlaneLink;

/// The console's `link` lines, whichever link was built.
#[cfg(feature = "link-canfd")]
pub fn link_lines() -> Vec<String> {
    can_link::lines()
}

/// The console's `link` lines, whichever link was built.
#[cfg(feature = "link-i2c")]
pub fn link_lines() -> Vec<String> {
    vec![
        String::from("link      i2c (rev D), EXP_INT GPIO0"),
        String::from("state     stub: every plane address answers NAK until the rev D driver"),
    ]
}

#[cfg(feature = "hwtest")]
pub mod hwtest;

#[cfg(feature = "rgb-led")]
pub mod rgb;

/// Everything [`init`] needs. One field per pin or peripheral, so the call
/// site reads as a pin map and a missing pin is a compile error.
pub struct HwInit {
    /// GPIO0, EXP_INT: the planes' shared interrupt line, open-drain.
    #[cfg(feature = "link-i2c")]
    pub plane_int: Gpio0<'static>,
    /// GPIO2, internal bus SDA (the plane link on rev D).
    #[cfg(feature = "link-i2c")]
    pub int_sda: Gpio2<'static>,
    /// GPIO3, internal bus SCL (the plane link on rev D).
    #[cfg(feature = "link-i2c")]
    pub int_scl: Gpio3<'static>,
    /// The SPI bus the MCP2518FD shares with the W5500
    /// ([`crate::platform::net::spi_bus`]).
    #[cfg(feature = "link-canfd")]
    pub spi: &'static SpiDriver<'static>,
    /// GPIO0, the TCAN1044V's STB: low is normal mode.
    #[cfg(feature = "link-canfd")]
    pub can_stb: Gpio0<'static>,
    /// GPIO2, the MCP2518FD's chip select.
    #[cfg(feature = "link-canfd")]
    pub can_cs: Gpio2<'static>,
    /// GPIO3, the MCP2518FD's nINT.
    #[cfg(feature = "link-canfd")]
    pub can_int: Gpio3<'static>,
    /// GPIO1, status LED.
    pub status_led: Gpio1<'static>,
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
    /// The plane link: the CAN FD link in an FD build, the NAK stub in
    /// an I2C one ([`PlaneLinkImpl`]).
    ///
    /// `None` when the link hardware did not come up, which is not
    /// fatal for the rest of the layer: the probes, the LED and the
    /// network keep working and `main` runs the dispatcher on
    /// [`crate::platform::NoPlaneLink`]. It *is* fatal for the image,
    /// which then cannot confirm itself (ADR 0006, architecture
    /// component 7).
    pub plane: Option<PlaneLinkImpl>,
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
    /// The external bus on J9 (and, in an FD build, the TMP1075).
    pub ext_i2c: ext_i2c::ExtI2c,
    /// The two internal-bus pins, held so nothing re-claims them. The
    /// ESP-IDF I2C driver owns the IOMUX for them. An FD build has no
    /// internal bus: GPIO2 / GPIO3 are the CAN controller's.
    #[cfg(feature = "link-i2c")]
    _int_pins: (Gpio2<'static>, Gpio3<'static>),
}

/// Internal (HP) I2C bus speed \[Hz\].
///
/// ADR 0004 decision 9 puts the rev D plane link on this peripheral at
/// 100 kHz. Until the stub link becomes a driver the bus carries only
/// the TMP1075, which is happy either way. An FD build does not open
/// this bus at all.
#[cfg(feature = "link-i2c")]
pub const INT_BUS_HZ: u32 = 100_000;

/// Bring the hardware layer up. See the module docs for the contract.
pub fn init(cfg: HwInit) -> anyhow::Result<Hw> {
    let dry = dry::DryGpio::unwired();
    // The local (LP) bus on GPIO6 / GPIO7: J9, and on controller-fd the
    // TMP1075 as well.
    let ext_i2c = ext_i2c::ExtI2c::new(cfg.ext_sda, cfg.ext_scl)?;

    // The plane link, and the board temperature that shares a bus with
    // it on rev D.
    #[cfg(feature = "link-i2c")]
    let (plane, board_temp) = {
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
        // The planes' side of that bus, once there is a board to drive:
        // the stub only claims EXP_INT. There is no chip on this link to
        // answer, so the stub itself passes the link-hardware step of
        // the probation ladder (platform::ota::mark_link_hw).
        let plane = Some(plane_link::StubPlaneLink::new(cfg.plane_int));
        let board_temp = tmp1075::Tmp1075::new(&int_bus, INT_BUS_HZ)?;
        (plane, board_temp)
    };

    #[cfg(feature = "link-canfd")]
    let (plane, board_temp) = {
        // A link that does not come up is not fatal to the layer: the
        // console, the network and the sensors have to stay up on a
        // board whose CAN controller is dead. It is fatal to the image,
        // which then never marks itself valid and rolls back after
        // probation (platform::ota, "the ladder") - unless this is a
        // `link-bench` build.
        let plane = match can_link::CanLink::new(cfg.spi, cfg.can_cs, cfg.can_int, cfg.can_stb) {
            Ok(link) => {
                crate::platform::ota::mark_link_hw();
                Some(link)
            }
            Err(e) => {
                can_link::note_down();
                if cfg!(feature = "link-bench") {
                    log::error!(
                        "hw: the CAN FD plane link did not come up ({e:#}); link-bench build, \
                         so the image may still confirm itself"
                    );
                } else {
                    log::error!(
                        "hw: the CAN FD plane link did not come up ({e:#}); this image cannot \
                         confirm itself and the bootloader will roll it back"
                    );
                }
                None
            }
        };
        let board_temp = tmp1075::Tmp1075::new(ext_i2c.bus(), ext_i2c::EXT_BUS_HZ)?;
        (plane, board_temp)
    };

    let probes = probes::OneWireProbes::new(cfg.onewire, cfg.uart_rx)?;
    let vin = vin::Vin::new(cfg.adc1, cfg.vin, cfg.vin_trim)?;
    let led = led::start(PinDriver::output(cfg.status_led)?)?;

    Ok(Hw {
        plane,
        dry,
        probes,
        board_temp,
        vin,
        led,
        clock: clock::EspClock,
        ext_i2c,
        #[cfg(feature = "link-i2c")]
        _int_pins: (cfg.int_sda, cfg.int_scl),
    })
}
