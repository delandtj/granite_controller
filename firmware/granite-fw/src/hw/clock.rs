//! Monotonic time from `esp_timer_get_time`.
//!
//! `esp_timer_get_time` is microseconds since boot from the 64-bit systimer
//! and never goes backwards, which is exactly what
//! [`granite_core::hal::Clock`] asks for. SNTP moves the wall clock, not
//! this one.

use granite_core::hal::Clock;

/// Milliseconds since boot.
pub fn now_ms() -> u64 {
    // The value is non-negative by construction.
    (unsafe { esp_idf_sys::esp_timer_get_time() } as u64) / 1000
}

/// [`Clock`] over `esp_timer_get_time`. Zero-sized and cloneable, so every
/// component can keep its own.
#[derive(Debug, Clone, Copy, Default)]
pub struct EspClock;

impl Clock for EspClock {
    fn now_ms(&self) -> u64 {
        now_ms()
    }
}
