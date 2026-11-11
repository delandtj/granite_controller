//! Every pin of the PP-48 core-fd board, named by its net (both
//! builds, `link-mcp` and `link-fdcan`).
//!
//! `firmware/plane-core-fd-pins.toml` is generated from the schematic
//! generator (power/pp-48-fd/tools/sch/blocks/mcu.py) and is the truth
//! for this board: `build.rs` parses it and the [`core_pins!`] table
//! below and fails the build when a net moved to another port or
//! changed mode. Nothing else in the firmware may name a `P..`
//! peripheral: it takes the fields of [`CoreBoard`], so a pin
//! reassignment is one line here plus whatever the compiler then
//! complains about.
//!
//! One table for both CAN builds, because it is one board: the
//! MCP2518FD is fitted on the G070 build and not on the G0B1 build
//! (hardware ADR 0006 decision 6), and the difference is which fields
//! of [`CoreBoard`] the link module takes. The two builds hold opposite
//! halves of it:
//!
//! - `link-mcp` takes MCP_CS / SPI_SCK / SPI_MISO / SPI_MOSI / MCP_INT
//!   and leaves CAN_RX and CAN_TX untouched. That is not tidiness: the
//!   MCP2518FD's TXCAN drives the same ISO1044 TXD line as PB9, so PB9
//!   **must** stay in its reset state (analog, high-Z). Because the
//!   peripheral singletons are moved, not copied, a build that forgot
//!   and drove PB9 would have to say so in this file.
//! - `link-fdcan` takes CAN_RX and CAN_TX as FDCAN1_RX / FDCAN1_TX AF3
//!   and leaves the SPI pins and MCP_INT untouched (no MCP2518FD is
//!   fitted).
//!
//! PD2 (rev D's INT_OUT through the ISO7721) is free on this board:
//! EVENT frames replace the line (ADR 0005 decision 6), so it is not in
//! the generated map and not here.
//!
//! The type aliases carry the net's own name, in the net's own spelling
//! (`EXP_RSTN`, not `ExpRstn`), which is why the non-camel-case lint is
//! off for them: a pin map that does not read like the schematic is a
//! pin map nobody checks.

use embassy_stm32::Peri;
use embassy_stm32::peripherals;

