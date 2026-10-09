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
//! 2. The press deadline really drops EXP_nRESET_INT: it arms a 600 ms
//!    deadline by hand and logs the GPIO10 level before and after.
//! 3. The probe bus enumerates to an empty list on an empty bus.
//! 4. VIN, the board temperature and the external bus scan all answer
//!    without panicking.
//! 5. The LED patterns run (watch GPIO1, or the log).
//!
//! Remove the feature from the build to drop every line of it.

use std::thread;
use std::time::Duration;

use esp_idf_hal::gpio::PinDriver;
use esp_idf_hal::peripherals::Peripherals;
use esp_idf_sys::{
    gpio_get_level, gpio_mode_t_GPIO_MODE_INPUT_OUTPUT, gpio_set_direction, gpio_set_level,
};
use granite_core::hal::{
    BoardTemp as _, BusVoltage as _, DryInputs as _, LedPattern, NodeSense as _,
    NodeSwitches as _, Probes as _, StatusLed as _, Switch,
};

use super::{HwInit, expanders, init};

/// Run the exercise. Never returns.
pub fn run() -> anyhow::Result<()> {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();

    let peripherals = Peripherals::take()?;
    let pins = peripherals.pins;

    // Same first action as main.rs: EXP_nRESET_INT low before anything.
    let mut exp_reset = PinDriver::output(pins.gpio10)?;
    exp_reset.set_low()?;
    let mut header_reset = PinDriver::output(pins.gpio4)?;
    header_reset.set_low()?;

    log::info!("hwtest: calling hw::init");
    let mut hw = init(HwInit {
        exp_reset,
        exp_int: pins.gpio0,
        status_led: pins.gpio1,
        int_sda: pins.gpio2,
        int_scl: pins.gpio3,
        ext_sda: pins.gpio6,
        ext_scl: pins.gpio7,
        vin: pins.gpio5,
        onewire: pins.gpio16,
        uart_rx: pins.gpio17,
        adc1: peripherals.adc1,
        vin_trim: 1.0,
    })?;
    log::info!("hwtest: hw::init returned, no panic");

    log::info!("hwtest: expander bus scan {:02x?}", {
        let exp = expanders::lock(&hw.expanders);
        exp.bus().scan()
    });

    // 1. The absent expanders must be an error, not a panic.
    match hw.relays.assert(1, Switch::Pwr) {
        Ok(()) => log::warn!("hwtest: assert succeeded - is U14 actually present?"),
        Err(err) => log::info!("hwtest: assert on an absent U14 -> {err} (expected)"),
    }
    match hw.relays.release_all() {
        Ok(()) => log::info!("hwtest: release_all ok"),
        Err(err) => log::info!("hwtest: release_all -> {err}"),
    }
    match hw.sense.read_leds() {
        Ok(bits) => log::warn!("hwtest: leds 0x{bits:02x} - is U15 actually present?"),
        Err(err) => log::info!("hwtest: read_leds on an absent U15 -> {err} (expected)"),
    }
    match hw.sense.read_dry() {
        Ok(bits) => log::warn!("hwtest: dry 0x{bits:02x}"),
        Err(err) => log::info!("hwtest: read_dry on an absent U15 -> {err} (expected)"),
    }

    // 2. The deadline timer, measured on GPIO10 with the mutex out of the
    // way: raise the line by hand, arm a short deadline, watch it drop.
    deadline_check(&mut hw);

    // 3. Probes on an empty bus.
    match hw.probes.scan() {
        Ok(roms) => log::info!("hwtest: probe scan -> {} rom id(s) {roms:016x?}", roms.len()),
        Err(err) => log::error!("hwtest: probe scan -> {err}"),
    }

    // 4. Analogue and external bus.
    match hw.vin.read_mv() {
        Ok(mv) => log::info!("hwtest: vin {mv} mV (floating pin on a devboard)"),
        Err(err) => log::error!("hwtest: vin -> {err}"),
    }
    match hw.board_temp.read_centi_c() {
        Ok(c) => log::warn!("hwtest: board temp {c} centi-C - is the TMP1075 present?"),
        Err(err) => log::info!("hwtest: board temp -> {err} (expected without a TMP1075)"),
    }
    log::info!("hwtest: external bus scan {:02x?}", hw.ext_i2c.scan());

    // 5. LED patterns.
    for pattern in [
        LedPattern::Heartbeat,
        LedPattern::OtaPending,
        LedPattern::NoLink,
        LedPattern::Press,
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
        log::info!(
            "hwtest: idle, gpio10 = {}",
            unsafe { gpio_get_level(expanders::EXP_RESET_GPIO) }
        );
    }
}

/// Arm the press deadline and log the EXP_nRESET_INT level around it.
///
/// Two devboard details matter here. The line is raised with
/// `gpio_set_level` rather than through the expander code, because on a
/// board with no U14 the configuration step would fail and park the line
/// again before the timer could be seen. And `main`/`hw::init` configure
/// GPIO10 as a plain output, whose input register reads 0 whatever the pad
/// does, so the direction is widened to input-output first - otherwise
/// this check measures nothing.
fn deadline_check(hw: &mut super::Hw) {
    const DEADLINE_MS: u32 = 600;
    unsafe {
        gpio_set_direction(expanders::EXP_RESET_GPIO, gpio_mode_t_GPIO_MODE_INPUT_OUTPUT);
        gpio_set_level(expanders::EXP_RESET_GPIO, 1);
    }
    let before = unsafe { gpio_get_level(expanders::EXP_RESET_GPIO) };
    let armed_us = unsafe { esp_idf_sys::esp_timer_get_time() };
    if let Err(err) = hw.relays.arm_deadline(DEADLINE_MS) {
        log::error!("hwtest: arming the deadline failed: {err}");
        return;
    }
    log::info!("hwtest: deadline armed for {DEADLINE_MS} ms, gpio10 = {before} (expected 1)");
    for _ in 0..30 {
        thread::sleep(Duration::from_millis(50));
        if unsafe { gpio_get_level(expanders::EXP_RESET_GPIO) } == 0 {
            let after_us = unsafe { esp_idf_sys::esp_timer_get_time() };
            log::info!(
                "hwtest: deadline dropped gpio10 after {} ms",
                (after_us - armed_us) / 1000
            );
            // The next call must report the fault once.
            match hw.relays.release_all() {
                Ok(()) => log::error!("hwtest: release_all did not report the deadline fault"),
                Err(err) => log::info!("hwtest: fault reported as {err} (expected)"),
            }
            match hw.relays.release_all() {
                Ok(()) => log::info!("hwtest: fault cleared after one report (expected)"),
                Err(err) => log::info!("hwtest: second release_all -> {err}"),
            }
            return;
        }
    }
    log::error!("hwtest: deadline did NOT drop gpio10 within 1.5 s");
}
