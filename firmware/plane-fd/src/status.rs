//! LED_RUN: the line that says what the plane is doing (ADR 0002
//! decision 8 and 11).
//!
//! Driven by one small task from an atomic the bus task and the link
//! task publish, which keeps the LED off the plane mutex: a poll cycle
//! that holds the plane for a few milliseconds cannot stall the
//! heartbeat.
//!
//! There is no INT_OUT on the FD board: an EVENT frame from the plane's
//! node replaces rev D's line (ADR 0005 decision 6, hardware ADR 0006
//! decision 7 frees PD2), and the schedule for it lives in
//! `plane_logic::can_link`, which reads the plane's `event_pending` bit.

use core::sync::atomic::{AtomicU8, Ordering};

use embassy_stm32::gpio::{Level, Output, Speed};
use embassy_time::{Duration, Ticker};
use plane_logic::{LedPattern, Plane};

use crate::pins;

/// What LED_RUN should be doing, as [`LedPattern`] in a byte.
static LED: AtomicU8 = AtomicU8::new(0);

/// Tick of the status task.
const TICK_MS: u64 = 10;

/// Publish what the plane wants on LED_RUN. Called by whichever task
/// last held the plane.
pub fn publish(plane: &Plane) {
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

/// Drive LED_RUN.
#[embassy_executor::task]
pub async fn task(led_run: embassy_stm32::Peri<'static, pins::LED_RUN>) {
    let mut led = Output::new(led_run, Level::Low, Speed::Low);
    let mut ticker = Ticker::every(Duration::from_millis(TICK_MS));
    let mut phase: u64 = 0;

    loop {
        led.set_level(Level::from(led_on(LED.load(Ordering::Relaxed), phase)));
        phase = phase.wrapping_add(1);
        ticker.next().await;
    }
}