/// Declare the board's nets: `NET : PORT = "mode";`.
///
/// `build.rs` reads the rows out of this file as text and checks them
/// against `firmware/plane-core-fd-pins.toml` (same port, same mode
/// string, and no net of the TOML that is neither listed here nor one
/// of the fixed debug / crystal / reset pins). The expansion is one
/// type alias per net, so `split` below cannot hand a net the wrong
/// port without a type error.
macro_rules! core_pins {
    ($( $(#[$doc:meta])* $net:ident : $port:ident = $mode:literal ; )*) => {
        $(
            $(#[$doc])*
            #[allow(non_camel_case_types)]
            pub type $net = peripherals::$port;
        )*
    };
}

core_pins! {
    /// ISO1044 RXD: the MCP2518FD's RXCAN hangs on the same net, so the
    /// G070 build leaves this pin in its reset state (analog) and only
    /// the G0B1 build drives it as FDCAN1_RX.
    CAN_RX : PB8 = "analog (G070) / FDCAN1_RX AF3 (G0B1)";
    /// ISO1044 TXD, driven by the MCP2518FD's TXCAN on the G070 build:
    /// that build must leave this pin analog and high-Z, or two drivers
    /// fight over the transceiver's input.
    CAN_TX : PB9 = "analog (G070) / FDCAN1_TX AF3 (G0B1)";

    /// MCP2518FD nCS, driven in software (the G0's SPI1 NSS output is
    /// not usable per transfer); 100k pull-up R33. G070 build.
    MCP_CS : PA4 = "output push-pull (SPI1_NSS by software)";
    /// SPI1 SCK to the MCP2518FD, at most 17 MHz at its 40 MHz SYSCLK.
    SPI_SCK : PA5 = "SPI1_SCK AF0";
    /// SPI1 MISO: the MCP2518FD's SDO.
    SPI_MISO : PA6 = "SPI1_MISO AF0";
    /// SPI1 MOSI: the MCP2518FD's SDI.
    SPI_MOSI : PA7 = "SPI1_MOSI AF0";
    /// MCP2518FD nINT, active low, 100k pull-up R34: an EXTI falling
    /// edge wakes the link task. G070 build.
    MCP_INT : PA0 = "input, EXTI falling";

    /// I2C2 SCL: the segment bus, TCA9548A 0x70 and one unit per
    /// channel.
    SCL2 : PA11 = "I2C2_SCL AF6";
    /// I2C2 SDA, same bus.
    SDA2 : PA12 = "I2C2_SDA AF6";
    /// High resets the mux and takes every channel off (Q6 inverts, so
    /// an MCU in reset leaves the mux running).
    MUX_RST : PB13 = "output push-pull";
    /// Low resets every unit's TCA6424A (bused, pull-ups on the units):
    /// CTRL 0x4D only.
    EXP_RSTN : PD0 = "output open-drain";

    /// Address straps, pulled up, DIP switch closed = 0; the plane's
    /// CAN node id is ADR\[2:0\] and its ADR 0003 address 0x30 + node.
    ADR0 : PA15 = "input";
    /// Address strap ADR1.
    ADR1 : PD1 = "input";
    /// Address strap ADR2.
    ADR2 : PA8 = "input";

    /// Core 48 V input through the 220k / 10k divider, ADC_IN15.
    VIN_SNS : PB11 = "ADC_IN15";
    /// Every edge retriggers the ~1 s press deadline monostable U301.
    /// Toggled only while a press runs (docs/press-deadline-line.md).
    PRESS_TRIG : PB12 = "output push-pull";
    /// Heartbeat LED, high = on.
    LED_RUN : PB14 = "output push-pull";
}

/// The core-fd board's pins, each in its net's type, plus the
/// peripherals behind them.
///
/// Built once by [`split`] from the peripheral singletons, then handed
/// out to the tasks. The link half differs per build and the rest does
/// not; a field a build does not take is a pin that stays in its reset
/// state, which for CAN_TX on the G070 build is a requirement and not
/// an accident (see the module docs).
pub struct CoreBoard {
    /// The segment bus peripheral (I2C2 master).
    pub i2c2: Peri<'static, peripherals::I2C2>,
    /// The ADC behind VIN_SNS.
    pub adc: Peri<'static, peripherals::ADC1>,
    /// The independent watchdog.
    pub iwdg: Peri<'static, peripherals::IWDG>,
    /// The flash controller, for the two CONFIG pages.
    pub flash: Peri<'static, peripherals::FLASH>,

    /// SPI1 to the MCP2518FD (`link-mcp`); no chip is fitted on the
    /// G0B1 build, so it stays here.
    #[cfg_attr(feature = "link-fdcan", allow(dead_code))]
    pub spi1: Peri<'static, peripherals::SPI1>,
    /// MCP2518FD nCS (`link-mcp`).
    #[cfg_attr(feature = "link-fdcan", allow(dead_code))]
    pub mcp_cs: Peri<'static, MCP_CS>,
    /// SPI1 SCK (`link-mcp`).
    #[cfg_attr(feature = "link-fdcan", allow(dead_code))]
    pub spi_sck: Peri<'static, SPI_SCK>,
    /// SPI1 MISO (`link-mcp`).
    #[cfg_attr(feature = "link-fdcan", allow(dead_code))]
    pub spi_miso: Peri<'static, SPI_MISO>,
    /// SPI1 MOSI (`link-mcp`).
    #[cfg_attr(feature = "link-fdcan", allow(dead_code))]
    pub spi_mosi: Peri<'static, SPI_MOSI>,
    /// MCP2518FD nINT (`link-mcp`).
    #[cfg_attr(feature = "link-fdcan", allow(dead_code))]
    pub mcp_int: Peri<'static, MCP_INT>,
    /// The EXTI line of MCP_INT, which is PA0's (`link-mcp`).
    #[cfg_attr(feature = "link-fdcan", allow(dead_code))]
    pub mcp_int_exti: Peri<'static, peripherals::EXTI0>,

    /// FDCAN1 (`link-fdcan`).
    #[cfg(feature = "link-fdcan")]
    pub fdcan1: Peri<'static, peripherals::FDCAN1>,
    /// ISO1044 RXD: FDCAN1_RX on the G0B1 build, untouched on the G070
    /// build. Held and not read there on purpose: see the module docs.
    #[cfg_attr(feature = "link-mcp", allow(dead_code))]
    pub can_rx: Peri<'static, CAN_RX>,
    /// ISO1044 TXD: FDCAN1_TX on the G0B1 build, untouched on the G070
    /// build, where the MCP2518FD's TXCAN drives the net and a second
    /// driver would fight it.
    #[cfg_attr(feature = "link-mcp", allow(dead_code))]
    pub can_tx: Peri<'static, CAN_TX>,

    /// I2C2 SCL (segment bus).
    pub scl2: Peri<'static, SCL2>,
    /// I2C2 SDA (segment bus).
    pub sda2: Peri<'static, SDA2>,
    /// MUX_RST.
    pub mux_rst: Peri<'static, MUX_RST>,
    /// EXP_RSTN.
    pub exp_rstn: Peri<'static, EXP_RSTN>,
    /// Address strap ADR0.
    pub adr0: Peri<'static, ADR0>,
    /// Address strap ADR1.
    pub adr1: Peri<'static, ADR1>,
    /// Address strap ADR2.
    pub adr2: Peri<'static, ADR2>,
    /// VIN_SNS.
    pub vin_sns: Peri<'static, VIN_SNS>,
    /// PRESS_TRIG.
    pub press_trig: Peri<'static, PRESS_TRIG>,
    /// LED_RUN.
    pub led_run: Peri<'static, LED_RUN>,
}

/// Name every pin of `p`.
///
/// Each assignment is checked against the alias above by the compiler,
/// and the alias against the generated pin map by `build.rs`.
pub fn split(p: embassy_stm32::Peripherals) -> CoreBoard {
    CoreBoard {
        i2c2: p.I2C2,
        adc: p.ADC1,
        iwdg: p.IWDG,
        flash: p.FLASH,
        spi1: p.SPI1,
        mcp_cs: p.PA4,
        spi_sck: p.PA5,
        spi_miso: p.PA6,
        spi_mosi: p.PA7,
        mcp_int: p.PA0,
        mcp_int_exti: p.EXTI0,
        #[cfg(feature = "link-fdcan")]
        fdcan1: p.FDCAN1,
        can_rx: p.PB8,
        can_tx: p.PB9,
        scl2: p.PA11,
        sda2: p.PA12,
        mux_rst: p.PB13,
        exp_rstn: p.PD0,
        adr0: p.PA15,
        adr1: p.PD1,
        adr2: p.PA8,
        vin_sns: p.PB11,
        press_trig: p.PB12,
        led_run: p.PB14,
    }
}
