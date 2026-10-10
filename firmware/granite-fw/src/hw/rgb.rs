//! WS2812 status mirror for a devboard: the `rgb-led` cargo feature.
//!
//! **This is not a path a controller board ever takes.** The controller
//! has one plain LED on GPIO1 (ADR 0001 component 13) and no addressable
//! LED anywhere; the feature is off by default and this module is not
//! compiled without it.
//!
//! What it is for: an Espressif ESP32-C6-DevKitC-1/DevKitM-1 carries a
//! single WS2812 on GPIO8, and on a bench it is the only LED that can say
//! *which* of the status patterns is running without counting blinks.
//! This module paints the pattern [`crate::hw::led`] is already running
//! in colour, on top of - never instead of - the GPIO1 LED:
//!
//! | pattern | colour | rate |
//! |---|---|---|
//! | `Heartbeat` | green, cyan-tinted while the broker session is up | breathing, 1 Hz |
//! | `NoLink` | amber | 0.25 Hz |
//! | `OtaPending` | blue | 4 Hz |
//! | `Press` | white | solid |
//! | `Fault` | red | 2 Hz |
//! | `Off` | dark | - |
//!
//! Brightness is capped at [`MAX`] of 255: the devboard LED sits a hand's
//! width from the operator's eyes and full scale is painful.
//!
//! # Which GPIO
//!
//! [`GPIO`] defaults to 8 and comes from `GRANITE_RGB_GPIO` at build
//! time, because the pin moved between devkit revisions (GPIO8 on the
//! C6-DevKitC-1 and DevKitM-1, GPIO2 on some C3 boards):
//!
//! ```sh
//! GRANITE_RGB_GPIO=2 cargo build --release --features rgb-led
//! ```
//!
//! The pin is not part of [`crate::hw::HwInit`]'s pin map: the number is
//! only known as a string at build time, so the driver takes it with
//! `AnyOutputPin::steal`. Nothing else in the firmware claims GPIO8, and
//! a number that collides with the pin map is a bench mistake that the
//! RMT or GPIO driver reports at startup.
//!
//! # RMT
//!
//! The bit stream is RMT, the same peripheral the DS18B20 probes use
//! ([`crate::hw::probes`], through the Espressif `onewire_bus`
//! component). Both sit on the **new** (ESP-IDF v5) RMT driver -
//! esp-idf-hal's `rmt-legacy` feature is off, and the crate refuses to
//! build with both that feature and the onewire component - so channels
//! are allocated by the IDF, not chosen by number: the C6 has two TX
//! channels and 1-wire takes one TX plus one RX. `hw::init` creates the
//! probe bus first (1 MHz) and this driver second (20 MHz); both ask for
//! the default clock source, so the group prescale the first channel
//! fixes (80 MHz / 1) divides cleanly for the second (80 MHz / 4) and
//! neither allocation conflicts. What the two channels do have to agree
//! on is channel memory: see [`MEM_BLOCK_SYMBOLS`].
//!
//! At 20 MHz one tick is 50 ns, so the WS2812 datasheet timing comes out
//! exact: 0 is 0.40 us high + 0.85 us low, 1 is 0.80 us high + 0.45 us
//! low, MSB first, green-red-blue. The >= 50 us reset gap needs no code:
//! frames are [`TICK`] apart.

use std::sync::atomic::{AtomicU16, Ordering};
use std::thread;
use std::time::Duration;

use esp_idf_hal::gpio::AnyOutputPin;
use esp_idf_hal::rmt::config::{MemoryAccess, TransmitConfig, TxChannelConfig};
use esp_idf_hal::rmt::encoder::{BytesEncoder, BytesEncoderConfig};
use esp_idf_hal::rmt::{PinState, Pulse, PulseTicks, Symbol, TxChannelDriver};
use esp_idf_hal::units::Hertz;
use granite_core::hal::LedPattern;
use granite_core::modbus_map::fault_bits;

use super::led::LedHandle;

/// The GPIO the WS2812 data line is on. `GRANITE_RGB_GPIO` overrides it
/// at build time; 8 is the Espressif C6 devkit convention.
pub const GPIO: u8 = match option_env!("GRANITE_RGB_GPIO") {
    Some(text) => parse_gpio(text),
    None => 8,
};

