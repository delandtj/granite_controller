//! LED_RUN and INT_OUT: the two lines that say what the plane is doing
//! (ADR 0002 decision 8 and 11).
//!
//! Both are driven by one small task, from two atomics the bus task and
//! the link task publish. That keeps the lines off the plane mutex: a
//! poll cycle that holds the plane for a few milliseconds cannot stall
//! the heartbeat, and INT_OUT follows within one 10 ms tick of the
//! event ring changing - the controller's own poll is 1 Hz, so that is
//! three orders of magnitude of slack.

use core::sync::atomic::{AtomicBool, AtomicU8, Ordering};

use embassy_stm32::gpio::{Level, Output, Speed};
use embassy_time::{Duration, Ticker};
use plane_logic::{LedPattern, Plane};

use crate::pins;

/// Unacknowledged events exist, so EXP_INT must be asserted.
static EVENTS_PENDING: AtomicBool = AtomicBool::new(false);

/// What LED_RUN should be doing, as [`LedPattern`] in a byte.
static LED: AtomicU8 = AtomicU8::new(0);

/// Tick of the status task.
const TICK_MS: u64 = 10;

/// Publish what the plane wants on the two lines. Called by whichever
/// task last held the plane.
pub fn publish(plane: &Plane) {
    EVENTS_PENDING.store(plane.event_pending(), Ordering::Relaxed);
    let pattern = match plane.led_pattern() {
        LedPattern::Heartbeat => 0,
        LedPattern::Fast => 1,
        LedPattern::Double => 2,
    };
    LED.store(pattern, Ordering::Relaxed);
}

/// True while LED_RUN should be lit, `phase` ticks into the pattern.
fn led_on(pattern: u8, phase: u64) -> bool {
    match pattern {
        // Fast blink, 5 Hz: an update is being received.
        1 => phase % 20 < 10,
        // Double blink once a second: the image is unconfirmed.
        2 => matches!(phase % 100, 0..10 | 20..30),
        // 1 Hz heartbeat: the plane is running.
        _ => phase % 100 < 50,
    }
}

/// Drive LED_RUN and INT_OUT.
#[embassy_executor::task]
pub async fn task(
    led_run: embassy_stm32::Peri<'static, pins::LED_RUN>,
    int_out: embassy_stm32::Peri<'static, pins::INT_OUT>,
) {
    let mut led = Output::new(led_run, Level::Low, Speed::Low);
    // Low releases EXP_INT on the controller side (the ISO7721 drives
    // an open-drain stage there): the plane asserts nothing until it
    // has an event to report.
    let mut int = Output::new(int_out, Level::Low, Speed::Low);
    let mut ticker = Ticker::every(Duration::from_millis(TICK_MS));
    let mut phase: u64 = 0;

    loop {
        int.set_level(Level::from(EVENTS_PENDING.load(Ordering::Relaxed)));
        led.set_level(Level::from(led_on(LED.load(Ordering::Relaxed), phase)));
        phase = phase.wrapping_add(1);
        ticker.next().await;
    }
}
