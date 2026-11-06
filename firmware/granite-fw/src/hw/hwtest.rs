//! Bring-up exercise for the hardware layer, behind the `hwtest` feature.
//!
//! This is the bare-board check from ADR 0001, "Bring-up order on
//! hardware", for the parts this layer owns. It is deliberately *not*
//! wired into `main.rs`: build it as the `hwtest` example instead, which
//! takes `Peripherals` for itself.
//!
//! ```text
//! cd firmware/granite-fw
//! cargo build --release --features hwtest --example hwtest
//! espflash flash --monitor --port /dev/ttyACM0 \
//!     target/riscv32imac-esp-espidf/release/examples/hwtest
//! ```
//!
//! What it proves, in order:
//!
//! 1. `hw::init` on a board with nothing on I2C logs errors and returns,
//!    rather than panicking.
//! 2. The plane link answers: in an FD build (`link-canfd`) the
//!    MCP2518FD came up and a read of plane 0x30 times out on an empty
//!    bus; in an I2C build it is the stub, where every address NAKs and
//!    EXP_INT reads as "no events" (controller ADR 0004 decision 9 -
//!    the real HP I2C driver comes with the rev D board).
//! 3. The dry inputs are unwired and say so.
//! 4. The probe bus enumerates to an empty list on an empty bus.
//! 5. VIN, the board temperature and the external bus scan all answer
//!    without panicking.
//! 6. The LED patterns run (watch GPIO1, or the log).
//!
//! Remove the feature from the build to drop every line of it.

use std::thread;
use std::time::Duration;

use esp_idf_hal::peripherals::Peripherals;
use granite_core::hal::{
    BoardTemp as _, BusVoltage as _, DryInputs as _, LedPattern, PlaneLink as _, Probes as _,
    StatusLed as _,
};

use super::{HwInit, init};

/// Run the exercise. Never returns.
pub fn run() -> anyhow::Result<()> {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();

    let peripherals = Peripherals::take()?;
    let pins = peripherals.pins;

    // The SPI bus the W5500 and (in an FD build) the MCP2518FD share.
    // Built here even in an I2C build, so the pin map stays one list.
    let spi = crate::platform::net::spi_bus(
        peripherals.spi2,
        pins.gpio19,
        pins.gpio20,
        pins.gpio21,
    )?;
    let _ = spi;

    log::info!("hwtest: calling hw::init");
    let mut hw = init(HwInit {
        #[cfg(feature = "link-i2c")]
        plane_int: pins.gpio0,
        #[cfg(feature = "link-i2c")]
        int_sda: pins.gpio2,
        #[cfg(feature = "link-i2c")]
        int_scl: pins.gpio3,
        #[cfg(feature = "link-canfd")]
        spi,
        #[cfg(feature = "link-canfd")]
        can_stb: pins.gpio0,
        #[cfg(feature = "link-canfd")]
        can_cs: pins.gpio2,
        #[cfg(feature = "link-canfd")]
        can_int: pins.gpio3,
        status_led: pins.gpio1,
        ext_sda: pins.gpio6,
        ext_scl: pins.gpio7,
        vin: pins.gpio5,
        onewire: pins.gpio16,
        uart_rx: pins.gpio17,
        adc1: peripherals.adc1,
        vin_trim: 1.0,
    })?;
    log::info!("hwtest: hw::init returned, no panic");

    // 1 and 2. The plane link. On a bus with no plane on it a read is a
    // timeout (CAN) or a NAK (I2C stub); both are how the client reads
    // "no plane there".
    let mut answer = [0u8; 64];
    match hw.plane.as_mut() {
        Some(plane) => {
            match plane.write_read(0x30, &[0x01, 0xff], &mut answer) {
                Ok(n) => log::warn!("hwtest: plane 0x30 answered {n} bytes - is a plane wired?"),
                Err(err) => log::info!("hwtest: plane 0x30 -> {err} (expected with no plane)"),
            }
            log::info!(
                "hwtest: planes that raised events = {:#04x} (expected 0x00 with no plane)",
                plane.event_nodes()
            );
        }
        None => log::error!("hwtest: the plane link did not come up; see the lines above"),
    }
    for line in super::link_lines() {
        log::info!("hwtest: {line}");
    }

    // 3. The dry inputs.
    log::info!("hwtest: dry inputs wired = {}", hw.dry.is_wired());
    match hw.dry.read_dry() {
        Ok(bits) => log::info!("hwtest: dry 0x{bits:02x}"),
        Err(err) => log::info!("hwtest: dry -> {err} (expected while the pin map is unknown)"),
    }

    // 4. Probes on an empty bus.
    match hw.probes.scan() {
        Ok(roms) => log::info!("hwtest: probe scan -> {} rom id(s) {roms:016x?}", roms.len()),
        Err(err) => log::error!("hwtest: probe scan -> {err}"),
    }

    // 5. Analogue and external bus.
    match hw.vin.read_mv() {
        Ok(mv) => log::info!("hwtest: vin {mv} mV (floating pin on a devboard)"),
        Err(err) => log::error!("hwtest: vin -> {err}"),
    }
    match hw.board_temp.read_centi_c() {
        Ok(c) => log::warn!("hwtest: board temp {c} centi-C - is the TMP1075 present?"),
        Err(err) => log::info!("hwtest: board temp -> {err} (expected without a TMP1075)"),
    }
    log::info!("hwtest: external bus scan {:02x?}", hw.ext_i2c.scan());

    // 6. LED patterns.
    for pattern in [
        LedPattern::Heartbeat,
        LedPattern::OtaPending,
        LedPattern::NoLink,
        LedPattern::Busy,
        LedPattern::Off,
    ] {
        log::info!("hwtest: led pattern {pattern:?} for 3 s");
        hw.led.set(pattern)?;
        thread::sleep(Duration::from_secs(3));
    }
    hw.led.set(LedPattern::Heartbeat)?;

    log::info!("hwtest: done, idling");
    loop {
        thread::sleep(Duration::from_secs(10));
        for line in super::link_lines() {
            log::info!("hwtest: idle, {line}");
        }
    }
}
