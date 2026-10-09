//! Status LED on GPIO1 (ADR 0001: 1 Hz healthy, 4 Hz OTA pending,
//! 0.25 Hz no link, solid while a press is in progress).
//!
//! One thread owns the pin and the timing; setting a pattern is a single
//! atomic store, so any task can do it without blocking. This replaces the
//! ad-hoc blink thread in `main.rs`: the platform side calls
//! [`start`] and keeps the returned [`LedHandle`].

use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};
use std::thread;
use std::time::Duration;

use esp_idf_hal::gpio::{Output, PinDriver};
use granite_core::hal::{HalResult, LedPattern, StatusLed};

/// Stack for the LED thread: one GPIO write and a sleep.
const STACK_SIZE: usize = 2048;
/// How often the thread re-reads the pattern, so a change shows up fast
/// even in the middle of a 2 s half period.
const TICK: Duration = Duration::from_millis(25);

/// Half period per pattern. A "1 Hz heartbeat" is one on-off cycle per
/// second, so the half period is 500 ms.
fn half_period(pattern: LedPattern) -> Option<Duration> {
    match pattern {
        LedPattern::Off | LedPattern::Press => None,
        LedPattern::Heartbeat => Some(Duration::from_millis(500)),
        LedPattern::OtaPending => Some(Duration::from_millis(125)),
        LedPattern::NoLink => Some(Duration::from_millis(2000)),
    }
}

fn encode(pattern: LedPattern) -> u8 {
    match pattern {
        LedPattern::Off => 0,
        LedPattern::Press => 1,
        LedPattern::Heartbeat => 2,
        LedPattern::OtaPending => 3,
        LedPattern::NoLink => 4,
    }
}

fn decode(raw: u8) -> LedPattern {
    match raw {
        1 => LedPattern::Press,
        2 => LedPattern::Heartbeat,
        3 => LedPattern::OtaPending,
        4 => LedPattern::NoLink,
        _ => LedPattern::Off,
    }
}

/// Cloneable [`StatusLed`]: every clone writes the same atomic.
#[derive(Clone)]
pub struct LedHandle {
    pattern: Arc<AtomicU8>,
}

impl LedHandle {
    /// The pattern currently running.
    pub fn pattern(&self) -> LedPattern {
        decode(self.pattern.load(Ordering::Acquire))
    }
}

impl StatusLed for LedHandle {
    fn set(&mut self, pattern: LedPattern) -> HalResult<()> {
        self.pattern.store(encode(pattern), Ordering::Release);
        Ok(())
    }
}

/// Start the LED thread on an already-created output driver for GPIO1.
pub fn start(mut pin: PinDriver<'static, Output>) -> anyhow::Result<LedHandle> {
    let pattern = Arc::new(AtomicU8::new(encode(LedPattern::Heartbeat)));
    let handle = LedHandle {
        pattern: pattern.clone(),
    };

    thread::Builder::new()
        .name("granite-led".into())
        .stack_size(STACK_SIZE)
        .spawn(move || {
            let mut on = false;
            let mut elapsed = Duration::ZERO;
            loop {
                let current = decode(pattern.load(Ordering::Acquire));
                let want = match half_period(current) {
                    None => {
                        // Solid or dark: start the next blinking pattern
                        // from a full half period.
                        elapsed = Duration::ZERO;
                        matches!(current, LedPattern::Press)
                    }
                    Some(half) => {
                        if elapsed >= half {
                            elapsed = Duration::ZERO;
                            !on
                        } else {
                            on
                        }
                    }
                };
                if want != on {
                    on = want;
                    if let Err(err) = pin.set_level(on.into()) {
                        log::error!("led: GPIO1 write failed: {err}");
                        return;
                    }
                }
                thread::sleep(TICK);
                elapsed += TICK;
            }
        })?;

    Ok(handle)
}
