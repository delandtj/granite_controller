//! Every pin of the PP-48 core board, named by its net.
//!
//! `firmware/plane-core-pins.toml` is generated from the schematic
//! generator (power/pp-48/tools/sch/blocks/mcu.py) and is the truth for
//! this board: `build.rs` parses it and the [`core_pins!`] table below
//! and fails the build when a net moved to another port or changed mode.
//! Nothing else in the firmware may name a `P..` peripheral: it takes
//! the fields of [`CoreBoard`], so a pin reassignment is one line here
//! plus whatever the compiler then complains about.
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
/// against `firmware/plane-core-pins.toml` (same port, same mode
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
    /// I2C1 SCL: the isolated link to the controller, through the
    /// ISO1640 (ADR 0003).
    SCL1 : PB8 = "I2C1_SCL AF6";
    /// I2C1 SDA, same link.
    SDA1 : PB9 = "I2C1_SDA AF6";
    /// High asserts EXP_INT on the controller, through the ISO7721,
    /// while unacknowledged events exist (ADR 0002 decision 8).
    INT_OUT : PD2 = "output push-pull";

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
    /// I2C address is 0x30 + ADR\[2:0\].
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

/// The core board's pins, each in its net's type, plus the peripherals
/// behind them.
///
/// Built once by [`split`] from the peripheral singletons, then handed
/// out to the tasks.
pub struct CoreBoard {
    /// The link peripheral (I2C1 slave).
    pub i2c1: Peri<'static, peripherals::I2C1>,
    /// The segment bus peripheral (I2C2 master).
    pub i2c2: Peri<'static, peripherals::I2C2>,
    /// The ADC behind VIN_SNS.
    pub adc: Peri<'static, peripherals::ADC1>,
    /// DMA for the link's slave transmit.
    pub link_tx_dma: Peri<'static, peripherals::DMA1_CH1>,
    /// DMA for the link's slave receive.
    pub link_rx_dma: Peri<'static, peripherals::DMA1_CH2>,
    /// The independent watchdog.
    pub iwdg: Peri<'static, peripherals::IWDG>,
    /// The flash controller, for the two CONFIG pages.
    pub flash: Peri<'static, peripherals::FLASH>,
    /// I2C1 SCL (link).
    pub scl1: Peri<'static, SCL1>,
    /// I2C1 SDA (link).
    pub sda1: Peri<'static, SDA1>,
    /// INT_OUT to the controller.
    pub int_out: Peri<'static, INT_OUT>,
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
        i2c1: p.I2C1,
        i2c2: p.I2C2,
        adc: p.ADC1,
        link_tx_dma: p.DMA1_CH1,
        link_rx_dma: p.DMA1_CH2,
        iwdg: p.IWDG,
        flash: p.FLASH,
        scl1: p.PB8,
        sda1: p.PB9,
        int_out: p.PD2,
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
