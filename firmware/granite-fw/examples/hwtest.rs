//! Devboard bring-up exercise for `granite_fw::hw`. See
//! `src/hw/hwtest.rs` for what it checks and how to flash it.
//!
//! Needs `--features hwtest`.

fn main() -> anyhow::Result<()> {
    granite_fw::hw::hwtest::run()
}