/// `GRANITE_RGB_GPIO` as a pin number, at compile time.
///
/// A value that is not a plain decimal pin number fails the build rather
/// than silently driving GPIO8.
const fn parse_gpio(text: &str) -> u8 {
    let bytes = text.as_bytes();
    if bytes.is_empty() {
        panic!("GRANITE_RGB_GPIO is empty; it takes a GPIO number, e.g. GRANITE_RGB_GPIO=8");
    }
    let mut value: u8 = 0;
    let mut i = 0;
    while i < bytes.len() {
        let digit = bytes[i];
        if digit < b'0' || digit > b'9' {
            panic!("GRANITE_RGB_GPIO takes a decimal GPIO number, e.g. GRANITE_RGB_GPIO=8");
        }
        value = match value.checked_mul(10) {
            Some(scaled) => scaled,
            None => panic!("GRANITE_RGB_GPIO is out of range for a GPIO number"),
        };
        value = match value.checked_add(digit - b'0') {
            Some(sum) => sum,
            None => panic!("GRANITE_RGB_GPIO is out of range for a GPIO number"),
        };
        i += 1;
    }
    value
}

/// Highest value any channel is driven to. Low on purpose.
pub const MAX: u8 = 40;
/// The green part of amber: red at full, green at under half.
const AMBER_GREEN: u8 = (MAX as u32 * 7 / 16) as u8;

/// RMT tick rate: one tick is 50 ns, which makes every WS2812 time exact.
const RMT_HZ: u32 = 20_000_000;

/// Symbols of channel memory to ask for: exactly one block.
///
/// This matters, and the default of 64 does not work here. A C6 RMT
/// channel owns 48 symbols (`SOC_RMT_MEM_WORDS_PER_CHANNEL`), and a
/// channel that asks for more takes the block of the next channel too.
/// The probe bus already holds one of the two TX channels with one
/// block, so a 64-symbol request finds no channel with two free blocks
/// and `rmt_new_tx_channel` fails with "no free tx channels". One frame
/// is 24 symbols, so one block is more than enough.
const MEM_BLOCK_SYMBOLS: usize = 48;

/// Frame interval. Also the reset gap between frames (>= 50 us) and the
/// resolution of the breathing ramp.
const TICK: Duration = Duration::from_millis(20);
/// Stack for the mirror thread: one 3-byte RMT frame and a log line.
const STACK_SIZE: usize = 4096;

/// Bits that mean "the hardware layer is not healthy" for [`for_faults`].
const FAULT_MASK: u16 = fault_bits::EXPANDER | fault_bits::SENSE;

/// The fault pattern, when the fault word says so.
///
/// This is the whole of the "fault" signal: the bits are the ones the
/// sense thread and `main` already set for Modbus input register 13, so
/// nothing new has to be detected to light the LED red. `main` calls it
/// where it picks the pattern, so both LEDs see the same decision.
pub fn for_faults(faults: &AtomicU16) -> Option<LedPattern> {
    if faults.load(Ordering::Relaxed) & FAULT_MASK != 0 {
        Some(LedPattern::Fault)
    } else {
        None
    }
}

/// Period and duty of a blinking pattern, in milliseconds. `None` means
/// the pattern is not a blink.
fn blink(pattern: LedPattern) -> Option<(u32, u32)> {
    match pattern {
        LedPattern::OtaPending => Some((250, 125)),
        LedPattern::NoLink => Some((4000, 2000)),
        LedPattern::Fault => Some((500, 250)),
        LedPattern::Off | LedPattern::Press | LedPattern::Heartbeat => None,
    }
}

/// The breathing ramp: 0 at the edges of the period, [`MAX`] in the
/// middle, squared so the eye reads it as linear.
fn breath(phase_ms: u32, period_ms: u32) -> u8 {
    let half = period_ms / 2;
    let up = if phase_ms < half {
        phase_ms
    } else {
        period_ms - phase_ms
    };
    let level = u32::from(MAX) * up * up / (half * half);
    level.min(u32::from(MAX)) as u8
}

/// What the LED should show, as (red, green, blue).
fn colour(pattern: LedPattern, phase_ms: u32, broker_up: bool) -> (u8, u8, u8) {
    let lit = match pattern {
        LedPattern::Off => (0, 0, 0),
        // A press is short and has to be unmistakable.
        LedPattern::Press => (MAX, MAX, MAX),
        LedPattern::Heartbeat => {
            let level = breath(phase_ms % 1000, 1000);
            if broker_up {
                // Towards cyan while the broker session is up, so a
                // working MQTT session is visible without the log.
                (0, level, level / 2)
            } else {
                (0, level, 0)
            }
        }
        LedPattern::NoLink => (MAX, AMBER_GREEN, 0),
        LedPattern::OtaPending => (0, 0, MAX),
        LedPattern::Fault => (MAX, 0, 0),
    };
    match blink(pattern) {
        Some((period, duty)) if phase_ms % period >= duty => (0, 0, 0),
        _ => lit,
    }
}

