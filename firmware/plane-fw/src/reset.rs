//! Why the plane last reset (STATUS 0x1B), and the strapped address.

use embassy_stm32::gpio::{Input, Pull};
use embassy_stm32::pac;
use plane_logic::plane::reset_cause;

use crate::pins;

/// The plane's base I2C address; the straps add 0-7 (ADR 0003: 0x30 to
/// 0x37, one per plane on the link).
pub const BASE_ADDRESS: u8 = 0x30;

/// Read RCC_CSR and clear the flags, so the next boot reports its own
/// cause.
///
/// Order matters: a software reset and a watchdog reset also set
/// PINRSTF on this family, so the specific causes are checked first.
pub fn cause() -> u8 {
    let csr = pac::RCC.csr().read();
    let cause = if csr.sftrstf() {
        reset_cause::SOFTWARE
    } else if csr.iwdgrstf() || csr.wwdgrstf() {
        reset_cause::WATCHDOG
    } else if csr.lpwrrstf() {
        reset_cause::BROWNOUT
    } else if csr.pwrrstf() {
        reset_cause::POWER_ON
    } else if csr.pinrstf() {
        reset_cause::PIN
    } else {
        reset_cause::UNKNOWN
    };
    pac::RCC.csr().modify(|w| w.set_rmvf(true));
    pac::RCC.csr().modify(|w| w.set_rmvf(false));
    cause
}

/// Read the three address straps.
///
/// They are pulled up and a closed DIP switch pulls one to ground, so a
/// closed switch is a 0 (plane-core-pins.toml, ADR 0002). The pins are
/// read once and then left as inputs: the address of a plane never
/// changes while it runs, and the controller's seq memory and CRC are
/// what notice a board strapped twice.
pub fn address(
    adr0: embassy_stm32::Peri<'static, pins::ADR0>,
    adr1: embassy_stm32::Peri<'static, pins::ADR1>,
    adr2: embassy_stm32::Peri<'static, pins::ADR2>,
) -> u8 {
    let bit0 = Input::new(adr0, Pull::Up).is_high();
    let bit1 = Input::new(adr1, Pull::Up).is_high();
    let bit2 = Input::new(adr2, Pull::Up).is_high();
    BASE_ADDRESS | u8::from(bit0) | (u8::from(bit1) << 1) | (u8::from(bit2) << 2)
}

/// A serial for INFO 0x05: a CRC-32 of the STM32's 96-bit unique ID
/// (ADR 0003 tag 0x05, "STM32 UID hash").
pub fn serial() -> u32 {
    plane_logic::crc32::crc32(&embassy_stm32::uid::uid())
}
