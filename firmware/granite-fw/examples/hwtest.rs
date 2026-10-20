//! Devboard bring-up exercise for `granite_fw::hw`. See
//! `src/hw/hwtest.rs` for what it checks and how to flash it.
//!
//! Needs `--features hwtest`.

fn main() -> anyhow::Result<()> {
    // The bootloader's RTC watchdog stays on into the app (sdkconfig).
    granite_fw::platform::ota::rtc_watchdog_at_boot();
    granite_fw::hw::hwtest::run()
}