/// How a transition is logged, so the monitor says which pattern is on
/// the LED and in what colour.
fn describe(pattern: LedPattern) -> &'static str {
    match pattern {
        LedPattern::Off => "off (dark)",
        LedPattern::Press => "press (white, solid)",
        LedPattern::Heartbeat => "heartbeat (green, breathing 1 Hz)",
        LedPattern::OtaPending => "ota pending (blue, 4 Hz)",
        LedPattern::NoLink => "no link (amber, 0.25 Hz)",
        LedPattern::Fault => "fault (red, 2 Hz)",
    }
}

/// Claim the pin and one RMT TX channel, and build the WS2812 bit
/// encoder for it.
fn claim(gpio: u8) -> anyhow::Result<(TxChannelDriver<'static>, BytesEncoder)> {
    // SAFETY: GPIO8 is outside the controller's pin map
    // (docs/controller.md, "MCU") and nothing else in the firmware takes
    // it; the number is only known at build time as a string, which is
    // why it cannot come off `Peripherals`.
    let pin = unsafe { AnyOutputPin::steal(gpio) };
    let channel = TxChannelDriver::new(
        pin,
        &TxChannelConfig {
            resolution: Hertz(RMT_HZ),
            memory_access: MemoryAccess::Indirect {
                memory_block_symbols: MEM_BLOCK_SYMBOLS,
            },
            ..Default::default()
        },
    )?;

    let high = |ticks: u16| -> anyhow::Result<Pulse> {
        Ok(Pulse::new(PinState::High, PulseTicks::new(ticks)?))
    };
    let low = |ticks: u16| -> anyhow::Result<Pulse> {
        Ok(Pulse::new(PinState::Low, PulseTicks::new(ticks)?))
    };
    // Ticks are 50 ns. WS2812: 0 = 0.40 us high / 0.85 us low, 1 = 0.80
    // us high / 0.45 us low, most significant bit first.
    let encoder = BytesEncoder::with_config(&BytesEncoderConfig {
        bit0: Symbol::new(high(8)?, low(17)?),
        bit1: Symbol::new(high(16)?, low(9)?),
        msb_first: true,
        ..Default::default()
    })?;

    Ok((channel, encoder))
}

/// Start the mirror thread.
///
/// `led` is the live [`LedHandle`] the GPIO1 thread reads, so the mirror
/// follows the one arbitration point in `main` - including the press and
/// the fault - and can never disagree with the plain LED. `broker_up` is
/// called once per frame.
///
/// A WS2812 that cannot be driven is logged and dropped: it is a bench
/// convenience and must never be the reason a board does not come up.
pub fn start(led: LedHandle, broker_up: fn() -> bool) -> anyhow::Result<()> {
    thread::Builder::new()
        .name("granite-rgb".into())
        .stack_size(STACK_SIZE)
        .spawn(move || {
            // The channel and its encoder are built here, in the thread
            // that uses them: the IDF byte encoder is a raw handle and is
            // not `Send`.
            let (mut channel, encoder) = match claim(GPIO) {
                Ok(parts) => parts,
                Err(err) => {
                    log::error!("rgb: WS2812 on GPIO{GPIO} could not be claimed: {err:#}");
                    return;
                }
            };
            log::info!(
                "rgb: WS2812 status mirror on GPIO{GPIO} (RMT tx, {} MHz ticks, max {MAX}/255)",
                RMT_HZ / 1_000_000
            );
            let tx = TransmitConfig::default();
            // One encoder, reused for the life of the thread: frames are
            // 20 ms apart and 30 us long, so the queue is never full.
            let mut queue = channel.queue([encoder]);
            let mut shown: Option<(u8, u8, u8)> = None;
            let mut announced: Option<LedPattern> = None;
            let mut phase_ms: u32 = 0;
            loop {
                let pattern = led.pattern();
                if announced != Some(pattern) {
                    log::info!("rgb: {}", describe(pattern));
                    announced = Some(pattern);
                    // Start every blink and every breath on its rising
                    // edge, so a transition is visible immediately.
                    phase_ms = 0;
                }

                let want = colour(pattern, phase_ms, broker_up());
                if shown != Some(want) {
                    let (r, g, b) = want;
                    // GRB on the wire.
                    if let Err(err) = queue.push(&[g, r, b], &tx) {
                        log::error!("rgb: GPIO{GPIO} frame failed: {err}");
                        return;
                    }
                    shown = Some(want);
                }

                thread::sleep(TICK);
                phase_ms = phase_ms.wrapping_add(TICK.as_millis() as u32);
            }
        })?;

    Ok(())
}
